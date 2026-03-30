// Terminal rendering via NSAttributedString.
//
// Builds a colored attributed string from a GridSnapshot, batching consecutive
// cells with identical attributes into single runs for performance.

use std::collections::BTreeMap;

use crate::style::{self, NormalizedColor, ansi_palette};
use crate::terminal_state::GridSnapshot;

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBackgroundColorAttributeName, NSColor, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSTextField,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSMutableAttributedString, NSRange, NSString,
};

use exaterm_ui::supervision::BattleCardStatus;
use exaterm_ui::theme::{self as theme, Color};

/// Cached NSColor instances for the 256-entry ANSI palette.
pub(crate) struct CachedPalette {
    fg_colors: Vec<Retained<NSColor>>,
    bg_colors: Vec<Retained<NSColor>>,
}

impl CachedPalette {
    fn new(palette: &[NormalizedColor; 256]) -> Self {
        let fg_colors: Vec<_> = palette.iter().map(|c| ns_color(c)).collect();
        let bg_colors: Vec<_> = palette.iter().map(|c| ns_color(c)).collect();
        Self {
            fg_colors,
            bg_colors,
        }
    }
}

fn ns_color(c: &NormalizedColor) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

/// Key for batching: cells with identical attributes go into one run.
#[derive(Clone, Debug, PartialEq)]
struct RunKey {
    fg_idx: u16, // palette index or 0xFFFF for RGB
    bg_idx: u16,
    fg_rgb: (u8, u8, u8),
    bg_rgb: (u8, u8, u8),
    bold: bool,
    is_cursor: bool,
}

fn cell_run_key(cell: &crate::terminal_state::CellSnapshot, is_cursor: bool) -> RunKey {
    use crate::terminal_state::CellColor;
    let (fg_idx, fg_rgb) = match cell.fg {
        CellColor::Named(i) => (i as u16, (0, 0, 0)),
        CellColor::Rgb(r, g, b) => (0xFFFF, (r, g, b)),
    };
    let (bg_idx, bg_rgb) = match cell.bg {
        CellColor::Named(i) => (i as u16, (0, 0, 0)),
        CellColor::Rgb(r, g, b) => (0xFFFF, (r, g, b)),
    };
    RunKey {
        fg_idx,
        bg_idx,
        fg_rgb,
        bg_rgb,
        bold: cell.bold,
        is_cursor,
    }
}

/// Build an `NSAttributedString` from a terminal grid snapshot.
///
/// Batches consecutive cells with identical attributes into single runs
/// to minimize the number of attribute operations (critical for performance).
pub fn build_attributed_string(
    snapshot: &GridSnapshot,
    palette: &[NormalizedColor; 256],
    cached: &CachedPalette,
    font: &NSFont,
    bold_font: &NSFont,
    cursor_bg_color: &Retained<NSColor>,
) -> Retained<NSAttributedString> {
    let result = NSMutableAttributedString::init(NSMutableAttributedString::alloc());
    let mut text_buf = String::with_capacity(snapshot.rows * (snapshot.cols + 1));

    for (row_idx, row) in snapshot.cells.iter().enumerate() {
        if row_idx > 0 {
            text_buf.push('\n');
        }

        for cell in row.iter() {
            let ch = if cell.character == '\0' {
                ' '
            } else {
                cell.character
            };
            text_buf.push(ch);
        }
    }

    // Set the full text first, then apply attributes.
    let ns_text = NSString::from_str(&text_buf);
    let full_attr = NSAttributedString::initWithString(NSAttributedString::alloc(), &ns_text);
    result.appendAttributedString(&full_attr);

    // Now apply attributes per row, per run.
    let mut utf16_offset: usize = 0;
    for (row_idx, row) in snapshot.cells.iter().enumerate() {
        if row_idx > 0 {
            utf16_offset += 1; // newline is 1 UTF-16 code unit
        }

        let mut run_start_utf16 = utf16_offset;
        let mut run_key = if !row.is_empty() {
            let is_cursor = row_idx == snapshot.cursor_row && 0 == snapshot.cursor_col;
            cell_run_key(&row[0], is_cursor)
        } else {
            continue;
        };

        for (col_idx, cell) in row.iter().enumerate() {
            let is_cursor = row_idx == snapshot.cursor_row && col_idx == snapshot.cursor_col;
            let key = cell_run_key(cell, is_cursor);
            if key != run_key {
                // Flush previous run.
                apply_run_attributes(
                    &result,
                    run_start_utf16,
                    utf16_offset - run_start_utf16,
                    &run_key,
                    palette,
                    cached,
                    font,
                    bold_font,
                    cursor_bg_color,
                );
                run_start_utf16 = utf16_offset;
                run_key = key;
            }
            let ch = if cell.character == '\0' {
                ' '
            } else {
                cell.character
            };
            utf16_offset += ch.len_utf16();
        }
        // Flush last run.
        if !row.is_empty() {
            apply_run_attributes(
                &result,
                run_start_utf16,
                utf16_offset - run_start_utf16,
                &run_key,
                palette,
                cached,
                font,
                bold_font,
                cursor_bg_color,
            );
        }
    }

    Retained::into_super(result)
}

