#[cfg(target_os = "macos")]
mod app_delegate;
#[cfg(target_os = "macos")]
mod app_state;
#[cfg(target_os = "macos")]
mod battlefield_view;
#[cfg(target_os = "macos")]
mod focus_view;
#[cfg(target_os = "macos")]
mod key_map;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(target_os = "macos")]
mod session_io;
#[cfg(target_os = "macos")]
mod style;
#[cfg(target_os = "macos")]
mod terminal_view;
#[cfg(target_os = "macos")]
mod window;

#[cfg(target_os = "macos")]
use std::cell::RefCell;
#[cfg(target_os = "macos")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "macos")]
use std::rc::Rc;
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;

#[cfg(target_os = "macos")]
use objc2_foundation::{NSPoint, NSRect, NSSize};

#[cfg(target_os = "macos")]
fn main() {
    let argv = std::env::args().collect::<Vec<_>>();
    if argv.get(1).map(|s| s.as_str()) == Some("--beachhead-daemon") {
        let code = exaterm_core::run_local_daemon();
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }
    let mode = match exaterm_ui::beachhead::parse_run_mode(argv.into_iter().skip(1)) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("usage: exaterm [--ssh user@host]");
            std::process::exit(2);
        }
    };
    run_app(mode);
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("exaterm-macos is only supported on macOS");
}

