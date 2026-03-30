// Theme-to-AppKit translation — intermediate representations for testability.

use exaterm_ui::supervision::BattleCardStatus;
use exaterm_ui::theme::{self, Color, FontSpec};

/// RGBA components normalized to 0.0-1.0 for NSColor.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// Layer styling properties for card views.
#[derive(Debug, Clone)]
pub struct LayerStyle {
    pub corner_radius: f64,
    pub border_color: NormalizedColor,
    pub background_top: NormalizedColor,
    pub background_bottom: NormalizedColor,
    pub shadow_offset_y: f64,
    pub shadow_blur: f64,
    pub shadow_color: NormalizedColor,
}

/// Convert a theme `Color` (u8 channels + f32 alpha) to normalized 0.0-1.0 doubles.
pub fn normalize_color(color: &Color) -> NormalizedColor {
    NormalizedColor {
        r: f64::from(color.r) / 255.0,
        g: f64::from(color.g) / 255.0,
        b: f64::from(color.b) / 255.0,
        a: f64::from(color.a),
    }
}

/// Build a `LayerStyle` for the given battle-card status by reading `theme::card_theme`.
pub fn card_layer_style(status: BattleCardStatus) -> LayerStyle {
    let ct = theme::card_theme(status);
    LayerStyle {
        corner_radius: f64::from(ct.border_radius),
        border_color: normalize_color(&ct.border_color),
        background_top: normalize_color(&ct.background.top),
        background_bottom: normalize_color(&ct.background.bottom),
        shadow_offset_y: f64::from(ct.shadow.offset_y),
        shadow_blur: f64::from(ct.shadow.blur),
        shadow_color: normalize_color(&ct.shadow.color),
    }
}

/// Return the native font family name for a `FontSpec`.
///
/// Monospace specs map to "Menlo"; all others to the system UI font descriptor.
pub fn font_family(spec: &FontSpec) -> &'static str {
    if spec.monospace {
        "Menlo"
    } else {
        ".AppleSystemUIFont"
    }
}

use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFont};

/// Map a CSS-style `FontSpec` weight to an AppKit font weight value.
///
/// AppKit weights range roughly from -1.0 (thin) to 1.0 (heavy), with 0.0 being
/// regular. This maps common CSS numeric weights to appropriate AppKit values.
fn appkit_weight(css_weight: u16) -> f64 {
    match css_weight {
        0..=399 => 0.0,   // regular
        400..=599 => 0.0, // regular / medium
        600 => 0.3,       // semibold
        650 => 0.35,      // between semi and bold
        700 => 0.5,       // bold
        _ => 0.7,         // heavy (800+)
    }
}

/// Create an `NSFont` from a theme `FontSpec`.
///
/// Monospace specs use `monospacedSystemFontOfSize_weight`; proportional specs
/// use `systemFontOfSize_weight`. The CSS weight is translated via `appkit_weight`.
pub fn font_from_spec(spec: &FontSpec) -> Retained<NSFont> {
    let size = f64::from(spec.size);
    let weight = appkit_weight(spec.weight);
    if spec.monospace {
        NSFont::monospacedSystemFontOfSize_weight(size, weight)
    } else {
        NSFont::systemFontOfSize_weight(size, weight)
    }
}

/// Create an `NSColor` from a theme `Color` (u8 channels + f32 alpha).
pub fn color_to_nscolor(c: &Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
        f64::from(c.a),
    )
}

/// Map a `BattleCardStatus` to the CSS class name used in the web UI.
///
/// This is useful for native-side code that needs to identify card states by
/// the same canonical string key as the CSS layer.
pub fn css_class_for_status(status: BattleCardStatus) -> &'static str {
    match status {
        BattleCardStatus::Idle => "card-idle",
        BattleCardStatus::Stopped => "card-stopped",
        BattleCardStatus::Active => "card-active",
        BattleCardStatus::Thinking => "card-thinking",
        BattleCardStatus::Working => "card-working",
        BattleCardStatus::Blocked => "card-blocked",
        BattleCardStatus::Failed => "card-failed",
        BattleCardStatus::Complete => "card-complete",
        BattleCardStatus::Detached => "card-detached",
    }
}

use crate::terminal_state::CellColor;