fn apply_run_attributes(
    result: &NSMutableAttributedString,
    offset: usize,
    length: usize,
    key: &RunKey,
    palette: &[NormalizedColor; 256],
    cached: &CachedPalette,
    font: &NSFont,
    bold_font: &NSFont,
    cursor_bg_color: &Retained<NSColor>,
) {
    if length == 0 {
        return;
    }
    let range = NSRange::new(offset, length);

    let ns_fg = if key.fg_idx < 256 {
        &cached.fg_colors[key.fg_idx as usize]
    } else {
        // RGB — create on the fly (rare).
        &ns_color(&NormalizedColor {
            r: key.fg_rgb.0 as f64 / 255.0,
            g: key.fg_rgb.1 as f64 / 255.0,
            b: key.fg_rgb.2 as f64 / 255.0,
            a: 1.0,
        })
    };

    let ns_bg = if key.is_cursor {
        cursor_bg_color
    } else if key.bg_idx < 256 {
        &cached.bg_colors[key.bg_idx as usize]
    } else {
        &ns_color(&NormalizedColor {
            r: key.bg_rgb.0 as f64 / 255.0,
            g: key.bg_rgb.1 as f64 / 255.0,
            b: key.bg_rgb.2 as f64 / 255.0,
            a: 1.0,
        })
    };

    let chosen_font = if key.bold { bold_font } else { font };

    unsafe {
        let fg_key: &NSAttributedStringKey = NSForegroundColorAttributeName;
        let bg_key: &NSAttributedStringKey = NSBackgroundColorAttributeName;
        let font_key: &NSAttributedStringKey = NSFontAttributeName;

        result.addAttribute_value_range(fg_key, &**ns_fg as &AnyObject, range);
        result.addAttribute_value_range(bg_key, &**ns_bg as &AnyObject, range);
        result.addAttribute_value_range(font_key, chosen_font as &AnyObject, range);
    }
}

/// Update an `NSTextField` with a colored attributed string built from the grid
/// snapshot. If no snapshot is available, sets a plain fallback string.
pub fn update_label_with_snapshot(
    label: &NSTextField,
    snapshot: Option<&GridSnapshot>,
    render: &TerminalRenderState,
    fallback: &str,
) {
    match snapshot {
        Some(snap) => {
            let attr_str = build_attributed_string(
                snap,
                &render.palette,
                &render.cached,
                &render.font,
                &render.bold_font,
                &render.cursor_bg,
            );
            label.setAttributedStringValue(&attr_str);
        }
        None => {
            let plain = NSString::from_str(fallback);
            label.setStringValue(&plain);
        }
    }
}

/// All statuses we iterate over to build per-status caches.
const ALL_STATUSES: &[BattleCardStatus] = &[
    BattleCardStatus::Idle,
    BattleCardStatus::Stopped,
    BattleCardStatus::Active,
    BattleCardStatus::Thinking,
    BattleCardStatus::Working,
    BattleCardStatus::Blocked,
    BattleCardStatus::Failed,
    BattleCardStatus::Complete,
    BattleCardStatus::Detached,
];

/// Convenience: create the ANSI palette, cached NSColor objects, and fonts.
///
/// All theme-derived fonts and colors are computed once and cached for use in
/// rendering functions. No hardcoded colors exist outside this constructor.
pub struct TerminalRenderState {
    // Terminal rendering (ANSI grid).
    pub palette: [NormalizedColor; 256],
    pub cached: CachedPalette,
    pub font: Retained<NSFont>,
    pub bold_font: Retained<NSFont>,
    pub cursor_bg: Retained<NSColor>,

    // Card UI fonts (from theme).
    pub title_font: Retained<NSFont>,
    pub status_font: Retained<NSFont>,
    pub recency_font: Retained<NSFont>,
    pub headline_font: Retained<NSFont>,
    pub alert_font: Retained<NSFont>,
    pub scrollback_font: Retained<NSFont>,

    // Card UI colors (from theme CSS values).
    pub title_color: Retained<NSColor>,
    pub headline_color: Retained<NSColor>,
    pub alert_color: Retained<NSColor>,
    pub recency_color: Retained<NSColor>,
    pub scrollback_color: Retained<NSColor>,
    pub selected_bg: Retained<NSColor>,

    // Per-status cached colors: discriminant -> (chip_text_color, chip_bg_color).
    pub status_chip_colors: BTreeMap<u8, (Retained<NSColor>, Retained<NSColor>)>,
    // Per-status card background: discriminant -> card background top color.
    pub card_bg_colors: BTreeMap<u8, Retained<NSColor>>,
}

