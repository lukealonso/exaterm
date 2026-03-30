use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, Processor, Rgb, StdSyncHandler};

/// Snapshot of a single terminal cell for rendering.
#[derive(Debug, Clone)]
pub struct CellSnapshot {
    pub character: char,
    pub fg: CellColor,
    pub bg: CellColor,
    pub bold: bool,
}

/// Terminal color — either a named ANSI color or an RGB value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellColor {
    Named(u8),
    Rgb(u8, u8, u8),
}

/// Snapshot of the visible terminal grid for rendering.
pub struct GridSnapshot {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Vec<CellSnapshot>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

struct Listener;

impl EventListener for Listener {
    fn send_event(&self, _event: Event) {}
}

struct TermSize {
    screen_lines: usize,
    columns: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// Compute the terminal grid size (rows, cols) from a view's pixel dimensions
/// and per-cell dimensions. Returns at least 1 row and 1 column.
pub fn compute_grid_size(view_w: f64, view_h: f64, cell_w: f64, cell_h: f64) -> (u16, u16) {
    let cols = (view_w / cell_w).floor().max(1.0) as u16;
    let rows = (view_h / cell_h).floor().max(1.0) as u16;
    (rows, cols)
}

/// Wraps `alacritty_terminal::Term` with a simpler interface for our use case.
pub struct TerminalState {
    term: Term<Listener>,
    processor: Processor<StdSyncHandler>,
}

impl TerminalState {
    pub fn new(rows: u16, cols: u16) -> Self {
        let config = TermConfig::default();
        let dimensions = TermSize {
            screen_lines: rows as usize,
            columns: cols as usize,
        };
        let term = Term::new(config, &dimensions, Listener);
        Self {
            term,
            processor: Processor::<StdSyncHandler>::new(),
        }
    }

    /// Feed raw bytes from PTY output into the terminal emulator.
    pub fn write_output(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    /// Take a snapshot of the visible grid for rendering.
    pub fn grid_snapshot(&self) -> GridSnapshot {
        let grid = self.term.grid();
        let num_rows = grid.screen_lines();
        let num_cols = grid.columns();
        let mut cells = Vec::with_capacity(num_rows);

        for row_idx in 0..num_rows {
            let line = Line(row_idx as i32);
            let row = &grid[line];
            let mut row_cells = Vec::with_capacity(num_cols);
            for col_idx in 0..num_cols {
                let cell = &row[Column(col_idx)];
                row_cells.push(CellSnapshot {
                    character: cell.c,
                    fg: convert_color(cell.fg),
                    bg: convert_color(cell.bg),
                    bold: cell.flags.contains(Flags::BOLD),
                });
            }
            cells.push(row_cells);
        }

        let cursor = grid.cursor.point;
        GridSnapshot {
            rows: num_rows,
            cols: num_cols,
            cells,
            cursor_row: cursor.line.0.max(0) as usize,
            cursor_col: cursor.column.0,
        }
    }

    /// Whether the terminal is in application cursor key mode (DECCKM).
    pub fn app_cursor_mode(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// Current terminal dimensions.
    pub fn size(&self) -> (u16, u16) {
        (self.term.screen_lines() as u16, self.term.columns() as u16)
    }

    /// Resize the terminal.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.term.resize(TermSize {
            screen_lines: rows as usize,
            columns: cols as usize,
        });
    }

