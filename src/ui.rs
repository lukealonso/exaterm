use crate::demo::WorkspaceBlueprint;
use crate::model::{
    ProbeLens, ProbeMode, SessionId, SessionLaunch, SessionStatus, WorkspaceState,
};
use crate::procfs::format_process_tree;
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use vte::prelude::*;
use vte4 as vte;

const TILE_COLUMNS: usize = 2;
const APP_ID: &str = "io.exaterm.Exaterm";

#[derive(Clone)]
struct SessionTileWidgets {
    root: gtk::Frame,
    status: gtk::Label,
    detail: gtk::Label,
    terminal: vte::Terminal,
    _probe: TileProbeWidgets,
}

#[derive(Clone)]
struct TileProbeWidgets {
    root: gtk::Frame,
    title: gtk::Label,
    source: gtk::Label,
    stack: gtk::Stack,
    output: gtk::TextView,
    events: gtk::TextView,
    process: gtk::TextView,
    output_button: gtk::Button,
    events_button: gtk::Button,
    process_button: gtk::Button,
    pin: gtk::Button,
    close: gtk::Button,
}

struct AppContext {
    state: Rc<RefCell<WorkspaceState>>,
    overlay: gtk::Overlay,
    grid: gtk::Grid,
    title: adw::WindowTitle,
    tiles: RefCell<BTreeMap<SessionId, SessionTileWidgets>>,
    probe: TileProbeWidgets,
}

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        adw::init().expect("libadwaita should initialize");
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    });
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &gtk::Application) {
    load_css();

    let grid = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .hexpand(true)
        .vexpand(true)
        .build();

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .hexpand(true)
        .vexpand(true)
        .build();

    let overlay = gtk::Overlay::builder()
        .child(&scroller)
        .hexpand(true)
        .vexpand(true)
        .build();
    let probe = build_tile_probe();
    overlay.add_overlay(&probe.root);

    let context = Rc::new(AppContext {
        state: Rc::new(RefCell::new(WorkspaceState::new())),
        overlay,
        grid,
        title: adw::WindowTitle::new("Exaterm", "Grid-first, detail-on-demand"),
        tiles: RefCell::new(BTreeMap::new()),
        probe,
    });

    {
        let button = context.probe.close.clone();
        let context = context.clone();
        button.connect_clicked(move |_| close_probe(&context));
    }
    {
        let button = context.probe.pin.clone();
        let context = context.clone();
        button.connect_clicked(move |_| toggle_probe_pin(&context));
    }
    {
        let button = context.probe.output_button.clone();
        let context = context.clone();
        button.connect_clicked(move |_| set_probe_lens(&context, ProbeLens::Output));
    }
    {
        let button = context.probe.events_button.clone();
        let context = context.clone();
        button.connect_clicked(move |_| set_probe_lens(&context, ProbeLens::Events));
    }
    {
        let button = context.probe.process_button.clone();
        let context = context.clone();
        button.connect_clicked(move |_| set_probe_lens(&context, ProbeLens::Process));
    }

    let add_shell_button = gtk::Button::builder()
        .label("Add Shell")
        .css_classes(vec!["pill".to_string()])
        .tooltip_text("Open a new generic command session")
        .build();

    let header = adw::HeaderBar::builder()
        .title_widget(&context.title)
        .show_end_title_buttons(true)
        .build();
    header.pack_end(&add_shell_button);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&context.overlay);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Exaterm")
        .default_width(1440)
        .default_height(920)
        .content(&body)
        .build();

    {
        let context = context.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                if let Some(session_id) = context.state.borrow().selected_session() {
                    activate_terminal_session(&context, session_id);
                    return glib::Propagation::Stop;
                }
            }

            if key == gdk::Key::Escape && context.state.borrow().open_probe().is_some() {
                close_probe(&context);
                return glib::Propagation::Stop;
            }

            if matches!(key.to_unicode(), Some('p' | 'P')) {
                toggle_probe_for_selection(&context);
                return glib::Propagation::Stop;
            }

            if matches!(key.to_unicode(), Some('1')) {
                set_probe_lens(&context, ProbeLens::Output);
                return glib::Propagation::Stop;
            }

            if matches!(key.to_unicode(), Some('2')) {
                set_probe_lens(&context, ProbeLens::Events);
                return glib::Propagation::Stop;
            }

            if matches!(key.to_unicode(), Some('3')) {
                set_probe_lens(&context, ProbeLens::Process);
                return glib::Propagation::Stop;
            }

            if matches!(key.to_unicode(), Some('f' | 'F')) {
                toggle_probe_pin(&context);
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    {
        let context = context.clone();
        add_shell_button.connect_clicked(move |_| {
            let number = context.state.borrow().sessions().len() + 1;
            let launch = WorkspaceBlueprint::add_shell(number);
            append_session_tile(&context, launch);
        });
    }

    {
        let context = context.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(750), move || {
            if let Some(probe) = context.state.borrow().open_probe() {
                refresh_probe_snapshot(&context, probe.session_id);
            }
            glib::ControlFlow::Continue
        });
    }

    for launch in WorkspaceBlueprint::demo().sessions {
        append_session_tile(&context, launch);
    }
    refresh_window_title(&context);
    refresh_tile_styles(&context);

    window.present();
}

