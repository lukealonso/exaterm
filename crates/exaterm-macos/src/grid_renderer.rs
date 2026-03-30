use crate::style::{NormalizedColor, resolve_cell_color};
use crate::terminal_state::GridSnapshot;

pub struct BgRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub color: NormalizedColor,
}

pub struct TextRun {
    pub x: f64,
    pub y: f64,
    pub chars: Vec<(char, NormalizedColor, bool)>,
}

pub struct CursorRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct DrawCommands {
    pub bg_rects: Vec<BgRect>,
    pub text_runs: Vec<TextRun>,
    pub cursor_rect: Option<CursorRect>,
}

pub fn build_draw_commands(
    snapshot: &GridSnapshot,
    palette: &[NormalizedColor; 256],
    cell_w: f64,
    cell_h: f64,
) -> DrawCommands {
    let mut bg_rects = Vec::new();
    let mut text_runs = Vec::new();

    for (row_idx, row) in snapshot.cells.iter().enumerate() {
        let y = row_idx as f64 * cell_h;
        let mut chars = Vec::with_capacity(row.len());

        for (col_idx, cell) in row.iter().enumerate() {
            let x = col_idx as f64 * cell_w;
            let bg = resolve_cell_color(&cell.bg, palette);
            bg_rects.push(BgRect {
                x,
                y,
                w: cell_w,
                h: cell_h,
                color: bg,
            });

            let fg = resolve_cell_color(&cell.fg, palette);
            chars.push((cell.character, fg, cell.bold));
        }

        text_runs.push(TextRun { x: 0.0, y, chars });
    }

    let cursor_rect = if snapshot.cursor_row < snapshot.rows && snapshot.cursor_col < snapshot.cols
    {
        Some(CursorRect {
            x: snapshot.cursor_col as f64 * cell_w,
            y: snapshot.cursor_row as f64 * cell_h,
            w: cell_w,
            h: cell_h,
        })
    } else {
        None
    };

    DrawCommands {
        bg_rects,
        text_runs,
        cursor_rect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_state::{CellColor, CellSnapshot, GridSnapshot};

    fn make_grid(rows: usize, cols: usize) -> GridSnapshot {
        GridSnapshot {
            rows,
            cols,
            cells: (0..rows)
                .map(|_| {
                    (0..cols)
                        .map(|_| CellSnapshot {
                            character: ' ',
                            fg: CellColor::Named(7),
                            bg: CellColor::Named(0),
                            bold: false,
                        })
                        .collect()
                })
                .collect(),
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    #[test]
    fn bg_rects_count_matches_grid() {
        let grid = make_grid(2, 3);
        let palette = crate::style::ansi_palette();
        let cmds = build_draw_commands(&grid, &palette, 8.0, 16.0);
        assert_eq!(cmds.bg_rects.len(), 6);
    }

    #[test]
    fn cursor_rect_position() {
        let mut grid = make_grid(3, 5);
        grid.cursor_row = 1;
        grid.cursor_col = 2;
        let palette = crate::style::ansi_palette();
        let cmds = build_draw_commands(&grid, &palette, 8.0, 16.0);
        let cursor = cmds.cursor_rect.expect("cursor should be Some");
        assert_eq!(cursor.x, 16.0);
        assert_eq!(cursor.y, 16.0);
        assert_eq!(cursor.w, 8.0);
        assert_eq!(cursor.h, 16.0);
    }

    #[test]
    fn bold_cell_in_text_run() {
        let mut grid = make_grid(1, 3);
        grid.cells[0][0].character = 'X';
        grid.cells[0][0].bold = true;
        let palette = crate::style::ansi_palette();
        let cmds = build_draw_commands(&grid, &palette, 8.0, 16.0);
        assert!(
            cmds.text_runs[0].chars[0].2,
            "expected bold flag to be true"
        );
    }

    #[test]
    fn text_run_per_row() {
        let grid = make_grid(3, 4);
        let palette = crate::style::ansi_palette();
        let cmds = build_draw_commands(&grid, &palette, 8.0, 16.0);
        assert_eq!(cmds.text_runs.len(), 3);
    }
}
