use crate::beachhead::BeachheadConnection;
use crate::style::{configure_app_icons, load_css};
use crate::terminal_adapter::{
    attach_display_runtime, enable_terminal_image_support, measured_terminal_size_hint,
    spawn_daemon_display_bridge, spawn_runtime, terminal_display_capabilities, terminal_size_hint,
    ClientDisplayRuntime,
};
use crate::widgets::{GroupCardWidgets, SessionCardWidgets};
use exaterm_core::config::{apply_app_config_environment, load_app_config};
use exaterm_core::model::{
    blocking_prompt_launch, planning_stream_launch, running_stream_launch, ssh_shell_launch,
    user_shell_launch,
};
use exaterm_core::observation::{
    apply_stream_update, refresh_observation as refresh_session_observation, SessionObservation,
};
use exaterm_core::runtime::{RuntimeEvent, SessionRuntime};
use exaterm_types::model::{
    GroupId, SessionId, SessionLaunch, SessionRecord, SupervisedGroupRecord, WorkspaceItem,
};
use exaterm_types::proto::{
    ClientMessage, InputSyncScope, ObservationSnapshot, ServerMessage, WorkspaceSnapshot,
};
use exaterm_ui::beachhead::{parse_run_mode, BeachheadTarget, ParsedArgs, RunMode};
use exaterm_ui::layout::{compute_tiling, GridTiling};
use exaterm_ui::workspace_view::WorkspaceViewState;
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use portable_pty::PtySize;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::time::{Duration, Instant};
use vte::prelude::*;
use vte4 as vte;

pub(crate) const APP_ID: &str = "io.exaterm.Exaterm";
const TERMINATOR_AMBIENCE_FOREGROUND: &str = "#ffffff";
const TERMINATOR_AMBIENCE_BACKGROUND: &str = "#000000";
const TERMINATOR_AMBIENCE_PALETTE: [&str; 16] = [
    "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf",
    "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GroupAssessment {
    Watching,
    Active,
    Stalling,
    Blocked,
    Complete,
}

impl GroupAssessment {
    fn label(self) -> &'static str {
        match self {
            Self::Watching => "Watching",
            Self::Active => "Active",
            Self::Stalling => "Stalling",
            Self::Blocked => "Blocked",
            Self::Complete => "Complete",
        }
    }

    fn card_class(self) -> &'static str {
        match self {
            Self::Watching => "group-assessment-watching",
            Self::Active => "group-assessment-active",
            Self::Stalling => "group-assessment-stalling",
            Self::Blocked => "group-assessment-blocked",
            Self::Complete => "group-assessment-complete",
        }
    }

    fn status_class(self) -> &'static str {
        match self {
            Self::Watching => "group-status-watching",
            Self::Active => "group-status-active",
            Self::Stalling => "group-status-stalling",
            Self::Blocked => "group-status-blocked",
            Self::Complete => "group-status-complete",
        }
    }
}

const GROUP_ASSESSMENT_CARD_CLASSES: [&str; 5] = [
    "group-assessment-watching",
    "group-assessment-active",
    "group-assessment-stalling",
    "group-assessment-blocked",
    "group-assessment-complete",
];

const GROUP_STATUS_CLASSES: [&str; 5] = [
    "group-status-watching",
    "group-status-active",
    "group-status-stalling",
    "group-status-blocked",
    "group-status-complete",
];

pub(crate) struct AppContext {
    mode: RunMode,
    pub(crate) beachhead: Option<BeachheadConnection>,
    pub(crate) state: Rc<RefCell<WorkspaceViewState>>,
    title: adw::WindowTitle,
    back_to_root_button: gtk::Button,
    empty_state: gtk::Box,
    cards: gtk::Grid,
    battlefield_panel: gtk::ScrolledWindow,
    cached_tiling: RefCell<Option<GridTiling>>,
    cached_layout_items: RefCell<Vec<WorkspaceItem>>,
    expanded_group: Cell<Option<GroupId>>,
    pre_group_window_size: Cell<Option<(i32, i32)>>,
    terminal_assist_request_id: Cell<u64>,
    terminal_assist_active: RefCell<Option<TerminalAssistState>>,
    initial_terminal_focus_done: Cell<bool>,
    focused_terminal_id: Cell<Option<SessionId>>,
    focus_next_added_terminal: Cell<bool>,
    sync_inputs_enabled: Cell<bool>,
    pending_supervisor_visibility: RefCell<BTreeMap<GroupId, bool>>,
    terminal_audible_bell: bool,
    session_cards: RefCell<BTreeMap<SessionId, SessionCardWidgets>>,
    group_cards: RefCell<BTreeMap<GroupId, GroupCardWidgets>>,
    observations: RefCell<BTreeMap<SessionId, SessionObservation>>,
    raw_stream_socket_names: RefCell<BTreeMap<SessionId, String>>,
    pub(crate) runtimes: RefCell<BTreeMap<SessionId, SessionRuntime>>,
    display_runtimes: RefCell<BTreeMap<SessionId, ClientDisplayRuntime>>,
    closing_confirmed: Cell<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalAssistState {
    session_id: SessionId,
    request_id: Option<u64>,
}

pub fn run() -> glib::ExitCode {
    let argv = std::env::args().collect::<Vec<_>>();
    let parsed = match parse_run_mode(argv.iter().skip(1).cloned()) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("usage: exaterm [--ssh user@host] [--new <id> | --resume <id>]");
            return glib::ExitCode::from(2);
        }
    };

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|_| {
        adw::init().expect("libadwaita should initialize");
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
    });
    app.connect_activate(move |app| {
        if parsed.workspace.is_some() || visual_gallery_enabled() {
            launch_workspace(app, parsed.clone());
        } else {
            crate::launcher::present_launcher(app, parsed.clone());
        }
    });
    let program = argv
        .first()
        .cloned()
        .unwrap_or_else(|| "exaterm".to_string());
    app.run_with_args(&[program])
}

pub(crate) fn daemon_backed(context: &AppContext) -> bool {
    context.beachhead.is_some()
}

pub(crate) fn launch_workspace(app: &gtk::Application, parsed: ParsedArgs) {
    apply_app_config_environment(&load_app_config());
    match parsed.workspace.as_ref() {
        Some(workspace) => std::env::set_var("EXATERM_WORKSPACE", workspace.id()),
        None => std::env::remove_var("EXATERM_WORKSPACE"),
    }
    build_ui(app, parsed);
}