fn append_session_tile(context: &Rc<AppContext>, launch: SessionLaunch) {
    let session_id = context.state.borrow_mut().add_session(launch);
    let session = context
        .state
        .borrow()
        .sessions()
        .iter()
        .find(|session| session.id == session_id)
        .cloned()
        .expect("newly added session must exist");

    let tile = build_tile(context, session_id, &session.launch, session.status);

    let index = context.tiles.borrow().len();
    let (column, row) = WorkspaceState::tile_position(index, TILE_COLUMNS);
    context.grid.attach(&tile.root, column, row, 1, 1);
    context.tiles.borrow_mut().insert(session_id, tile.clone());

    update_tile_labels(context, session_id);
    refresh_window_title(context);
    refresh_tile_styles(context);
    spawn_session(context, session_id, &session.launch, &tile.terminal);
}

fn build_tile(
    context: &Rc<AppContext>,
    session_id: SessionId,
    launch: &SessionLaunch,
    status: SessionStatus,
) -> SessionTileWidgets {
    let title = gtk::Label::builder()
        .label(&launch.name)
        .xalign(0.0)
        .css_classes(vec!["tile-title".to_string()])
        .build();
    let subtitle = gtk::Label::builder()
        .label(&launch.subtitle)
        .xalign(0.0)
        .css_classes(vec!["tile-subtitle".to_string()])
        .build();
    let status_label = build_status_label(status);
    let detail = gtk::Label::builder()
        .label(launch.status_hint(status))
        .xalign(0.0)
        .css_classes(vec!["tile-footnote".to_string()])
        .build();
    let peek_button = gtk::Button::builder()
        .label("Peek · P")
        .css_classes(vec!["peek-button".to_string()])
        .tooltip_text("Open a tile-local probe for this session, or press P")
        .build();

    let title_stack = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    title_stack.append(&title);
    title_stack.append(&subtitle);

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();
    header.append(&title_stack);
    header.append(&peek_button);
    header.append(&status_label);

    let terminal = vte::Terminal::builder()
        .scroll_on_output(true)
        .scroll_on_keystroke(true)
        .input_enabled(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal.set_scrollback_lines(10_000);
    terminal.add_css_class("terminal-surface");

    let terminal_wrapper = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal_wrapper.append(&terminal);

    let probe = build_tile_probe();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.append(&header);
    content.append(&terminal_wrapper);
    content.append(&detail);

    let overlay = gtk::Overlay::builder()
        .child(&content)
        .hexpand(true)
        .vexpand(true)
        .build();
    overlay.add_overlay(&probe.root);

    let root = gtk::Frame::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&overlay)
        .build();
    root.add_css_class("session-tile");
    install_activate_click(&title, context, session_id);
    install_activate_click(&subtitle, context, session_id);
    install_activate_click(&status_label, context, session_id);
    install_activate_click(&detail, context, session_id);

    let terminal_click = gtk::GestureClick::new();
    {
        let context = context.clone();
        terminal_click.connect_pressed(move |_, _, _, _| {
            activate_terminal_session(&context, session_id);
        });
    }
    terminal.add_controller(terminal_click);

    {
        let context = context.clone();
        peek_button.connect_clicked(move |_| open_probe(&context, session_id));
    }

    {
        let context = context.clone();
        probe.close.connect_clicked(move |_| close_probe(&context));
    }

    {
        let context = context.clone();
        probe.pin.connect_clicked(move |_| toggle_probe_pin(&context));
    }

    {
        let context = context.clone();
        probe.output_button
            .connect_clicked(move |_| set_probe_lens(&context, ProbeLens::Output));
    }

    {
        let context = context.clone();
        probe.events_button
            .connect_clicked(move |_| set_probe_lens(&context, ProbeLens::Events));
    }

    {
        let context = context.clone();
        probe.process_button
            .connect_clicked(move |_| set_probe_lens(&context, ProbeLens::Process));
    }

    {
        let context = context.clone();
        terminal.connect_notify_local(Some("has-focus"), move |term, _| {
            {
                let mut state = context.state.borrow_mut();
                if term.has_focus() {
                    state.activate_session(session_id);
                } else if state.focused_terminal() == Some(session_id) {
                    state.set_terminal_focus(None);
                }
            }
            refresh_tile_styles(&context);
        });
    }

    SessionTileWidgets {
        root,
        status: status_label,
        detail,
        terminal,
        _probe: probe,
    }
}

