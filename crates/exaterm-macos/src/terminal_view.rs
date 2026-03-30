// Cached render state for card UI rendering (fonts, colors).

use std::collections::BTreeMap;

use crate::style::{self, NormalizedColor};

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFont};

use exaterm_ui::supervision::BattleCardStatus;
use exaterm_ui::theme::{self as theme, Color};

fn ns_color(c: &NormalizedColor) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a)
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

/// Convenience: create cached NSColor/NSFont objects from the theme.
///
/// All theme-derived fonts and colors are computed once and cached for use in
/// rendering functions. No hardcoded colors exist outside this constructor.
pub struct TerminalRenderState {
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
    pub attention_chip_text: Retained<NSColor>,

    // Per-status cached colors: discriminant -> (chip_text_color, chip_bg_color).
    pub status_chip_colors: BTreeMap<u8, (Retained<NSColor>, Retained<NSColor>)>,
    // Per-status card background: discriminant -> card background top color.
    pub card_bg_colors: BTreeMap<u8, Retained<NSColor>>,
    pub attention_bg_colors: BTreeMap<usize, Retained<NSColor>>,
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
    pub fn new() -> Self {
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
        let attention_chip_text = style::color_to_nscolor(&Color {
            r: 248,
            g: 250,
            b: 252,
            a: 1.0,
        });

        // Per-status cached colors.
        let mut status_chip_colors = BTreeMap::new();
        let mut card_bg_colors = BTreeMap::new();
        let mut attention_bg_colors = BTreeMap::new();
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
        attention_bg_colors.insert(
            1,
            style::color_to_nscolor(&Color {
                r: 17,
                g: 88,
                b: 51,
                a: 0.24,
            }),
        );
        attention_bg_colors.insert(
            2,
            style::color_to_nscolor(&Color {
                r: 33,
                g: 82,
                b: 145,
                a: 0.22,
            }),
        );
        attention_bg_colors.insert(
            3,
            style::color_to_nscolor(&Color {
                r: 120,
                g: 87,
                b: 10,
                a: 0.22,
            }),
        );
        attention_bg_colors.insert(
            4,
            style::color_to_nscolor(&Color {
                r: 114,
                g: 28,
                b: 35,
                a: 0.24,
            }),
        );
        attention_bg_colors.insert(
            5,
            style::color_to_nscolor(&Color {
                r: 130,
                g: 35,
                b: 35,
                a: 0.32,
            }),
        );

        Self {
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
            attention_chip_text,
            status_chip_colors,
            card_bg_colors,
            attention_bg_colors,
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

    pub fn attention_chip_bg(&self, fill: usize) -> &Retained<NSColor> {
        &self.attention_bg_colors[&fill.clamp(1, 5)]
    }
}