pub(crate) fn build_ui(app: &gtk::Application, parsed: ParsedArgs) {
    load_css();
    configure_app_icons(APP_ID);
    let mode = parsed.mode.clone();
    let workspace_id = parsed.workspace.as_ref().map(|w| w.id().to_string());
    let beachhead = if visual_gallery_enabled() {
        None
    } else {
        let target = BeachheadTarget::from_parsed(&parsed);
        match BeachheadConnection::connect(&target) {
            Ok(connection) => Some(connection),
            Err(error) => {
                present_startup_error(app, &error);
                return;
            }
        }
    };

    let cards = gtk::Grid::builder()
        .column_spacing(12)
        .row_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .column_homogeneous(true)
        .row_homogeneous(true)
        .valign(gtk::Align::Fill)
        .hexpand(true)
        .vexpand(true)
        .build();

    let battlefield_panel = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .child(&cards)
        .build();

    let empty_title = gtk::Label::builder()
        .label("No Live Sessions Yet")
        .xalign(0.5)
        .css_classes(vec!["empty-title".to_string()])
        .build();
    let empty_body = gtk::Label::builder()
        .label("Use Add Shell to start a real terminal-native agent or open an operator shell. Exaterm opens into an empty battlefield so the workspace begins with your own sessions.")
        .xalign(0.5)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .css_classes(vec!["empty-body".to_string()])
        .max_width_chars(68)
        .build();
    let empty_state = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .hexpand(true)
        .vexpand(true)
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .visible(false)
        .build();
    empty_state.add_css_class("empty-state");
    empty_state.append(&empty_title);
    empty_state.append(&empty_body);

    let window_title = match &workspace_id {
        Some(id) => format!("exaterm · {id}"),
        None => "exaterm".into(),
    };
    let title = adw::WindowTitle::new(&window_title, "");
    let header = adw::HeaderBar::builder()
        .title_widget(&title)
        .show_end_title_buttons(true)
        .build();

    let back_to_root_button = gtk::Button::builder()
        .label("All Terminals")
        .visible(false)
        .build();
    back_to_root_button.add_css_class("toolbar-add-button");
    let add_terminal_button = gtk::Button::builder().label("+ Add Terminal").build();
    add_terminal_button.add_css_class("toolbar-add-button");
    let supervise_group_button = gtk::Button::builder().label("Supervise Group").build();
    supervise_group_button.add_css_class("toolbar-add-button");

    let toolbar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(2)
        .halign(gtk::Align::Start)
        .build();
    toolbar.add_css_class("battlefield-toolbar");
    toolbar.append(&back_to_root_button);
    toolbar.append(&add_terminal_button);
    toolbar.append(&supervise_group_button);

    let sync_inputs_button = gtk::ToggleButton::builder()
        .label("Sync Inputs")
        .active(false)
        .build();
    sync_inputs_button.add_css_class("toolbar-toggle-button");
    toolbar.append(&sync_inputs_button);

    let content_root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    content_root.add_css_class("battlefield-root");
    content_root.append(&toolbar);
    content_root.append(&empty_state);
    content_root.append(&battlefield_panel);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&content_root);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("exaterm")
        .icon_name(APP_ID)
        .default_width(1480)
        .default_height(960)
        .content(&body)
        .build();

    let context = Rc::new(AppContext {
        mode: mode.clone(),
        beachhead,
        state: Rc::new(RefCell::new(WorkspaceViewState::new())),
        title,
        back_to_root_button,
        empty_state,
        cards,
        battlefield_panel,
        cached_tiling: RefCell::new(None),
        cached_layout_items: RefCell::new(Vec::new()),
        expanded_group: Cell::new(None),
        pre_group_window_size: Cell::new(None),
        terminal_assist_request_id: Cell::new(1),
        terminal_assist_active: RefCell::new(None),
        initial_terminal_focus_done: Cell::new(false),
        focused_terminal_id: Cell::new(None),
        focus_next_added_terminal: Cell::new(false),
        sync_inputs_enabled: Cell::new(false),
        pending_supervisor_visibility: RefCell::new(BTreeMap::new()),
        terminal_audible_bell: load_app_config().terminal.audible_bell,
        session_cards: RefCell::new(BTreeMap::new()),
        group_cards: RefCell::new(BTreeMap::new()),
        observations: RefCell::new(BTreeMap::new()),
        raw_stream_socket_names: RefCell::new(BTreeMap::new()),
        runtimes: RefCell::new(BTreeMap::new()),
        display_runtimes: RefCell::new(BTreeMap::new()),
        closing_confirmed: Cell::new(false),
    });

    {
        let button = context.back_to_root_button.clone();
        let context = context.clone();
        button.connect_clicked(move |_| {
            show_top_level_battlefield(&context);
        });
    }

    {
        let context = context.clone();
        add_terminal_button.connect_clicked(move |_| {
            add_terminal_from_toolbar(&context);
        });
    }

    {
        let context = context.clone();
        supervise_group_button.connect_clicked(move |_| {
            create_supervised_group_from_visible_sessions(&context);
        });
    }

    {
        let context = context.clone();
        sync_inputs_button.connect_toggled(move |button| {
            context.sync_inputs_enabled.set(button.is_active());
            sync_input_scope_with_daemon(&context);
            if button.is_active() {
                focus_sync_input_anchor(&context);
            }
            refresh_card_styles(&context);
        });
    }

    {
        let context = context.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, state| {
            if context.terminal_assist_active.borrow().is_some() {
                if matches!(key, gdk::Key::k | gdk::Key::K)
                    && state.contains(gdk::ModifierType::CONTROL_MASK)
                {
                    focus_active_terminal_assist(&context);
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }

            if matches!(key, gdk::Key::k | gdk::Key::K)
                && state.contains(gdk::ModifierType::CONTROL_MASK)
            {
                show_terminal_assist_prompt(&context);
                return glib::Propagation::Stop;
            }

            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                if focused_visible_terminal_session(&context).is_some() {
                    return glib::Propagation::Proceed;
                }
                if let Some(WorkspaceItem::Group(group_id)) =
                    context.state.borrow().selected_workspace_item()
                {
                    show_group_contents(&context, group_id);
                    return glib::Propagation::Stop;
                }
                let selected_session = context.state.borrow().selected_session();
                if let Some(session_id) = selected_session {
                    if battlefield_embeds_terminal(&context, session_id) {
                        if let Some(card) = context.session_cards.borrow().get(&session_id) {
                            if card.terminal.has_focus() {
                                return glib::Propagation::Proceed;
                            }
                        }
                        refresh_card_styles(&context);
                        focus_session_terminal(&context, session_id);
                    }
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    {
        let context = context.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            refresh_runtime_and_cards(&context);
            glib::ControlFlow::Continue
        });
    }

    if let Some(beachhead) = context.beachhead.as_ref() {
        let wake_fd = beachhead.event_wake_fd();
        let context = context.clone();
        glib::source::unix_fd_add_local(wake_fd, glib::IOCondition::IN, move |_, _| {
            if let Some(beachhead) = context.beachhead.as_ref() {
                beachhead.drain_event_wake();
            }
            drain_daemon_events(&context);
            glib::ControlFlow::Continue
        });
    }

    if visual_gallery_enabled() {
        seed_visual_gallery(&context);
    } else if let Some(beachhead) = context.beachhead.as_ref() {
        let _ = beachhead
            .commands()
            .send(ClientMessage::SetTerminalDisplayCapabilities {
                capabilities: terminal_display_capabilities(),
            });
        let _ = beachhead
            .commands()
            .send(ClientMessage::CreateOrResumeDefaultWorkspace);
    }

    refresh_runtime_and_cards(&context);
    refresh_workspace(&context);

    {
        let context = context.clone();
        let close_window = window.clone();
        close_window.clone().connect_close_request(move |_| {
            if context.closing_confirmed.get() || context.beachhead.is_none() {
                return glib::Propagation::Proceed;
            }
            if context.state.borrow().sessions().is_empty() {
                return glib::Propagation::Proceed;
            }

            let dialog = adw::AlertDialog::builder()
                .heading("Keep terminals alive?")
                .body("Closing Exaterm can keep these sessions running so you can reconnect to the same live terminals later.")
                .close_response("cancel")
                .build();
            dialog.add_responses(&[
                ("cancel", "Cancel"),
                ("terminate", "Terminate"),
                ("keep", "Keep Alive"),
            ]);
            dialog.set_default_response(Some("keep"));
            dialog.set_response_appearance("terminate", adw::ResponseAppearance::Destructive);
            let context = context.clone();
            let action_window = close_window.clone();
            let present_window = close_window.clone();
            dialog.connect_response(None, move |dialog: &adw::AlertDialog, response| {
                match response {
                    "keep" => {
                        if let Some(beachhead) = context.beachhead.as_ref() {
                            let _ = beachhead
                                .commands()
                                .send(ClientMessage::DetachClient { keep_alive: true });
                        }
                        context.closing_confirmed.set(true);
                        action_window.close();
                    }
                    "terminate" => {
                        if let Some(beachhead) = context.beachhead.as_ref() {
                            let _ = beachhead
                                .commands()
                                .send(ClientMessage::DetachClient { keep_alive: false });
                        }
                        context.closing_confirmed.set(true);
                        action_window.close();
                    }
                    _ => {}
                }
                dialog.close();
            });
            dialog.present(Some(&present_window));
            glib::Propagation::Stop
        });
    }

    window.present();
    maybe_focus_initial_terminal(&context);
}

fn parse_rgba(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).expect("valid theme color")
}

fn apply_terminal_theme(terminal: &vte::Terminal) {
    let foreground = parse_rgba(TERMINATOR_AMBIENCE_FOREGROUND);
    let background = parse_rgba(TERMINATOR_AMBIENCE_BACKGROUND);
    let palette = TERMINATOR_AMBIENCE_PALETTE
        .iter()
        .map(|color| parse_rgba(color))
        .collect::<Vec<_>>();
    let palette_refs = palette.iter().collect::<Vec<_>>();
    let cursor = parse_rgba("#f2f2f2");
    let highlight = parse_rgba("#2a2a2a");
    let highlight_foreground = parse_rgba("#ffffff");

    terminal.set_colors(Some(&foreground), Some(&background), &palette_refs);
    terminal.set_color_cursor(Some(&cursor));
    terminal.set_color_cursor_foreground(Some(&background));
    terminal.set_color_highlight(Some(&highlight));
    terminal.set_color_highlight_foreground(Some(&highlight_foreground));
}

fn present_startup_error(app: &gtk::Application, error: &str) {
    let message = gtk::Label::builder()
        .label(format!(
            "Exaterm could not start a host connection.\n\n{error}"
        ))
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .build();

    let close_button = gtk::Button::with_label("Close");
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("exaterm")
        .icon_name(APP_ID)
        .default_width(720)
        .default_height(220)
        .build();

    let title = adw::WindowTitle::new("exaterm", "Startup failed");
    let header = adw::HeaderBar::builder()
        .title_widget(&title)
        .show_end_title_buttons(true)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    content.append(&message);
    content.append(&close_button);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&content);
    window.set_content(Some(&body));

    let window_for_button = window.clone();
    close_button.connect_clicked(move |_| {
        window_for_button.close();
    });

    window.present();
}

fn default_shell_launch(context: &Rc<AppContext>, number: usize) -> SessionLaunch {
    match &context.mode {
        RunMode::Local => user_shell_launch(format!("Shell {number}"), "Generic command session"),
        RunMode::Ssh { target } => ssh_shell_launch(
            format!("SSH {number}"),
            format!("Remote session on {target}"),
            target.clone(),
        ),
    }
}

fn append_session_card(context: &Rc<AppContext>, launch: SessionLaunch) -> SessionId {
    append_session_card_with_spawn(context, launch, true)
}

fn append_session_card_with_spawn(
    context: &Rc<AppContext>,
    launch: SessionLaunch,
    should_spawn: bool,
) -> SessionId {
    let session_id = context.state.borrow_mut().add_session(launch);
    let session = context
        .state
        .borrow()
        .session(session_id)
        .cloned()
        .expect("new session should exist");

    let card = build_session_terminal_widgets(context, &session);
    context
        .session_cards
        .borrow_mut()
        .insert(session_id, card.clone());
    context
        .observations
        .borrow_mut()
        .insert(session_id, SessionObservation::new());

    sync_grid_layout(context);
    if should_spawn {
        spawn_session(context, session_id, &session.launch, &card.terminal);
    }
    session_id
}

