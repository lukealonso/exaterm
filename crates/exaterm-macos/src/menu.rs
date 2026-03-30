use objc2::rc::Retained;
use objc2::sel;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::ns_string;

/// Menu tag constants.
pub const TAG_TOGGLE_AUTO_NUDGE: i32 = 3;
pub const TAG_SYNC_INPUTS: i32 = 4;
pub const TAG_NEW_SHELL: i32 = 11;

/// Builds the application menu bar.
pub fn build_menu_bar(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu_bar = NSMenu::new(mtm);

    // Application menu (named "Exaterm")
    let app_menu_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Exaterm"),
            None,
            ns_string!(""),
        )
    };
    let app_menu = NSMenu::new(mtm);
    let quit_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Quit Exaterm"),
            Some(sel!(terminate:)),
            ns_string!("q"),
        )
    };
    app_menu.addItem(&quit_item);
    app_menu_item.setSubmenu(Some(&app_menu));
    menu_bar.addItem(&app_menu_item);

    // Shell menu
    let shell_menu_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Shell"),
            None,
            ns_string!(""),
        )
    };
    let shell_menu = NSMenu::new(mtm);
    let new_shell_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Add Shells"),
            Some(sel!(newShell:)),
            ns_string!("n"),
        )
    };
    new_shell_item.setTag(TAG_NEW_SHELL as isize);
    shell_menu.addItem(&new_shell_item);
    shell_menu_item.setSubmenu(Some(&shell_menu));
    menu_bar.addItem(&shell_menu_item);

    // Edit menu
    let edit_menu_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Edit"),
            None,
            ns_string!(""),
        )
    };
    let edit_menu = NSMenu::new(mtm);
    let copy_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Copy"),
            Some(sel!(copy:)),
            ns_string!("c"),
        )
    };
    let paste_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Paste"),
            Some(sel!(paste:)),
            ns_string!("v"),
        )
    };
    let auto_nudge_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Toggle Auto-Nudge"),
            Some(sel!(toggleAutoNudge:)),
            ns_string!(""),
        )
    };
    auto_nudge_item.setTag(TAG_TOGGLE_AUTO_NUDGE as isize);
    let sync_inputs_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Synchronize Inputs"),
            Some(sel!(toggleSyncInputs:)),
            ns_string!(""),
        )
    };
    sync_inputs_item.setTag(TAG_SYNC_INPUTS as isize);
    let select_all_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Select All"),
            Some(sel!(selectAll:)),
            ns_string!("a"),
        )
    };
    edit_menu.addItem(&copy_item);
    edit_menu.addItem(&paste_item);
    edit_menu.addItem(&auto_nudge_item);
    edit_menu.addItem(&sync_inputs_item);
    edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
    edit_menu.addItem(&select_all_item);
    edit_menu_item.setSubmenu(Some(&edit_menu));
    menu_bar.addItem(&edit_menu_item);

    // Window menu
    let window_menu_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Window"),
            None,
            ns_string!(""),
        )
    };
    let window_menu = NSMenu::new(mtm);
    let minimize_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Minimize"),
            Some(sel!(performMiniaturize:)),
            ns_string!("m"),
        )
    };
    let close_item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            ns_string!("Close Window"),
            Some(sel!(performClose:)),
            ns_string!("w"),
        )
    };
    window_menu.addItem(&minimize_item);
    window_menu.addItem(&close_item);
    window_menu_item.setSubmenu(Some(&window_menu));
    menu_bar.addItem(&window_menu_item);

    menu_bar
}
