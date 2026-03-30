// Custom NSView for rendering battlefield cards with Core Graphics.
//
// Uses thread-local storage to pass card data to the view's drawRect:
// implementation, avoiding complex objc2 define_class! ivars.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::define_class;
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadOnly};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBezierPath, NSColor, NSGraphicsContext, NSView,
};
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::app_state::CardRenderData;
use crate::style;
use crate::terminal_view::TerminalRenderState;
use exaterm_types::model::SessionId;
use exaterm_ui::layout::{CardRect, MARGIN, card_layout};

// ---------------------------------------------------------------------------
// Thread-local data bridge (main thread only)
// ---------------------------------------------------------------------------

thread_local! {
    static CARDS: RefCell<Vec<CardRenderData>> = const { RefCell::new(Vec::new()) };
    static SELECTED: Cell<Option<SessionId>> = const { Cell::new(None) };
    static RENDER: RefCell<Option<Rc<TerminalRenderState>>> = RefCell::new(None);
}

/// Push new card data for the next drawRect: cycle.
pub fn set_battlefield_data(
    cards: Vec<CardRenderData>,
    selected: Option<SessionId>,
    render: Rc<TerminalRenderState>,
) {
    CARDS.with(|c| *c.borrow_mut() = cards);
    SELECTED.with(|s| s.set(selected));
    RENDER.with(|r| *r.borrow_mut() = Some(render));
}

// ---------------------------------------------------------------------------
// BattlefieldView — custom NSView subclass
// ---------------------------------------------------------------------------

define_class!(
    // SAFETY: NSView has no special subclassing requirements beyond drawRect:.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "BattlefieldView"]
    pub struct BattlefieldView;

    unsafe impl NSObjectProtocol for BattlefieldView {}

    impl BattlefieldView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            draw_battlefield(self.frame());
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

// ---------------------------------------------------------------------------
// Drawing — reads thread-locals and paints via Core Graphics
// ---------------------------------------------------------------------------

fn draw_battlefield(frame: NSRect) {
    let cards = CARDS.with(|c| c.borrow().clone());
    let selected = SELECTED.with(|s| s.get());
    let render = RENDER.with(|r| r.borrow().clone());

    let render = match render {
        Some(r) => r,
        None => return,
    };

    if cards.is_empty() {
        // Draw fallback text.
        let text = NSString::from_str("Connecting to daemon...");
        let fallback = NSAttributedString::initWithString(NSAttributedString::alloc(), &text);
        fallback.drawAtPoint(NSPoint {
            x: MARGIN,
            y: MARGIN,
        });
        return;
    }

    let rects = card_layout(cards.len(), frame.size.width, frame.size.height);

    for (card, rect) in cards.iter().zip(rects.iter()) {
        let is_selected = selected == Some(card.id);
        draw_card(card, rect, is_selected, &render);
    }
}

fn draw_card(
    card: &CardRenderData,
    rect: &CardRect,
    is_selected: bool,
    render: &TerminalRenderState,
) {
    let ns_rect = NSRect::new(
        NSPoint {
            x: rect.x,
            y: rect.y,
        },
        NSSize {
            width: rect.w,
            height: rect.h,
        },
    );

    // Card background — rounded rect with per-status fill.
    let layer = style::card_layer_style(card.status);
    let corner = layer.corner_radius;
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect, corner, corner);

    render.card_bg(card.status).setFill();
    path.fill();

    // Card border.
    let bc = &layer.border_color;
    let border_color = NSColor::colorWithSRGBRed_green_blue_alpha(bc.r, bc.g, bc.b, bc.a);
    border_color.setStroke();
    path.setLineWidth(1.0);
    path.stroke();

    // Selected card highlight.
    if is_selected {
        render.selected_bg.setStroke();
        path.setLineWidth(2.0);
        path.stroke();
    }

    // Clip to card bounds so text cannot overflow the rounded rect.
    NSGraphicsContext::saveGraphicsState_class();
    path.addClip();

    // --- Text content ---
    let pad_x = 16.0;
    let pad_y = 14.0;
    let mut y_cursor = rect.y + pad_y;

    // Title.
    let title_str = build_simple_attr_string(&card.title, &render.title_font, &render.title_color);
    title_str.drawAtPoint(NSPoint {
        x: rect.x + pad_x,
        y: y_cursor,
    });
    y_cursor += 24.0;

    // Status chip pill.
    draw_status_chip(card, rect.x + pad_x, &mut y_cursor, render);
    y_cursor += 8.0;

    // Headline (synthesis).
    if !card.headline.is_empty() {
        let headline_str = build_simple_attr_string(
            &card.headline,
            &render.headline_font,
            &render.headline_color,
        );
        headline_str.drawAtPoint(NSPoint {
            x: rect.x + pad_x,
            y: y_cursor,
        });
        y_cursor += 20.0;
    }

    // Alert.
    if let Some(ref alert_text) = card.alert {
        let alert_line = format!("\u{26a0} {}", alert_text);
        let alert_str =
            build_simple_attr_string(&alert_line, &render.alert_font, &render.alert_color);
        alert_str.drawAtPoint(NSPoint {
            x: rect.x + pad_x,
            y: y_cursor,
        });
        y_cursor += 18.0;
    }

    // Recency.
    let recency_str =
        build_simple_attr_string(&card.recency, &render.recency_font, &render.recency_color);
    recency_str.drawAtPoint(NSPoint {
        x: rect.x + pad_x,
        y: y_cursor,
    });
    y_cursor += 18.0;

    // Scrollback lines.
    let max_y = rect.y + rect.h - 8.0;
    for line in &card.scrollback {
        if y_cursor > max_y {
            break;
        }
        let line_str =
            build_simple_attr_string(line, &render.scrollback_font, &render.scrollback_color);
        line_str.drawAtPoint(NSPoint {
            x: rect.x + pad_x,
            y: y_cursor,
        });
        y_cursor += 16.0;
    }

    NSGraphicsContext::restoreGraphicsState_class();
}

fn draw_status_chip(
    card: &CardRenderData,
    x: f64,
    y_cursor: &mut f64,
    render: &TerminalRenderState,
) {
    let label = card.status.label();
    let chip_text = render.chip_text_color(card.status);
    let chip_bg = render.chip_bg_color(card.status);

    // Approximate chip width from label length.
    let chip_w = label.len() as f64 * 7.0 + 16.0;
    let chip_h = 20.0;
    let chip_rect = NSRect::new(
        NSPoint { x, y: *y_cursor },
        NSSize {
            width: chip_w,
            height: chip_h,
        },
    );
    let chip_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(chip_rect, 8.0, 8.0);
    chip_bg.setFill();
    chip_path.fill();

    let chip_str = build_simple_attr_string(label, &render.status_font, chip_text);
    chip_str.drawAtPoint(NSPoint {
        x: x + 8.0,
        y: *y_cursor + 2.0,
    });
    *y_cursor += chip_h + 4.0;
}

/// Build an NSAttributedString with a single font + color.
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