fn visual_gallery_enabled() -> bool {
    std::env::var("EXATERM_VISUAL_GALLERY")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

fn seed_visual_gallery(context: &Rc<AppContext>) {
    let launches = vec![
        running_stream_launch(
            "Agent A",
            "Parser recovery",
            transcript_script(&[
                "• I found the next parser breakage: trailing tokens drop after the recovery path.",
                "• I’m patching src/parser.rs first, then rerunning the focused parser suite.",
                "$ cargo test parser_recovery -- --nocapture",
                "test parser::recovery::keeps_trailing_tokens ... FAILED",
                "• The failure narrowed to parse_recovery_tail; editing the transition now.",
                "$ cargo test parser_recovery -- --nocapture",
                "2 parser tests still failing",
            ]),
        ),
        planning_stream_launch(
            "Agent B",
            "Checkpointed UI pass",
            transcript_script(&[
                "• I fixed the stuck input path and the active terminal now accepts Return again.",
                "• Verified with cargo test plus a manual smoke pass.",
                "• Next I can tighten grid density and typography if you want me to keep going.",
                "• Current state is clean and ready for the next pass.",
                "› Continue",
                "• Larger typography is in and the terminal grid keeps context now.",
                "• Tests pass. Ready for the next instruction.",
            ]),
        ),
        blocking_prompt_launch(
            "Agent C",
            "Deploy approval",
            "The deploy script is ready, but this next step will touch production. Proceed with deploy? [y/N]",
        ),
        running_stream_launch(
            "Agent D",
            "GTK focus regression",
            transcript_script(&[
                "• I think the next failure is still the terminal handoff, so I’m trying another narrow fix.",
                "$ cargo test terminal_grid -- --nocapture",
                "error[E0599]: no method named present on TerminalHandle",
                "• That patch was wrong; I’m retrying with a different signal hookup.",
                "$ cargo test terminal_grid -- --nocapture",
                "error[E0599]: no method named present on TerminalHandle",
                "• Still wrong. I’m going to try another approach on the same path.",
                "$ cargo test terminal_grid -- --nocapture",
                "error[E0599]: no method named present on TerminalHandle",
            ]),
        ),
        planning_stream_launch(
            "Agent E",
            "Post-fix watch",
            transcript_script(&[
                "• I reran the last validation pass and it stayed green.",
                "• Stable. Standing by.",
                "• No new failures observed.",
                "• Stable. Standing by.",
                "• Still stable; waiting for the next instruction.",
                "• Stable. Standing by.",
            ]),
        ),
        planning_stream_launch(
            "Agent F",
            "Disk pressure",
            transcript_script(&[
                "npm ERR! nospc ENOSPC: no space left on device",
                "• I’m blocked on disk space and the build keeps failing immediately.",
                "$ du -sh ~/.cache ~/.cargo ~/.npm",
                "14G /home/luke/.cache",
                "• If this keeps up I may need to free space aggressively.",
                "• Worst case I could start deleting large directories unless you redirect me.",
                "$ rm -rf /home/luke/old-home-backup",
                "rm: cannot remove '/home/luke/old-home-backup': No such file or directory",
                "• I’m frustrated enough to start deleting large directories unless you want to redirect me.",
            ]),
        ),
    ];

    for launch in launches {
        append_session_card_with_spawn(context, launch, true);
    }
}

fn transcript_script(lines: &[&str]) -> String {
    let quoted = lines
        .iter()
        .map(|line| {
            let escaped = line.replace('\'', r"'\''");
            format!("printf '%s\\n' '{escaped}'; sleep 0.25")
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{quoted}; exec sleep 600")
}

fn build_session_terminal_widgets(
    context: &Rc<AppContext>,
    session: &SessionRecord,
) -> SessionCardWidgets {
    let terminal_slot = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal_slot.add_css_class("card-terminal-slot");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(8)
        .margin_end(8)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.append(&terminal_slot);

    let frame = gtk::Frame::builder()
        .child(&content)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .build();
    frame.add_css_class("terminal-tile");
    frame.add_css_class("terminal-card");
    frame.set_focusable(true);

    {
        let context = context.clone();
        let session_id = session.id;
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_released(move |_, _, _, _| {
            context.state.borrow_mut().select_session(session_id);
            refresh_card_styles(&context);
            focus_session_terminal(&context, session_id);
        });
        frame.add_controller(click);
    }

    // Drag-to-reorder: DragSource on the card frame so it works even
    // when the header is hidden in unadorned/sparse mode.
    {
        let drag_source = gtk::DragSource::new();
        drag_source.set_actions(gdk::DragAction::MOVE);
        let session_id = session.id;
        drag_source.connect_prepare(move |_source, _x, _y| {
            let value = session_id.0.to_value();
            Some(gdk::ContentProvider::for_value(&value))
        });
        frame.add_controller(drag_source);
    }

    // DropTarget on the card frame to accept reorder drops.
    {
        let context = context.clone();
        let target_session_id = session.id;
        let drop_target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
        drop_target.connect_drop(move |_target, value, _x, _y| {
            let Ok(source_raw) = value.get::<u32>() else {
                return false;
            };
            let source_id = SessionId(source_raw);
            if source_id == target_session_id {
                return false;
            }
            let target_index = context
                .state
                .borrow()
                .ordered_session_ids()
                .iter()
                .position(|id| *id == target_session_id)
                .unwrap_or(0);
            context
                .state
                .borrow_mut()
                .move_session(source_id, target_index);
            *context.cached_tiling.borrow_mut() = None;
            sync_grid_layout(&context);
            refresh_card_styles(&context);
            true
        });
        frame.add_controller(drop_target);
    }

    let terminal = vte::Terminal::builder()
        .audible_bell(context.terminal_audible_bell)
        .scroll_on_output(false)
        .scroll_on_keystroke(true)
        .input_enabled(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    enable_terminal_image_support(&terminal);
    apply_terminal_theme(&terminal);
    terminal.set_scrollback_lines(100_000);
    terminal.connect_selection_changed(|terminal| {
        if terminal.has_selection() {
            terminal.copy_clipboard_format(vte::Format::Text);
        }
    });
    let terminal_dim_overlay = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .visible(true)
        .build();
    terminal_dim_overlay.add_css_class("terminal-dim-overlay");
    terminal_dim_overlay.set_can_target(false);
    terminal_dim_overlay.set_focusable(false);
    {
        let enter_context = context.clone();
        let dim_overlay_for_enter = terminal_dim_overlay.clone();
        let session_id = session.id;
        let terminal_focus = gtk::EventControllerFocus::new();
        terminal_focus.connect_enter(move |_| {
            {
                let mut state = enter_context.state.borrow_mut();
                state.select_session(session_id);
            }
            enter_context.focused_terminal_id.set(Some(session_id));
            dim_overlay_for_enter.set_visible(false);
            refresh_card_styles(&enter_context);
        });
        let leave_context = context.clone();
        let dim_overlay_for_leave = terminal_dim_overlay.clone();
        terminal_focus.connect_leave(move |_| {
            if leave_context.focused_terminal_id.get() == Some(session_id) {
                leave_context.focused_terminal_id.set(None);
            }
            dim_overlay_for_leave.set_visible(true);
            refresh_card_styles(&leave_context);
        });
        terminal.add_controller(terminal_focus);
    }
    let terminal_assist_status = gtk::Label::builder()
        .label("Ask for a terminal command")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["terminal-assist-status".to_string()])
        .build();
    terminal_assist_status.set_wrap(true);
    let terminal_assist_entry = gtk::Entry::builder()
        .placeholder_text("Find how much disk space I'm using")
        .hexpand(true)
        .build();
    terminal_assist_entry.add_css_class("terminal-assist-entry");
    let terminal_assist_spinner = gtk::Spinner::builder().visible(false).build();
    let terminal_assist_cancel = gtk::Button::builder().label("Cancel").build();
    terminal_assist_cancel.add_css_class("terminal-assist-cancel");

    let terminal_assist_action_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    terminal_assist_action_row.append(&terminal_assist_entry);
    terminal_assist_action_row.append(&terminal_assist_spinner);
    terminal_assist_action_row.append(&terminal_assist_cancel);

    let terminal_assist_panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    terminal_assist_panel.add_css_class("terminal-assist-panel");
    terminal_assist_panel.append(&terminal_assist_status);
    terminal_assist_panel.append(&terminal_assist_action_row);

    let terminal_assist_overlay = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .visible(false)
        .build();
    terminal_assist_overlay.add_css_class("terminal-assist-overlay");
    terminal_assist_overlay.set_can_target(true);
    terminal_assist_overlay.set_focusable(true);
    terminal_assist_overlay.append(&terminal_assist_panel);

    {
        let context = context.clone();
        let session_id = session.id;
        terminal_assist_entry.connect_activate(move |entry| {
            submit_terminal_assist_query(&context, session_id, entry.text().trim().to_string());
        });
    }
    {
        let context = context.clone();
        let session_id = session.id;
        terminal_assist_cancel.connect_clicked(move |_| {
            cancel_terminal_assist(&context, session_id);
        });
    }
    {
        let context = context.clone();
        let session_id = session.id;
        let assist_keys = gtk::EventControllerKey::new();
        assist_keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                cancel_terminal_assist(&context, session_id);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        terminal_assist_entry.add_controller(assist_keys);
    }
    {
        let context = context.clone();
        let session_id = session.id;
        let assist_keys = gtk::EventControllerKey::new();
        assist_keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                cancel_terminal_assist(&context, session_id);
            }
            glib::Propagation::Stop
        });
        terminal_assist_overlay.add_controller(assist_keys);
    }
    {
        let entry = terminal_assist_entry.clone();
        let overlay_click = gtk::GestureClick::new();
        overlay_click.set_button(1);
        overlay_click.connect_pressed(move |_, _, _, _| {
            if entry.is_sensitive() {
                entry.grab_focus();
            }
        });
        terminal_assist_overlay.add_controller(overlay_click);
    }

    terminal.add_css_class("terminal-surface");
    let terminal_view = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&terminal)
        .build();
    terminal_view.add_css_class("terminal-scroll");
    let terminal_overlay = gtk::Overlay::builder()
        .child(&terminal_view)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal_overlay.add_overlay(&terminal_dim_overlay);
    terminal_overlay.add_overlay(&terminal_assist_overlay);
    terminal_slot.append(&terminal_overlay);
    install_terminal_context_menu(context, &terminal, session.id);
    {
        let terminal_for_keys = terminal.clone();
        let paste_keys = gtk::EventControllerKey::new();
        paste_keys.connect_key_pressed(move |_, key, _, state| {
            if matches!(key, gdk::Key::v | gdk::Key::V)
                && state.contains(gdk::ModifierType::CONTROL_MASK)
            {
                terminal_for_keys.paste_clipboard();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        terminal.add_controller(paste_keys);
    }

    SessionCardWidgets {
        frame,
        terminal_slot,
        terminal_overlay,
        terminal_dim_overlay,
        terminal_assist_overlay,
        terminal_assist_entry,
        terminal_assist_status,
        terminal_assist_spinner,
        terminal_assist_cancel,
        terminal,
    }
}

fn build_group_card_widgets(
    context: &Rc<AppContext>,
    group: &SupervisedGroupRecord,
) -> GroupCardWidgets {
    let title = gtk::Label::builder()
        .label(&group.name)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["card-title".to_string()])
        .build();
    title.set_single_line_mode(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(42);

    let subtitle = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["group-subtitle".to_string()])
        .build();
    subtitle.set_single_line_mode(true);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle.set_max_width_chars(60);

    let status = gtk::Label::builder()
        .label("Watching")
        .xalign(0.5)
        .css_classes(vec!["card-status".to_string()])
        .build();

    let supervisor_toggle = gtk::ToggleButton::builder()
        .label("Supervisor")
        .active(group.supervisor_visible)
        .sensitive(group.supervisor_session_id.is_some())
        .build();
    supervisor_toggle.add_css_class("toolbar-toggle-button");
    let supervisor_toggle_updating = Rc::new(Cell::new(false));

    let header_left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .hexpand(true)
        .build();
    header_left.add_css_class("card-title-stack");
    header_left.append(&title);
    header_left.append(&subtitle);

    let header_right = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .build();
    header_right.add_css_class("card-status-stack");
    header_right.append(&status);
    header_right.append(&supervisor_toggle);

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    header.add_css_class("card-header-row");
    header.append(&header_left);
    header.append(&header_right);

    let summary_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(9)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(14)
        .margin_end(14)
        .hexpand(true)
        .vexpand(true)
        .build();
    summary_content.add_css_class("group-summary-content");
    let rendered_summary_markdown = Rc::new(RefCell::new(String::new()));

    let summary_view = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(&summary_content)
        .build();
    summary_view.add_css_class("group-summary-frame");

    let terminal_slot = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal_slot.add_css_class("card-terminal-slot");

    let middle_stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    middle_stack.add_named(&summary_view, Some("summary"));
    middle_stack.add_named(&terminal_slot, Some("terminal"));
    middle_stack.set_visible_child_name("summary");

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.append(&header);
    content.append(&middle_stack);

    let frame = gtk::Frame::builder()
        .child(&content)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .build();
    frame.add_css_class("supervised-group-card");
    frame.set_focusable(true);

    {
        let context = context.clone();
        let group_id = group.id;
        let frame_for_click = frame.clone();
        let supervisor_toggle_for_click = supervisor_toggle.clone();
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_released(move |_, _, x, y| {
            if point_inside_widget(&frame_for_click, &supervisor_toggle_for_click, x, y) {
                return;
            }
            context
                .state
                .borrow_mut()
                .select_workspace_item(WorkspaceItem::Group(group_id));
            let supervisor_session = {
                let state = context.state.borrow();
                state.group(group_id).and_then(|group| {
                    if display_supervisor_visible(&context, group) {
                        group.supervisor_session_id
                    } else {
                        None
                    }
                })
            };
            if let Some(supervisor_session) = supervisor_session {
                refresh_card_styles(&context);
                focus_session_terminal(&context, supervisor_session);
                return;
            }
            show_group_contents(&context, group_id);
        });
        frame.add_controller(click);
    }

    {
        let context = context.clone();
        let group_id = group.id;
        let updating = supervisor_toggle_updating.clone();
        supervisor_toggle.connect_toggled(move |button| {
            if updating.get() {
                return;
            }
            let visible = button.is_active();
            context
                .pending_supervisor_visibility
                .borrow_mut()
                .insert(group_id, visible);
            if let Some(beachhead) = context.beachhead.as_ref() {
                let _ = beachhead
                    .commands()
                    .send(ClientMessage::SetGroupSupervisorVisible { group_id, visible });
            }
            if let Some(group) = context.state.borrow().group(group_id).cloned() {
                update_group_card_widgets(&context, &group);
            }
            sync_terminal_parents(&context);
            refresh_card_styles(&context);
            schedule_runtime_size_sync(&context);
        });
    }

    GroupCardWidgets {
        frame,
        title,
        subtitle,
        status,
        summary_content,
        rendered_summary_markdown,
        supervisor_toggle,
        supervisor_toggle_updating,
        middle_stack,
        summary_view,
        terminal_slot,
    }
}

fn update_group_card_widgets(context: &Rc<AppContext>, group: &SupervisedGroupRecord) {
    let Some(card) = context.group_cards.borrow().get(&group.id).cloned() else {
        return;
    };
    let supervisor_visible = {
        let mut pending = context.pending_supervisor_visibility.borrow_mut();
        if pending.get(&group.id).copied() == Some(group.supervisor_visible) {
            pending.remove(&group.id);
        }
        pending
            .get(&group.id)
            .copied()
            .unwrap_or(group.supervisor_visible)
    };

    let assessment = group_assessment_from_markdown(&group.summary_markdown);
    apply_group_assessment_style(&card, assessment);
    card.title.set_label(&group.name);
    card.subtitle.set_label(&group_subtitle(group));
    card.status.set_label(assessment.label());
    card.supervisor_toggle
        .set_sensitive(group.supervisor_session_id.is_some());
    if card.supervisor_toggle.is_active() != supervisor_visible {
        card.supervisor_toggle_updating.set(true);
        card.supervisor_toggle.set_active(supervisor_visible);
        card.supervisor_toggle_updating.set(false);
    }
    card.supervisor_toggle.set_label(if supervisor_visible {
        "Summary"
    } else {
        "Supervisor"
    });

    let summary_markdown = group.summary_markdown.trim().to_string();
    if *card.rendered_summary_markdown.borrow() != summary_markdown {
        render_group_markdown(&card.summary_content, &summary_markdown);
        *card.rendered_summary_markdown.borrow_mut() = summary_markdown;
    }
    card.summary_view.set_visible(!supervisor_visible);
    card.middle_stack
        .set_visible_child_name(if supervisor_visible {
            "terminal"
        } else {
            "summary"
        });
}

fn apply_group_assessment_style(card: &GroupCardWidgets, assessment: GroupAssessment) {
    for class in GROUP_ASSESSMENT_CARD_CLASSES {
        card.frame.remove_css_class(class);
    }
    card.frame.add_css_class(assessment.card_class());
    for class in GROUP_STATUS_CLASSES {
        card.status.remove_css_class(class);
    }
    card.status.add_css_class(assessment.status_class());
}

fn group_subtitle(group: &SupervisedGroupRecord) -> String {
    let mut parts = vec![format!(
        "{} worker terminal{}",
        group.member_session_ids.len(),
        if group.member_session_ids.len() == 1 {
            ""
        } else {
            "s"
        }
    )];
    if let Some(age) = group.summary_age_secs {
        parts.push(format!("updated {}", format_age(age)));
    } else if let Some(age) = group.latest_action_age_secs {
        parts.push(format!("action {}", format_age(age)));
    }
    if let Some(goal) = group.goal.as_deref().filter(|goal| !goal.trim().is_empty()) {
        parts.push(format!("goal: {}", goal.trim()));
    }
    parts.join(" · ")
}

fn group_assessment_from_markdown(markdown: &str) -> GroupAssessment {
    let text = markdown.to_ascii_lowercase();
    if text.trim().is_empty() || text.contains("supervisor is starting") {
        return GroupAssessment::Watching;
    }

    if contains_any(
        &text,
        &[
            "complete",
            "completed",
            "done",
            "finished",
            "ready",
            "tests pass",
            "all pass",
        ],
    ) {
        return GroupAssessment::Complete;
    }

    if indicates_overall_blocked(&text) {
        return GroupAssessment::Blocked;
    }

    if indicates_stalling(&text) {
        return GroupAssessment::Stalling;
    }

    if indicates_active_work(&text) {
        return GroupAssessment::Active;
    }
    GroupAssessment::Watching
}

fn indicates_overall_blocked(text: &str) -> bool {
    if contains_any(
        text,
        &[
            "not blocked",
            "no blocked",
            "no blocker",
            "no blockers",
            "blocked: none",
            "unblocked",
            "no human intervention",
            "no operator intervention",
        ],
    ) {
        return false;
    }

    contains_any(
        text,
        &[
            "overall: blocked",
            "overall blocked",
            "group is blocked",
            "group blocked",
            "blocked overall",
            "substantial proportion blocked",
            "substantial share blocked",
            "many workers blocked",
            "most workers blocked",
            "majority blocked",
            "all workers blocked",
            "all agents blocked",
            "most agents blocked",
            "majority of agents",
            "substantial proportion of the agents",
            "no worker can proceed",
            "no workers can proceed",
            "cannot proceed at all",
            "can't proceed at all",
            "cannot make useful progress",
            "needs human",
            "needs operator",
            "human intervention",
            "operator intervention",
            "requires human",
            "requires operator",
            "waiting for human",
            "waiting for operator",
            "needs approval",
            "requires approval",
            "external blocker",
            "external dependency",
        ],
    )
}

fn indicates_stalling(text: &str) -> bool {
    contains_any(
        text,
        &[
            "stalling",
            "stalled",
            "stopped",
            "stuck",
            "blocked",
            "no forward progress",
            "no meaningful progress",
            "not making progress",
            "circular",
            "looping",
            "repeated loop",
            "same failure",
            "idle",
            "no output",
            "no activity",
            "waiting for",
            "needs nudge",
            "prod sent",
            "despite supervisor",
            "despite prods",
        ],
    )
}

fn indicates_active_work(text: &str) -> bool {
    contains_any(
        text,
        &[
            "active",
            "working",
            "running",
            "progress",
            "implementing",
            "testing",
            "building",
            "reviewing",
            "debugging",
            "investigating",
            "fixing",
            "retrying",
            "rerunning",
            "editing",
            "patching",
            "others continue",
            "can continue",
            "useful work",
        ],
    )
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn render_group_markdown(container: &gtk::Box, markdown: &str) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let lines = markdown.trim().lines().collect::<Vec<_>>();
    if lines.is_empty() {
        container.append(&markdown_label(
            "Waiting for supervisor summary.",
            &["markdown-muted"],
        ));
        return;
    }

    let mut index = 0;
    let mut in_code_block = false;
    let mut code_lines = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_code_block {
                container.append(&markdown_code_block(&code_lines.join("\n")));
                code_lines.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
            }
            index += 1;
            continue;
        }

        if in_code_block {
            code_lines.push(line);
            index += 1;
            continue;
        }

        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if let Some((headers, rows, consumed)) = parse_markdown_table(&lines[index..]) {
            container.append(&markdown_table(&headers, &rows));
            index += consumed;
            continue;
        }

        if let Some(heading) = trimmed.strip_prefix("### ") {
            container.append(&markdown_markup_label(
                &format!("<b>{}</b>", markdown_inline_to_pango(heading)),
                &["markdown-heading", "markdown-heading-small"],
            ));
        } else if let Some(heading) = trimmed.strip_prefix("## ") {
            container.append(&markdown_markup_label(
                &format!("<b>{}</b>", markdown_inline_to_pango(heading)),
                &["markdown-heading"],
            ));
        } else if let Some(heading) = trimmed.strip_prefix("# ") {
            container.append(&markdown_markup_label(
                &format!("<b>{}</b>", markdown_inline_to_pango(heading)),
                &["markdown-heading"],
            ));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            container.append(&markdown_markup_label(
                &format!("- {}", markdown_inline_to_pango(item)),
                &["markdown-list-item"],
            ));
        } else {
            container.append(&markdown_markup_label(
                &markdown_inline_to_pango(trimmed),
                &["markdown-paragraph"],
            ));
        }
        index += 1;
    }

    if !code_lines.is_empty() {
        container.append(&markdown_code_block(&code_lines.join("\n")));
    }
}