fn build_status_label(status: SessionStatus) -> gtk::Label {
    gtk::Label::builder()
        .label(status.chip_label())
        .css_classes(vec![
            "status-chip".to_string(),
            status.css_class().to_string(),
        ])
        .build()
}

fn build_tile_probe() -> TileProbeWidgets {
    let title = gtk::Label::builder()
        .label("Output Probe")
        .xalign(0.0)
        .css_classes(vec!["probe-title".to_string()])
        .build();
    let source = gtk::Label::builder()
        .label("No session selected")
        .xalign(0.0)
        .css_classes(vec!["probe-source".to_string()])
        .build();
    let output_button = gtk::Button::builder()
        .label("Output")
        .css_classes(vec!["probe-lens".to_string()])
        .build();
    let events_button = gtk::Button::builder()
        .label("Events")
        .css_classes(vec!["probe-lens".to_string()])
        .build();
    let process_button = gtk::Button::builder()
        .label("Process")
        .css_classes(vec!["probe-lens".to_string()])
        .build();
    let pin = gtk::Button::builder()
        .label("Pin")
        .css_classes(vec!["probe-pin".to_string()])
        .build();
    let close = gtk::Button::builder()
        .label("X")
        .css_classes(vec!["probe-close".to_string()])
        .build();
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();

    let title_stack = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    title_stack.append(&title);
    title_stack.append(&source);
    let lenses = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    lenses.append(&output_button);
    lenses.append(&events_button);
    lenses.append(&process_button);
    title_stack.append(&lenses);
    header.append(&title_stack);
    header.append(&pin);
    header.append(&close);

    let output = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let events = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let process = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(12)
        .bottom_margin(12)
        .left_margin(12)
        .right_margin(12)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_titled(&output, Some("output"), "Output");
    stack.add_titled(&events, Some("events"), "Events");
    stack.add_titled(&process, Some("process"), "Process");
    let scroller = gtk::ScrolledWindow::builder()
        .child(&stack)
        .min_content_height(180)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&header);
    content.append(&scroller);

    let root = gtk::Frame::builder()
        .child(&content)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(56)
        .margin_end(12)
        .width_request(340)
        .height_request(260)
        .visible(false)
        .build();
    root.add_css_class("probe-surface");

    TileProbeWidgets {
        root,
        title,
        source,
        stack,
        output,
        events,
        process,
        output_button,
        events_button,
        process_button,
        pin,
        close,
    }
}

fn spawn_session(
    context: &Rc<AppContext>,
    session_id: SessionId,
    launch: &SessionLaunch,
    terminal: &vte::Terminal,
) {
    let argv_owned = launch.argv();
    let argv: Vec<&str> = argv_owned.iter().map(String::as_str).collect();
    let envv: [&str; 0] = [];
    let cwd = launch.cwd.as_ref().and_then(|path| path.to_str());

    {
        let context = context.clone();
        terminal.connect_child_exited(move |_, exit_code| {
            context
                .state
                .borrow_mut()
                .mark_exited(session_id, exit_code);
            update_tile_labels(&context, session_id);
            refresh_window_title(&context);
            refresh_tile_styles(&context);
            if context
                .state
                .borrow()
                .open_probe()
                .map(|probe| probe.session_id)
                == Some(session_id)
            {
                refresh_probe_snapshot(&context, session_id);
            }
        });
    }

    {
        let context = context.clone();
        terminal.spawn_async(
            vte::PtyFlags::DEFAULT,
            cwd,
            &argv,
            &envv,
            glib::SpawnFlags::SEARCH_PATH,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| {
                match result {
                    Ok(pid) => {
                        context
                            .state
                            .borrow_mut()
                            .mark_spawned(session_id, pid.0 as u32);
                    }
                    Err(error) => {
                        eprintln!("failed to spawn session {session_id:?}: {error}");
                        context.state.borrow_mut().mark_exited(session_id, -1);
                    }
                }
                update_tile_labels(&context, session_id);
                refresh_window_title(&context);
                refresh_tile_styles(&context);
                if context
                    .state
                    .borrow()
                    .open_probe()
                    .map(|probe| probe.session_id)
                    == Some(session_id)
                {
                    refresh_probe_snapshot(&context, session_id);
                }
            },
        );
    }
}