/// Return a `u8` discriminant for a `BattleCardStatus` variant (used as map key).
fn status_discriminant(s: BattleCardStatus) -> u8 {
    match s {
        BattleCardStatus::Idle => 0,
        BattleCardStatus::Stopped => 1,
        BattleCardStatus::Active => 2,
        BattleCardStatus::Thinking => 3,
        BattleCardStatus::Working => 4,
        BattleCardStatus::Blocked => 5,
        BattleCardStatus::Failed => 6,
        BattleCardStatus::Complete => 7,
        BattleCardStatus::Detached => 8,
    }
}

impl TerminalRenderState {
    pub fn new(font_size: f64) -> Self {
        let palette = ansi_palette();
        let cached = CachedPalette::new(&palette);
        let font = NSFont::monospacedSystemFontOfSize_weight(font_size, 0.0);
        let bold_font = NSFont::monospacedSystemFontOfSize_weight(font_size, 0.7);
        let cursor_bg = NSColor::colorWithSRGBRed_green_blue_alpha(0.8, 0.8, 0.8, 0.7);

        // Card UI fonts from theme specs.
        let title_font = style::font_from_spec(&theme::card_title_font());
        let status_font = style::font_from_spec(&theme::card_status_font());
        let recency_font = style::font_from_spec(&theme::card_recency_font());
        let headline_font = style::font_from_spec(&theme::card_headline_font());
        let alert_font = style::font_from_spec(&theme::card_alert_font());
        let scrollback_font = style::font_from_spec(&theme::scrollback_line_font());

        // Card UI colors from theme CSS values.
        let title_color = style::color_to_nscolor(&Color {
            r: 248,
            g: 250,
            b: 252,
            a: 1.0,
        });
        let headline_color = style::color_to_nscolor(&Color {
            r: 248,
            g: 250,
            b: 252,
            a: 1.0,
        });
        let alert_color = style::color_to_nscolor(&Color {
            r: 202,
            g: 214,
            b: 227,
            a: 0.78,
        });
        let recency_color = style::color_to_nscolor(&Color {
            r: 188,
            g: 201,
            b: 216,
            a: 0.88,
        });
        let scrollback_color = style::color_to_nscolor(&Color {
            r: 202,
            g: 214,
            b: 227,
            a: 0.88,
        });
        let selected_bg = style::color_to_nscolor(&Color {
            r: 113,
            g: 197,
            b: 255,
            a: 0.15,
        });

        // Per-status cached colors.
        let mut status_chip_colors = BTreeMap::new();
        let mut card_bg_colors = BTreeMap::new();
        for &status in ALL_STATUSES {
            let disc = status_discriminant(status);
            let chip = theme::status_chip_theme(status);
            status_chip_colors.insert(
                disc,
                (
                    style::color_to_nscolor(&chip.text_color),
                    style::color_to_nscolor(&chip.background),
                ),
            );
            let layer = style::card_layer_style(status);
            card_bg_colors.insert(disc, ns_color(&layer.background_top));
        }

        Self {
            palette,
            cached,
            font,
            bold_font,
            cursor_bg,
            title_font,
            status_font,
            recency_font,
            headline_font,
            alert_font,
            scrollback_font,
            title_color,
            headline_color,
            alert_color,
            recency_color,
            scrollback_color,
            selected_bg,
            status_chip_colors,
            card_bg_colors,
        }
    }

    /// Look up the cached chip text color for a given status.
    pub fn chip_text_color(&self, status: BattleCardStatus) -> &Retained<NSColor> {
        &self.status_chip_colors[&status_discriminant(status)].0
    }

    /// Look up the cached chip background color for a given status.
    pub fn chip_bg_color(&self, status: BattleCardStatus) -> &Retained<NSColor> {
        &self.status_chip_colors[&status_discriminant(status)].1
    }

    /// Look up the cached card background (top gradient) color for a given status.
    pub fn card_bg(&self, status: BattleCardStatus) -> &Retained<NSColor> {
        &self.card_bg_colors[&status_discriminant(status)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_state::{CellColor, CellSnapshot, GridSnapshot};

    fn make_snapshot(rows: usize, cols: usize) -> GridSnapshot {
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
    fn ns_color_roundtrip() {
        let c = NormalizedColor {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let nsc = ns_color(&c);
        let _ = nsc;
    }

    #[test]
    fn run_key_batches_identical_cells() {
        let cell = CellSnapshot {
            character: 'a',
            fg: CellColor::Named(7),
            bg: CellColor::Named(0),
            bold: false,
        };
        let k1 = cell_run_key(&cell, false);
        let k2 = cell_run_key(&cell, false);
        assert_eq!(k1, k2);
    }

    #[test]
    fn run_key_differs_on_bold() {
        let normal = CellSnapshot {
            character: 'a',
            fg: CellColor::Named(7),
            bg: CellColor::Named(0),
            bold: false,
        };
        let bold = CellSnapshot {
            character: 'a',
            fg: CellColor::Named(7),
            bg: CellColor::Named(0),
            bold: true,
        };
        assert_ne!(cell_run_key(&normal, false), cell_run_key(&bold, false));
    }
}
