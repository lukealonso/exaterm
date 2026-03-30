//! SwiftTerm-backed terminal emulator bridge for macOS.
//!
//! This crate provides a type-safe Rust API over the Swift `ExatermTerminalBridge`
//! class, which wraps SwiftTerm's `TerminalView`. The bridge is compiled from Swift
//! sources and linked via `build.rs`.
//!
//! # Usage
//!
//! ```ignore
//! use exaterm_swiftterm::TerminalBridge;
//!
//! let bridge = TerminalBridge::new(frame);
//! let view = bridge.view(); // NSView to embed in your window
//! bridge.feed(b"Hello, terminal!\r\n");
//! ```

mod ffi;
mod terminal;

pub use terminal::{TerminalAppearance, TerminalBridge, TerminalSize};
