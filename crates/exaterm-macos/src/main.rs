mod app_delegate;
mod app_state;
mod battlefield_view;
mod beachhead;
mod event_bridge;
mod grid_renderer;
mod key_map;
mod menu;
mod session_io;
mod style;
mod terminal_state;
mod terminal_view;
mod window;

use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--beachhead-daemon") {
        let code = exaterm_core::run_local_daemon();
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }
    run_app();
}

fn run_app() {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, ns_string};

    let mtm = MainThreadMarker::new().expect("must be called from the main thread");

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // Create the delegate.
    let delegate = app_delegate::AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // Connect to the daemon.
    let beachhead = match exaterm_core::daemon::LocalBeachheadClient::connect_or_spawn() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("exaterm: failed to connect to daemon: {error}");
            std::process::exit(1);
        }
    };

    // Request the default workspace.
    let _ = beachhead
        .commands
        .send(exaterm_types::proto::ClientMessage::CreateOrResumeDefaultWorkspace);

    // Store command sender for menu actions (thread-local, used by AppDelegate).
    app_delegate::set_command_sender(beachhead.commands.clone());

    let beachhead = Rc::new(beachhead);

    // Shared mutable state.
    let state = Rc::new(RefCell::new(app_state::AppState::new()));
    let session_ios = Rc::new(RefCell::new(session_io::SessionIOMap::new()));

    // Create and configure the main window.
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;

    let content_rect = NSRect::new(
        NSPoint::new(200.0, 200.0),
        NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
    );

    let main_window: Retained<NSWindow> = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            content_rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };

    main_window.setTitle(ns_string!("Exaterm"));
    main_window.setMinSize(NSSize::new(
        window::WINDOW_MIN_WIDTH,
        window::WINDOW_MIN_HEIGHT,
    ));

    // Dark appearance.
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSAppearanceName};
    let dark_name: &NSAppearanceName = unsafe { objc2_app_kit::NSAppearanceNameDarkAqua };
    if let Some(dark) = NSAppearance::appearanceNamed(dark_name) {
        main_window.setAppearance(Some(&dark));
    }

    // Window background from theme.
    let bg = style::color_to_nscolor(&window::window_background());
    main_window.setBackgroundColor(Some(&bg));

    // Create a text field for terminal (focus mode) and a custom view for battlefield.
    // We toggle visibility between them based on presentation mode.
    use objc2::msg_send;
    use objc2_app_kit::NSView;

    let content_view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
        ),
    );

    let terminal_label = create_session_label(mtm);
    let battlefield_view: Retained<battlefield_view::BattlefieldView> = unsafe {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
        );
        msg_send![battlefield_view::BattlefieldView::alloc(mtm), initWithFrame: frame]
    };

    terminal_label.setHidden(true);
    battlefield_view.setHidden(false);

    // Use autoresizing masks so both views fill the content view.
    terminal_label.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    battlefield_view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    terminal_label.setFrame(content_view.frame());
    battlefield_view.setFrame(content_view.frame());

    content_view.addSubview(&terminal_label);
    content_view.addSubview(&battlefield_view);
    main_window.setContentView(Some(&content_view));

    // Build and set the menu bar.
    let menu_bar = menu::build_menu_bar(mtm);
    app.setMainMenu(Some(&menu_bar));

    // Set up a 100ms repeating timer to drain daemon events, session output, and refresh display.
    let timer_state = Rc::clone(&state);
    let timer_terminal_label = terminal_label.clone();
    let timer_battlefield_view = battlefield_view.clone();
    let timer_ios = Rc::clone(&session_ios);
    let render_state = Rc::new(terminal_view::TerminalRenderState::new(14.0));

    // Track last known content view size for resize detection.
    let last_size: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((
        window::WINDOW_DEFAULT_WIDTH,
        window::WINDOW_DEFAULT_HEIGHT,
    )));
    let timer_content_view = content_view.clone();
    let timer_last_size = Rc::clone(&last_size);

    // Cell dimensions for Menlo 14pt (standard values).
    const CELL_W: f64 = 8.4;
    const CELL_H: f64 = 17.0;

    let timer_beachhead = Rc::clone(&beachhead);
    let timer_block = block2::StackBlock::new(
        move |_timer: std::ptr::NonNull<objc2_foundation::NSTimer>| {
            // Drain all pending events from the daemon.
            while let Ok(message) = timer_beachhead.events.try_recv() {
                match message {
                    exaterm_types::proto::ServerMessage::WorkspaceSnapshot { snapshot } => {
                        timer_state.borrow_mut().apply_snapshot(&snapshot);
                    }
                    exaterm_types::proto::ServerMessage::Error { message } => {
                        eprintln!("exaterm: daemon error: {message}");
                    }
                }
            }
            timer_beachhead.drain_event_wake();

            // Update the first session ID for menu actions (e.g., New Shell).
            let first_id = timer_state
                .borrow()
                .workspace
                .sessions()
                .first()
                .map(|s| s.id);
            app_delegate::set_first_session(first_id);

            // Check for window resize and update terminal dimensions.
            {
                let frame = timer_content_view.frame();
                let (cur_w, cur_h) = (frame.size.width, frame.size.height);
                let mut last = timer_last_size.borrow_mut();
                if (cur_w - last.0).abs() > 1.0 || (cur_h - last.1).abs() > 1.0 {
                    *last = (cur_w, cur_h);
                    let (new_rows, new_cols) =
                        terminal_state::compute_grid_size(cur_w, cur_h, CELL_W, CELL_H);
                    // Resize all connected terminals and notify the daemon.
                    let mut ios = timer_ios.borrow_mut();
                    let borrowed = timer_state.borrow();
                    for &session_id in borrowed.raw_socket_names.keys() {
                        if let Some(sio) = ios.get_mut(&session_id) {
                            let (old_rows, old_cols) = sio.terminal.size();
                            if old_rows != new_rows || old_cols != new_cols {
                                sio.terminal.resize(new_rows, new_cols);
                                let _ = timer_beachhead.commands.send(
                                    exaterm_types::proto::ClientMessage::ResizeTerminal {
                                        session_id,
                                        rows: new_rows,
                                        cols: new_cols,
                                    },
                                );
                            }
                        }
                    }
                }
            }

            // Connect to any new session raw streams using the current window size.
            {
                let frame = timer_content_view.frame();
                let (init_rows, init_cols) = terminal_state::compute_grid_size(
                    frame.size.width,
                    frame.size.height,
                    CELL_W,
                    CELL_H,
                );
                let borrowed = timer_state.borrow();
                let mut ios = timer_ios.borrow_mut();
                ios.connect_new_sessions(&borrowed.raw_socket_names, init_rows, init_cols);

                // Remove sessions that are no longer present.
                let active_ids: Vec<_> = borrowed.raw_socket_names.keys().copied().collect();
                ios.retain_sessions(&active_ids);
            }

            // Drain PTY output and feed to terminal emulators.
            let mut ios = timer_ios.borrow_mut();
            ios.drain_all_output();

            let borrowed = timer_state.borrow();
            let focused = borrowed.workspace.focused_session();

            match focused {
                Some(session_id) => {
                    // Focus mode: show the focused session's terminal, hide battlefield.
                    timer_terminal_label.setHidden(false);
                    timer_battlefield_view.setHidden(true);

                    let snapshot = ios.session_snapshot(&session_id);
                    let fallback = format!("Session {} — connecting...", session_id.0);
                    terminal_view::update_label_with_snapshot(
                        &timer_terminal_label,
                        snapshot.as_ref(),
                        &render_state,
                        &fallback,
                    );
                }
                None => {
                    // Battlefield mode: show the card grid, hide terminal.
                    timer_terminal_label.setHidden(true);
                    timer_battlefield_view.setHidden(false);

                    let cards = borrowed.card_render_data(&ios);
                    let selected = borrowed.workspace.selected_session();
                    battlefield_view::set_battlefield_data(
                        cards,
                        selected,
                        Rc::clone(&render_state),
                    );
                    timer_battlefield_view.setNeedsDisplay(true);
                }
            }
        },
    );

    // SAFETY: Block captures only main-thread state and timer fires on the main run loop.
    let _timer = unsafe {
        objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            0.1,
            true,
            &timer_block,
        )
    };

    // Set up keyboard event monitoring to forward input to the PTY.
    let key_ios = Rc::clone(&session_ios);
    let key_state = Rc::clone(&state);
    let key_block = block2::StackBlock::new(
        move |event: std::ptr::NonNull<objc2_app_kit::NSEvent>| -> *mut objc2_app_kit::NSEvent {
            // SAFETY: The event pointer is valid for the duration of this callback.
            let event_ref = unsafe { event.as_ref() };
            let key_code = event_ref.keyCode();
            let flags = event_ref.modifierFlags();

            let modifiers = key_map::Modifiers {
                shift: flags.contains(objc2_app_kit::NSEventModifierFlags::Shift),
                control: flags.contains(objc2_app_kit::NSEventModifierFlags::Control),
                option: flags.contains(objc2_app_kit::NSEventModifierFlags::Option),
                command: flags.contains(objc2_app_kit::NSEventModifierFlags::Command),
            };

            let characters = event_ref.characters().map(|s| s.to_string());

            let in_focus = key_state.borrow().workspace.focused_session().is_some();

            // Cmd+N: add a new shell session (consume, don't pass to menu).
            if modifiers.command && key_code == 45 {
                app_delegate::send_add_terminals();
                return std::ptr::null_mut();
            }

            // Let other Cmd+key combos through to the menu system.
            if modifiers.command {
                return event.as_ptr();
            }

            // Battlefield mode keyboard handling.
            if !in_focus {
                match key_code {
                    // Enter: focus the selected session.
                    36 => {
                        let selected = key_state.borrow().workspace.selected_session();
                        if let Some(session_id) = selected {
                            key_state
                                .borrow_mut()
                                .workspace
                                .enter_focus_mode(session_id);
                        }
                        return std::ptr::null_mut();
                    }
                    // Up arrow: select previous session.
                    126 => {
                        key_state.borrow_mut().select_previous_session();
                        return std::ptr::null_mut();
                    }
                    // Down arrow: select next session.
                    125 => {
                        key_state.borrow_mut().select_next_session();
                        return std::ptr::null_mut();
                    }
                    _ => {
                        // Consume everything else to prevent beeps.
                        return std::ptr::null_mut();
                    }
                }
            }

            // Focus mode: Escape returns to battlefield.
            if in_focus && key_code == 53 {
                key_state.borrow_mut().workspace.return_to_battlefield();
                return std::ptr::null_mut();
            }

            // Focus mode: forward keys to the focused session's PTY.
            let input = key_map::KeyInput {
                key_code,
                modifiers,
                characters,
            };

            let focused_id = key_state.borrow().workspace.focused_session();
            let app_cursor = focused_id
                .and_then(|id| key_ios.borrow().session_app_cursor(&id))
                .unwrap_or(false);
            let action = key_map::key_event_to_action(&input, app_cursor);
            match action {
                key_map::KeyAction::Bytes(bytes) => {
                    if let Some(id) = focused_id {
                        key_ios.borrow_mut().write_input(&id, &bytes);
                    } else {
                        key_ios.borrow_mut().write_input_first(&bytes);
                    }
                    // Return null to consume the event (prevent beep).
                    std::ptr::null_mut()
                }
                key_map::KeyAction::Paste => {
                    // Read from clipboard and send to PTY.
                    if let Some(text) = clipboard_text() {
                        if let Some(id) = focused_id {
                            key_ios.borrow_mut().write_input(&id, text.as_bytes());
                        } else {
                            key_ios.borrow_mut().write_input_first(text.as_bytes());
                        }
                    }
                    std::ptr::null_mut()
                }
                key_map::KeyAction::Copy | key_map::KeyAction::None => {
                    // Let the system handle it.
                    event.as_ptr()
                }
            }
        },
    );

    let _key_monitor = unsafe {
        objc2_app_kit::NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            objc2_app_kit::NSEventMask::KeyDown,
            &key_block,
        )
    };

    // Show the window.
    main_window.makeKeyAndOrderFront(None);
    app.activate();

    // Keep everything alive for the lifetime of the app.
    std::mem::forget(main_window);
    std::mem::forget(beachhead);
    std::mem::forget(state);
    std::mem::forget(session_ios);

    app.run();
}

/// Read the current clipboard text content, if any.
fn clipboard_text() -> Option<String> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::ns_string;

    let pb = NSPasteboard::generalPasteboard();
    let string_type = ns_string!("public.utf8-plain-text");
    pb.stringForType(string_type).map(|s| s.to_string())
}

/// Create a multi-line, read-only text label for displaying session info.
fn create_session_label(
    mtm: objc2::MainThreadMarker,
) -> objc2::rc::Retained<objc2_app_kit::NSTextField> {
    use objc2_app_kit::NSTextField;
    use objc2_foundation::ns_string;

    let label = NSTextField::labelWithString(ns_string!("Connecting to daemon..."), mtm);
    label.setEditable(false);
    label.setBezeled(false);
    label.setDrawsBackground(false);
    label.setSelectable(false);

    // Use the scrollback line font from the theme for the fallback text.
    let font = style::font_from_spec(&exaterm_ui::theme::scrollback_line_font());
    label.setFont(Some(&font));

    let white = style::color_to_nscolor(&exaterm_ui::theme::Color {
        r: 248,
        g: 250,
        b: 252,
        a: 1.0,
    });
    label.setTextColor(Some(&white));

    label
}