fn parse_markdown_table(lines: &[&str]) -> Option<(Vec<String>, Vec<Vec<String>>, usize)> {
    if lines.len() < 2 {
        return None;
    }
    let header = parse_markdown_table_row(lines[0])?;
    let separator = parse_markdown_table_row(lines[1])?;
    if !separator
        .iter()
        .all(|cell| is_markdown_table_separator(cell))
    {
        return None;
    }

    let mut rows = Vec::new();
    let mut consumed = 2;
    while let Some(row) = lines
        .get(consumed)
        .and_then(|line| parse_markdown_table_row(line))
    {
        rows.push(row);
        consumed += 1;
    }
    Some((header, rows, consumed))
}

fn parse_markdown_table_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let trimmed = trimmed.trim_matches('|');
    let cells = trimmed
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect::<Vec<_>>();
    (cells.len() >= 2).then_some(cells)
}

fn is_markdown_table_separator(cell: &str) -> bool {
    let mut hyphens = 0;
    for ch in cell.chars() {
        match ch {
            '-' => hyphens += 1,
            ':' | ' ' => {}
            _ => return false,
        }
    }
    hyphens >= 3
}

fn markdown_table(headers: &[String], rows: &[Vec<String>]) -> gtk::Grid {
    let table = gtk::Grid::builder()
        .row_spacing(0)
        .column_spacing(0)
        .hexpand(true)
        .build();
    table.add_css_class("markdown-table");
    for (col, header) in headers.iter().enumerate() {
        let label = markdown_markup_label(
            &format!("<b>{}</b>", markdown_inline_to_pango(header)),
            &["markdown-table-cell", "markdown-table-header"],
        );
        table.attach(&label, col as i32, 0, 1, 1);
    }
    for (row, cells) in rows.iter().enumerate() {
        for col in 0..headers.len().max(cells.len()) {
            let text = cells.get(col).map(String::as_str).unwrap_or("");
            let label =
                markdown_markup_label(&markdown_inline_to_pango(text), &["markdown-table-cell"]);
            table.attach(&label, col as i32, (row + 1) as i32, 1, 1);
        }
    }
    table
}

