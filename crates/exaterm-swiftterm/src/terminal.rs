//! Idiomatic Rust API for the SwiftTerm-backed terminal bridge.

use std::marker::PhantomData;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::{NSData, NSRect};

use crate::ffi;
use exaterm_ui::theme::Color;

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalAppearance {
    pub font_name: String,
    pub font_size: f64,
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

/// A terminal emulator backed by SwiftTerm, wrapped in a type-safe Rust API.
///
/// The underlying NSView can be embedded in any AppKit view hierarchy.
/// Feed PTY output via [`feed`](Self::feed), and receive user keystrokes
/// via the input handler callback.
pub struct TerminalBridge {
    inner: Retained<NSObject>,
    _not_send_sync: PhantomData<*const ()>,
}

impl TerminalBridge {
    /// Create a new terminal bridge. Must be called on the main thread.
    pub fn new(frame: NSRect) -> Self {
        // SAFETY: Called on main thread, ExatermTerminalBridge is linked.
        let inner = unsafe { ffi::bridge_new(frame) };
        Self {
            inner,
            _not_send_sync: PhantomData,
        }
    }

    /// Get the terminal's NSView for embedding in a view hierarchy.
    pub fn view(&self) -> Retained<NSView> {
        // SAFETY: inner is a valid ExatermTerminalBridge.
        unsafe { ffi::bridge_terminal_view(&self.inner) }
    }

    /// Feed raw PTY output bytes into the terminal emulator.
    pub fn feed(&self, data: &[u8]) {
        // SAFETY: inner is a valid ExatermTerminalBridge.
        unsafe { ffi::bridge_feed(&self.inner, data) }
    }

    /// Reset the terminal before reusing the view for a different session.
    pub fn clear(&self) {
        // SAFETY: inner is a valid ExatermTerminalBridge.
        unsafe { ffi::bridge_clear(&self.inner) }
    }

    /// Get the current terminal grid size.
    pub fn terminal_size(&self) -> TerminalSize {
        // SAFETY: inner is a valid ExatermTerminalBridge.
        let (cols, rows) = unsafe { ffi::bridge_terminal_size(&self.inner) };
        TerminalSize { rows, cols }
    }

    /// Set the terminal font by name and size.
    pub fn set_font(&self, name: &str, size: f64) {
        // SAFETY: inner is a valid ExatermTerminalBridge.
        unsafe { ffi::bridge_set_font(&self.inner, name, size) }
    }

    /// Set the input handler that receives user keystrokes as byte slices.
    pub fn set_input_handler<F>(&self, handler: F)
    where
        F: Fn(&[u8]) + 'static,
    {
        let block = block2::RcBlock::new(move |data: NonNull<NSData>| {
            let ns_data = unsafe { data.as_ref() };
            let bytes = unsafe { ns_data.as_bytes_unchecked() };
            handler(bytes);
        });
        unsafe { ffi::bridge_set_input_handler(&self.inner, &*block) };
    }

    /// Set the size handler that receives terminal resize events as (rows, cols).
    pub fn set_size_handler<F>(&self, handler: F)
    where
        F: Fn(TerminalSize) + 'static,
    {
        let block = block2::RcBlock::new(move |cols: i32, rows: i32| {
            handler(TerminalSize {
                rows: rows as u16,
                cols: cols as u16,
            });
        });
        unsafe { ffi::bridge_set_size_handler(&self.inner, &*block) };
    }

    /// Set terminal colors using the shared theme color type.
    /// Converts from `exaterm_ui::theme::Color` to NSColor internally.
    pub fn set_colors(&self, fg: &Color, bg: &Color, cursor: &Color) {
        let ns_fg = color_to_nscolor(fg);
        let ns_bg = color_to_nscolor(bg);
        let ns_cursor = color_to_nscolor(cursor);
        // SAFETY: inner is a valid ExatermTerminalBridge, colors are valid.
        unsafe { ffi::bridge_set_colors(&self.inner, &ns_fg, &ns_bg, &ns_cursor) }
    }

    /// Apply the complete terminal appearance in one type-safe call.
    pub fn set_appearance(&self, appearance: &TerminalAppearance) {
        self.set_font(&appearance.font_name, appearance.font_size);
        self.set_colors(
            &appearance.foreground,
            &appearance.background,
            &appearance.cursor,
        );
    }
}

/// Convert a theme Color to an NSColor.
fn color_to_nscolor(c: &Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        c.r as f64 / 255.0,
        c.g as f64 / 255.0,
        c.b as f64 / 255.0,
        c.a as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_to_nscolor_converts_red() {
        let c = Color {
            r: 255,
            g: 0,
            b: 0,
            a: 1.0,
        };
        let ns = color_to_nscolor(&c);
        // Verify it doesn't panic and produces a valid color.
        let _ = ns;
    }

    #[test]
    fn color_to_nscolor_converts_transparent() {
        let c = Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0.0,
        };
        let ns = color_to_nscolor(&c);
        let _ = ns;
    }

    /// Integration tests that require AppKit/NSApplication. Run manually with:
    /// `cargo test -p exaterm-swiftterm -- --ignored`
    #[test]
    #[ignore = "requires AppKit event loop"]
    fn creates_terminal_view() {
        let frame = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(640.0, 480.0),
        );
        let bridge = TerminalBridge::new(frame);
        let _view = bridge.view();
    }

    #[test]
    #[ignore = "requires AppKit event loop"]
    fn feed_does_not_crash() {
        let frame = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(640.0, 480.0),
        );
        let bridge = TerminalBridge::new(frame);
        bridge.feed(b"Hello, terminal!\r\n");
        bridge.feed(b"\x1b[31mred text\x1b[0m");
    }

    #[test]
    #[ignore = "requires AppKit event loop"]
    fn terminal_size_returns_positive() {
        let frame = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(640.0, 480.0),
        );
        let bridge = TerminalBridge::new(frame);
        let size = bridge.terminal_size();
        assert!(size.rows > 0, "rows should be positive, got {}", size.rows);
        assert!(size.cols > 0, "cols should be positive, got {}", size.cols);
    }
}
