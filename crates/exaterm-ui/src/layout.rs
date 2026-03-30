const ESTIMATED_TERMINAL_CELL_WIDTH: i32 = 8;
const ESTIMATED_TERMINAL_CELL_HEIGHT: i32 = 18;
const MIN_EMBEDDED_TERMINAL_COLS: i32 = 80;
const MIN_EMBEDDED_TERMINAL_ROWS: i32 = 24;
const EMBEDDED_TERMINAL_CARD_CHROME_WIDTH: i32 = 72;
const EMBEDDED_TERMINAL_CARD_CHROME_HEIGHT: i32 = 168;
const EMBEDDED_TERMINAL_MIN_WIDTH: i32 = (ESTIMATED_TERMINAL_CELL_WIDTH
    * MIN_EMBEDDED_TERMINAL_COLS)
    + EMBEDDED_TERMINAL_CARD_CHROME_WIDTH;
const EMBEDDED_TERMINAL_MIN_HEIGHT: i32 = (ESTIMATED_TERMINAL_CELL_HEIGHT
    * MIN_EMBEDDED_TERMINAL_ROWS)
    + EMBEDDED_TERMINAL_CARD_CHROME_HEIGHT;

pub fn battlefield_columns(total: usize, available_width: i32, focused: bool) -> u32 {
    if total == 0 {
        return 0;
    }

    if available_width <= 0 {
        return (if focused || total <= 2 {
            total
        } else if total <= 4 {
            2
        } else if total <= 6 {
            3
        } else {
            4
        }) as u32;
    }

    (if focused {
        total
    } else if total == 1 {
        1
    } else if total == 2 {
        if (available_width / 2) >= EMBEDDED_TERMINAL_MIN_WIDTH {
            2
        } else {
            1
        }
    } else if total == 4 {
        2
    } else if total == 6 {
        3
    } else if total <= 4 {
        if available_width >= 1800 { total } else { 2 }
    } else if total == 5 {
        ((available_width as usize) / 420).clamp(3, 5)
    } else {
        ((available_width as usize) / 380).clamp(3, total.min(4))
    }) as u32
}

pub fn battlefield_can_embed_terminals(
    total: usize,
    columns: usize,
    available_width: i32,
    available_height: i32,
) -> bool {
    if total == 0 || columns == 0 {
        return false;
    }

    let tile_width =
        (available_width.max(0) - ((columns.saturating_sub(1)) as i32 * 12) - 24) / columns as i32;
    let rows = ((total as f32) / (columns as f32)).ceil() as i32;
    let tile_height = if rows > 0 {
        (available_height.max(0) - ((rows - 1) * 12) - 24) / rows
    } else {
        0
    };

    tile_width >= EMBEDDED_TERMINAL_MIN_WIDTH && tile_height >= EMBEDDED_TERMINAL_MIN_HEIGHT
}

pub fn visible_scrollback_line_capacity(height: i32) -> usize {
    const SCROLLBACK_VERTICAL_PADDING: i32 = 16;
    const SCROLLBACK_LINE_HEIGHT: i32 = 14;
    const SCROLLBACK_LINE_SPACING: i32 = 4;
    const MIN_SCROLLBACK_LINES: usize = 1;

    if height <= 0 {
        return 3;
    }

    let usable_height = (height - SCROLLBACK_VERTICAL_PADDING).max(SCROLLBACK_LINE_HEIGHT);
    let line_block = SCROLLBACK_LINE_HEIGHT + SCROLLBACK_LINE_SPACING;
    let lines = ((usable_height + SCROLLBACK_LINE_SPACING) / line_block).max(1);
    (lines as usize).max(MIN_SCROLLBACK_LINES)
}

// ---------------------------------------------------------------------------
// Card layout — pure math, fully testable without AppKit rendering
// ---------------------------------------------------------------------------

/// Card layout constants matching the shared theme.
pub const GAP: f64 = 12.0;
pub const MARGIN: f64 = 12.0;
pub const CARD_MIN_HEIGHT: f64 = 220.0;