    /// Extract text from the visible grid between two positions (inclusive).
    ///
    /// Positions are given as (row, col). If the start is after the end, the
    /// result is empty. Columns outside the grid are clamped. Each row in the
    /// range is included with a trailing newline between rows; trailing
    /// whitespace within each row segment is preserved.
    pub fn text_in_range(&self, r0: usize, c0: usize, r1: usize, c1: usize) -> String {
        let grid = self.term.grid();
        let num_rows = grid.screen_lines();
        let num_cols = grid.columns();

        if r0 > r1 || r0 >= num_rows {
            return String::new();
        }
        let r1 = r1.min(num_rows - 1);

        let mut result = String::new();
        for row_idx in r0..=r1 {
            if row_idx > r0 {
                result.push('\n');
            }
            let line = Line(row_idx as i32);
            let row = &grid[line];
            let start_col = if row_idx == r0 { c0.min(num_cols) } else { 0 };
            let end_col = if row_idx == r1 {
                (c1 + 1).min(num_cols)
            } else {
                num_cols
            };
            if start_col >= end_col {
                continue;
            }
            for col_idx in start_col..end_col {
                let ch = row[Column(col_idx)].c;
                result.push(if ch == '\0' { ' ' } else { ch });
            }
        }
        result
    }
}

fn convert_color(color: Color) -> CellColor {
    match color {
        Color::Named(named) => {
            let idx = named as u16;
            match idx {
                // Special alacritty named colors above the 256 palette range.
                256 => CellColor::Rgb(229, 229, 229), // Foreground — light gray
                257 => CellColor::Rgb(0, 0, 0),       // Background — black
                258 => CellColor::Rgb(255, 255, 255), // Cursor — white
                // Dim colors (259-270) — map to their normal counterparts.
                259..=270 => CellColor::Named((idx - 259) as u8),
                i if i <= 255 => CellColor::Named(i as u8),
                _ => CellColor::Named(7), // fallback to white
            }
        }
        Color::Spec(Rgb { r, g, b }) => CellColor::Rgb(r, g, b),
        Color::Indexed(idx) => CellColor::Named(idx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_appears_in_grid() {
        let mut state = TerminalState::new(24, 80);
        state.write_output(b"hello\r\nworld");
        let snap = state.grid_snapshot();
        let row0: String = snap.cells[0]
            .iter()
            .map(|c| c.character)
            .collect::<String>();
        let row1: String = snap.cells[1]
            .iter()
            .map(|c| c.character)
            .collect::<String>();
        assert!(row0.starts_with("hello"), "row 0: {row0:?}");
        assert!(row1.starts_with("world"), "row 1: {row1:?}");
    }

    #[test]
    fn resize_updates_dimensions() {
        let mut state = TerminalState::new(24, 80);
        assert_eq!(state.size(), (24, 80));
        state.resize(40, 120);
        assert_eq!(state.size(), (40, 120));
    }

    #[test]
    fn compute_grid_size_basic() {
        let (rows, cols) = compute_grid_size(840.0, 340.0, 8.4, 17.0);
        assert_eq!(cols, 100);
        assert_eq!(rows, 20);
    }

    #[test]
    fn compute_grid_size_fractional_remainder() {
        // 100.0 / 8.4 = 11.9… → floor = 11
        let (rows, cols) = compute_grid_size(100.0, 50.0, 8.4, 17.0);
        assert_eq!(cols, 11);
        assert_eq!(rows, 2);
    }

    #[test]
    fn compute_grid_size_very_small_clamps_to_one() {
        let (rows, cols) = compute_grid_size(1.0, 1.0, 8.4, 17.0);
        assert_eq!(cols, 1);
        assert_eq!(rows, 1);
    }

    #[test]
    fn compute_grid_size_zero_view_clamps_to_one() {
        let (rows, cols) = compute_grid_size(0.0, 0.0, 8.4, 17.0);
        assert_eq!(cols, 1);
        assert_eq!(rows, 1);
    }

    #[test]
    fn text_in_range_single_row() {
        let mut state = TerminalState::new(24, 80);
        state.write_output(b"hello world");
        let text = state.text_in_range(0, 0, 0, 4);
        assert_eq!(text, "hello");
    }

    #[test]
    fn text_in_range_multi_row() {
        let mut state = TerminalState::new(24, 80);
        state.write_output(b"abc\r\ndef\r\nghi");
        let text = state.text_in_range(0, 0, 2, 2);
        // Row 0 full (80 cols), row 1 full, row 2 cols 0..2
        assert!(text.starts_with("abc"));
        assert!(text.contains("def"));
        assert!(text.contains("ghi"));
    }

    #[test]
    fn text_in_range_partial_columns() {
        let mut state = TerminalState::new(24, 80);
        state.write_output(b"hello world");
        let text = state.text_in_range(0, 6, 0, 10);
        assert_eq!(text, "world");
    }

    #[test]
    fn text_in_range_out_of_bounds_row() {
        let state = TerminalState::new(24, 80);
        let text = state.text_in_range(30, 0, 35, 10);
        assert!(text.is_empty());
    }

    #[test]
    fn text_in_range_start_after_end() {
        let mut state = TerminalState::new(24, 80);
        state.write_output(b"hello");
        let text = state.text_in_range(5, 0, 2, 10);
        assert!(text.is_empty());
    }

    #[test]
    fn ansi_bold_sets_attribute() {
        let mut state = TerminalState::new(24, 80);
        // ESC[1m = bold on, then "hi", then ESC[0m = reset
        state.write_output(b"\x1b[1mhi\x1b[0m");
        let snap = state.grid_snapshot();
        assert!(snap.cells[0][0].bold, "first char should be bold");
        assert_eq!(snap.cells[0][0].character, 'h');
    }
}