fn update_tile_labels(context: &Rc<AppContext>, session_id: SessionId) {
    let Some(tile) = context.tiles.borrow().get(&session_id).cloned() else {
        return;
    };
    let (status, hint) = {
        let state = context.state.borrow();
        let Some(session) = state
            .sessions()
            .iter()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        (session.status, session.launch.status_hint(session.status))
    };

    tile.status.set_label(&status.chip_label());
    tile.status.remove_css_class("status-launching");
    tile.status.remove_css_class("status-running");
    tile.status.remove_css_class("status-waiting");
    tile.status.remove_css_class("status-blocked");
    tile.status.remove_css_class("status-failed");
    tile.status.remove_css_class("status-complete");
    tile.status.remove_css_class("status-detached");
    tile.status.add_css_class(status.css_class());
    tile.detail.set_label(&hint);
}

fn refresh_window_title(context: &Rc<AppContext>) {
    let state = context.state.borrow();
    let total = state.sessions().len();
    let attention = state
        .sessions()
        .iter()
        .filter(|session| session.status.needs_attention())
        .count();
    let running = state
        .sessions()
        .iter()
        .filter(|session| matches!(session.status, SessionStatus::Running))
        .count();
    let waiting = state
        .sessions()
        .iter()
        .filter(|session| matches!(session.status, SessionStatus::Waiting))
        .count();
    context.title.set_subtitle(&format!(
        "{total} sessions · {attention} attention · {running} running · {waiting} waiting"
    ));
}

fn refresh_tile_styles(context: &Rc<AppContext>) {
    let state = context.state.borrow();
    let open_probe = state.open_probe();
    for (session_id, tile) in context.tiles.borrow().iter() {
        tile.root.remove_css_class("selected");
        tile.root.remove_css_class("terminal-focused");
        tile.root.remove_css_class("probe-source");
        if state.selected_session() == Some(*session_id) {
            tile.root.add_css_class("selected");
        }
        if state.focused_terminal() == Some(*session_id) {
            tile.root.add_css_class("terminal-focused");
        }
        if open_probe.map(|probe| probe.session_id) == Some(*session_id) {
            tile.root.add_css_class("probe-source");
        }
    }
    context.probe.root.set_visible(open_probe.is_some());
}

fn open_probe(context: &Rc<AppContext>, session_id: SessionId) {
    context.state.borrow_mut().show_probe(session_id);
    refresh_probe_snapshot(context, session_id);
    refresh_tile_styles(context);
}

fn close_probe(context: &Rc<AppContext>) {
    context.state.borrow_mut().close_probe();
    refresh_tile_styles(context);
}

fn toggle_probe_for_selection(context: &Rc<AppContext>) {
    let selected = context.state.borrow().selected_session();
    let current_probe = context.state.borrow().open_probe();
    match selected {
        Some(session_id) if current_probe.map(|probe| probe.session_id) == Some(session_id) => {
            close_probe(context)
        }
        Some(session_id) => open_probe(context, session_id),
        None => {}
    }
}