/// A positioned card rectangle (origin + size) in the view coordinate space.
#[derive(Debug, Clone, PartialEq)]
pub struct CardRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Compute card positions for `card_count` cards within a view of `(view_w, view_h)`.
///
/// Uses `battlefield_columns()` for column count, then flows
/// cards left-to-right, top-to-bottom in a grid with `GAP` spacing and `MARGIN` insets.
pub fn card_layout(card_count: usize, view_w: f64, view_h: f64) -> Vec<CardRect> {
    if card_count == 0 {
        return Vec::new();
    }

    let cols = battlefield_columns(card_count, view_w as i32, false) as usize;
    let cols = cols.max(1);
    let rows = (card_count + cols - 1) / cols;

    let card_w = (view_w - MARGIN * 2.0 - GAP * (cols as f64 - 1.0)) / cols as f64;
    let card_h = if rows > 0 {
        let available_h = view_h - MARGIN * 2.0 - GAP * (rows as f64 - 1.0);
        (available_h / rows as f64).max(CARD_MIN_HEIGHT)
    } else {
        CARD_MIN_HEIGHT
    };

    let mut rects = Vec::with_capacity(card_count);
    for i in 0..card_count {
        let col = i % cols;
        let row = i / cols;
        let x = MARGIN + col as f64 * (card_w + GAP);
        let y = MARGIN + row as f64 * (card_h + GAP);
        rects.push(CardRect {
            x,
            y,
            w: card_w,
            h: card_h,
        });
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_terminal_interstitial_layout_collapses_to_scrollback() {
        assert!(!battlefield_can_embed_terminals(4, 2, 1500, 1100));
        assert!(!battlefield_can_embed_terminals(4, 2, 1600, 1150));
    }

    #[test]
    fn embedded_terminals_require_genuinely_roomy_battlefield() {
        assert!(battlefield_can_embed_terminals(1, 1, 1200, 900));
        assert!(battlefield_can_embed_terminals(2, 2, 1480, 900));
        assert!(battlefield_can_embed_terminals(4, 2, 1700, 1300));
    }

    #[test]
    fn column_policy_keeps_two_terminal_layout_side_by_side() {
        assert_eq!(battlefield_columns(2, 1480, false), 2);
        assert_eq!(battlefield_columns(2, 1000, false), 1);
    }

    #[test]
    fn card_layout_zero_cards() {
        assert!(card_layout(0, 1400.0, 900.0).is_empty());
    }

    #[test]
    fn card_layout_single_card() {
        let rects = card_layout(1, 1400.0, 900.0);
        assert_eq!(rects.len(), 1);
        // Single card should start at margin.
        assert!((rects[0].x - MARGIN).abs() < 0.01);
        assert!((rects[0].y - MARGIN).abs() < 0.01);
        // Full width minus margins.
        let expected_w = 1400.0 - MARGIN * 2.0;
        assert!((rects[0].w - expected_w).abs() < 0.01);
    }

    #[test]
    fn card_layout_two_cards_wide_window() {
        // At 1480 width, battlefield_columns(2, 1480, false) == 2
        let rects = card_layout(2, 1480.0, 900.0);
        assert_eq!(rects.len(), 2);
        // Both on the same row.
        assert!((rects[0].y - rects[1].y).abs() < 0.01);
        // Second card starts after first + gap.
        assert!(rects[1].x > rects[0].x + rects[0].w);
    }

    #[test]
    fn card_layout_two_cards_narrow_window() {
        // At 1000 width, battlefield_columns(2, 1000, false) == 1
        let rects = card_layout(2, 1000.0, 900.0);
        assert_eq!(rects.len(), 2);
        // Stacked vertically: second card below first.
        assert!(rects[1].y > rects[0].y);
    }

    #[test]
    fn card_layout_four_cards() {
        // battlefield_columns(4, 1400, false) == 2 → 2×2 grid
        let rects = card_layout(4, 1400.0, 900.0);
        assert_eq!(rects.len(), 4);
        // Row 0: cards 0, 1.
        assert!((rects[0].y - rects[1].y).abs() < 0.01);
        // Row 1: cards 2, 3.
        assert!((rects[2].y - rects[3].y).abs() < 0.01);
        // Second row is below first row.
        assert!(rects[2].y > rects[0].y);
        // All cards have equal width.
        assert!((rects[0].w - rects[1].w).abs() < 0.01);
        assert!((rects[0].w - rects[2].w).abs() < 0.01);
    }

    #[test]
    fn card_layout_respects_min_height() {
        // Very short window — cards should not be smaller than CARD_MIN_HEIGHT.
        let rects = card_layout(4, 1400.0, 100.0);
        for r in &rects {
            assert!(r.h >= CARD_MIN_HEIGHT);
        }
    }

    #[test]
    fn card_layout_cards_within_bounds() {
        let (w, h) = (1400.0, 900.0);
        let rects = card_layout(6, w, h);
        for r in &rects {
            assert!(r.x >= 0.0);
            assert!(r.y >= 0.0);
            assert!(r.x + r.w <= w + 0.01);
        }
    }
}
