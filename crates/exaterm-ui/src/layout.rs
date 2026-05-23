const ESTIMATED_TERMINAL_CELL_WIDTH: i32 = 8;

// ---------------------------------------------------------------------------
// Grid tiling — placement of cards into a column-homogeneous grid with
// automatic 2x1 span-to-fill for incomplete last rows.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CardPlacement {
    pub col: usize,
    pub row: usize,
    pub col_span: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GridTiling {
    pub columns: usize,
    pub placements: Vec<CardPlacement>,
}

fn is_tileable(columns: usize, n: usize) -> bool {
    if columns == 0 || n == 0 {
        return false;
    }
    let remainder = n % columns;
    remainder == 0 || columns <= 2 * remainder
}

/// Column count heuristic based on session count and available pixel width.
/// This is the "natural" preference before tileability adjustment.
fn natural_columns(total: usize, available_width: i32) -> usize {
    if total == 0 {
        return 0;
    }

    if available_width <= 0 {
        return if total <= 2 {
            total
        } else if total <= 4 {
            2
        } else if total <= 6 {
            3
        } else if total == 9 {
            3
        } else {
            4
        };
    }

    if total == 1 {
        1
    } else if total == 2 {
        if (available_width / 2) >= ESTIMATED_TERMINAL_CELL_WIDTH * 80 {
            2
        } else {
            1
        }
    } else if total == 4 {
        2
    } else if total == 6 {
        3
    } else if total == 9 {
        3
    } else if total <= 4 {
        if available_width >= 1800 {
            total
        } else {
            2
        }
    } else if total == 5 {
        ((available_width as usize) / 420).clamp(3, 5)
    } else {
        ((available_width as usize) / 380).clamp(3, total.min(4))
    }
}

/// Find the nearest tileable column count to `natural`, searching outward.
/// Prefers fewer columns (wider cards) when equidistant.
fn nearest_tileable_columns(natural: usize, n: usize) -> usize {
    if is_tileable(natural, n) {
        return natural;
    }
    for delta in 1..=n {
        let down = natural.wrapping_sub(delta);
        let up = natural + delta;
        if down >= 1 && down <= n && is_tileable(down, n) {
            return down;
        }
        if up <= n && is_tileable(up, n) {
            return up;
        }
    }
    1
}

fn build_placements(columns: usize, session_count: usize) -> Vec<CardPlacement> {
    if columns == 0 || session_count == 0 {
        return Vec::new();
    }
    let rows = (session_count + columns - 1) / columns;
    let last_row = rows - 1;
    let last_row_count = session_count - columns * last_row;
    let wide_in_last = columns - last_row_count;

    let mut placements = Vec::with_capacity(session_count);
    for row in 0..rows {
        if row < last_row {
            for col in 0..columns {
                placements.push(CardPlacement {
                    col,
                    row,
                    col_span: 1,
                });
            }
        } else {
            let mut col = 0;
            for i in 0..last_row_count {
                let span = if i < wide_in_last { 2 } else { 1 };
                placements.push(CardPlacement {
                    col,
                    row,
                    col_span: span,
                });
                col += span;
            }
        }
    }
    placements
}

pub fn compute_tiling(session_count: usize, available_width: i32) -> GridTiling {
    if session_count == 0 {
        return GridTiling {
            columns: 0,
            placements: Vec::new(),
        };
    }

    let natural = natural_columns(session_count, available_width);
    let columns = nearest_tileable_columns(natural.max(1), session_count);
    let placements = build_placements(columns, session_count);
    GridTiling {
        columns,
        placements,
    }
}