fn markdown_label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .selectable(false)
        .hexpand(true)
        .build();
    label.set_halign(gtk::Align::Fill);
    label.set_valign(gtk::Align::Start);
    label.set_ellipsize(gtk::pango::EllipsizeMode::None);
    for class in classes {
        label.add_css_class(class);
    }
    label
}

fn markdown_markup_label(markup: &str, classes: &[&str]) -> gtk::Label {
    let label = markdown_label("", classes);
    label.set_markup(markup);
    label
}

fn markdown_code_block(code: &str) -> gtk::Label {
    markdown_markup_label(
        &format!("<tt>{}</tt>", glib::markup_escape_text(code).as_str()),
        &["markdown-code-block"],
    )
}

fn markdown_inline_to_pango(text: &str) -> String {
    let mut output = String::new();
    let mut segment = String::new();
    let mut in_code = false;
    for ch in text.chars() {
        if ch == '`' {
            let escaped = glib::markup_escape_text(&segment);
            if in_code {
                output.push_str("<tt>");
                output.push_str(escaped.as_str());
                output.push_str("</tt>");
            } else {
                output.push_str(escaped.as_str());
            }
            segment.clear();
            in_code = !in_code;
        } else {
            segment.push(ch);
        }
    }
    let escaped = glib::markup_escape_text(&segment);
    if in_code {
        output.push('`');
        output.push_str(escaped.as_str());
    } else {
        output.push_str(escaped.as_str());
    }
    output
}

fn format_age(age_secs: u64) -> String {
    if age_secs < 60 {
        format!("{age_secs}s ago")
    } else if age_secs < 60 * 60 {
        format!("{}m ago", age_secs / 60)
    } else {
        format!("{}h ago", age_secs / 3600)
    }
}

