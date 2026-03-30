// Custom NSView for rendering battlefield cards with Core Graphics.
//
// Uses thread-local storage to pass card data to the view's drawRect:
// implementation, avoiding complex objc2 define_class! ivars.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use objc2::define_class;
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadOnly};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBezierPath, NSColor, NSEvent, NSGraphicsContext, NSView,
};
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::app_state::CardRenderData;
use crate::style;
use crate::terminal_view::TerminalRenderState;
use exaterm_types::model::SessionId;
use exaterm_ui::layout::{
    card_layout, card_terminal_slot_rect, focus_card_layout, CardRect, MARGIN,
};
use exaterm_ui::presentation::NudgeStateTone;

// ---------------------------------------------------------------------------
// Thread-local data bridge (main thread only)
// ---------------------------------------------------------------------------

thread_local! {
    static CARDS: RefCell<Vec<CardRenderData>> = const { RefCell::new(Vec::new()) };
    static SELECTED: Cell<Option<SessionId>> = const { Cell::new(None) };
    static RENDER: RefCell<Option<Rc<TerminalRenderState>>> = RefCell::new(None);
    static INTERACTION: RefCell<Option<Rc<dyn Fn(BattlefieldInteraction)>>> = RefCell::new(None);
    static EMBEDDED: RefCell<BTreeSet<SessionId>> = RefCell::new(BTreeSet::new());
    static FOCUSED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattlefieldInteraction {
    Select(SessionId),
    Focus(SessionId),
}

/// Push new card data for the next drawRect: cycle.
pub fn set_battlefield_data(
    cards: Vec<CardRenderData>,
    selected: Option<SessionId>,
    render: Rc<TerminalRenderState>,
    embedded: BTreeSet<SessionId>,
    focused: bool,
) {
    CARDS.with(|c| *c.borrow_mut() = cards);
    SELECTED.with(|s| s.set(selected));
    RENDER.with(|r| *r.borrow_mut() = Some(render));
    EMBEDDED.with(|slot| *slot.borrow_mut() = embedded);
    FOCUSED.with(|slot| slot.set(focused));
}

pub fn set_interaction_handler<F>(handler: F)
where
    F: Fn(BattlefieldInteraction) + 'static,
{
    INTERACTION.with(|slot| *slot.borrow_mut() = Some(Rc::new(handler)));
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

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            if let Some(session_id) = session_at_point(self.frame(), point) {
                let interaction = if event.clickCount() >= 2 {
                    BattlefieldInteraction::Focus(session_id)
                } else {
                    BattlefieldInteraction::Select(session_id)
                };
                INTERACTION.with(|slot| {
                    if let Some(handler) = slot.borrow().as_ref() {
                        handler(interaction);
                    }
                });
                self.setNeedsDisplay(true);
            }
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
    let embedded = EMBEDDED.with(|slot| slot.borrow().clone());
    let focused = FOCUSED.with(|slot| slot.get());

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

    let rects = layout_for_mode(cards.len(), frame, focused);

    for (card, rect) in cards.iter().zip(rects.iter()) {
        let is_selected = selected == Some(card.id);
        draw_card(
            card,
            rect,
            is_selected,
            embedded.contains(&card.id),
            focused,
            &render,
        );
    }
}

fn session_at_point(frame: NSRect, point: NSPoint) -> Option<SessionId> {
    let cards = CARDS.with(|c| c.borrow().clone());
    let focused = FOCUSED.with(|slot| slot.get());
    let rects = layout_for_mode(cards.len(), frame, focused);
    cards
        .iter()
        .zip(rects.iter())
        .find(|(_, rect)| point_in_rect(point, rect))
        .map(|(card, _)| card.id)
}

fn layout_for_mode(card_count: usize, frame: NSRect, focused: bool) -> Vec<CardRect> {
    if focused {
        focus_card_layout(card_count, frame.size.width, frame.size.height)
    } else {
        card_layout(card_count, frame.size.width, frame.size.height)
    }
}

fn point_in_rect(point: NSPoint, rect: &CardRect) -> bool {
    point.x >= rect.x
        && point.x <= rect.x + rect.w
        && point.y >= rect.y
        && point.y <= rect.y + rect.h
}

fn draw_card(
    card: &CardRenderData,
    rect: &CardRect,
    is_selected: bool,
    embedded_terminal: bool,
    focused_mode: bool,
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
    let content_width = rect.w - 32.0;

    // Title.
    let title_str = build_simple_attr_string(&card.title, &render.title_font, &render.title_color);
    title_str.drawAtPoint(NSPoint {
        x: rect.x + pad_x,
        y: y_cursor,
    });
    y_cursor += if focused_mode { 20.0 } else { 24.0 };

    draw_status_chip(
        &card.status_label,
        card.status,
        rect.x + pad_x,
        &mut y_cursor,
        render,
    );
    if let Some(attention) = card.attention {
        draw_attention_chip(
            attention.label,
            attention.fill,
            rect.x + rect.w - 132.0,
            &mut y_cursor,
            render,
        );
    }
    y_cursor += if focused_mode { 4.0 } else { 8.0 };

    // Headline (synthesis).
    let headline = if embedded_terminal && !card.headline.is_empty() {
        &card.headline
    } else if !card.combined_headline.is_empty() {
        &card.combined_headline
    } else {
        &card.headline
    };
    if !headline.is_empty() {
        let headline_str =
            build_simple_attr_string(headline, &render.headline_font, &render.headline_color);
        headline_str.drawInRect(NSRect::new(
            NSPoint {
                x: rect.x + pad_x,
                y: y_cursor,
            },
            NSSize {
                width: rect.w - 32.0,
                height: 42.0,
            },
        ));
        y_cursor += if focused_mode {
            28.0
        } else if embedded_terminal {
            24.0
        } else {
            38.0
        };
    }

    if let Some(ref detail) = card.detail {
        if !detail.is_empty() && !embedded_terminal {
            let detail_str =
                build_simple_attr_string(detail, &render.alert_font, &render.alert_color);
            detail_str.drawInRect(NSRect::new(
                NSPoint::new(rect.x + pad_x, y_cursor),
                NSSize::new(content_width, 36.0),
            ));
            y_cursor += 28.0;
        }
    }

    // Alert.
    if let Some(ref alert_text) = card.alert {
        if !alert_text.is_empty() {
            let alert_line = format!("! {}", alert_text);
            let alert_str =
                build_simple_attr_string(&alert_line, &render.alert_font, &render.alert_color);
            alert_str.drawInRect(NSRect::new(
                NSPoint::new(rect.x + pad_x, y_cursor),
                NSSize::new(content_width, 32.0),
            ));
            y_cursor += 24.0;
        }
    }

    // Recency.
    let recency_str =
        build_simple_attr_string(&card.recency, &render.recency_font, &render.recency_color);
    if !focused_mode {
        recency_str.drawAtPoint(NSPoint {
            x: rect.x + pad_x,
            y: y_cursor,
        });
        draw_nudge_chip(
            card.nudge_state.label,
            card.nudge_state.tone,
            rect.x + rect.w - 164.0,
            y_cursor - 2.0,
            render,
        );
        y_cursor += 26.0;
    }

    if embedded_terminal {
        let slot = card_terminal_slot_rect(rect);
        let slot_rect = NSRect::new(NSPoint::new(slot.x, slot.y), NSSize::new(slot.w, slot.h));
        let terminal_path =
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(slot_rect, 16.0, 16.0);
        let terminal_bg = NSColor::colorWithSRGBRed_green_blue_alpha(0.02, 0.04, 0.07, 0.92);
        terminal_bg.setFill();
        terminal_path.fill();
        let label =
            build_simple_attr_string("LIVE TERMINAL", &render.recency_font, &render.recency_color);
        label.drawAtPoint(NSPoint::new(slot.x + 10.0, slot.y + 8.0));
        if let Some(attention_bar) = card.attention_bar {
            draw_attention_condition_bar(
                rect.x + pad_x,
                (slot.y - 52.0).max(y_cursor),
                content_width,
                attention_bar.fill,
                card.attention_bar_reason.as_deref(),
                render,
            );
        }
        NSGraphicsContext::restoreGraphicsState_class();
        return;
    }

    let transcript_lines = transcript_lines(card);
    if !transcript_lines.is_empty() {
        let transcript_height = (transcript_lines.len() as f64 * 18.0) + 16.0;
        draw_transcript_block(
            rect.x + pad_x,
            y_cursor,
            content_width,
            transcript_height,
            &transcript_lines,
            render,
        );
        y_cursor += transcript_height + 10.0;
    }

    if let Some(attention_bar) = card.attention_bar {
        draw_attention_condition_bar(
            rect.x + pad_x,
            y_cursor,
            content_width,
            attention_bar.fill,
            card.attention_bar_reason.as_deref(),
            render,
        );
    }

    NSGraphicsContext::restoreGraphicsState_class();
}

fn draw_status_chip(
    label: &str,
    status: exaterm_ui::supervision::BattleCardStatus,
    x: f64,
    y_cursor: &mut f64,
    render: &TerminalRenderState,
) {
    let chip_text = render.chip_text_color(status);
    let chip_bg = render.chip_bg_color(status);

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

fn draw_attention_chip(
    label: &str,
    fill: usize,
    x: f64,
    y_cursor: &mut f64,
    render: &TerminalRenderState,
) {
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
    render.attention_chip_bg(fill).setFill();
    chip_path.fill();

    let chip_str =
        build_simple_attr_string(label, &render.status_font, &render.attention_chip_text);
    chip_str.drawAtPoint(NSPoint {
        x: x + 8.0,
        y: *y_cursor + 2.0,
    });
}

fn draw_nudge_chip(
    label: &str,
    tone: NudgeStateTone,
    x: f64,
    y: f64,
    render: &TerminalRenderState,
) {
    let chip_w = label.len() as f64 * 6.9 + 18.0;
    let chip_rect = NSRect::new(NSPoint::new(x, y), NSSize::new(chip_w, 22.0));
    let chip_path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(chip_rect, 10.0, 10.0);
    render.nudge_bg_color(tone).setFill();
    chip_path.fill();
    let chip_str =
        build_simple_attr_string(label, &render.status_font, render.nudge_text_color(tone));
    chip_str.drawAtPoint(NSPoint::new(x + 9.0, y + 3.0));
}

fn draw_transcript_block(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    lines: &[String],
    render: &TerminalRenderState,
) {
    let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 12.0, 12.0);
    render.transcript_bg.setFill();
    path.fill();
    render.transcript_border.setStroke();
    path.setLineWidth(1.0);
    path.stroke();

    let mut line_y = y + 10.0;
    for line in lines {
        let line_str =
            build_simple_attr_string(line, &render.scrollback_font, &render.scrollback_color);
        line_str.drawAtPoint(NSPoint::new(x + 10.0, line_y));
        line_y += 18.0;
    }
}

fn draw_attention_condition_bar(
    x: f64,
    y: f64,
    width: f64,
    fill: usize,
    reason: Option<&str>,
    render: &TerminalRenderState,
) {
    let caption = build_simple_attr_string(
        "ATTENTION CONDITION",
        &render.bar_caption_font,
        &render.bar_caption_color,
    );
    caption.drawAtPoint(NSPoint::new(x, y));

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
            let reason_str =
                build_simple_attr_string(reason, &render.bar_reason_font, &render.bar_reason_color);
            reason_str.drawInRect(NSRect::new(
                NSPoint::new(x, segment_y + 14.0),
                NSSize::new(width, 42.0),
            ));
        }
    }
}

fn transcript_lines(card: &CardRenderData) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(nudge) = card.last_nudge.as_deref() {
        if !nudge.is_empty() {
            lines.push(format!("Nudge: {nudge}"));
        }
    }
    lines.extend(card.scrollback.iter().take(4).cloned());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_rect_accepts_interior_point() {
        let rect = CardRect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 80.0,
        };
        assert!(point_in_rect(NSPoint::new(50.0, 60.0), &rect));
    }

    #[test]
    fn point_in_rect_rejects_exterior_point() {
        let rect = CardRect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 80.0,
        };
        assert!(!point_in_rect(NSPoint::new(5.0, 60.0), &rect));
        assert!(!point_in_rect(NSPoint::new(50.0, 105.0), &rect));
    }
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
