use exaterm_ui::theme::Color;

/// Window configuration constants.
pub const WINDOW_MIN_WIDTH: f64 = 800.0;
pub const WINDOW_MIN_HEIGHT: f64 = 600.0;
pub const WINDOW_DEFAULT_WIDTH: f64 = 1400.0;
pub const WINDOW_DEFAULT_HEIGHT: f64 = 900.0;

/// Returns the default window background color.
pub fn window_background() -> Color {
    Color {
        r: 0,
        g: 0,
        b: 0,
        a: 1.0,
    }
}
