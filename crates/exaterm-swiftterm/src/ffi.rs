//! Raw FFI bindings to the ExatermTerminalBridge Swift class via objc2.

use std::ptr::NonNull;

use block2::Block;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2_app_kit::{NSColor, NSView};
use objc2_foundation::{NSData, NSRect, NSSize, NSString};

unsafe extern "C" {
    fn exaterm_terminal_bridge_force_link();
}

/// Create an ExatermTerminalBridge instance and set the terminal view's frame.
///
/// # Safety
/// Must be called on the main thread.
pub unsafe fn bridge_new(frame: NSRect) -> Retained<NSObject> {
    exaterm_terminal_bridge_force_link();
    let cls = objc2::runtime::AnyClass::get(c"ExatermTerminalBridge")
        .expect("ExatermTerminalBridge class not found — is the Swift library linked?");
    let obj: Retained<NSObject> = msg_send![cls, new];
    // Set the terminal view's frame to the requested size.
    let view: Retained<NSView> = msg_send![&*obj, terminalView];
    let _: () = msg_send![&*view, setFrame: frame];
    obj
}

/// Get the terminal NSView from the bridge.
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_terminal_view(bridge: &NSObject) -> Retained<NSView> {
    msg_send![bridge, terminalView]
}

/// Feed raw PTY output data to the terminal.
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_feed(bridge: &NSObject, data: &[u8]) {
    let ns_data = NSData::with_bytes(data);
    let _: () = msg_send![bridge, feed: &*ns_data];
}

/// Reset the terminal state before reusing the view for a different session.
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_clear(bridge: &NSObject) {
    let _: () = msg_send![bridge, clear];
}

/// Get the current terminal grid size as (cols, rows).
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_terminal_size(bridge: &NSObject) -> (u16, u16) {
    let size: NSSize = msg_send![bridge, terminalSize];
    (size.width as u16, size.height as u16)
}

/// Set the terminal font.
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_set_font(bridge: &NSObject, name: &str, size: f64) {
    let ns_name = NSString::from_str(name);
    let _: () = msg_send![bridge, setFontName: &*ns_name, size: size];
}

/// Set terminal colors (foreground, background, cursor).
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_set_colors(bridge: &NSObject, fg: &NSColor, bg: &NSColor, cursor: &NSColor) {
    let _: () = msg_send![bridge, setForegroundColor: fg, backgroundColor: bg, cursorColor: cursor];
}

/// Set the input handler callback that receives keystrokes as NSData.
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_set_input_handler(bridge: &NSObject, block: &Block<dyn Fn(NonNull<NSData>)>) {
    let _: () = msg_send![bridge, setInputHandler: block];
}

/// Set the size handler callback that receives terminal resize events.
///
/// # Safety
/// `bridge` must be a valid ExatermTerminalBridge instance.
pub unsafe fn bridge_set_size_handler(bridge: &NSObject, block: &Block<dyn Fn(i32, i32)>) {
    let _: () = msg_send![bridge, setSizeHandler: block];
}