fn refresh_probe_snapshot(context: &Rc<AppContext>, session_id: SessionId) {
    let Some(tile) = context.tiles.borrow().get(&session_id).cloned() else {
        return;
    };
    let state = context.state.borrow();
    let Some(session) = state.session(session_id).cloned() else {
        return;
    };
    let Some(probe) = state.open_probe().filter(|probe| probe.session_id == session_id) else {
        context.probe.root.set_visible(false);
        return;
    };

    let rows = tile.terminal.row_count();
    let cols = tile.terminal.column_count();
    let output_snapshot = if rows > 0 && cols > 0 {
        let (text, _) =
            tile.terminal
                .text_range_format(vte::Format::Text, 0, 0, rows - 1, cols - 1);
        text.map(|text| text.to_string())
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "No visible terminal output yet.".into())
    } else {
        "Terminal buffer is not ready yet.".into()
    };

    let event_snapshot = if session.events.is_empty() {
        "No supervision events recorded yet.".into()
    } else {
        session
            .events
            .iter()
            .rev()
            .map(|event| format!("#{:02} {}", event.sequence, event.summary))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let process_snapshot = session
        .pid
        .map(|pid| {
            format_process_tree(pid)
                .unwrap_or_else(|error| format!("Process tree unavailable: {error}"))
        })
        .unwrap_or_else(|| "Main process has already exited.".into());

    context.probe.title.set_label(&format!(
        "{} {} Probe",
        probe.lens.title(),
        match probe.mode {
            ProbeMode::Peek => "Peek",
            ProbeMode::Pinned => "Pinned",
        }
    ));
    context.probe.source.set_label(&format!(
        "{} · {} · {}",
        session.launch.name,
        session.status.chip_label(),
        match probe.mode {
            ProbeMode::Peek => "temporary watch",
            ProbeMode::Pinned => "pinned watch",
        }
    ));
    context.probe.pin.set_label(probe.mode.action_label());
    context.probe.output.buffer().set_text(&output_snapshot);
    context.probe.events.buffer().set_text(&event_snapshot);
    context.probe.process.buffer().set_text(&process_snapshot);
    context.probe.stack.set_visible_child_name(match probe.lens {
        ProbeLens::Output => "output",
        ProbeLens::Events => "events",
        ProbeLens::Process => "process",
    });
    apply_lens_button_state(&context.probe, probe.lens);
    update_probe_position(context, &tile);
}

fn set_probe_lens(context: &Rc<AppContext>, lens: ProbeLens) {
    if let Some(session_id) = context.state.borrow().open_probe().map(|probe| probe.session_id) {
        context.state.borrow_mut().set_probe_lens(lens);
        refresh_probe_snapshot(context, session_id);
        refresh_tile_styles(context);
    }
}

fn toggle_probe_pin(context: &Rc<AppContext>) {
    if let Some(session_id) = context.state.borrow().open_probe().map(|probe| probe.session_id) {
        context.state.borrow_mut().toggle_probe_pin();
        refresh_probe_snapshot(context, session_id);
        refresh_tile_styles(context);
    }
}

fn apply_lens_button_state(probe: &TileProbeWidgets, active_lens: ProbeLens) {
    for button in [&probe.output_button, &probe.events_button, &probe.process_button] {
        button.remove_css_class("probe-lens-active");
    }
    match active_lens {
        ProbeLens::Output => probe.output_button.add_css_class("probe-lens-active"),
        ProbeLens::Events => probe.events_button.add_css_class("probe-lens-active"),
        ProbeLens::Process => probe.process_button.add_css_class("probe-lens-active"),
    }
}

fn update_probe_position(context: &Rc<AppContext>, tile: &SessionTileWidgets) {
    let Some(bounds) = tile.root.compute_bounds(&context.overlay) else {
        return;
    };
    let overlay_width = context.overlay.width();
    let probe_width = context.probe.root.width_request().max(340);
    let desired_x = if bounds.x() < (overlay_width as f32 / 2.0) {
        bounds.x() + bounds.width() - (probe_width as f32 * 0.55)
    } else {
        bounds.x() - (probe_width as f32 * 0.15)
    };
    let max_x = (overlay_width - probe_width - 12).max(12);
    let clamped_x = desired_x.round() as i32;
    let margin_start = clamped_x.clamp(12, max_x);
    let margin_top = (bounds.y().round() as i32 + 56).max(24);

    context.probe.root.set_halign(gtk::Align::Start);
    context.probe.root.set_valign(gtk::Align::Start);
    context.probe.root.set_margin_start(margin_start);
    context.probe.root.set_margin_top(margin_top);
}

fn activate_terminal_session(context: &Rc<AppContext>, session_id: SessionId) {
    {
        let mut state = context.state.borrow_mut();
        state.activate_session(session_id);
    }
    if let Some(tile) = context.tiles.borrow().get(&session_id) {
        tile.terminal.grab_focus();
    }
    refresh_tile_styles(context);
}