#[cfg(target_os = "macos")]
fn run_app(mode: exaterm_ui::beachhead::RunMode) {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSView, NSWindow,
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
    let target = exaterm_ui::beachhead::BeachheadTarget::from(&mode);
    let beachhead = match exaterm_ui::beachhead::BeachheadConnection::connect(&target) {
        Ok(client) => client,
        Err(error) => {
            present_startup_error(mtm, &error);
            std::process::exit(1);
        }
    };

    // Request the default workspace.
    let _ = beachhead
        .commands()
        .send(exaterm_types::proto::ClientMessage::CreateOrResumeDefaultWorkspace);

    // Store command sender for menu actions (thread-local, used by AppDelegate).
    app_delegate::set_command_sender(beachhead.commands().clone());

    let beachhead = Rc::new(beachhead);

    // Shared mutable state.
    let state = Rc::new(RefCell::new(app_state::AppState::new()));
    let session_ios = Rc::new(RefCell::new(session_io::SessionIOMap::new()));
    let sync_inputs_enabled = Arc::new(AtomicBool::new(false));
    app_delegate::set_sync_inputs_state(sync_inputs_enabled.clone());

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

    let content_view = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
        ),
    );

    let battlefield_view: Retained<battlefield_view::BattlefieldView> = unsafe {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(window::WINDOW_DEFAULT_WIDTH, window::WINDOW_DEFAULT_HEIGHT),
        );
        msg_send![battlefield_view::BattlefieldView::alloc(mtm), initWithFrame: frame]
    };
    let focus_panel: Retained<focus_view::FocusView> = unsafe {
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(
                window::WINDOW_DEFAULT_WIDTH,
                window::WINDOW_DEFAULT_HEIGHT - 240.0,
            ),
        );
        msg_send![focus_view::FocusView::alloc(mtm), initWithFrame: frame]
    };

    battlefield_view.setHidden(false);
    focus_panel.setHidden(true);

    // Use autoresizing masks so both views fill the content view.
    battlefield_view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    battlefield_view.setFrame(content_view.frame());
    focus_panel.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );

    let battlefield_state = Rc::clone(&state);
    let battlefield_window = main_window.clone();
    let interaction_window = battlefield_window.clone();
    battlefield_view::set_interaction_handler(move |interaction| match interaction {
        battlefield_view::BattlefieldInteraction::Select(session_id) => {
            battlefield_state
                .borrow_mut()
                .workspace
                .select_session(session_id);
            interaction_window.makeFirstResponder(None);
        }
        battlefield_view::BattlefieldInteraction::Focus(session_id) => {
            battlefield_state
                .borrow_mut()
                .workspace
                .enter_focus_mode(session_id);
            interaction_window.makeFirstResponder(None);
        }
    });

    content_view.addSubview(&battlefield_view);
    content_view.addSubview(&focus_panel);
    main_window.setContentView(Some(&content_view));

    // Build and set the menu bar.
    let menu_bar = menu::build_menu_bar(mtm);
    app.setMainMenu(Some(&menu_bar));

    // Set up a 100ms repeating timer to drain daemon events, session output, and refresh display.
    let timer_state = Rc::clone(&state);
    let timer_battlefield_view = battlefield_view.clone();
    let timer_focus_panel = focus_panel.clone();
    let timer_ios = Rc::clone(&session_ios);
    let render_state = Rc::new(terminal_view::TerminalRenderState::new());
    let terminal_surfaces = Rc::new(RefCell::new(BTreeMap::<
        exaterm_types::model::SessionId,
        TerminalSurface,
    >::new()));

    let timer_beachhead = Rc::clone(&beachhead);
    let displayed_focus = Rc::new(RefCell::new(None::<exaterm_types::model::SessionId>));

    let timer_displayed_focus = Rc::clone(&displayed_focus);
    let timer_surfaces = Rc::clone(&terminal_surfaces);
    let timer_sync_inputs = sync_inputs_enabled.clone();
    let timer_block = block2::StackBlock::new(
        move |_timer: std::ptr::NonNull<objc2_foundation::NSTimer>| {
            // Drain all pending events from the daemon.
            while let Ok(message) = timer_beachhead.events().try_recv() {
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
            let borrowed_state = timer_state.borrow();
            let first_id = borrowed_state.workspace.sessions().first().map(|s| s.id);
            app_delegate::set_first_session(first_id);
            app_delegate::set_selected_session(borrowed_state.workspace.selected_session());
            app_delegate::set_has_sessions(!borrowed_state.workspace.sessions().is_empty());
            let selected_auto_nudge = borrowed_state
                .workspace
                .selected_session()
                .and_then(|id| borrowed_state.auto_nudge_enabled.get(&id).copied())
                .unwrap_or(false);
            app_delegate::set_selected_auto_nudge(selected_auto_nudge);
            drop(borrowed_state);

            // Connect to any new session raw streams.
            {
                let borrowed = timer_state.borrow();
                let mut ios = timer_ios.borrow_mut();
                ios.connect_new_sessions(
                    &timer_beachhead.raw_session_connector(),
                    &borrowed.raw_socket_names,
                );

                // Remove sessions that are no longer present.
                let active_ids: Vec<_> = borrowed.raw_socket_names.keys().copied().collect();
                ios.retain_sessions(&active_ids);
            }

            let content_bounds = content_view.frame();
            let borrowed = timer_state.borrow();
            ensure_terminal_surfaces(
                &mut timer_surfaces.borrow_mut(),
                borrowed.workspace.sessions(),
                &timer_ios,
                &timer_beachhead,
                timer_sync_inputs.clone(),
            );

            let focused = borrowed.workspace.focused_session();
            {
                let mut displayed = timer_displayed_focus.borrow_mut();
                if *displayed != focused {
                    *displayed = focused;
                }
            }

            // Drain all PTY output every tick to prevent background buffer growth.
            let all_output = timer_ios.borrow_mut().drain_all_output();
            for (session_id, bytes) in &all_output {
                if let Some(surface) = timer_surfaces.borrow().get(session_id) {
                    surface.bridge.feed(bytes);
                }
            }

            let cards = borrowed.card_render_data();
            let selected = borrowed.workspace.selected_session();
            let card_rects = exaterm_ui::layout::card_layout(
                cards.len(),
                content_bounds.size.width,
                if focused.is_some() {
                    240.0
                } else {
                    content_bounds.size.height
                },
            );
            let columns = exaterm_ui::layout::battlefield_columns(
                cards.len(),
                content_bounds.size.width as i32,
                focused.is_some(),
            ) as usize;
            let embeds = focused.is_none()
                && exaterm_ui::layout::battlefield_can_embed_terminals(
                    cards.len(),
                    columns.max(1),
                    content_bounds.size.width as i32,
                    content_bounds.size.height as i32,
                );
            let mut embedded_ids = BTreeSet::new();
            if embeds {
                for card in &cards {
                    embedded_ids.insert(card.id);
                }
            }
            layout_views(
                &content_view,
                &timer_battlefield_view,
                &timer_focus_panel,
                focused,
            );
            battlefield_view::set_battlefield_data(
                cards.clone(),
                selected,
                Rc::clone(&render_state),
                embedded_ids.clone(),
                focused.is_some(),
            );
            focus_view::set_focus_data(
                focused.and_then(|session_id| borrowed.focus_render_data(session_id)),
                Rc::clone(&render_state),
            );
            timer_battlefield_view.setNeedsDisplay(true);
            timer_focus_panel.setNeedsDisplay(true);
            apply_terminal_layout(
                &timer_surfaces.borrow(),
                &timer_battlefield_view,
                &timer_focus_panel,
                &cards,
                &card_rects,
                &embedded_ids,
                focused,
            );
            if let Some(session_id) = focused {
                if let Some(surface) = timer_surfaces.borrow().get(&session_id) {
                    battlefield_window.makeFirstResponder(Some(&*surface.view));
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

    // Set up keyboard event monitoring.
    // In focus mode, SwiftTerm handles input as first responder — we only intercept
    // Escape (exit focus) and Cmd+N (add shell). In battlefield mode, we handle navigation.
    let key_state = Rc::clone(&state);
    let key_window = main_window.clone();
    let key_surfaces = Rc::clone(&terminal_surfaces);
    let key_block = block2::StackBlock::new(
        move |event: std::ptr::NonNull<objc2_app_kit::NSEvent>| -> *mut objc2_app_kit::NSEvent {
            // SAFETY: The event pointer is valid for the duration of this callback.
            let event_ref = unsafe { event.as_ref() };
            let key_code = event_ref.keyCode();
            let flags = event_ref.modifierFlags();

            let modifiers = key_map::Modifiers {
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
                            key_state
                                .borrow_mut()
                                .workspace
                                .enter_focus_mode(session_id);
                            if let Some(surface) = key_surfaces.borrow().get(&session_id) {
                                key_window.makeFirstResponder(Some(&*surface.view));
                            }
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
    std::mem::forget(sync_inputs_enabled);
    std::mem::forget(terminal_surfaces);

    app.run();
}

#[cfg(target_os = "macos")]
struct TerminalSurface {
    bridge: Rc<exaterm_swiftterm::TerminalBridge>,
    view: objc2::rc::Retained<objc2_app_kit::NSView>,
}

#[cfg(target_os = "macos")]
fn terminal_appearance() -> exaterm_swiftterm::TerminalAppearance {
    let terminal_font = exaterm_ui::theme::terminal_font();
    exaterm_swiftterm::TerminalAppearance {
        font_name: style::font_family(&terminal_font).to_string(),
        font_size: terminal_font.size as f64,
        foreground: exaterm_ui::theme::terminal_foreground_color(),
        background: exaterm_ui::theme::terminal_background_color(),
        cursor: exaterm_ui::theme::terminal_cursor_color(),
    }
}

#[cfg(target_os = "macos")]
fn ensure_terminal_surfaces(
    surfaces: &mut BTreeMap<exaterm_types::model::SessionId, TerminalSurface>,
    sessions: &[exaterm_types::model::SessionRecord],
    ios: &Rc<RefCell<session_io::SessionIOMap>>,
    beachhead: &Rc<exaterm_ui::beachhead::BeachheadConnection>,
    sync_inputs_enabled: Arc<AtomicBool>,
) {
    let active_ids: BTreeSet<_> = sessions.iter().map(|session| session.id).collect();
    surfaces.retain(|id, _| active_ids.contains(id));
    for session in sessions {
        surfaces.entry(session.id).or_insert_with(|| {
            let bridge = Rc::new(exaterm_swiftterm::TerminalBridge::new(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(640.0, 360.0),
            )));
            bridge.set_appearance(&terminal_appearance());
            let session_id = session.id;
            let ios = Rc::clone(ios);
            let sync = sync_inputs_enabled.clone();
            bridge.set_input_handler(move |bytes: &[u8]| {
                if sync.load(std::sync::atomic::Ordering::Relaxed) {
                    ios.borrow_mut().write_input_all(bytes);
                } else {
                    ios.borrow_mut().write_input(&session_id, bytes);
                }
            });
            let commands = beachhead.commands().clone();
            bridge.set_size_handler(move |size| {
                let _ = commands.send(exaterm_types::proto::ClientMessage::ResizeTerminal {
                    session_id,
                    rows: size.rows,
                    cols: size.cols,
                });
            });
            let view = bridge.view();
            view.setAutoresizingMask(
                objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                    | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            TerminalSurface { bridge, view }
        });
    }
}

#[cfg(target_os = "macos")]
fn layout_views(
    content_view: &objc2_app_kit::NSView,
    battlefield_view: &battlefield_view::BattlefieldView,
    focus_panel: &focus_view::FocusView,
    focused: Option<exaterm_types::model::SessionId>,
) {
    const FOCUS_RAIL_HEIGHT: f64 = 240.0;

    let frame = content_view.frame();
    if focused.is_some() {
        battlefield_view.setHidden(false);
        focus_panel.setHidden(false);
        let rail_height = frame.size.height.min(FOCUS_RAIL_HEIGHT);
        let focus_height = (frame.size.height - rail_height).max(0.0);
        battlefield_view.setFrame(NSRect::new(
            NSPoint::new(0.0, focus_height),
            NSSize::new(frame.size.width, rail_height),
        ));
        focus_panel.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(frame.size.width, focus_height),
        ));
    } else {
        battlefield_view.setHidden(false);
        focus_panel.setHidden(true);
        battlefield_view.setFrame(frame);
    }
}

#[cfg(target_os = "macos")]
fn apply_terminal_layout(
    surfaces: &BTreeMap<exaterm_types::model::SessionId, TerminalSurface>,
    battlefield_view: &battlefield_view::BattlefieldView,
    focus_panel: &focus_view::FocusView,
    cards: &[app_state::CardRenderData],
    rects: &[exaterm_ui::layout::CardRect],
    embedded_ids: &BTreeSet<exaterm_types::model::SessionId>,
    focused: Option<exaterm_types::model::SessionId>,
) {
    for (session_id, surface) in surfaces {
        surface.view.removeFromSuperview();
        surface.view.setHidden(true);

        if focused == Some(*session_id) {
            let slot = exaterm_ui::layout::focus_terminal_slot_rect(
                focus_panel.frame().size.width as i32,
                focus_panel.frame().size.height as i32,
            );
            focus_panel.addSubview(&surface.view);
            surface.view.setFrame(NSRect::new(
                NSPoint::new(slot.x, slot.y),
                NSSize::new(slot.w, slot.h),
            ));
            surface.view.setHidden(false);
            continue;
        }

        if embedded_ids.contains(session_id) {
            if let Some((_, rect)) = cards
                .iter()
                .zip(rects.iter())
                .find(|(card, _)| card.id == *session_id)
            {
                let slot = exaterm_ui::layout::card_terminal_slot_rect(rect);
                battlefield_view.addSubview(&surface.view);
                surface.view.setFrame(NSRect::new(
                    NSPoint::new(slot.x, slot.y),
                    NSSize::new(slot.w, slot.h),
                ));
                surface.view.setHidden(false);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn present_startup_error(mtm: objc2::MainThreadMarker, error: &str) {
    use objc2_app_kit::{NSAlert, NSAlertStyle};
    use objc2_foundation::NSString;

    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Critical);
    let message = NSString::from_str("Exaterm could not start a live beachhead connection.");
    let info = NSString::from_str(error);
    alert.setMessageText(&message);
    alert.setInformativeText(&info);
    alert.runModal();
}
