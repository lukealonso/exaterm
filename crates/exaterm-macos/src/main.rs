mod app_delegate;
mod app_state;
mod battlefield_view;
mod beachhead;
mod event_bridge;
mod key_map;
mod menu;
mod session_io;
mod style;
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
    let style_mask = NSWindowStyleMask::Titled
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
            style_mask,
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

    // Create views: SwiftTerm bridge for focus mode, BattlefieldView for card grid.
    use objc2::msg_send;
    use objc2_app_kit::NSView;

    let content_view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
        ),
    );

    // Create the SwiftTerm-backed terminal view for focus mode.
    let terminal_frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
    );
    let terminal_bridge = exaterm_swiftterm::TerminalBridge::new(terminal_frame);
    let terminal_view = terminal_bridge.view();

    let battlefield_view: Retained<battlefield_view::BattlefieldView> = unsafe {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
        );
        msg_send![battlefield_view::BattlefieldView::alloc(mtm), initWithFrame: frame]
    };

    terminal_view.setHidden(true);
    battlefield_view.setHidden(false);

    // Use autoresizing masks so both views fill the content view.
    terminal_view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    battlefield_view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    terminal_view.setFrame(content_view.frame());
    battlefield_view.setFrame(content_view.frame());

    content_view.addSubview(&terminal_view);
    content_view.addSubview(&battlefield_view);
    main_window.setContentView(Some(&content_view));

    // Build and set the menu bar.
    let menu_bar = menu::build_menu_bar(mtm);
    app.setMainMenu(Some(&menu_bar));

    // Set up a 100ms repeating timer to drain daemon events, session output, and refresh display.
    let timer_state = Rc::clone(&state);
    let timer_terminal_view = terminal_view.clone();
    let timer_battlefield_view = battlefield_view.clone();
    let timer_ios = Rc::clone(&session_ios);
    let render_state = Rc::new(terminal_view::TerminalRenderState::new());

    let timer_beachhead = Rc::clone(&beachhead);

    // The terminal bridge is !Send, so we wrap it in Rc for shared access on the main thread.
    let timer_bridge = Rc::new(terminal_bridge);
    let displayed_focus =
        Rc::new(RefCell::new(None::<exaterm_types::model::SessionId>));

    let terminal_font = exaterm_ui::theme::terminal_font();
    let terminal_appearance = exaterm_swiftterm::TerminalAppearance {
        font_name: style::font_family(&terminal_font).to_string(),
        font_size: terminal_font.size as f64,
        foreground: exaterm_ui::theme::terminal_foreground_color(),
        background: exaterm_ui::theme::terminal_background_color(),
        cursor: exaterm_ui::theme::terminal_cursor_color(),
    };
    timer_bridge.set_appearance(&terminal_appearance);

    // Wire the SwiftTerm input handler so keystrokes reach the PTY.
    let input_ios = Rc::clone(&session_ios);
    let input_state = Rc::clone(&state);
    timer_bridge.set_input_handler(move |bytes: &[u8]| {
        let focused = input_state.borrow().workspace.focused_session();
        if let Some(id) = focused {
            input_ios.borrow_mut().write_input(&id, bytes);
        }
    });

    // Wire the SwiftTerm resize handler so the daemon gets resize messages.
    let resize_beachhead = Rc::clone(&beachhead);
    let resize_state = Rc::clone(&state);
    timer_bridge.set_size_handler(move |size| {
        let focused = resize_state.borrow().workspace.focused_session();
        if let Some(session_id) = focused {
            let _ = resize_beachhead.commands.send(
                exaterm_types::proto::ClientMessage::ResizeTerminal {
                    session_id,
                    rows: size.rows,
                    cols: size.cols,
                },
            );
        }
    });

    let timer_displayed_focus = Rc::clone(&displayed_focus);
    let timer_block = block2::StackBlock::new(move |_timer: std::ptr::NonNull<objc2_foundation::NSTimer>| {
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
            timer_beachhead.drain_event_wake();

            // Update the first session ID for menu actions (e.g., New Shell).
            let first_id = timer_state
                .borrow()
                .workspace
                .sessions()
                .first()
                .map(|s| s.id);
            app_delegate::set_first_session(first_id);

        // Connect to any new session raw streams.
        {
            let borrowed = timer_state.borrow();
            let mut ios = timer_ios.borrow_mut();
            ios.connect_new_sessions(&borrowed.raw_socket_names);

            let borrowed = timer_state.borrow();
            let focused = borrowed.workspace.focused_session();

        let borrowed = timer_state.borrow();
        let focused = borrowed.workspace.focused_session();
        {
            let mut displayed = timer_displayed_focus.borrow_mut();
            if *displayed != focused {
                timer_bridge.clear();
                if let Some(session_id) = focused {
                    let size = timer_bridge.terminal_size();
                    let _ = timer_beachhead.commands.send(
                        exaterm_types::proto::ClientMessage::ResizeTerminal {
                            session_id,
                            rows: size.rows,
                            cols: size.cols,
                        },
                    );
                }
                *displayed = focused;
            }
        }

        // Drain all PTY output every tick to prevent background buffer growth.
        let all_output = timer_ios.borrow_mut().drain_all_output();

        match focused {
            Some(session_id) => {
                // Focus mode: feed the focused session's output to SwiftTerm.
                timer_terminal_view.setHidden(false);
                timer_battlefield_view.setHidden(true);

                if let Some(bytes) = all_output.get(&session_id) {
                    timer_bridge.feed(bytes);
                }
            }
            None => {
                // Battlefield mode: show the card grid, hide terminal.
                timer_terminal_view.setHidden(true);
                timer_battlefield_view.setHidden(false);

                let cards = borrowed.card_render_data();
                let selected = borrowed.workspace.selected_session();
                battlefield_view::set_battlefield_data(
                    cards, selected, Rc::clone(&render_state),
                );
                timer_battlefield_view.setNeedsDisplay(true);
            }
        }
    });

    // SAFETY: Block captures only main-thread state and timer fires on the main run loop.
    let _timer = unsafe {
        objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            0.1,
            true,
            &timer_block,
        )
    };

    // Set up keyboard event monitoring.
    // In focus mode, SwiftTerm handles input as first responder — we only intercept
    // Escape (exit focus) and Cmd+N (add shell). In battlefield mode, we handle navigation.
    let key_state = Rc::clone(&state);
    let key_window = main_window.clone();
    let key_terminal_view = terminal_view.clone();
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
                            key_state.borrow_mut().workspace.enter_focus_mode(session_id);
                            // Make SwiftTerm first responder so it receives keyboard input.
                            key_window.makeFirstResponder(Some(&*key_terminal_view));
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
                key_window.makeFirstResponder(None);
                return std::ptr::null_mut();
            }

            // Focus mode: let SwiftTerm handle all other keys as first responder.
            // Pass the event through to the responder chain.
            event.as_ptr()
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