pub fn battlefield_columns(total: usize, available_width: i32) -> u32 {
    compute_tiling(total, available_width).columns as u32
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

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalSlotRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Compute card positions for `card_count` cards within a view of `(view_w, view_h)`.
///
/// Uses `compute_tiling()` for column count and span assignments, then positions
/// cards left-to-right, top-to-bottom with `GAP` spacing and `MARGIN` insets.
/// Wide (2x1) cards get double cell width plus one gap.
pub fn card_layout(card_count: usize, view_w: f64, view_h: f64) -> Vec<CardRect> {
    if card_count == 0 {
        return Vec::new();
    }

    let tiling = compute_tiling(card_count, view_w as i32);
    let cols = tiling.columns.max(1);
    let rows = tiling.placements.last().map_or(1, |p| p.row + 1);

    let cell_w = (view_w - MARGIN * 2.0 - GAP * (cols as f64 - 1.0)) / cols as f64;
    let card_h = if rows > 0 {
        let available_h = view_h - MARGIN * 2.0 - GAP * (rows as f64 - 1.0);
        (available_h / rows as f64).max(CARD_MIN_HEIGHT)
    } else {
        CARD_MIN_HEIGHT
    };

    tiling
        .placements
        .iter()
        .map(|p| {
            let card_w = cell_w * p.col_span as f64 + GAP * (p.col_span as f64 - 1.0);
            let x = MARGIN + p.col as f64 * (cell_w + GAP);
            let y = MARGIN + p.row as f64 * (card_h + GAP);
            CardRect {
                x,
                y,
                w: card_w,
                h: card_h,
            }
        })
        .collect()
}

pub fn card_terminal_slot_rect(card: &CardRect) -> TerminalSlotRect {
    const PADDING_X: f64 = 16.0;
    const TOP_CHROME: f64 = 104.0;
    const BOTTOM_CHROME: f64 = 18.0;
    TerminalSlotRect {
        x: card.x + PADDING_X,
        y: card.y + TOP_CHROME,
        w: (card.w - (PADDING_X * 2.0)).max(0.0),
        h: (card.h - TOP_CHROME - BOTTOM_CHROME).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Tileability.
    // -----------------------------------------------------------------------

    #[test]
    fn tileability_basics() {
        assert!(is_tileable(1, 1));
        assert!(is_tileable(2, 2));
        assert!(is_tileable(2, 4));
        assert!(is_tileable(2, 3)); // 3%2=1, 2<=2
        assert!(is_tileable(3, 5)); // 5%3=2, 3<=4
        assert!(!is_tileable(3, 7)); // 7%3=1, 3>2
        assert!(is_tileable(4, 7)); // 7%4=3, 4<=6
        assert!(!is_tileable(4, 5)); // 5%4=1, 4>2
        assert!(!is_tileable(3, 10)); // 10%3=1, 3>2
        assert!(is_tileable(4, 10)); // 10%4=2, 4<=4
    }

    #[test]
    fn nearest_tileable_adjusts_correctly() {
        // n=7, natural=3 → not tileable, down to 2 (tileable).
        assert_eq!(nearest_tileable_columns(3, 7), 2);
        // n=10, natural=3 → not tileable, down to 2 (tileable).
        assert_eq!(nearest_tileable_columns(3, 10), 2);
        // n=5, natural=4 → not tileable, down to 3 (tileable).
        assert_eq!(nearest_tileable_columns(4, 5), 3);
        // Already tileable — no change.
        assert_eq!(nearest_tileable_columns(2, 4), 2);
        assert_eq!(nearest_tileable_columns(3, 9), 3);
    }

    // -----------------------------------------------------------------------
    // compute_tiling: exhaustive n=1..12.
    // -----------------------------------------------------------------------

    fn assert_tiling_fills_grid(t: &GridTiling, n: usize) {
        assert_eq!(t.placements.len(), n);
        if n == 0 {
            return;
        }
        let cols = t.columns;
        let rows = t.placements.last().unwrap().row + 1;
        // Build occupancy grid to verify no overlaps and full coverage of each row.
        let mut grid = vec![vec![false; cols]; rows];
        for p in &t.placements {
            for c in p.col..p.col + p.col_span {
                assert!(
                    !grid[p.row][c],
                    "overlap at row={} col={} (n={n}, cols={cols})",
                    p.row, c
                );
                grid[p.row][c] = true;
            }
        }
        for (r, row) in grid.iter().enumerate() {
            assert!(
                row.iter().all(|&v| v),
                "gap in row {r} (n={n}, cols={cols})"
            );
        }
    }

    #[test]
    fn tiling_n1_through_n12_fill_without_gaps() {
        for n in 1..=12 {
            let t = compute_tiling(n, 1600);
            assert_tiling_fills_grid(&t, n);
        }
    }

    #[test]
    fn tiling_n3_uses_two_columns_with_wide_bottom() {
        let t = compute_tiling(3, 1400);
        assert_eq!(t.columns, 2);
        assert_eq!(t.placements[0].col_span, 1);
        assert_eq!(t.placements[1].col_span, 1);
        assert_eq!(t.placements[2].col_span, 2);
    }

    #[test]
    fn tiling_n5_at_moderate_width() {
        let t = compute_tiling(5, 1400);
        assert!(is_tileable(t.columns, 5));
        assert_tiling_fills_grid(&t, 5);
    }

    #[test]
    fn tiling_n7_avoids_three_columns() {
        // c=3 is not tileable for 7. Should pick c=2 or c=4.
        let t = compute_tiling(7, 1400);
        assert_ne!(t.columns, 3);
        assert_tiling_fills_grid(&t, 7);
    }

    #[test]
    fn tiling_perfect_grids_have_no_spanning() {
        for &(n, expected_cols) in &[(4, 2), (6, 3), (9, 3)] {
            let t = compute_tiling(n, 1600);
            assert_eq!(t.columns, expected_cols, "n={n}");
            assert!(
                t.placements.iter().all(|p| p.col_span == 1),
                "n={n} should have no spanning"
            );
        }
    }

    #[test]
    fn column_policy_keeps_two_terminal_layout_side_by_side() {
        assert_eq!(battlefield_columns(2, 1480), 2);
        assert_eq!(battlefield_columns(2, 1000), 1);
    }

    #[test]
    fn nine_terminal_layout_stays_three_by_three() {
        assert_eq!(battlefield_columns(9, 2200), 3);
        assert_eq!(battlefield_columns(9, 1400), 3);
        assert_eq!(battlefield_columns(9, -1), 3);
    }

    // -----------------------------------------------------------------------
    // card_layout.
    // -----------------------------------------------------------------------

    #[test]
    fn card_layout_zero_cards() {
        assert!(card_layout(0, 1400.0, 900.0).is_empty());
    }

    #[test]
    fn card_layout_single_card() {
        let rects = card_layout(1, 1400.0, 900.0);
        assert_eq!(rects.len(), 1);
        assert!((rects[0].x - MARGIN).abs() < 0.01);
        assert!((rects[0].y - MARGIN).abs() < 0.01);
        let expected_w = 1400.0 - MARGIN * 2.0;
        assert!((rects[0].w - expected_w).abs() < 0.01);
    }

    #[test]
    fn card_layout_two_cards_wide_window() {
        let rects = card_layout(2, 1480.0, 900.0);
        assert_eq!(rects.len(), 2);
        assert!((rects[0].y - rects[1].y).abs() < 0.01);
        assert!(rects[1].x > rects[0].x + rects[0].w);
    }

    #[test]
    fn card_layout_two_cards_narrow_window() {
        let rects = card_layout(2, 1000.0, 900.0);
        assert_eq!(rects.len(), 2);
        assert!(rects[1].y > rects[0].y);
    }

    #[test]
    fn card_layout_four_cards() {
        let rects = card_layout(4, 1400.0, 900.0);
        assert_eq!(rects.len(), 4);
        assert!((rects[0].y - rects[1].y).abs() < 0.01);
        assert!((rects[2].y - rects[3].y).abs() < 0.01);
        assert!(rects[2].y > rects[0].y);
        assert!((rects[0].w - rects[1].w).abs() < 0.01);
        assert!((rects[0].w - rects[2].w).abs() < 0.01);
    }

    #[test]
    fn card_layout_three_cards_wide_card_is_double_width() {
        let rects = card_layout(3, 1400.0, 900.0);
        assert_eq!(rects.len(), 3);
        // Cards 0,1 in row 0, card 2 (wide) in row 1.
        let cell_w = rects[0].w;
        let wide_w = rects[2].w;
        let expected_wide = cell_w * 2.0 + GAP;
        assert!(
            (wide_w - expected_wide).abs() < 0.01,
            "wide card width {wide_w} should be ~{expected_wide}"
        );
    }

    #[test]
    fn card_layout_respects_min_height() {
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

    #[test]
    fn card_terminal_slot_stays_within_card_bounds() {
        let card = CardRect {
            x: 12.0,
            y: 12.0,
            w: 600.0,
            h: 420.0,
        };
        let slot = card_terminal_slot_rect(&card);
        assert!(slot.x >= card.x);
        assert!(slot.y >= card.y);
        assert!(slot.x + slot.w <= card.x + card.w + 0.01);
        assert!(slot.y + slot.h <= card.y + card.h + 0.01);
    }
}
