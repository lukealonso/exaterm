use std::cell::RefCell;
use std::rc::Rc;

use objc2::define_class;
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadOnly};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBezierPath, NSColor, NSGraphicsContext, NSView,
};
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::app_state::FocusRenderData;
use crate::terminal_view::TerminalRenderState;
use exaterm_ui::layout::focus_terminal_slot_rect;
use exaterm_ui::theme::Color;

thread_local! {
    static FOCUS: RefCell<Option<FocusRenderData>> = const { RefCell::new(None) };
    static RENDER: RefCell<Option<Rc<TerminalRenderState>>> = RefCell::new(None);
}

pub fn set_focus_data(data: Option<FocusRenderData>, render: Rc<TerminalRenderState>) {
    FOCUS.with(|slot| *slot.borrow_mut() = data);
    RENDER.with(|slot| *slot.borrow_mut() = Some(render));
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "FocusView"]
    pub struct FocusView;

    unsafe impl NSObjectProtocol for FocusView {}

    impl FocusView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            draw_focus(self.frame());
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

fn draw_focus(frame: NSRect) {
    let Some(render) = RENDER.with(|slot| slot.borrow().clone()) else {
        return;
    };
    let Some(data) = FOCUS.with(|slot| slot.borrow().clone()) else {
        return;
    };

    let bg = NSColor::colorWithSRGBRed_green_blue_alpha(0.02, 0.04, 0.07, 1.0);
    bg.setFill();
    NSBezierPath::fillRect(frame);

    let card_rect = NSRect::new(
        NSPoint::new(12.0, 0.0),
        NSSize::new((frame.size.width - 24.0).max(0.0), frame.size.height),
    );
    let corner = 24.0;
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(card_rect, corner, corner);
    let border = NSColor::colorWithSRGBRed_green_blue_alpha(0.07, 0.18, 0.25, 1.0);
    render.card_bg(data.status).setFill();
    path.fill();
    border.setStroke();
    path.setLineWidth(1.0);
    path.stroke();

    NSGraphicsContext::saveGraphicsState_class();
    path.addClip();

    let pad_x = card_rect.origin.x + 18.0;
    let mut y = card_rect.origin.y + 16.0;
    build_simple_attr_string(&data.title, &render.title_font, &render.title_color)
        .drawAtPoint(NSPoint::new(pad_x, y));
    y += 28.0;

    draw_chip(
        &data.status_label,
        render.chip_text_color(data.status),
        render.chip_bg_color(data.status),
        &render.status_font,
        pad_x,
        y,
    );
    if let Some(attention) = data.attention {
        draw_chip(
            attention.label,
            &render.attention_chip_text,
            render.attention_chip_bg(attention.fill),
            &render.status_font,
            pad_x + 140.0,
            y,
        );
    }
    y += 34.0;

    if !data.combined_headline.is_empty() {
        build_simple_attr_string(
            &data.combined_headline,
            &render.headline_font,
            &render.headline_color,
        )
        .drawInRect(NSRect::new(
            NSPoint::new(pad_x, y),
            NSSize::new((card_rect.size.width - 36.0).max(0.0), 56.0),
        ));
        y += 64.0;
    }

    if let Some(attention_bar) = data.attention_bar {
        draw_attention_bar(
            pad_x,
            y,
            (card_rect.size.width - 36.0).max(0.0),
            attention_bar.fill,
            data.attention_bar_reason.as_deref(),
            &render,
        );
    }

    let slot = focus_terminal_slot_rect(frame.size.width as i32, frame.size.height as i32);
    let slot_rect = NSRect::new(
        NSPoint::new(slot.x, slot.y),
        NSSize::new(slot.w.max(0.0), slot.h.max(0.0)),
    );
    let terminal_bg = ns_color(Color {
        r: 4,
        g: 8,
        b: 12,
        a: 0.94,
    });
    terminal_bg.setFill();
    let terminal_path =
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(slot_rect, 18.0, 18.0);
    terminal_path.fill();

    NSGraphicsContext::restoreGraphicsState_class();
}

fn draw_chip(
    label: &str,
    text: &Retained<NSColor>,
    bg: &Retained<NSColor>,
    font: &objc2_app_kit::NSFont,
    x: f64,
    y: f64,
) {
    let chip_w = label.len() as f64 * 7.4 + 18.0;
    let chip_rect = NSRect::new(NSPoint::new(x, y), NSSize::new(chip_w, 22.0));
    let chip_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(chip_rect, 9.0, 9.0);
    bg.setFill();
    chip_path.fill();
    build_simple_attr_string(label, font, text).drawAtPoint(NSPoint::new(x + 9.0, y + 3.0));
}

fn draw_attention_bar(
    x: f64,
    y: f64,
    width: f64,
    fill: usize,
    reason: Option<&str>,
    render: &TerminalRenderState,
) {
    build_simple_attr_string(
        "ATTENTION CONDITION",
        &render.bar_caption_font,
        &render.bar_caption_color,
    )
    .drawAtPoint(NSPoint::new(x, y));

    let segment_y = y + 18.0;
    let gap = 4.0;
    let segment_width = ((width - (gap * 4.0)).max(0.0)) / 5.0;
    for index in 0..5 {
        let segment_x = x + (index as f64 * (segment_width + gap));
        let rect = NSRect::new(
            NSPoint::new(segment_x, segment_y),
            NSSize::new(segment_width, 8.0),
        );
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 4.0, 4.0);
        if index < fill {
            render.attention_bar_fill(fill).setFill();
        } else {
            render.bar_empty.setFill();
        }
        path.fill();
    }

    if let Some(reason) = reason {
        if !reason.is_empty() {
            build_simple_attr_string(reason, &render.bar_reason_font, &render.bar_reason_color)
                .drawInRect(NSRect::new(
                    NSPoint::new(x, segment_y + 14.0),
                    NSSize::new(width, 42.0),
                ));
        }
    }
}

fn ns_color(c: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
        f64::from(c.a),
    )
}

fn build_simple_attr_string(
    text: &str,
    font: &objc2_app_kit::NSFont,
    color: &Retained<NSColor>,
) -> Retained<NSAttributedString> {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSFontAttributeName, NSForegroundColorAttributeName};
    use objc2_foundation::{NSMutableAttributedString, NSRange};

    let ns_text = NSString::from_str(text);
    let result = NSMutableAttributedString::new();
    let plain = NSAttributedString::initWithString(NSAttributedString::alloc(), &ns_text);
    result.appendAttributedString(&plain);

    let range = NSRange::new(0, result.length());
    unsafe {
        let font_key: &objc2_foundation::NSAttributedStringKey = NSFontAttributeName;
        let fg_key: &objc2_foundation::NSAttributedStringKey = NSForegroundColorAttributeName;
        result.addAttribute_value_range(font_key, font as &AnyObject, range);
        result.addAttribute_value_range(fg_key, &**color as &AnyObject, range);
    }

    Retained::into_super(result)
}