fn install_activate_click<W: IsA<gtk::Widget>>(
    widget: &W,
    context: &Rc<AppContext>,
    session_id: SessionId,
) {
    let click = gtk::GestureClick::new();
    {
        let context = context.clone();
        click.connect_pressed(move |_, _, _, _| {
            activate_terminal_session(&context, session_id);
        });
    }
    widget.add_controller(click);
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "
        window {
            background: linear-gradient(180deg, #0a111a 0%, #0f1622 100%);
        }

        .session-tile {
            border-radius: 18px;
            border: 1px solid rgba(160, 174, 192, 0.18);
            background: rgba(12, 19, 30, 0.96);
            box-shadow: 0 18px 36px rgba(0, 0, 0, 0.28);
        }

        .session-tile.selected {
            border-color: rgba(119, 198, 255, 0.85);
            box-shadow: 0 0 0 1px rgba(119, 198, 255, 0.9), 0 20px 44px rgba(11, 92, 161, 0.28);
        }

        .session-tile.terminal-focused {
            border-color: rgba(255, 189, 89, 0.85);
            box-shadow: 0 0 0 1px rgba(255, 189, 89, 0.95), 0 22px 46px rgba(255, 189, 89, 0.18);
        }

        .session-tile.probe-source {
            border-color: rgba(144, 230, 189, 0.95);
            box-shadow: 0 0 0 1px rgba(144, 230, 189, 0.9), 0 24px 52px rgba(33, 84, 56, 0.28);
        }

        .tile-title {
            font-weight: 700;
            font-size: 15px;
            color: #f8fafc;
        }

        .tile-subtitle {
            color: rgba(226, 232, 240, 0.72);
            font-size: 12px;
        }

        .tile-footnote {
            color: rgba(148, 163, 184, 0.82);
            font-size: 11px;
            letter-spacing: 0.04em;
            margin: 8px 12px 10px 12px;
        }

        .status-chip {
            border-radius: 999px;
            padding: 4px 10px;
            font-weight: 700;
            font-size: 11px;
        }

        .status-launching {
            background: rgba(96, 165, 250, 0.18);
            color: #93c5fd;
        }

        .status-running {
            background: rgba(74, 222, 128, 0.18);
            color: #86efac;
        }

        .status-waiting {
            background: rgba(226, 232, 240, 0.16);
            color: #e2e8f0;
        }

        .status-blocked {
            background: rgba(251, 191, 36, 0.18);
            color: #fde68a;
        }

        .status-failed {
            background: rgba(248, 113, 113, 0.18);
            color: #fca5a5;
        }

        .status-complete {
            background: rgba(94, 234, 212, 0.16);
            color: #99f6e4;
        }

        .status-detached {
            background: rgba(192, 132, 252, 0.18);
            color: #e9d5ff;
        }

        .pill, .peek-button, .probe-close, .probe-pin, .probe-lens {
            border-radius: 999px;
            padding: 6px 14px;
        }

        .pill {
            background: rgba(119, 198, 255, 0.16);
            color: #dbeafe;
        }

        .peek-button {
            background: rgba(144, 230, 189, 0.18);
            color: #d1fae5;
        }

        .probe-pin {
            background: rgba(96, 165, 250, 0.16);
            color: #dbeafe;
        }

        .probe-lens {
            background: rgba(148, 163, 184, 0.12);
            color: #cbd5e1;
            font-size: 11px;
        }

        .probe-lens-active {
            background: rgba(144, 230, 189, 0.2);
            color: #d1fae5;
        }

        .probe-close {
            background: rgba(226, 232, 240, 0.14);
            color: #e2e8f0;
        }

        .probe-surface {
            border-radius: 18px;
            border: 1px solid rgba(144, 230, 189, 0.85);
            background: rgba(7, 13, 20, 0.97);
            box-shadow: 0 24px 56px rgba(0, 0, 0, 0.42);
        }

        .probe-title {
            font-weight: 700;
            font-size: 14px;
            color: #f8fafc;
        }

        .probe-source {
            color: rgba(167, 243, 208, 0.86);
            font-size: 12px;
        }

        textview, textview text {
            background: transparent;
            color: #e2e8f0;
        }

        terminal {
            border-top: 1px solid rgba(148, 163, 184, 0.08);
            border-bottom-left-radius: 14px;
            border-bottom-right-radius: 14px;
            padding: 10px;
        }
        ",
    );

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display should exist during app startup"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