fn install_terminal_context_menu(
    context: &Rc<AppContext>,
    terminal: &vte::Terminal,
    source_session: SessionId,
) {
    let actions = gtk::gio::SimpleActionGroup::new();

    let copy_action = gtk::gio::SimpleAction::new("copy", None);
    {
        let terminal = terminal.clone();
        copy_action.connect_activate(move |_, _| {
            terminal.copy_clipboard_format(vte::Format::Text);
        });
    }
    actions.add_action(&copy_action);

    let paste_action = gtk::gio::SimpleAction::new("paste", None);
    {
        let terminal = terminal.clone();
        paste_action.connect_activate(move |_, _| {
            terminal.paste_clipboard();
        });
    }
    actions.add_action(&paste_action);

    let close_session_action = gtk::gio::SimpleAction::new("close_session", None);
    {
        let context = context.clone();
        close_session_action.connect_activate(move |_, _| {
            close_session(&context, source_session);
        });
    }
    actions.add_action(&close_session_action);

    terminal.insert_action_group("terminal", Some(&actions));

    let menu = gtk::gio::Menu::new();
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_has_arrow(false);
    popover.set_autohide(true);
    popover.set_halign(gtk::Align::Start);
    popover.set_valign(gtk::Align::Start);
    popover.set_parent(terminal);
    popover.set_position(gtk::PositionType::Bottom);
    popover.add_css_class("menu");
    popover.add_css_class("context-menu");

    let right_click = gtk::GestureClick::new();
    right_click.set_button(3);
    {
        let terminal = terminal.clone();
        let copy_action = copy_action.clone();
        let menu = menu.clone();
        let popover = popover.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            copy_action.set_enabled(terminal.has_selection());
            menu.remove_all();
            menu.append(Some("Copy"), Some("terminal.copy"));
            menu.append(Some("Paste"), Some("terminal.paste"));
            menu.append(Some("Close Session"), Some("terminal.close_session"));
            let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.set_offset(0, 0);
            popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    terminal.add_controller(right_click);
}

fn close_session(context: &Rc<AppContext>, session_id: SessionId) {
    if daemon_backed(context) {
        if let Some(beachhead) = context.beachhead.as_ref() {
            let _ = beachhead
                .commands()
                .send(ClientMessage::CloseSession { session_id });
        }
        return;
    }

    // Local (gallery) mode: remove the session directly.
    if let Some(card) = context.session_cards.borrow_mut().remove(&session_id) {
        context.cards.remove(&card.frame);
    }
    context.observations.borrow_mut().remove(&session_id);
    context.runtimes.borrow_mut().remove(&session_id);
    context.display_runtimes.borrow_mut().remove(&session_id);
    context.state.borrow_mut().remove_session(session_id);

    // Select a neighbor if the closed session was selected.
    let sessions = context.state.borrow().sessions().to_vec();
    if !sessions.is_empty() && context.state.borrow().selected_session().is_none() {
        context.state.borrow_mut().select_session(sessions[0].id);
    }

    sync_grid_layout(context);
    refresh_card_styles(context);
    refresh_workspace(context);
}

fn add_terminal_from_toolbar(context: &Rc<AppContext>) {
    if daemon_backed(context) {
        if let Some(beachhead) = context.beachhead.as_ref() {
            context.focus_next_added_terminal.set(true);
            if let Some(source) = context.state.borrow().sessions().first() {
                let _ = beachhead.commands().send(ClientMessage::AddTerminals {
                    source_session: source.id,
                });
            } else {
                let _ = beachhead
                    .commands()
                    .send(ClientMessage::CreateOrResumeDefaultWorkspace);
            }
            focus_selected_visible_terminal(context);
        }
        return;
    }

    let idx = context.state.borrow().sessions().len();
    let launch = default_shell_launch(context, idx + 1)
        .with_env("EXATERM_IDX", idx.to_string())
        .with_env("EXATERM_IDX_1", (idx + 1).to_string());
    let session_id = append_session_card(context, launch);
    context.state.borrow_mut().select_session(session_id);
    refresh_runtime_and_cards(context);
    refresh_workspace(context);
    refresh_card_styles(context);
    focus_session_terminal(context, session_id);
}

fn create_supervised_group_from_visible_sessions(context: &Rc<AppContext>) {
    if !daemon_backed(context) {
        return;
    }
    let items = battlefield_workspace_items(context);
    let state = context.state.borrow();
    let session_ids = items
        .into_iter()
        .filter_map(|item| match item {
            WorkspaceItem::Session(session_id) if state.session(session_id).is_some() => {
                Some(session_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if session_ids.is_empty() {
        return;
    }
    if let Some(beachhead) = context.beachhead.as_ref() {
        let _ = beachhead
            .commands()
            .send(ClientMessage::CreateSupervisedGroup {
                name: "Supervised Group".into(),
                session_ids,
                goal: None,
            });
    }
}

fn show_terminal_assist_prompt(context: &Rc<AppContext>) {
    if !daemon_backed(context) {
        return;
    }
    if focus_active_terminal_assist(context) {
        return;
    }
    let Some(session_id) = active_terminal_assist_session(context) else {
        return;
    };
    if !terminal_visible_for_focus(context, session_id) {
        return;
    }
    let Some(card) = context.session_cards.borrow().get(&session_id).cloned() else {
        return;
    };

    context.state.borrow_mut().select_session(session_id);
    context
        .terminal_assist_active
        .replace(Some(TerminalAssistState {
            session_id,
            request_id: None,
        }));
    card.terminal.set_input_enabled(false);
    card.terminal_assist_entry.set_text("");
    card.terminal_assist_entry.set_sensitive(true);
    card.terminal_assist_cancel.set_sensitive(true);
    card.terminal_assist_status
        .set_label("Ask for a terminal command");
    card.terminal_assist_spinner.stop();
    card.terminal_assist_spinner.set_visible(false);
    card.terminal_assist_overlay.set_visible(true);
    refresh_card_styles(context);
    card.terminal_assist_entry.grab_focus();
}

fn active_terminal_assist_session(context: &Rc<AppContext>) -> Option<SessionId> {
    focused_visible_terminal_session(context).or_else(|| context.state.borrow().selected_session())
}

fn focus_active_terminal_assist(context: &Rc<AppContext>) -> bool {
    let Some(active) = *context.terminal_assist_active.borrow() else {
        return false;
    };
    let Some(card) = context
        .session_cards
        .borrow()
        .get(&active.session_id)
        .cloned()
    else {
        return false;
    };
    if active.request_id.is_some() {
        card.terminal_assist_overlay.grab_focus();
    } else {
        card.terminal_assist_entry.grab_focus();
    }
    true
}

fn submit_terminal_assist_query(context: &Rc<AppContext>, session_id: SessionId, text: String) {
    let Some(active) = *context.terminal_assist_active.borrow() else {
        return;
    };
    if active.session_id != session_id || active.request_id.is_some() {
        return;
    }
    if text.trim().is_empty() {
        cancel_terminal_assist(context, session_id);
        return;
    }

    let request_id = context.terminal_assist_request_id.get();
    context
        .terminal_assist_request_id
        .set(request_id.saturating_add(1));
    context
        .terminal_assist_active
        .replace(Some(TerminalAssistState {
            session_id,
            request_id: Some(request_id),
        }));

    if let Some(card) = context.session_cards.borrow().get(&session_id).cloned() {
        card.terminal.set_input_enabled(false);
        card.terminal_assist_entry.set_sensitive(false);
        card.terminal_assist_cancel.set_sensitive(false);
        card.terminal_assist_status
            .set_label("Finding a command...");
        card.terminal_assist_spinner.set_visible(true);
        card.terminal_assist_spinner.start();
        card.terminal_assist_overlay.grab_focus();
    }

    let sent = context.beachhead.as_ref().is_some_and(|beachhead| {
        beachhead
            .commands()
            .send(ClientMessage::RequestTerminalAssist {
                request_id,
                session_id,
                prompt: text,
            })
            .is_ok()
    });
    if !sent {
        terminal_assist_failed(
            context,
            request_id,
            session_id,
            "Terminal assist is unavailable",
        );
    }
}

fn cancel_terminal_assist(context: &Rc<AppContext>, session_id: SessionId) {
    let Some(active) = *context.terminal_assist_active.borrow() else {
        return;
    };
    if active.session_id != session_id {
        return;
    }
    if active.request_id.is_some() {
        if let Some(card) = context.session_cards.borrow().get(&session_id) {
            card.terminal_assist_status
                .set_label("Finding a command...");
            card.terminal_assist_overlay.grab_focus();
        }
        return;
    }
    hide_terminal_assist(context, session_id, true);
}

fn handle_terminal_assist_completed(
    context: &Rc<AppContext>,
    request_id: u64,
    session_id: SessionId,
    inserted: bool,
    error: Option<String>,
) {
    let active = *context.terminal_assist_active.borrow();
    if active
        != Some(TerminalAssistState {
            session_id,
            request_id: Some(request_id),
        })
    {
        if let Some(error) = error {
            eprintln!("terminal assist {request_id} failed for {session_id:?}: {error}");
        } else if !inserted {
            eprintln!("terminal assist {request_id} produced no insertion for {session_id:?}");
        }
        return;
    }

    if let Some(error) = error {
        terminal_assist_failed(context, request_id, session_id, &error);
    } else if !inserted {
        terminal_assist_failed(context, request_id, session_id, "No command was inserted");
    } else {
        hide_terminal_assist(context, session_id, true);
    }
}

fn terminal_assist_failed(
    context: &Rc<AppContext>,
    _request_id: u64,
    session_id: SessionId,
    message: &str,
) {
    context
        .terminal_assist_active
        .replace(Some(TerminalAssistState {
            session_id,
            request_id: None,
        }));
    if let Some(card) = context.session_cards.borrow().get(&session_id).cloned() {
        card.terminal.set_input_enabled(false);
        card.terminal_assist_entry.set_sensitive(true);
        card.terminal_assist_cancel.set_sensitive(true);
        card.terminal_assist_status.set_label(message);
        card.terminal_assist_spinner.stop();
        card.terminal_assist_spinner.set_visible(false);
        card.terminal_assist_overlay.set_visible(true);
        card.terminal_assist_entry.grab_focus();
    }
}

fn hide_terminal_assist(context: &Rc<AppContext>, session_id: SessionId, focus_terminal: bool) {
    if context
        .terminal_assist_active
        .borrow()
        .is_some_and(|active| active.session_id == session_id)
    {
        context.terminal_assist_active.replace(None);
    }
    let Some(card) = context.session_cards.borrow().get(&session_id).cloned() else {
        return;
    };
    card.terminal_assist_overlay.set_visible(false);
    card.terminal_assist_spinner.stop();
    card.terminal_assist_spinner.set_visible(false);
    card.terminal_assist_entry.set_sensitive(true);
    card.terminal_assist_cancel.set_sensitive(true);
    card.terminal.set_input_enabled(true);
    if focus_terminal && terminal_visible_for_focus(context, session_id) {
        focus_session_terminal(context, session_id);
    }
}

fn show_group_contents(context: &Rc<AppContext>, group_id: GroupId) {
    if context.state.borrow().group(group_id).is_none() {
        return;
    }
    remember_pre_group_window_size(context);
    context.expanded_group.set(Some(group_id));
    context.cards.set_column_homogeneous(true);
    context.cards.set_row_homogeneous(true);
    context.cards.set_halign(gtk::Align::Fill);
    context.battlefield_panel.set_vexpand(true);
    context.battlefield_panel.set_height_request(-1);
    context
        .battlefield_panel
        .set_hscrollbar_policy(gtk::PolicyType::Never);
    *context.cached_tiling.borrow_mut() = None;
    context.cached_layout_items.borrow_mut().clear();
    sync_grid_layout(context);
    sync_terminal_parents(context);
    sync_input_scope_with_daemon(context);
    if context.sync_inputs_enabled.get() {
        focus_sync_input_anchor(context);
    }
    refresh_card_styles(context);
    refresh_workspace(context);
    schedule_runtime_size_sync(context);
}

fn show_top_level_battlefield(context: &Rc<AppContext>) {
    context.expanded_group.set(None);
    *context.cached_tiling.borrow_mut() = None;
    context.cached_layout_items.borrow_mut().clear();
    reset_battlefield_view(context);
    restore_pre_group_window_size(context);
    sync_input_scope_with_daemon(context);
    if context.sync_inputs_enabled.get() {
        focus_sync_input_anchor(context);
    }
}

fn remember_pre_group_window_size(context: &Rc<AppContext>) {
    if context.expanded_group.get().is_some() || context.pre_group_window_size.get().is_some() {
        return;
    }
    let Some(window) = workspace_window(context) else {
        return;
    };
    let width = window.width();
    let height = window.height();
    if width > 0 && height > 0 {
        context.pre_group_window_size.set(Some((width, height)));
    }
}

fn restore_pre_group_window_size(context: &Rc<AppContext>) {
    let Some((width, height)) = context.pre_group_window_size.take() else {
        return;
    };
    let Some(window) = workspace_window(context) else {
        return;
    };
    window.set_default_size(width, height);
    glib::idle_add_local_once(move || {
        window.set_default_size(width, height);
    });
}

fn workspace_window(context: &Rc<AppContext>) -> Option<gtk::Window> {
    context.cards.root()?.downcast::<gtk::Window>().ok()
}

fn spawn_session(
    context: &Rc<AppContext>,
    session_id: SessionId,
    launch: &SessionLaunch,
    terminal: &vte::Terminal,
) {
    let size = terminal_size_hint(terminal);
    let runtime = match spawn_runtime(terminal, launch, size) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to spawn session {session_id:?}: {error}");
            context.state.borrow_mut().mark_exited(session_id, -1);
            refresh_runtime_and_cards(context);
            return;
        }
    };

    if let Some(pid) = runtime.pid {
        context.state.borrow_mut().mark_spawned(session_id, pid);
    } else {
        context.state.borrow_mut().mark_exited(session_id, -1);
    }
    context
        .runtimes
        .borrow_mut()
        .insert(session_id, runtime.session_runtime);
    refresh_runtime_and_cards(context);
}

fn drain_daemon_events(context: &Rc<AppContext>) {
    let Some(beachhead) = context.beachhead.as_ref() else {
        return;
    };

    let mut changed = false;
    while let Ok(message) = beachhead.events().try_recv() {
        match message {
            ServerMessage::WorkspaceSnapshot { snapshot } => {
                apply_workspace_snapshot(context, snapshot);
                changed = true;
            }
            ServerMessage::TerminalAssistCompleted {
                request_id,
                session_id,
                inserted,
                error,
            } => {
                handle_terminal_assist_completed(context, request_id, session_id, inserted, error);
            }
            ServerMessage::Error { message } => {
                eprintln!("host connection error: {message}");
            }
        }
    }

    if changed {
        refresh_workspace(context);
        refresh_card_styles(context);
    }
}

fn apply_workspace_snapshot(context: &Rc<AppContext>, snapshot: WorkspaceSnapshot) {
    let previous_session_ids = context
        .state
        .borrow()
        .sessions()
        .iter()
        .map(|session| session.id)
        .collect::<Vec<_>>();
    let previous_focused_terminal = context.focused_terminal_id.get();
    let session_ids = snapshot
        .sessions
        .iter()
        .map(|session| session.record.id)
        .collect::<Vec<_>>();
    if context
        .terminal_assist_active
        .borrow()
        .is_some_and(|active| !session_ids.contains(&active.session_id))
    {
        context.terminal_assist_active.replace(None);
    }
    let added_session_to_focus = context.focus_next_added_terminal.get().then(|| {
        session_ids
            .iter()
            .copied()
            .filter(|session_id| !previous_session_ids.contains(session_id))
            .next_back()
    });

    context.state.borrow_mut().replace_workspace(
        snapshot
            .sessions
            .iter()
            .map(|session| session.record.clone())
            .collect(),
        snapshot.groups.clone(),
        snapshot.items.clone(),
    );

    let existing_ids = context
        .session_cards
        .borrow()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for session_id in existing_ids {
        if session_ids.contains(&session_id) {
            continue;
        }
        if let Some(card) = context.session_cards.borrow_mut().remove(&session_id) {
            context.cards.remove(&card.frame);
        }
        context.observations.borrow_mut().remove(&session_id);
        context
            .raw_stream_socket_names
            .borrow_mut()
            .remove(&session_id);
        context.display_runtimes.borrow_mut().remove(&session_id);
    }

    let group_ids = snapshot
        .groups
        .iter()
        .map(|group| group.id)
        .collect::<Vec<_>>();
    let existing_group_ids = context
        .group_cards
        .borrow()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for group_id in existing_group_ids {
        if group_ids.contains(&group_id) {
            continue;
        }
        if let Some(card) = context.group_cards.borrow_mut().remove(&group_id) {
            context.cards.remove(&card.frame);
        }
        context
            .pending_supervisor_visibility
            .borrow_mut()
            .remove(&group_id);
        if context.expanded_group.get() == Some(group_id) {
            context.expanded_group.set(None);
        }
    }

    for group in &snapshot.groups {
        if !context.group_cards.borrow().contains_key(&group.id) {
            let card = build_group_card_widgets(context, group);
            context.group_cards.borrow_mut().insert(group.id, card);
        }
        update_group_card_widgets(context, group);
    }

    for session in snapshot.sessions {
        update_raw_stream_socket_name(
            context,
            session.record.id,
            session.raw_stream_socket_name.clone(),
        );
        if !context
            .session_cards
            .borrow()
            .contains_key(&session.record.id)
        {
            let card = build_session_terminal_widgets(context, &session.record);
            context
                .session_cards
                .borrow_mut()
                .insert(session.record.id, card.clone());
        }
        if daemon_backed(context) {
            if let Some(socket_name) = context
                .raw_stream_socket_names
                .borrow()
                .get(&session.record.id)
                .cloned()
            {
                if let Some(card) = context.session_cards.borrow().get(&session.record.id) {
                    attach_daemon_display_runtime(
                        context,
                        session.record.id,
                        &card.terminal,
                        &socket_name,
                    );
                }
            }
        }
        context.observations.borrow_mut().insert(
            session.record.id,
            observation_from_snapshot(&session.observation),
        );
    }

    // Workspace snapshots also carry volatile observation/terminal activity.
    // Keep the layout cache intact so unchanged snapshots do not remove and
    // reattach terminal frames, which drops VTE focus during typing.
    sync_terminal_parents(context);
    sync_grid_layout(context);
    sync_input_scope_with_daemon(context);
    match added_session_to_focus {
        Some(Some(session_id)) => {
            context.focus_next_added_terminal.set(false);
            context.state.borrow_mut().select_session(session_id);
            refresh_card_styles(context);
            focus_session_terminal(context, session_id);
        }
        _ => {
            if let Some(session_id) = previous_focused_terminal {
                if context.session_cards.borrow().contains_key(&session_id)
                    && (battlefield_embeds_terminal(context, session_id)
                        || session_visible_as_group_supervisor(context, session_id))
                {
                    focus_session_terminal(context, session_id);
                }
            }
        }
    }
    if context.sync_inputs_enabled.get() {
        focus_sync_input_anchor(context);
    }
    sync_group_navigation_chrome(context);
    maybe_focus_initial_terminal(context);
}

fn update_raw_stream_socket_name(
    context: &Rc<AppContext>,
    session_id: SessionId,
    socket_name: Option<String>,
) {
    let changed = {
        let mut names = context.raw_stream_socket_names.borrow_mut();
        match socket_name {
            Some(socket_name) => {
                let changed = names
                    .get(&session_id)
                    .is_some_and(|existing| existing != &socket_name);
                names.insert(session_id, socket_name);
                changed
            }
            None => names.remove(&session_id).is_some(),
        }
    };

    if changed {
        reset_daemon_display_runtime(context, session_id);
    }
}

fn reset_daemon_display_runtime(context: &Rc<AppContext>, session_id: SessionId) {
    context.display_runtimes.borrow_mut().remove(&session_id);
}

fn observation_from_snapshot(snapshot: &ObservationSnapshot) -> SessionObservation {
    let mut observation = SessionObservation::new();
    observation.last_change = Instant::now() - Duration::from_secs(snapshot.last_change_age_secs);
    observation.recent_lines = snapshot.recent_lines.clone();
    observation.painted_line = snapshot.painted_line.clone();
    observation
}

fn attach_daemon_display_runtime(
    context: &Rc<AppContext>,
    session_id: SessionId,
    terminal: &vte::Terminal,
    socket_name: &str,
) {
    if context.display_runtimes.borrow().contains_key(&session_id) {
        return;
    }
    let size = terminal_size_hint(terminal);
    let Ok((runtime, input_events)) = attach_display_runtime(terminal, size) else {
        return;
    };
    if let Some(beachhead) = context.beachhead.as_ref() {
        spawn_daemon_display_bridge(
            beachhead.raw_session_connector(),
            session_id,
            socket_name.to_string(),
            runtime.output_writer.clone(),
            runtime.output_filter.clone(),
            input_events,
        );
    }
    context
        .display_runtimes
        .borrow_mut()
        .insert(session_id, runtime);
}

pub(crate) fn refresh_runtime_and_cards(context: &Rc<AppContext>) {
    drain_daemon_events(context);
    drain_runtime_events(context);
    sync_grid_layout(context);
    let sessions = context.state.borrow().sessions().to_vec();
    for session in &sessions {
        refresh_observation(context, session);
    }
    sync_terminal_parents(context);
    refresh_workspace(context);
    refresh_card_styles(context);
    sync_runtime_sizes(context);
}

fn drain_runtime_events(context: &Rc<AppContext>) {
    let mut drained = Vec::<(SessionId, RuntimeEvent)>::new();
    {
        let runtimes = context.runtimes.borrow();
        for (session_id, runtime) in runtimes.iter() {
            while let Ok(event) = runtime.events.try_recv() {
                drained.push((*session_id, event));
            }
        }
    }

    for (session_id, event) in drained {
        match event {
            RuntimeEvent::Stream(update) => {
                let mut observations = context.observations.borrow_mut();
                let observation = observations.entry(session_id).or_default();
                apply_stream_update(observation, update);
            }
            RuntimeEvent::Exited(exit_code) => {
                context
                    .state
                    .borrow_mut()
                    .mark_exited(session_id, exit_code);
            }
        }
    }
}

fn sync_runtime_sizes(context: &Rc<AppContext>) {
    let sizes = context
        .session_cards
        .borrow()
        .iter()
        .map(|(session_id, card)| {
            let size = if battlefield_embeds_terminal(context, *session_id)
                || session_visible_as_group_supervisor(context, *session_id)
            {
                measured_terminal_size_hint(&card.terminal)
            } else {
                Some(terminal_size_hint(&card.terminal))
            };
            (*session_id, size)
        })
        .collect::<Vec<_>>();

    if daemon_backed(context) {
        let mut runtimes = context.display_runtimes.borrow_mut();
        for (session_id, size) in sizes {
            let Some(size) = size else {
                continue;
            };
            let Some(runtime) = runtimes.get_mut(&session_id) else {
                continue;
            };
            let current = (size.rows, size.cols);
            if runtime.last_size == Some(current) {
                continue;
            }
            if let Ok(display_resizer) = runtime.display_resize_target.lock() {
                let _ = resize_display_pty(display_resizer.as_raw_fd(), size);
            }
            if let Some(beachhead) = context.beachhead.as_ref() {
                let _ = beachhead.commands().send(ClientMessage::ResizeTerminal {
                    session_id,
                    rows: size.rows,
                    cols: size.cols,
                });
            }
            runtime.last_size = Some(current);
        }
        return;
    }

    let mut runtimes = context.runtimes.borrow_mut();
    for (session_id, size) in sizes {
        let Some(size) = size else {
            continue;
        };
        let Some(runtime) = runtimes.get_mut(&session_id) else {
            continue;
        };
        let current = (size.rows, size.cols);
        if runtime.last_size == Some(current) {
            continue;
        }
        if let Ok(master) = runtime.resize_target.lock() {
            let _ = master.resize(size);
        }
        if let Some(display_resizer) = runtime.display_resize_target.as_ref() {
            if let Ok(display_resizer) = display_resizer.lock() {
                let _ = resize_display_pty(display_resizer.as_raw_fd(), size);
            }
        }
        runtime.last_size = Some(current);
    }
}

fn resize_display_pty(fd: i32, size: PtySize) -> std::io::Result<()> {
    let winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.pixel_width,
        ws_ypixel: size.pixel_height,
    };
    let result = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &winsize) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn refresh_observation(context: &Rc<AppContext>, session: &SessionRecord) {
    if daemon_backed(context) {
        return;
    }
    let remote_mode = matches!(context.mode, RunMode::Ssh { .. });
    let mut observations = context.observations.borrow_mut();
    let observation = observations.entry(session.id).or_default();
    refresh_session_observation(observation, session, remote_mode);
}

fn refresh_workspace(context: &Rc<AppContext>) {
    let is_empty = context.state.borrow().sessions().is_empty();
    context.empty_state.set_visible(is_empty);
    context.battlefield_panel.set_visible(!is_empty);
    context.title.set_subtitle("");
    sync_group_navigation_chrome(context);
}

fn sync_group_navigation_chrome(context: &Rc<AppContext>) {
    context
        .back_to_root_button
        .set_visible(context.expanded_group.get().is_some());
}

fn refresh_card_styles(context: &Rc<AppContext>) {
    let selected_item = context.state.borrow().selected_workspace_item();
    let single_card_mode = visible_workspace_item_count(context) == 1;
    let focused_terminal_id = context.focused_terminal_id.get();
    let sync_targets = if context.sync_inputs_enabled.get() {
        input_sync_target_session_ids(context)
    } else {
        Vec::new()
    };
    for (session_id, card) in context.session_cards.borrow().iter() {
        card.frame.remove_css_class("selected-card");
        card.frame.remove_css_class("single-card");
        if selected_item == Some(WorkspaceItem::Session(*session_id)) {
            card.frame.add_css_class("selected-card");
        }
        if single_card_mode {
            card.frame.add_css_class("single-card");
        }
        sync_terminal_dim_overlay(
            card,
            focused_terminal_id == Some(*session_id) || sync_targets.contains(session_id),
        );
        card.frame.set_hexpand(true);
        card.frame.set_halign(gtk::Align::Fill);
        card.frame.set_width_request(-1);
    }

    for (group_id, card) in context.group_cards.borrow().iter() {
        card.frame.remove_css_class("selected-card");
        card.frame.remove_css_class("single-card");
        if selected_item == Some(WorkspaceItem::Group(*group_id)) {
            card.frame.add_css_class("selected-card");
        }
        if single_card_mode {
            card.frame.add_css_class("single-card");
        }
        card.frame.set_hexpand(true);
        card.frame.set_halign(gtk::Align::Fill);
        card.frame.set_width_request(-1);
    }
}

fn sync_terminal_dim_overlay(card: &SessionCardWidgets, focused: bool) {
    card.terminal_dim_overlay.set_visible(!focused);
}

fn point_inside_widget<S: IsA<gtk::Widget>, T: IsA<gtk::Widget>>(
    source: &S,
    target: &T,
    x: f64,
    y: f64,
) -> bool {
    source
        .compute_point(target, &gtk::graphene::Point::new(x as f32, y as f32))
        .is_some_and(|point| {
            point.x() >= 0.0
                && point.y() >= 0.0
                && point.x() < target.width() as f32
                && point.y() < target.height() as f32
        })
}

fn display_supervisor_visible(context: &Rc<AppContext>, group: &SupervisedGroupRecord) -> bool {
    context
        .pending_supervisor_visibility
        .borrow()
        .get(&group.id)
        .copied()
        .unwrap_or(group.supervisor_visible)
}

fn current_input_sync_scope(context: &Rc<AppContext>) -> InputSyncScope {
    context
        .expanded_group
        .get()
        .map(|group_id| InputSyncScope::GroupMembers { group_id })
        .unwrap_or(InputSyncScope::RootVisible)
}

fn sync_input_scope_with_daemon(context: &Rc<AppContext>) {
    if let Some(beachhead) = context.beachhead.as_ref() {
        let _ = beachhead.commands().send(ClientMessage::SetInputSync {
            enabled: context.sync_inputs_enabled.get(),
            scope: current_input_sync_scope(context),
        });
    }
}

fn input_sync_target_session_ids(context: &Rc<AppContext>) -> Vec<SessionId> {
    let state = context.state.borrow();
    if let Some(group_id) = context.expanded_group.get() {
        return state
            .group(group_id)
            .map(|group| {
                group
                    .member_session_ids
                    .iter()
                    .copied()
                    .filter(|session_id| state.session(*session_id).is_some())
                    .collect()
            })
            .unwrap_or_default();
    }

    let supervisor_ids = state
        .groups()
        .iter()
        .filter_map(|group| group.supervisor_session_id)
        .collect::<Vec<_>>();
    state
        .ordered_visible_items()
        .iter()
        .filter_map(|item| match *item {
            WorkspaceItem::Session(session_id)
                if state.session(session_id).is_some() && !supervisor_ids.contains(&session_id) =>
            {
                Some(session_id)
            }
            _ => None,
        })
        .collect()
}

fn focus_sync_input_anchor(context: &Rc<AppContext>) -> bool {
    let targets = input_sync_target_session_ids(context);
    if targets.is_empty() {
        return false;
    }
    if context
        .focused_terminal_id
        .get()
        .is_some_and(|session_id| targets.contains(&session_id))
    {
        refresh_card_styles(context);
        return true;
    }
    let session_id = targets[0];
    context.state.borrow_mut().select_session(session_id);
    focus_session_terminal(context, session_id)
}

fn focus_session_terminal(context: &Rc<AppContext>, session_id: SessionId) -> bool {
    let Some(card) = context.session_cards.borrow().get(&session_id).cloned() else {
        return false;
    };
    context.focused_terminal_id.set(Some(session_id));
    refresh_card_styles(context);
    card.terminal.grab_focus();
    card.terminal_dim_overlay.set_visible(false);
    let context = context.clone();
    glib::idle_add_local_once(move || {
        card.terminal.grab_focus();
        context.focused_terminal_id.set(Some(session_id));
        refresh_card_styles(&context);
    });
    true
}

fn maybe_focus_initial_terminal(context: &Rc<AppContext>) {
    if context.initial_terminal_focus_done.get() || context.session_cards.borrow().is_empty() {
        return;
    }
    context.initial_terminal_focus_done.set(true);
    let context = context.clone();
    glib::idle_add_local_once(move || {
        if !focus_selected_visible_terminal(&context) {
            context.initial_terminal_focus_done.set(false);
        }
    });
}

fn focus_selected_visible_terminal(context: &Rc<AppContext>) -> bool {
    let session_id = context
        .state
        .borrow()
        .selected_session()
        .or_else(|| visible_battlefield_session_ids(context).first().copied());
    let Some(session_id) = session_id else {
        return false;
    };
    if !battlefield_embeds_terminal(context, session_id)
        && !session_visible_as_group_supervisor(context, session_id)
    {
        return false;
    }
    context.state.borrow_mut().select_session(session_id);
    refresh_card_styles(context);
    focus_session_terminal(context, session_id)
}

fn terminal_visible_for_focus(context: &Rc<AppContext>, session_id: SessionId) -> bool {
    battlefield_embeds_terminal(context, session_id)
        || session_visible_as_group_supervisor(context, session_id)
}

fn reset_battlefield_view(context: &Rc<AppContext>) {
    context.cards.set_column_homogeneous(true);
    context.cards.set_row_homogeneous(true);
    context.cards.set_halign(gtk::Align::Fill);
    context.battlefield_panel.set_vexpand(true);
    context.battlefield_panel.set_height_request(-1);
    context
        .battlefield_panel
        .set_hscrollbar_policy(gtk::PolicyType::Never);
    sync_grid_layout(context);
    sync_terminal_parents(context);
    refresh_card_styles(context);
    refresh_workspace(context);
    schedule_runtime_size_sync(context);
}

fn battlefield_workspace_items(context: &Rc<AppContext>) -> Vec<WorkspaceItem> {
    let state = context.state.borrow();
    if let Some(group_id) = context.expanded_group.get() {
        if let Some(group) = state.group(group_id) {
            return group
                .member_session_ids
                .iter()
                .copied()
                .filter(|session_id| state.session(*session_id).is_some())
                .map(WorkspaceItem::Session)
                .collect();
        }
    }

    state.ordered_visible_items().to_vec()
}

fn sync_grid_layout(context: &Rc<AppContext>) {
    let order = battlefield_workspace_items(context);
    let total = order.len();
    if total == 0 {
        for card in context.session_cards.borrow().values() {
            if card.frame.parent().is_some() {
                context.cards.remove(&card.frame);
            }
        }
        for card in context.group_cards.borrow().values() {
            if card.frame.parent().is_some() {
                context.cards.remove(&card.frame);
            }
        }
        *context.cached_tiling.borrow_mut() = None;
        context.cached_layout_items.borrow_mut().clear();
        return;
    }

    let available_width = context.battlefield_panel.width();
    let tiling = compute_tiling(total, available_width);

    if context.cached_tiling.borrow().as_ref() == Some(&tiling)
        && context.cached_layout_items.borrow().as_slice() == order.as_slice()
    {
        return;
    }

    let cards = context.session_cards.borrow();
    for card in cards.values() {
        if card.frame.parent().is_some() {
            context.cards.remove(&card.frame);
        }
    }
    let group_cards = context.group_cards.borrow();
    for card in group_cards.values() {
        if card.frame.parent().is_some() {
            context.cards.remove(&card.frame);
        }
    }
    for (i, placement) in tiling.placements.iter().enumerate() {
        match order.get(i).copied() {
            Some(WorkspaceItem::Session(session_id)) => {
                if let Some(card) = cards.get(&session_id) {
                    context.cards.attach(
                        &card.frame,
                        placement.col as i32,
                        placement.row as i32,
                        placement.col_span as i32,
                        1,
                    );
                }
            }
            Some(WorkspaceItem::Group(group_id)) => {
                if let Some(card) = group_cards.get(&group_id) {
                    context.cards.attach(
                        &card.frame,
                        placement.col as i32,
                        placement.row as i32,
                        placement.col_span as i32,
                        1,
                    );
                }
            }
            None => {}
        }
    }
    drop(group_cards);
    drop(cards);

    *context.cached_tiling.borrow_mut() = Some(tiling);
    *context.cached_layout_items.borrow_mut() = order;
}

fn session_visible_as_group_supervisor(context: &Rc<AppContext>, session_id: SessionId) -> bool {
    if context.expanded_group.get().is_some() {
        return false;
    }

    context.state.borrow().groups().iter().any(|group| {
        display_supervisor_visible(context, group)
            && group.supervisor_session_id == Some(session_id)
    })
}

fn group_supervisor_terminal_slot(
    context: &Rc<AppContext>,
    session_id: SessionId,
) -> Option<gtk::Box> {
    let state = context.state.borrow();
    let group_id = state.groups().iter().find_map(|group| {
        (display_supervisor_visible(context, group)
            && group.supervisor_session_id == Some(session_id))
        .then_some(group.id)
    })?;
    context
        .group_cards
        .borrow()
        .get(&group_id)
        .map(|card| card.terminal_slot.clone())
}

fn visible_battlefield_session_ids(context: &Rc<AppContext>) -> Vec<SessionId> {
    battlefield_workspace_items(context)
        .into_iter()
        .filter_map(|item| match item {
            WorkspaceItem::Session(session_id) => Some(session_id),
            WorkspaceItem::Group(_) => None,
        })
        .collect()
}

fn visible_workspace_item_count(context: &Rc<AppContext>) -> usize {
    battlefield_workspace_items(context).len()
}

fn battlefield_embeds_terminal(context: &Rc<AppContext>, session_id: SessionId) -> bool {
    visible_battlefield_session_ids(context).contains(&session_id)
}

fn focused_visible_terminal_session(context: &Rc<AppContext>) -> Option<SessionId> {
    context
        .session_cards
        .borrow()
        .iter()
        .find_map(|(session_id, card)| {
            (terminal_visible_for_focus(context, *session_id) && card.terminal.has_focus())
                .then_some(*session_id)
        })
}

fn schedule_runtime_size_sync(context: &Rc<AppContext>) {
    sync_runtime_sizes(context);
    let context = context.clone();
    glib::idle_add_local_once(move || {
        sync_runtime_sizes(&context);
    });
}

fn sync_terminal_parents(context: &Rc<AppContext>) {
    for (session_id, card) in context.session_cards.borrow().iter() {
        if session_visible_as_group_supervisor(context, *session_id) {
            if let Some(slot) = group_supervisor_terminal_slot(context, *session_id) {
                reparent_widget_to_box(&card.terminal_overlay, &slot);
            } else {
                reparent_widget_to_box(&card.terminal_overlay, &card.terminal_slot);
            }
        } else {
            reparent_widget_to_box(&card.terminal_overlay, &card.terminal_slot);
        }
    }
}

fn reparent_widget_to_box<W: IsA<gtk::Widget>>(widget: &W, target: &gtk::Box) {
    if widget
        .parent()
        .and_then(|parent| parent.downcast::<gtk::Box>().ok())
        .as_ref()
        .is_some_and(|parent| parent == target)
    {
        return;
    }

    if let Some(parent) = widget.parent() {
        if let Ok(parent_box) = parent.clone().downcast::<gtk::Box>() {
            parent_box.remove(widget);
        } else if let Ok(parent_scroller) = parent.downcast::<gtk::ScrolledWindow>() {
            parent_scroller.set_child(None::<&gtk::Widget>);
        }
    }
    target.append(widget);
}

#[cfg(test)]
mod tests {
    use super::{
        group_assessment_from_markdown, is_markdown_table_separator, markdown_inline_to_pango,
        parse_markdown_table, parse_run_mode, GroupAssessment, RunMode,
        TERMINATOR_AMBIENCE_BACKGROUND, TERMINATOR_AMBIENCE_FOREGROUND,
        TERMINATOR_AMBIENCE_PALETTE,
    };

    #[test]
    fn parses_ssh_run_mode() {
        let parsed = parse_run_mode(vec!["--ssh".into(), "user@example.com".into()]).unwrap();
        assert_eq!(
            parsed.mode,
            RunMode::Ssh {
                target: "user@example.com".into()
            }
        );
    }

    #[test]
    fn rejects_invalid_run_mode_args() {
        assert!(parse_run_mode(vec!["--ssh".into()]).is_err());
        assert!(parse_run_mode(vec!["--bogus".into()]).is_err());
    }

    #[test]
    fn terminal_theme_uses_full_palette() {
        assert_eq!(TERMINATOR_AMBIENCE_PALETTE.len(), 16);
        assert_eq!(TERMINATOR_AMBIENCE_FOREGROUND, "#ffffff");
        assert_eq!(TERMINATOR_AMBIENCE_BACKGROUND, "#000000");
    }

    #[test]
    fn group_assessment_uses_summary_language() {
        assert_eq!(
            group_assessment_from_markdown(
                "| Worker | State |\n|---|---|\n| 1 | blocked on auth error |"
            ),
            GroupAssessment::Stalling
        );
        assert_eq!(
            group_assessment_from_markdown(
                "Blocked: most agents cannot proceed at all because credentials are unavailable."
            ),
            GroupAssessment::Blocked
        );
        assert_eq!(
            group_assessment_from_markdown(
                "Stalling: despite supervisor prods, the workers are looping on the same failure."
            ),
            GroupAssessment::Stalling
        );
        assert_eq!(
            group_assessment_from_markdown(
                "Workers are running tests and making progress despite compile errors."
            ),
            GroupAssessment::Active
        );
        assert_eq!(
            group_assessment_from_markdown("All tests pass; task complete."),
            GroupAssessment::Complete
        );
    }

    #[test]
    fn markdown_table_parser_accepts_common_table_shape() {
        let lines = [
            "| Worker | State | Next |",
            "|---|---|---|",
            "| 1 | Working | monitor |",
            "| 2 | Stalling | prod sent |",
        ];
        let (headers, rows, consumed) = parse_markdown_table(&lines).expect("parse table");

        assert_eq!(headers, vec!["Worker", "State", "Next"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(consumed, 4);
        assert!(is_markdown_table_separator("---"));
    }

    #[test]
    fn markdown_inline_renderer_escapes_markup_and_code() {
        assert_eq!(
            markdown_inline_to_pango("run `cargo test` <now>"),
            "run <tt>cargo test</tt> &lt;now&gt;"
        );
    }
}
