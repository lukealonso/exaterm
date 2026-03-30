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

#[cfg(test)]
mod tests {
    use super::{battlefield_can_embed_terminals, battlefield_columns};

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
}