pub fn ansi_palette() -> [NormalizedColor; 256] {
    let mut palette = [const {
        NormalizedColor {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
    }; 256];

    let from_u8 = |r: u8, g: u8, b: u8| NormalizedColor {
        r: r as f64 / 255.0,
        g: g as f64 / 255.0,
        b: b as f64 / 255.0,
        a: 1.0,
    };

    // Standard colors 0-7
    palette[0] = from_u8(0, 0, 0);
    palette[1] = from_u8(205, 0, 0);
    palette[2] = from_u8(0, 205, 0);
    palette[3] = from_u8(205, 205, 0);
    palette[4] = from_u8(0, 0, 238);
    palette[5] = from_u8(205, 0, 205);
    palette[6] = from_u8(0, 205, 205);
    palette[7] = from_u8(229, 229, 229);

    // Bright colors 8-15
    palette[8] = from_u8(127, 127, 127);
    palette[9] = from_u8(255, 0, 0);
    palette[10] = from_u8(0, 255, 0);
    palette[11] = from_u8(255, 255, 0);
    palette[12] = from_u8(92, 92, 255);
    palette[13] = from_u8(255, 0, 255);
    palette[14] = from_u8(0, 255, 255);
    palette[15] = from_u8(255, 255, 255);

    // 6x6x6 RGB cube: colors 16-231
    for i in 16..232 {
        let idx = i - 16;
        let r_idx = idx / 36;
        let g_idx = (idx % 36) / 6;
        let b_idx = idx % 6;
        let component = |c: usize| -> u8 { if c == 0 { 0 } else { (55 + 40 * c) as u8 } };
        palette[i] = from_u8(component(r_idx), component(g_idx), component(b_idx));
    }

    // Grayscale ramp: colors 232-255
    for i in 232..256 {
        let gray = (8 + 10 * (i - 232)) as u8;
        palette[i] = from_u8(gray, gray, gray);
    }

    palette
}

pub fn resolve_cell_color(color: &CellColor, palette: &[NormalizedColor; 256]) -> NormalizedColor {
    match *color {
        CellColor::Named(idx) => palette[idx.min(255) as usize].clone(),
        CellColor::Rgb(r, g, b) => NormalizedColor {
            r: r as f64 / 255.0,
            g: g as f64 / 255.0,
            b: b as f64 / 255.0,
            a: 1.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exaterm_ui::supervision::BattleCardStatus;
    use exaterm_ui::theme::{Color, FontSpec};

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

    // ---- normalize_color ----

    #[test]
    fn normalize_color_pure_red() {
        let c = Color {
            r: 255,
            g: 0,
            b: 0,
            a: 1.0,
        };
        let nc = normalize_color(&c);
        assert_eq!(
            nc,
            NormalizedColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            }
        );
    }

    #[test]
    fn normalize_color_mixed_with_alpha() {
        let c = Color {
            r: 128,
            g: 64,
            b: 32,
            a: 0.5,
        };
        let nc = normalize_color(&c);
        let eps = 0.002;
        assert!((nc.r - 128.0 / 255.0).abs() < eps, "r: {}", nc.r);
        assert!((nc.g - 64.0 / 255.0).abs() < eps, "g: {}", nc.g);
        assert!((nc.b - 32.0 / 255.0).abs() < eps, "b: {}", nc.b);
        assert!((nc.a - 0.5).abs() < eps, "a: {}", nc.a);
    }

    #[test]
    fn normalize_color_black_transparent() {
        let c = Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0.0,
        };
        let nc = normalize_color(&c);
        assert_eq!(
            nc,
            NormalizedColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0
            }
        );
    }

    #[test]
    fn normalize_color_white_opaque() {
        let c = Color {
            r: 255,
            g: 255,
            b: 255,
            a: 1.0,
        };
        let nc = normalize_color(&c);
        assert_eq!(
            nc,
            NormalizedColor {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0
            }
        );
    }

    // ---- card_layer_style ----

    #[test]
    fn card_layer_style_active_corner_radius() {
        let style = card_layer_style(BattleCardStatus::Active);
        assert!((style.corner_radius - 24.0).abs() < f64::EPSILON);
    }

    #[test]
    fn card_layer_style_active_gradient_matches_theme() {
        let style = card_layer_style(BattleCardStatus::Active);
        // Active top: Color { r: 14, g: 33, b: 52, a: 0.98 }
        let expected_top = normalize_color(&Color {
            r: 14,
            g: 33,
            b: 52,
            a: 0.98,
        });
        assert_eq!(style.background_top, expected_top);
        // Active bottom: Color { r: 9, g: 18, b: 31, a: 0.97 }
        let expected_bottom = normalize_color(&Color {
            r: 9,
            g: 18,
            b: 31,
            a: 0.97,
        });
        assert_eq!(style.background_bottom, expected_bottom);
    }

    #[test]
    fn card_layer_style_shadow_values() {
        let style = card_layer_style(BattleCardStatus::Idle);
        assert!((style.shadow_offset_y - 24.0).abs() < f64::EPSILON);
        assert!((style.shadow_blur - 46.0).abs() < f64::EPSILON);
        assert_eq!(
            style.shadow_color,
            normalize_color(&Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0.28
            }),
        );
    }

    #[test]
    fn every_status_produces_layer_style() {
        for &status in ALL_STATUSES {
            let _style = card_layer_style(status);
        }
    }

    // ---- font_family ----

    #[test]
    fn font_family_monospace_returns_menlo() {
        let spec = FontSpec {
            size: 12.0,
            weight: 400,
            letter_spacing: 0.0,
            line_height: None,
            monospace: true,
        };
        assert_eq!(font_family(&spec), "Menlo");
    }

    #[test]
    fn font_family_regular_returns_system_font() {
        let spec = FontSpec {
            size: 18.0,
            weight: 800,
            letter_spacing: 0.0,
            line_height: None,
            monospace: false,
        };
        assert_eq!(font_family(&spec), ".AppleSystemUIFont");
    }

    #[test]
    fn font_family_evidence_is_menlo() {
        assert_eq!(
            font_family(&exaterm_ui::theme::card_evidence_font()),
            "Menlo"
        );
    }

    #[test]
    fn font_family_title_is_system() {
        assert_eq!(
            font_family(&exaterm_ui::theme::card_title_font()),
            ".AppleSystemUIFont",
        );
    }

    // ---- appkit_weight ----

    #[test]
    fn appkit_weight_regular() {
        assert!((appkit_weight(400) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn appkit_weight_semibold() {
        assert!((appkit_weight(600) - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn appkit_weight_between_semi_and_bold() {
        assert!((appkit_weight(650) - 0.35).abs() < f64::EPSILON);
    }

    #[test]
    fn appkit_weight_bold() {
        assert!((appkit_weight(700) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn appkit_weight_heavy() {
        assert!((appkit_weight(800) - 0.7).abs() < f64::EPSILON);
    }

    // ---- font_from_spec ----

    #[test]
    fn font_from_spec_card_title_does_not_panic() {
        let f = font_from_spec(&exaterm_ui::theme::card_title_font());
        // Title font is 18pt.
        assert!((f.pointSize() - 18.0).abs() < 0.5);
    }

    #[test]
    fn font_from_spec_scrollback_does_not_panic() {
        let f = font_from_spec(&exaterm_ui::theme::scrollback_line_font());
        assert!((f.pointSize() - 11.0).abs() < 0.5);
    }

    #[test]
    fn font_from_spec_all_card_fonts() {
        // Ensure none of the card font specs panic.
        let _ = font_from_spec(&exaterm_ui::theme::card_status_font());
        let _ = font_from_spec(&exaterm_ui::theme::card_recency_font());
        let _ = font_from_spec(&exaterm_ui::theme::card_headline_font());
        let _ = font_from_spec(&exaterm_ui::theme::card_alert_font());
    }

    // ---- color_to_nscolor ----

    #[test]
    fn color_to_nscolor_pure_red_does_not_panic() {
        let _ = color_to_nscolor(&Color {
            r: 255,
            g: 0,
            b: 0,
            a: 1.0,
        });
    }

    #[test]
    fn color_to_nscolor_transparent_black() {
        let _ = color_to_nscolor(&Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0.0,
        });
    }

    #[test]
    fn color_to_nscolor_white_opaque() {
        let _ = color_to_nscolor(&Color {
            r: 255,
            g: 255,
            b: 255,
            a: 1.0,
        });
    }

    // ---- css_class_for_status ----

    #[test]
    fn css_class_maps_all_variants() {
        let expected = &[
            (BattleCardStatus::Idle, "card-idle"),
            (BattleCardStatus::Stopped, "card-stopped"),
            (BattleCardStatus::Active, "card-active"),
            (BattleCardStatus::Thinking, "card-thinking"),
            (BattleCardStatus::Working, "card-working"),
            (BattleCardStatus::Blocked, "card-blocked"),
            (BattleCardStatus::Failed, "card-failed"),
            (BattleCardStatus::Complete, "card-complete"),
            (BattleCardStatus::Detached, "card-detached"),
        ];
        for &(status, class) in expected {
            assert_eq!(css_class_for_status(status), class);
        }
    }

    #[test]
    fn css_class_covers_all_statuses() {
        // Ensure no panic for every variant.
        for &status in ALL_STATUSES {
            let _class = css_class_for_status(status);
        }
    }

    // ---- ansi_palette ----

    #[test]
    fn ansi_palette_black() {
        let palette = ansi_palette();
        assert_eq!(
            palette[0],
            NormalizedColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0
            }
        );
    }

    #[test]
    fn ansi_palette_red() {
        let palette = ansi_palette();
        assert!(palette[1].r > 0.5, "r: {}", palette[1].r);
        assert!(palette[1].g < 0.2, "g: {}", palette[1].g);
        assert!(palette[1].b < 0.2, "b: {}", palette[1].b);
    }

    #[test]
    fn ansi_palette_bright_white() {
        let palette = ansi_palette();
        assert!(palette[15].r > 0.9, "r: {}", palette[15].r);
        assert!(palette[15].g > 0.9, "g: {}", palette[15].g);
        assert!(palette[15].b > 0.9, "b: {}", palette[15].b);
    }

    // ---- resolve_cell_color ----

    #[test]
    fn resolve_named_color() {
        use crate::terminal_state::CellColor;
        let palette = ansi_palette();
        let resolved = resolve_cell_color(&CellColor::Named(1), &palette);
        assert_eq!(resolved, palette[1]);
    }

    #[test]
    fn resolve_rgb_color() {
        use crate::terminal_state::CellColor;
        let palette = ansi_palette();
        let resolved = resolve_cell_color(&CellColor::Rgb(42, 43, 44), &palette);
        assert_eq!(
            resolved,
            NormalizedColor {
                r: 42.0 / 255.0,
                g: 43.0 / 255.0,
                b: 44.0 / 255.0,
                a: 1.0
            },
        );
    }
}
