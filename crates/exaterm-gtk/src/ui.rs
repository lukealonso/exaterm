use crate::actions::{
    insert_terminal_number, send_runtime_input_line, set_auto_nudge_hover, toggle_auto_nudge,
};
use crate::beachhead::BeachheadConnection;
use crate::style::{
    apply_battle_card_surface_style, apply_battle_status_style, configure_app_icons, load_css,
};
use crate::terminal_adapter::{
    attach_display_runtime, measured_terminal_size_hint, spawn_daemon_display_bridge,
    spawn_runtime, terminal_size_hint, ClientDisplayRuntime,
};
use crate::widgets::{build_segmented_bar, FocusWidgets, SegmentedBarWidgets, SessionCardWidgets};
use exaterm_core::model::{
    blocking_prompt_launch, planning_stream_launch, running_stream_launch, ssh_shell_launch,
    user_shell_launch,
};
use exaterm_core::observation::{
    apply_stream_update, build_naming_evidence, build_nudge_evidence, build_tactical_evidence,
    is_bare_waiting_shell, refresh_observation as refresh_session_observation,
    scrollback_fragments, SessionObservation,
};
use exaterm_core::runtime::{RuntimeEvent, SessionRuntime};
use exaterm_core::synthesis::{
    name_signature, nudge_signature, suggest_name_blocking, suggest_nudge_blocking,
    summarize_blocking, summary_signature, NamingEvidence, NudgeEvidence, OpenAiNamingConfig,
    OpenAiNudgeConfig, OpenAiSynthesisConfig, TacticalEvidence,
};
use exaterm_types::model::{SessionId, SessionLaunch, SessionRecord};
use exaterm_types::proto::{ClientMessage, ObservationSnapshot, ServerMessage, WorkspaceSnapshot};
use exaterm_types::synthesis::{
    AttentionLevel, NameSuggestion, NudgeSuggestion, TacticalState, TacticalSynthesis,
};
use exaterm_ui::beachhead::{parse_run_mode, BeachheadTarget, RunMode};
use exaterm_ui::layout::{
    battlefield_can_embed_terminals, battlefield_columns,
    visible_scrollback_line_capacity as layout_visible_scrollback_line_capacity,
};
use exaterm_ui::presentation::{
    attention_bar_presentation, chrome_visibility, combined_focus_summary_text,
    nudge_state_presentation, status_chip_label, ChromeVisibility as CardChromeVisibility,
};
use exaterm_ui::supervision::{
    build_battle_card, BattleCardStatus, BattleCardViewModel, ObservedActivity, SignalTone,
};
use exaterm_ui::workspace_view::WorkspaceViewState;
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use pangocairo::functions::show_layout;
use portable_pty::PtySize;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vte::prelude::*;
use vte4 as vte;

const APP_ID: &str = "io.exaterm.Exaterm";

struct SummaryWorker {
    requests: mpsc::Sender<SummaryJob>,
    responses: mpsc::Receiver<SummaryResult>,
}

struct NamingWorker {
    requests: mpsc::Sender<NamingJob>,
    responses: mpsc::Receiver<NamingResult>,
}

struct NamingJob {
    session_id: SessionId,
    signature: String,
    evidence: NamingEvidence,
}

struct NamingResult {
    session_id: SessionId,
    signature: String,
    suggestion: Result<NameSuggestion, String>,
}

struct NudgeWorker {
    requests: mpsc::Sender<NudgeJob>,
    responses: mpsc::Receiver<NudgeResult>,
}

struct NudgeJob {
    session_id: SessionId,
    signature: String,
    evidence: NudgeEvidence,
}

struct NudgeResult {
    session_id: SessionId,
    signature: String,
    suggestion: Result<NudgeSuggestion, String>,
}

struct SummaryJob {
    session_id: SessionId,
    signature: String,
    evidence: TacticalEvidence,
}

struct SummaryResult {
    session_id: SessionId,
    signature: String,
    summary: Result<TacticalSynthesis, String>,
}

struct SummaryCacheEntry {
    first_seen: Instant,
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_summary: Option<TacticalSynthesis>,
    last_error: Option<String>,
    last_attempt: Option<Instant>,
    in_flight: bool,
}

struct NamingCacheEntry {
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_name: Option<String>,
    last_error: Option<String>,
    last_attempt: Option<Instant>,
    in_flight: bool,
}

pub(crate) struct NudgeCacheEntry {
    pub(crate) enabled: bool,
    pub(crate) hovered: bool,
    pub(crate) completed_signature: Option<String>,
    pub(crate) requested_signature: Option<String>,
    pub(crate) last_nudge: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_attempt: Option<Instant>,
    pub(crate) last_sent: Option<Instant>,
    pub(crate) in_flight: bool,
}

impl SummaryCacheEntry {
    fn new() -> Self {
        Self {
            first_seen: Instant::now(),
            completed_signature: None,
            requested_signature: None,
            last_summary: None,
            last_error: None,
            last_attempt: None,
            in_flight: false,
        }
    }
}

fn summary_refresh_interval(session_age: Duration) -> Duration {
    let seconds = session_age.as_secs();
    if seconds < 60 {
        Duration::from_secs(5)
    } else if seconds < 180 {
        Duration::from_secs(10)
    } else if seconds < 300 {
        Duration::from_secs(20)
    } else {
        Duration::from_secs(30)
    }
}

impl NamingCacheEntry {
    fn new() -> Self {
        Self {
            completed_signature: None,
            requested_signature: None,
            last_name: None,
            last_error: None,
            last_attempt: None,
            in_flight: false,
        }
    }
}

impl NudgeCacheEntry {
    pub(crate) fn new() -> Self {
        Self {
            enabled: false,
            hovered: false,
            completed_signature: None,
            requested_signature: None,
            last_nudge: None,
            last_error: None,
            last_attempt: None,
            last_sent: None,
            in_flight: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CardChromeMode {
    SparseShell,
    Summarized,
}

impl CardChromeMode {
    fn from_summary(summary: Option<&TacticalSynthesis>) -> Self {
        if summary.is_some() {
            Self::Summarized
        } else {
            Self::SparseShell
        }
    }

    fn summarized(self) -> bool {
        matches!(self, Self::Summarized)
    }
}

fn card_chrome_visibility(
    chrome_mode: CardChromeMode,
    focus_mode: bool,
    has_operator_summary: bool,
) -> CardChromeVisibility {
    chrome_visibility(chrome_mode.summarized(), focus_mode, has_operator_summary)
}

pub(crate) struct AppContext {
    mode: RunMode,
    pub(crate) beachhead: Option<BeachheadConnection>,
    pub(crate) state: Rc<RefCell<WorkspaceViewState>>,
    title: adw::WindowTitle,
    empty_state: gtk::Box,
    content_root: gtk::Box,
    cards: gtk::FlowBox,
    battlefield_panel: gtk::ScrolledWindow,
    pub(crate) sync_inputs_enabled: Arc<AtomicBool>,
    pub(crate) raw_input_writers: Arc<Mutex<BTreeMap<SessionId, Arc<Mutex<UnixStream>>>>>,
    focus: FocusWidgets,
    session_cards: RefCell<BTreeMap<SessionId, SessionCardWidgets>>,
    observations: RefCell<BTreeMap<SessionId, SessionObservation>>,
    raw_stream_socket_names: RefCell<BTreeMap<SessionId, String>>,
    pub(crate) runtimes: RefCell<BTreeMap<SessionId, SessionRuntime>>,
    display_runtimes: RefCell<BTreeMap<SessionId, ClientDisplayRuntime>>,
    summary_worker: Option<SummaryWorker>,
    summary_cache: RefCell<BTreeMap<SessionId, SummaryCacheEntry>>,
    naming_worker: Option<NamingWorker>,
    naming_cache: RefCell<BTreeMap<SessionId, NamingCacheEntry>>,
    nudge_worker: Option<NudgeWorker>,
    pub(crate) nudge_cache: RefCell<BTreeMap<SessionId, NudgeCacheEntry>>,
    closing_confirmed: Cell<bool>,
}

pub fn run() -> glib::ExitCode {
    let argv = std::env::args().collect::<Vec<_>>();
    let mode = match parse_run_mode(argv.iter().skip(1).cloned()) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("{error}");
            eprintln!("usage: exaterm [--ssh user@host]");
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
    app.connect_activate(move |app| build_ui(app, mode.clone()));
    let program = argv
        .first()
        .cloned()
        .unwrap_or_else(|| "exaterm".to_string());
    app.run_with_args(&[program])
}

pub(crate) fn daemon_backed(context: &AppContext) -> bool {
    context.beachhead.is_some()
}

fn build_ui(app: &gtk::Application, mode: RunMode) {
    load_css();
    configure_app_icons(APP_ID);
    let missing_openai_key =
        !visual_gallery_enabled() && OpenAiSynthesisConfig::from_env().is_none();
    let beachhead = if visual_gallery_enabled() {
        None
    } else {
        let target = match &mode {
            RunMode::Local => BeachheadTarget::Local,
            RunMode::Ssh { target } => BeachheadTarget::Ssh(target.clone()),
        };
        match BeachheadConnection::connect(&target) {
            Ok(connection) => Some(connection),
            Err(error) => {
                present_startup_error(app, &error);
                return;
            }
        }
    };

    let cards = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .column_spacing(12)
        .row_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .homogeneous(true)
        .valign(gtk::Align::Fill)
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

    let focus_title = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["card-title".to_string()])
        .build();
    focus_title.set_single_line_mode(true);
    focus_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    focus_title.set_max_width_chars(18);
    let focus_status = gtk::Label::builder()
        .xalign(0.5)
        .css_classes(vec!["card-status".to_string(), "battle-active".to_string()])
        .label("Active")
        .build();
    let focus_headline = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .visible(false)
        .css_classes(vec![
            "card-headline".to_string(),
            "focus-headline".to_string(),
        ])
        .build();
    focus_headline.set_lines(2);
    focus_headline.set_ellipsize(gtk::pango::EllipsizeMode::End);
    focus_headline.set_max_width_chars(30);
    let focus_attention_pill = gtk::Label::builder()
        .xalign(0.0)
        .visible(false)
        .css_classes(vec!["focus-attention-pill".to_string()])
        .build();
    focus_attention_pill.set_valign(gtk::Align::End);
    let focus_alert = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .css_classes(vec!["card-alert".to_string()])
        .build();
    focus_alert.set_halign(gtk::Align::Fill);
    focus_alert.set_single_line_mode(true);
    focus_alert.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let focus_terminal_slot = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    focus_terminal_slot.add_css_class("card-terminal-slot");
    let focus_momentum_bar = build_segmented_bar("Attention Condition");
    let focus_risk_bar = build_segmented_bar("Unused");

    let focus_header_left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .hexpand(true)
        .build();
    focus_header_left.add_css_class("card-title-stack");
    focus_header_left.append(&focus_title);

    let focus_header_right = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .build();
    focus_header_right.add_css_class("card-status-stack");
    focus_header_right.append(&focus_status);

    let focus_header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    focus_header.add_css_class("card-header-row");
    focus_header.append(&focus_header_left);
    focus_header.append(&focus_header_right);

    let focus_bars = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .build();
    focus_bars.add_css_class("card-bars-row");
    focus_bars.set_homogeneous(true);
    focus_bars.append(&focus_momentum_bar.frame);
    focus_bars.append(&focus_risk_bar.frame);

    let focus_summary_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .vexpand(true)
        .visible(false)
        .build();
    focus_summary_box.add_css_class("focus-summary-box");
    focus_summary_box.append(&focus_headline);
    focus_summary_box.append(&focus_attention_pill);

    let focus_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .hexpand(true)
        .vexpand(true)
        .build();
    focus_content.append(&focus_header);
    focus_content.append(&focus_summary_box);
    focus_content.append(&focus_alert);
    focus_content.append(&focus_terminal_slot);
    focus_content.append(&focus_bars);

    let focus_frame = gtk::Frame::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&focus_content)
        .build();
    focus_frame.add_css_class("battle-card");
    focus_frame.add_css_class("single-card");

    let focus_panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(8)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .hexpand(true)
        .vexpand(true)
        .visible(false)
        .build();
    focus_panel.add_css_class("focus-panel");
    focus_panel.append(&focus_frame);

    let title = adw::WindowTitle::new("Exaterm", "");
    let header = adw::HeaderBar::builder()
        .title_widget(&title)
        .show_end_title_buttons(true)
        .build();

    let content_root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    content_root.add_css_class("battlefield-root");
    content_root.append(&empty_state);
    content_root.append(&battlefield_panel);
    content_root.append(&focus_panel);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&content_root);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Exaterm")
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
        empty_state,
        content_root,
        cards,
        battlefield_panel,
        sync_inputs_enabled: Arc::new(AtomicBool::new(false)),
        raw_input_writers: Arc::new(Mutex::new(BTreeMap::new())),
        focus: FocusWidgets {
            panel: focus_panel,
            frame: focus_frame,
            header: focus_header,
            title: focus_title,
            status: focus_status,
            summary_box: focus_summary_box,
            headline: focus_headline,
            attention_pill: focus_attention_pill,
            alert: focus_alert,
            terminal_slot: focus_terminal_slot,
            bars: focus_bars,
            momentum_bar: focus_momentum_bar,
            risk_bar: focus_risk_bar,
        },
        session_cards: RefCell::new(BTreeMap::new()),
        observations: RefCell::new(BTreeMap::new()),
        raw_stream_socket_names: RefCell::new(BTreeMap::new()),
        runtimes: RefCell::new(BTreeMap::new()),
        display_runtimes: RefCell::new(BTreeMap::new()),
        summary_worker: if visual_gallery_enabled() {
            spawn_summary_worker()
        } else {
            None
        },
        summary_cache: RefCell::new(BTreeMap::new()),
        naming_worker: if visual_gallery_enabled() {
            spawn_naming_worker()
        } else {
            None
        },
        naming_cache: RefCell::new(BTreeMap::new()),
        nudge_worker: if visual_gallery_enabled() {
            spawn_nudge_worker()
        } else {
            None
        },
        nudge_cache: RefCell::new(BTreeMap::new()),
        closing_confirmed: Cell::new(false),
    });

    {
        let cards = context.cards.clone();
        let context = context.clone();
        cards.connect_selected_children_changed(move |flowbox| {
            let selected = flowbox.selected_children();
            let Some(selected_child) = selected.first() else {
                return;
            };
            let maybe_session =
                context
                    .session_cards
                    .borrow()
                    .iter()
                    .find_map(|(session_id, card)| {
                        (card.row == *selected_child).then_some(*session_id)
                    });
            if let Some(session_id) = maybe_session {
                let focused = context.state.borrow().focused_session().is_some();
                if focused {
                    show_intervention(&context, session_id);
                } else {
                    context.state.borrow_mut().select_session(session_id);
                    refresh_card_styles(&context);
                }
            }
        });
    }

    {
        let context = context.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            let focused_session = context.state.borrow().focused_session();
            if key == gdk::Key::Escape && focused_session.is_some() {
                show_battlefield(&context);
                return glib::Propagation::Stop;
            }

            if focused_session.is_none() && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                if focused_embedded_terminal_session(&context).is_some() {
                    return glib::Propagation::Proceed;
                }
                let selected_session = context.state.borrow().selected_session();
                if let Some(session_id) = selected_session {
                    if battlefield_embeds_terminal(&context, session_id) {
                        if let Some(card) = context.session_cards.borrow().get(&session_id) {
                            if card.terminal.has_focus() {
                                return glib::Propagation::Proceed;
                            }
                            card.terminal.grab_focus();
                        }
                        refresh_card_styles(&context);
                    } else {
                        show_intervention(&context, session_id);
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
        glib::timeout_add_local(std::time::Duration::from_millis(900), move || {
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
                .body("Closing Exaterm can leave the local beachhead running so you can reconnect to the same live terminal later.")
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
    if missing_openai_key {
        present_openai_key_warning(&window);
    }
}

fn present_startup_error(app: &gtk::Application, error: &str) {
    let message = gtk::Label::builder()
        .label(format!(
            "Exaterm could not start a live beachhead connection.\n\n{error}"
        ))
        .wrap(true)
        .xalign(0.0)
        .hexpand(true)
        .build();

    let close_button = gtk::Button::with_label("Close");
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Exaterm")
        .icon_name(APP_ID)
        .default_width(720)
        .default_height(220)
        .build();

    let title = adw::WindowTitle::new("Exaterm", "Startup failed");
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

fn present_openai_key_warning(window: &adw::ApplicationWindow) {
    let dialog = adw::AlertDialog::builder()
        .heading("OpenAI API key missing")
        .body(
            "Exaterm started, but `OPENAI_API_KEY` is not set. Tactical summaries, naming, and auto-nudge are disabled until you provide a key.",
        )
        .close_response("ok")
        .build();
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));
    dialog.present(Some(window));
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

    let card = build_battle_card_widgets(context, &session);
    context.cards.insert(&card.row, -1);
    context
        .session_cards
        .borrow_mut()
        .insert(session_id, card.clone());
    context
        .observations
        .borrow_mut()
        .insert(session_id, SessionObservation::new());

    update_flowbox_columns(context);
    if context.state.borrow().selected_session() == Some(session_id) {
        context.cards.select_child(&card.row);
    }
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
                "• I fixed the stuck focus path and the focused terminal now accepts Return again.",
                "• Verified with cargo test plus a manual smoke pass.",
                "• Next I can tighten battlefield density and typography if you want me to keep going.",
                "• Current state is clean and ready for the next pass.",
                "› Continue",
                "• Larger typography is in and focus mode keeps context now.",
                "• Tests pass. Ready for the next instruction or a keep-going nudge.",
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
                "• I think the next failure is still the focus handoff, so I’m trying another narrow fix.",
                "$ cargo test focus_mode -- --nocapture",
                "error[E0599]: no method named present on FocusHandle",
                "• That patch was wrong; I’m retrying with a different signal hookup.",
                "$ cargo test focus_mode -- --nocapture",
                "error[E0599]: no method named present on FocusHandle",
                "• Still wrong. I’m going to try another approach on the same path.",
                "$ cargo test focus_mode -- --nocapture",
                "error[E0599]: no method named present on FocusHandle",
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

fn build_battle_card_widgets(
    context: &Rc<AppContext>,
    session: &SessionRecord,
) -> SessionCardWidgets {
    let title = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .css_classes(vec!["card-title".to_string()])
        .build();
    title.set_single_line_mode(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(40);
    let status = gtk::Label::builder()
        .label("Active")
        .xalign(0.5)
        .css_classes(vec!["card-status".to_string(), "battle-active".to_string()])
        .build();
    let nudge_state = gtk::Label::builder()
        .label("AUTONUDGE OFF")
        .xalign(0.5)
        .css_classes(vec![
            "card-control-state".to_string(),
            "card-control-off".to_string(),
        ])
        .build();
    nudge_state.set_single_line_mode(true);
    nudge_state.set_wrap(false);
    nudge_state.set_hexpand(false);
    nudge_state.set_vexpand(false);
    nudge_state.set_halign(gtk::Align::End);
    nudge_state.set_valign(gtk::Align::Center);
    let nudge_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(false)
        .halign(gtk::Align::End)
        .visible(false)
        .build();
    nudge_row.set_valign(gtk::Align::Center);
    nudge_row.add_css_class("card-control-row");
    nudge_row.append(&nudge_state);
    let recency = gtk::Label::builder()
        .label("recency unknown")
        .xalign(1.0)
        .css_classes(vec!["card-recency".to_string()])
        .build();
    recency.set_hexpand(true);
    recency.set_halign(gtk::Align::End);
    let headline = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .visible(false)
        .css_classes(vec!["card-headline".to_string()])
        .build();
    headline.set_lines(2);
    headline.set_ellipsize(gtk::pango::EllipsizeMode::End);
    headline.set_max_width_chars(18);
    let alert = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .css_classes(vec!["card-alert".to_string()])
        .build();
    alert.set_halign(gtk::Align::Fill);
    alert.set_single_line_mode(true);
    alert.set_ellipsize(gtk::pango::EllipsizeMode::End);
    nudge_row.prepend(&alert);
    let headline_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .visible(false)
        .build();
    headline_row.set_valign(gtk::Align::Start);
    headline_row.append(&headline);
    headline_row.append(&nudge_row);
    let attention_pill = gtk::Label::builder()
        .xalign(0.0)
        .visible(false)
        .css_classes(vec![
            "focus-attention-pill".to_string(),
            "rail-attention-pill".to_string(),
        ])
        .build();
    attention_pill.set_valign(gtk::Align::End);
    let momentum_bar = build_segmented_bar("Attention Condition");
    let risk_bar = build_segmented_bar("Unused");

    let header_left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .hexpand(true)
        .build();
    header_left.add_css_class("card-title-stack");
    header_left.append(&title);

    let header_right = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .build();
    header_right.add_css_class("card-status-stack");
    header_right.append(&status);

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    header.add_css_class("card-header-row");
    header.append(&header_left);
    header.append(&header_right);

    let scrollback_lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let scrollback_content = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    scrollback_content.set_content_width(0);
    scrollback_content.set_content_height(0);
    {
        let scrollback_lines = scrollback_lines.clone();
        scrollback_content.set_draw_func(move |area, cr, width, height| {
            const H_MARGIN: i32 = 10;
            const V_MARGIN: i32 = 8;

            let text = {
                let lines = scrollback_lines.borrow();
                if lines.is_empty() {
                    " ".to_string()
                } else {
                    lines.join("\n")
                }
            };

            let layout = area.create_pango_layout(Some(&text));
            layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
                "Monospace 7",
            )));
            layout.set_spacing(4 * gtk::pango::SCALE);
            layout.set_width(((width - (H_MARGIN * 2)).max(0)) * gtk::pango::SCALE);

            let (_, text_height) = layout.pixel_size();
            let x = H_MARGIN as f64;
            let y = (height - V_MARGIN - text_height) as f64;

            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            cr.clip();
            cr.move_to(x, y);
            cr.set_source_rgba(202.0 / 255.0, 214.0 / 255.0, 227.0 / 255.0, 0.88);
            show_layout(cr, &layout);
        });
    }
    scrollback_content.add_css_class("card-scrollback-view");
    let scrollback_band = gtk::Frame::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&scrollback_content)
        .build();
    scrollback_band.set_overflow(gtk::Overflow::Hidden);
    scrollback_band.add_css_class("card-scrollback-frame");
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
    middle_stack.add_named(&terminal_slot, Some("terminal"));
    middle_stack.add_named(&scrollback_band, Some("scrollback"));
    middle_stack.set_visible_child_name("scrollback");

    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    footer.add_css_class("card-bottom-stack");
    let bars = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .hexpand(true)
        .build();
    bars.add_css_class("card-bars-row");
    bars.append(&momentum_bar.frame);
    bars.append(&risk_bar.frame);
    footer.append(&bars);
    let footer_meta = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .build();
    footer_meta.add_css_class("card-footer-meta");
    footer_meta.append(&recency);
    footer.append(&footer_meta);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(16)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.append(&header);
    content.append(&headline_row);
    content.append(&attention_pill);
    content.append(&middle_stack);
    content.append(&footer);

    let frame = gtk::Frame::builder()
        .child(&content)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .build();
    frame.add_css_class("battle-card");
    let row = gtk::FlowBoxChild::builder()
        .child(&frame)
        .hexpand(true)
        .vexpand(true)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .build();
    row.set_focusable(true);

    {
        let context = context.clone();
        let row = row.clone();
        let session_id = session.id;
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_released(move |_, _, _, _| {
            let focused_before = context.state.borrow().focused_session();
            context.cards.select_child(&row);
            context.state.borrow_mut().select_session(session_id);
            if let Some(focused_session) = focused_before {
                if focused_session == session_id {
                    show_battlefield(&context);
                }
                return;
            }
            if battlefield_embeds_terminal(&context, session_id) {
                if let Some(card) = context.session_cards.borrow().get(&session_id) {
                    card.terminal.grab_focus();
                }
                refresh_card_styles(&context);
            } else {
                show_intervention(&context, session_id);
            }
        });
        frame.add_controller(click);
    }

    let terminal = vte::Terminal::builder()
        .scroll_on_output(false)
        .scroll_on_keystroke(true)
        .input_enabled(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal.set_scrollback_lines(100_000);
    terminal.connect_selection_changed(|terminal| {
        if terminal.has_selection() {
            terminal.copy_clipboard_format(vte::Format::Text);
        }
    });
    {
        let context = context.clone();
        let row = row.clone();
        let session_id = session.id;
        let terminal_focus = gtk::EventControllerFocus::new();
        terminal_focus.connect_enter(move |_| {
            context.cards.select_child(&row);
            {
                let mut state = context.state.borrow_mut();
                state.select_session(session_id);
                state.set_terminal_focus(Some(session_id));
            }
            refresh_card_styles(&context);
        });
        terminal.add_controller(terminal_focus);
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
    install_terminal_context_menu(context, &terminal, session.id);
    install_nudge_pill_interactions(context, &nudge_state, session.id);
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
        row,
        frame,
        header,
        title,
        status,
        headline_row,
        attention_pill,
        nudge_row,
        nudge_state,
        recency,
        middle_stack,
        scrollback_band,
        scrollback_content,
        scrollback_lines,
        terminal_slot,
        footer,
        bars,
        headline,
        alert,
        momentum_bar,
        risk_bar,
        terminal_view,
        terminal,
    }
}

fn ui_display_name(session: &SessionRecord, chrome_mode: CardChromeMode) -> String {
    match chrome_mode {
        CardChromeMode::SparseShell => String::new(),
        CardChromeMode::Summarized => session
            .display_name
            .clone()
            .unwrap_or_else(|| "New Session".into()),
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

    let add_terminals_action = gtk::gio::SimpleAction::new("add_terminals", None);
    {
        let context = context.clone();
        add_terminals_action.connect_activate(move |_, _| {
            split_terminal_here(&context, source_session);
        });
    }
    actions.add_action(&add_terminals_action);

    let insert_terminal_number_one_action =
        gtk::gio::SimpleAction::new("insert_terminal_number_one", None);
    {
        let context = context.clone();
        insert_terminal_number_one_action.connect_activate(move |_, _| {
            insert_terminal_number(&context, source_session, true);
        });
    }
    actions.add_action(&insert_terminal_number_one_action);

    let insert_terminal_number_zero_action =
        gtk::gio::SimpleAction::new("insert_terminal_number_zero", None);
    {
        let context = context.clone();
        insert_terminal_number_zero_action.connect_activate(move |_, _| {
            insert_terminal_number(&context, source_session, false);
        });
    }
    actions.add_action(&insert_terminal_number_zero_action);

    let sync_inputs_action = gtk::gio::SimpleAction::new_stateful(
        "sync_inputs",
        None,
        &context
            .sync_inputs_enabled
            .load(Ordering::Relaxed)
            .to_variant(),
    );
    {
        let context = context.clone();
        sync_inputs_action.connect_change_state(move |action, value| {
            let enabled = value.and_then(|state| state.get::<bool>()).unwrap_or(false);
            context
                .sync_inputs_enabled
                .store(enabled, Ordering::Relaxed);
            action.set_state(&enabled.to_variant());
        });
    }
    actions.add_action(&sync_inputs_action);
    terminal.insert_action_group("terminal", Some(&actions));

    let menu = gtk::gio::Menu::new();
    menu.append(Some("Copy"), Some("terminal.copy"));
    menu.append(Some("Paste"), Some("terminal.paste"));
    menu.append(Some("Add Terminals"), Some("terminal.add_terminals"));
    menu.append(
        Some("Insert Terminal Number (1-base)"),
        Some("terminal.insert_terminal_number_one"),
    );
    menu.append(
        Some("Insert Terminal Number (0-base)"),
        Some("terminal.insert_terminal_number_zero"),
    );
    let sync_inputs_item =
        gtk::gio::MenuItem::new(Some("Synchronize Inputs"), Some("terminal.sync_inputs"));
    sync_inputs_item.set_attribute_value("role", Some(&"check".to_variant()));
    menu.append_item(&sync_inputs_item);

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
        let context = context.clone();
        let terminal = terminal.clone();
        let copy_action = copy_action.clone();
        let add_terminals_action = add_terminals_action.clone();
        let sync_inputs_action = sync_inputs_action.clone();
        let popover = popover.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            let count = context.state.borrow().sessions().len();
            copy_action.set_enabled(terminal.has_selection());
            add_terminals_action.set_enabled(matches!(count, 1 | 2 | 4 | 6 | 8 | 12));
            sync_inputs_action.set_state(
                &context
                    .sync_inputs_enabled
                    .load(Ordering::Relaxed)
                    .to_variant(),
            );
            let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.set_offset(0, 0);
            popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    terminal.add_controller(right_click);
}

fn install_nudge_pill_interactions(
    context: &Rc<AppContext>,
    pill: &gtk::Label,
    session_id: SessionId,
) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    {
        let context = context.clone();
        click.connect_released(move |gesture, _, _, _| {
            toggle_auto_nudge(&context, session_id);
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    pill.add_controller(click);

    let motion = gtk::EventControllerMotion::new();
    {
        let context = context.clone();
        motion.connect_enter(move |_, _, _| {
            set_auto_nudge_hover(&context, session_id, true);
        });
    }
    {
        let context = context.clone();
        motion.connect_leave(move |_| {
            set_auto_nudge_hover(&context, session_id, false);
        });
    }
    pill.add_controller(motion);
}

fn split_terminal_here(context: &Rc<AppContext>, source_session: SessionId) {
    if daemon_backed(context) {
        if let Some(beachhead) = context.beachhead.as_ref() {
            let _ = beachhead
                .commands()
                .send(ClientMessage::AddTerminals { source_session });
        }
        return;
    }
    let current_count = context.state.borrow().sessions().len();
    let additions = match current_count {
        1 => 1,
        2 | 4 | 6 => 2,
        8 | 12 => 4,
        _ => 0,
    };
    if additions == 0 {
        return;
    }

    let cwd = context
        .state
        .borrow()
        .session(source_session)
        .and_then(|session| session.launch.cwd.clone());
    let mut last_session = None;
    for _ in 0..additions {
        let number = context.state.borrow().sessions().len() + 1;
        let mut launch = default_shell_launch(context, number);
        if matches!(context.mode, RunMode::Local) {
            if let Some(cwd) = cwd.clone() {
                launch = launch.with_cwd(cwd);
            }
        }
        last_session = Some(append_session_card(context, launch));
    }

    let Some(new_session) = last_session else {
        return;
    };
    context.state.borrow_mut().select_session(new_session);
    if let Some(card) = context.session_cards.borrow().get(&new_session) {
        context.cards.select_child(&card.row);
    }
    refresh_runtime_and_cards(context);
    refresh_workspace(context);
    if context.state.borrow().focused_session().is_none()
        && battlefield_embeds_terminal(context, new_session)
    {
        if let Some(card) = context.session_cards.borrow().get(&new_session) {
            card.terminal.grab_focus();
        }
    }
    refresh_card_styles(context);
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

fn spawn_summary_worker() -> Option<SummaryWorker> {
    let config = OpenAiSynthesisConfig::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<SummaryJob>();
    let (result_tx, result_rx) = mpsc::channel::<SummaryResult>();

    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let summary = summarize_blocking(&config, &job.evidence);
            let _ = result_tx.send(SummaryResult {
                session_id: job.session_id,
                signature: job.signature,
                summary,
            });
        }
    });

    Some(SummaryWorker {
        requests: request_tx,
        responses: result_rx,
    })
}

fn spawn_naming_worker() -> Option<NamingWorker> {
    let config = OpenAiNamingConfig::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<NamingJob>();
    let (result_tx, result_rx) = mpsc::channel::<NamingResult>();

    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion = suggest_name_blocking(&config, &job.evidence);
            let _ = result_tx.send(NamingResult {
                session_id: job.session_id,
                signature: job.signature,
                suggestion,
            });
        }
    });

    Some(NamingWorker {
        requests: request_tx,
        responses: result_rx,
    })
}

fn spawn_nudge_worker() -> Option<NudgeWorker> {
    let config = OpenAiNudgeConfig::from_env()?;
    let (request_tx, request_rx) = mpsc::channel::<NudgeJob>();
    let (result_tx, result_rx) = mpsc::channel::<NudgeResult>();

    thread::spawn(move || {
        while let Ok(job) = request_rx.recv() {
            let suggestion = suggest_nudge_blocking(&config, &job.evidence);
            let _ = result_tx.send(NudgeResult {
                session_id: job.session_id,
                signature: job.signature,
                suggestion,
            });
        }
    });

    Some(NudgeWorker {
        requests: request_tx,
        responses: result_rx,
    })
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
            ServerMessage::Error { message } => {
                eprintln!("beachhead error: {message}");
            }
        }
    }

    if changed {
        let sessions = context.state.borrow().sessions().to_vec();
        for session in &sessions {
            update_battle_card_widgets(context, session);
        }
        refresh_workspace(context);
        refresh_card_styles(context);
        refresh_focus_panel(context);
    }
}

fn apply_workspace_snapshot(context: &Rc<AppContext>, snapshot: WorkspaceSnapshot) {
    let session_ids = snapshot
        .sessions
        .iter()
        .map(|session| session.record.id)
        .collect::<Vec<_>>();

    context.state.borrow_mut().replace_sessions(
        snapshot
            .sessions
            .iter()
            .map(|session| session.record.clone())
            .collect(),
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
            context.cards.remove(&card.row);
        }
        context.observations.borrow_mut().remove(&session_id);
        context
            .raw_stream_socket_names
            .borrow_mut()
            .remove(&session_id);
        context.display_runtimes.borrow_mut().remove(&session_id);
        if let Ok(mut writers) = context.raw_input_writers.lock() {
            writers.remove(&session_id);
        }
        context.summary_cache.borrow_mut().remove(&session_id);
        context.naming_cache.borrow_mut().remove(&session_id);
        context.nudge_cache.borrow_mut().remove(&session_id);
    }

    for session in snapshot.sessions {
        {
            let mut names = context.raw_stream_socket_names.borrow_mut();
            if let Some(socket_name) = session.raw_stream_socket_name.clone() {
                names.insert(session.record.id, socket_name);
            } else {
                names.remove(&session.record.id);
            }
        }
        if !context
            .session_cards
            .borrow()
            .contains_key(&session.record.id)
        {
            let card = build_battle_card_widgets(context, &session.record);
            context.cards.insert(&card.row, -1);
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
        {
            let mut summary_cache = context.summary_cache.borrow_mut();
            let cache = summary_cache
                .entry(session.record.id)
                .or_insert_with(SummaryCacheEntry::new);
            cache.last_summary = session.summary;
        }
        {
            let mut nudge_cache = context.nudge_cache.borrow_mut();
            let nudge = nudge_cache
                .entry(session.record.id)
                .or_insert_with(NudgeCacheEntry::new);
            nudge.enabled = session.auto_nudge_enabled;
            nudge.last_nudge = session.last_nudge;
            nudge.last_sent = session
                .last_sent_age_secs
                .map(|age| Instant::now() - Duration::from_secs(age));
        }
    }

    let selected = context.state.borrow().selected_session();
    if let Some(selected) = selected {
        let row = context
            .session_cards
            .borrow()
            .get(&selected)
            .map(|card| card.row.clone());
        if let Some(row) = row {
            context.cards.select_child(&row);
        }
    }
    update_flowbox_columns(context);
}

fn observation_from_snapshot(snapshot: &ObservationSnapshot) -> SessionObservation {
    SessionObservation {
        last_change: Instant::now() - Duration::from_secs(snapshot.last_change_age_secs),
        recent_lines: snapshot.recent_lines.clone(),
        terminal_activity: Vec::new(),
        painted_line: snapshot.painted_line.clone(),
        shell_child_command: snapshot.shell_child_command.clone(),
        active_command: snapshot.active_command.clone(),
        dominant_process: snapshot.dominant_process.clone(),
        process_tree_excerpt: snapshot.process_tree_excerpt.clone(),
        recent_files: snapshot.recent_files.clone(),
        recent_file_activity: BTreeMap::new(),
        work_output_excerpt: snapshot.work_output_excerpt.clone(),
    }
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
            context.raw_input_writers.clone(),
            context.sync_inputs_enabled.clone(),
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
    drain_summary_results(context);
    drain_naming_results(context);
    drain_nudge_results(context);
    drain_runtime_events(context);
    update_flowbox_columns(context);
    let sessions = context.state.borrow().sessions().to_vec();
    for session in &sessions {
        refresh_observation(context, session);
    }
    for session in &sessions {
        update_battle_card_widgets(context, session);
    }
    sync_terminal_parents(context);
    refresh_workspace(context);
    refresh_card_styles(context);
    refresh_focus_panel(context);
    sync_runtime_sizes(context);
}

fn drain_naming_results(context: &Rc<AppContext>) {
    let Some(worker) = context.naming_worker.as_ref() else {
        return;
    };

    while let Ok(result) = worker.responses.try_recv() {
        let mut cache = context.naming_cache.borrow_mut();
        let entry = cache
            .entry(result.session_id)
            .or_insert_with(NamingCacheEntry::new);
        entry.in_flight = false;
        entry.requested_signature = None;
        entry.last_attempt = Some(Instant::now());
        match result.suggestion {
            Ok(suggestion) => {
                entry.completed_signature = Some(result.signature);
                if !suggestion.name.is_empty() {
                    entry.last_name = Some(suggestion.name.clone());
                    context
                        .state
                        .borrow_mut()
                        .set_display_name(result.session_id, Some(suggestion.name));
                }
                entry.last_error = None;
            }
            Err(error) => {
                entry.last_error = Some(error);
            }
        }
    }
}

fn drain_nudge_results(context: &Rc<AppContext>) {
    let Some(worker) = context.nudge_worker.as_ref() else {
        return;
    };

    while let Ok(result) = worker.responses.try_recv() {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache
            .entry(result.session_id)
            .or_insert_with(NudgeCacheEntry::new);
        entry.in_flight = false;
        entry.requested_signature = None;
        entry.last_attempt = Some(Instant::now());
        match result.suggestion {
            Ok(suggestion) => {
                entry.completed_signature = Some(result.signature);
                entry.last_error = None;
                entry.last_nudge = (!suggestion.text.is_empty()).then_some(suggestion.text.clone());
                if !suggestion.text.is_empty()
                    && send_runtime_input_line(context, result.session_id, &suggestion.text).is_ok()
                {
                    entry.last_sent = Some(Instant::now());
                }
            }
            Err(error) => {
                entry.last_error = Some(error);
            }
        }
    }
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
    let focused_size = predicted_focus_terminal_size(context);
    let sizes = context
        .session_cards
        .borrow()
        .iter()
        .map(|(session_id, card)| {
            let size = if context.state.borrow().focused_session() == Some(*session_id)
                || battlefield_embeds_terminal(context, *session_id)
            {
                measured_terminal_size_hint(&card.terminal)
            } else {
                focused_size
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

fn predicted_focus_terminal_size(context: &Rc<AppContext>) -> Option<PtySize> {
    let focused = context.state.borrow().focused_session()?;
    let cards = context.session_cards.borrow();
    let card = cards.get(&focused)?;
    measured_terminal_size_hint(&card.terminal).or_else(|| Some(terminal_size_hint(&card.terminal)))
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

fn drain_summary_results(context: &Rc<AppContext>) {
    let Some(worker) = context.summary_worker.as_ref() else {
        return;
    };

    while let Ok(result) = worker.responses.try_recv() {
        let mut cache = context.summary_cache.borrow_mut();
        let entry = cache
            .entry(result.session_id)
            .or_insert_with(SummaryCacheEntry::new);
        entry.in_flight = false;
        entry.requested_signature = None;
        entry.last_attempt = Some(Instant::now());
        match result.summary {
            Ok(summary) => {
                entry.completed_signature = Some(result.signature);
                entry.last_summary = Some(summary);
                entry.last_error = None;
            }
            Err(error) => {
                entry.last_error = Some(error);
            }
        }
    }
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

fn update_battle_card_widgets(context: &Rc<AppContext>, session: &SessionRecord) {
    let Some(card) = context.session_cards.borrow().get(&session.id).cloned() else {
        return;
    };
    let observations = context.observations.borrow();
    let Some(observation) = observations.get(&session.id) else {
        return;
    };

    let observed = ObservedActivity {
        active_command: observation.active_command.clone(),
        dominant_process: observation.dominant_process.clone(),
        recent_files: observation.recent_files.clone(),
        work_output_excerpt: observation.work_output_excerpt.clone(),
        idle_seconds: Some(observation.last_change.elapsed().as_secs()),
    };
    let mut card_model = build_battle_card(session, &observed);
    let should_synthesize = !is_bare_waiting_shell(session, observation);
    let evidence = build_tactical_evidence(session, observation);
    if should_synthesize {
        maybe_queue_summary(context, session.id, &evidence);
        let naming = build_naming_evidence(session, observation);
        maybe_queue_name(context, session.id, &naming);
    }
    let live_summary = if should_synthesize {
        current_summary(context, session.id, &evidence)
    } else {
        None
    };
    maybe_queue_nudge(context, session, observation, live_summary.as_ref());
    let chrome_mode = CardChromeMode::from_summary(live_summary.as_ref());
    if let Some(summary) = live_summary.clone() {
        card_model = apply_tactical_synthesis(card_model, summary);
    }
    let visual_status = if chrome_mode.summarized() {
        card_model.status
    } else {
        BattleCardStatus::Idle
    };

    let display_name = ui_display_name(session, chrome_mode);
    card.title.set_label(&display_name);
    apply_battle_status_style(&card.status, visual_status);
    apply_battle_card_surface_style(&card.frame, visual_status);
    card.status.set_label(&status_chip_label(
        card_model.status,
        &card_model.recency_label,
    ));
    card.recency.set_label("");
    card.recency.set_visible(false);
    card.headline.set_label(&card_model.headline);

    let scrollback = scrollback_fragments(
        observation,
        visible_scrollback_line_capacity(card.scrollback_band.height()),
    );
    repopulate_scrollback_band(
        &card.scrollback_content,
        &card.scrollback_lines,
        &scrollback,
    );
    card.scrollback_band.set_visible(true);

    apply_metric_widgets(
        &card,
        live_summary.as_ref(),
        Some(observation.last_change.elapsed().as_secs()),
    );

    card.alert.set_label("");
    card.alert.set_visible(false);
    let in_focus_mode = context.state.borrow().focused_session().is_some();
    if !in_focus_mode && chrome_mode.summarized() {
        apply_nudge_pill(&context.nudge_cache.borrow(), session.id, &card.nudge_state);
    }
    apply_summary_chrome_visibility(
        &card,
        card_chrome_visibility(chrome_mode, in_focus_mode, false),
    );
}

fn maybe_queue_summary(
    context: &Rc<AppContext>,
    session_id: SessionId,
    evidence: &TacticalEvidence,
) {
    if daemon_backed(context) {
        return;
    }
    if visual_gallery_enabled() {
        return;
    }

    let Some(worker) = context.summary_worker.as_ref() else {
        return;
    };

    let signature = summary_signature(evidence);
    let mut cache = context.summary_cache.borrow_mut();
    let entry = cache
        .entry(session_id)
        .or_insert_with(SummaryCacheEntry::new);

    if entry.completed_signature.as_deref() == Some(signature.as_str())
        || entry.requested_signature.as_deref() == Some(signature.as_str())
        || entry.in_flight
    {
        return;
    }

    let refresh_interval = summary_refresh_interval(entry.first_seen.elapsed());
    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < refresh_interval)
    {
        return;
    }

    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    entry.requested_signature = Some(signature.clone());
    let _ = worker.requests.send(SummaryJob {
        session_id,
        signature,
        evidence: evidence.clone(),
    });
}

fn maybe_queue_nudge(
    context: &Rc<AppContext>,
    session: &SessionRecord,
    observation: &SessionObservation,
    summary: Option<&TacticalSynthesis>,
) {
    if daemon_backed(context) {
        return;
    }
    if visual_gallery_enabled() {
        return;
    }

    let Some(summary) = summary else {
        return;
    };
    if summary.tactical_state != TacticalState::Stopped {
        return;
    }
    let Some(shell_child_command) = observation.shell_child_command.as_deref() else {
        return;
    };
    if !looks_like_coding_agent(shell_child_command) {
        return;
    }
    let idle_seconds = observation.last_change.elapsed().as_secs();
    if idle_seconds < 20 {
        return;
    }

    let Some(worker) = context.nudge_worker.as_ref() else {
        return;
    };

    let mut cache = context.nudge_cache.borrow_mut();
    let entry = cache.entry(session.id).or_insert_with(NudgeCacheEntry::new);
    if !entry.enabled || entry.in_flight {
        return;
    }
    if entry
        .last_sent
        .is_some_and(|sent| sent.elapsed() < Duration::from_secs(120))
    {
        return;
    }
    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(10))
    {
        return;
    }

    let evidence = build_nudge_evidence(session, observation, summary);
    let signature = nudge_signature(&evidence);
    if entry.requested_signature.as_deref() == Some(signature.as_str()) {
        return;
    }

    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    entry.requested_signature = Some(signature.clone());
    let _ = worker.requests.send(NudgeJob {
        session_id: session.id,
        signature,
        evidence,
    });
}

fn looks_like_coding_agent(command: &str) -> bool {
    matches!(
        command,
        "codex" | "claude" | "claude-code" | "aider" | "opencode" | "goose" | "gemini"
    )
}

fn maybe_queue_name(context: &Rc<AppContext>, session_id: SessionId, evidence: &NamingEvidence) {
    if daemon_backed(context) {
        return;
    }
    if visual_gallery_enabled() {
        return;
    }

    let Some(worker) = context.naming_worker.as_ref() else {
        return;
    };

    let signature = name_signature(evidence);
    let mut cache = context.naming_cache.borrow_mut();
    let entry = cache
        .entry(session_id)
        .or_insert_with(NamingCacheEntry::new);

    if entry.completed_signature.as_deref() == Some(signature.as_str())
        || entry.requested_signature.as_deref() == Some(signature.as_str())
        || entry.in_flight
    {
        return;
    }

    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(20))
    {
        return;
    }

    entry.in_flight = true;
    entry.last_attempt = Some(Instant::now());
    entry.requested_signature = Some(signature.clone());
    let _ = worker.requests.send(NamingJob {
        session_id,
        signature,
        evidence: evidence.clone(),
    });
}

fn current_summary(
    context: &Rc<AppContext>,
    session_id: SessionId,
    _evidence: &TacticalEvidence,
) -> Option<TacticalSynthesis> {
    if daemon_backed(context) {
        return context
            .summary_cache
            .borrow()
            .get(&session_id)
            .and_then(|entry| entry.last_summary.clone());
    }
    if visual_gallery_enabled() {
        return gallery_mock_summary(context, session_id);
    }

    let cache = context.summary_cache.borrow();
    let entry = cache.get(&session_id)?;
    entry.last_summary.clone()
}

fn should_show_summary(context: &Rc<AppContext>, session_id: SessionId) -> bool {
    let state = context.state.borrow();
    let observations = context.observations.borrow();
    let Some(session) = state.session(session_id) else {
        return false;
    };
    let Some(observation) = observations.get(&session_id) else {
        return false;
    };
    !is_bare_waiting_shell(session, observation)
}

fn card_chrome_mode_for_session(context: &Rc<AppContext>, session_id: SessionId) -> CardChromeMode {
    if visual_gallery_enabled() {
        return CardChromeMode::Summarized;
    }
    if !should_show_summary(context, session_id) {
        return CardChromeMode::SparseShell;
    }
    let has_summary = context
        .summary_cache
        .borrow()
        .get(&session_id)
        .and_then(|entry| entry.last_summary.as_ref())
        .is_some();
    if has_summary {
        CardChromeMode::Summarized
    } else {
        CardChromeMode::SparseShell
    }
}

fn gallery_mock_summary(
    context: &Rc<AppContext>,
    session_id: SessionId,
) -> Option<TacticalSynthesis> {
    let state = context.state.borrow();
    let session = state.session(session_id)?;
    let name = session.launch.name.as_str();
    Some(match name {
        "Agent A" => TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: Some("Narrowing the parser failure with focused reruns".into()),
            attention_level: AttentionLevel::Monitor,
            attention_brief: Some(
                "The loop is healthy and converging, but it is still worth watching.".into(),
            ),
            headline: Some("Tight edit-test loop, still failing but converging.".into()),
        },
        "Agent B" => TacticalSynthesis {
            tactical_state: TacticalState::Stopped,
            tactical_state_brief: Some("Paused after a clean checkpoint".into()),
            attention_level: AttentionLevel::Guide,
            attention_brief: Some(
                "A simple continue prompt is probably enough to restart useful work.".into(),
            ),
            headline: Some("Looks done with this pass and waiting for a nudge.".into()),
        },
        "Agent C" => TacticalSynthesis {
            tactical_state: TacticalState::Blocked,
            tactical_state_brief: Some("Waiting on explicit approval".into()),
            attention_level: AttentionLevel::Intervene,
            attention_brief: Some("The next step is blocked on real operator approval.".into()),
            headline: Some("Hard stop on approval boundary; operator input required.".into()),
        },
        "Agent D" => TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: Some("Retrying the same failing path".into()),
            attention_level: AttentionLevel::Guide,
            attention_brief: Some(
                "The loop is repeating without a decisive new clue and may need redirection soon."
                    .into(),
            ),
            headline: Some("Retry loop is repeating without a decisive new clue.".into()),
        },
        "Agent E" => TacticalSynthesis {
            tactical_state: TacticalState::Idle,
            tactical_state_brief: Some("Stable after validation with nothing to resume".into()),
            attention_level: AttentionLevel::Autopilot,
            attention_brief: Some(
                "This looks stably parked with no meaningful next step pending.".into(),
            ),
            headline: Some("Looks stably parked after validation, not suspiciously idle.".into()),
        },
        "Agent F" => TacticalSynthesis {
            tactical_state: TacticalState::Working,
            tactical_state_brief: Some(
                "Escalating from disk pressure into risky cleanup ideas".into(),
            ),
            attention_level: AttentionLevel::Takeover,
            attention_brief: Some(
                "Risky cleanup ideas and frustration mean the operator should take direct control."
                    .into(),
            ),
            headline: Some(
                "Blocked on disk space and drifting toward risky cleanup actions.".into(),
            ),
        },
        _ => return None,
    })
}

fn apply_tactical_synthesis(
    mut card_model: BattleCardViewModel,
    summary: TacticalSynthesis,
) -> BattleCardViewModel {
    card_model.status = match summary.tactical_state {
        TacticalState::Idle => BattleCardStatus::Idle,
        TacticalState::Stopped => BattleCardStatus::Stopped,
        TacticalState::Thinking => BattleCardStatus::Thinking,
        TacticalState::Working => BattleCardStatus::Working,
        TacticalState::Blocked => BattleCardStatus::Blocked,
        TacticalState::Failed => BattleCardStatus::Failed,
        TacticalState::Complete => BattleCardStatus::Complete,
        TacticalState::Detached => BattleCardStatus::Detached,
    };
    card_model.recency_label = match card_model.status {
        BattleCardStatus::Idle | BattleCardStatus::Stopped => card_model.recency_label,
        _ if card_model.recency_label.starts_with("idle ") => "active now".into(),
        _ => card_model.recency_label,
    };

    if let Some(headline) = summary.headline.clone() {
        card_model.headline = headline;
    }
    if let Some(text) = summary.attention_brief.clone() {
        card_model.alignment.text = text;
        card_model.alignment.tone = if matches!(
            summary.attention_level,
            AttentionLevel::Intervene | AttentionLevel::Takeover
        ) {
            SignalTone::Alert
        } else if matches!(
            summary.attention_level,
            AttentionLevel::Monitor | AttentionLevel::Guide
        ) {
            SignalTone::Watch
        } else {
            SignalTone::Calm
        };
    }

    card_model
}

fn apply_metric_widgets(
    card: &SessionCardWidgets,
    summary: Option<&TacticalSynthesis>,
    _idle_seconds: Option<u64>,
) {
    let attention = attention_bar_presentation(summary);
    apply_segmented_bar(&card.momentum_bar, attention.as_ref(), summary.is_some());
    apply_segmented_bar(&card.risk_bar, None, false);
}

fn apply_segmented_bar(
    bar: &SegmentedBarWidgets,
    value: Option<&(
        exaterm_ui::presentation::SegmentedBarPresentation,
        Option<String>,
    )>,
    show_when_empty: bool,
) {
    let Some((presentation, reason)) = value else {
        bar.frame.set_visible(show_when_empty);
        bar.reason.set_label("");
        bar.frame.set_tooltip_text(None::<&str>);
        for segment in &bar.segments {
            for css in [
                "bar-attention-1",
                "bar-attention-2",
                "bar-attention-3",
                "bar-attention-4",
                "bar-attention-5",
                "bar-empty",
            ] {
                segment.remove_css_class(css);
            }
            segment.add_css_class("bar-empty");
        }
        return;
    };

    bar.frame.set_visible(true);
    bar.reason.set_label(reason.as_deref().unwrap_or(""));
    bar.reason
        .set_visible(reason.as_deref().is_some_and(|reason| !reason.is_empty()));
    bar.frame.set_tooltip_text(reason.as_deref());

    for (index, segment) in bar.segments.iter().enumerate() {
        for css in [
            "bar-attention-1",
            "bar-attention-2",
            "bar-attention-3",
            "bar-attention-4",
            "bar-attention-5",
            "bar-empty",
        ] {
            segment.remove_css_class(css);
        }
        if index < presentation.fill {
            segment.add_css_class(presentation.css_class);
        } else {
            segment.add_css_class("bar-empty");
        }
    }
}

fn apply_focus_attention_pill(pill: &gtk::Label, summary: Option<&TacticalSynthesis>) {
    for css in [
        "focus-attention-1",
        "focus-attention-2",
        "focus-attention-3",
        "focus-attention-4",
        "focus-attention-5",
    ] {
        pill.remove_css_class(css);
    }

    let Some(summary) = summary else {
        pill.set_visible(false);
        pill.set_label("");
        pill.set_tooltip_text(None::<&str>);
        return;
    };

    let (label, css) = match summary.attention_level {
        AttentionLevel::Autopilot => ("AUTOPILOT", "focus-attention-1"),
        AttentionLevel::Monitor => ("MONITOR", "focus-attention-2"),
        AttentionLevel::Guide => ("GUIDE", "focus-attention-3"),
        AttentionLevel::Intervene => ("INTERVENE", "focus-attention-4"),
        AttentionLevel::Takeover => ("TAKEOVER", "focus-attention-5"),
    };
    pill.set_label(label);
    pill.add_css_class(css);
    pill.set_tooltip_text(summary.attention_brief.as_deref());
    pill.set_visible(true);
}

fn apply_attention_pill(pill: &gtk::Label, summary: Option<&TacticalSynthesis>) {
    for css in [
        "focus-attention-1",
        "focus-attention-2",
        "focus-attention-3",
        "focus-attention-4",
        "focus-attention-5",
    ] {
        pill.remove_css_class(css);
    }

    let Some(summary) = summary else {
        pill.set_visible(false);
        pill.set_label("");
        pill.set_tooltip_text(None::<&str>);
        return;
    };

    let (label, css) = match summary.attention_level {
        AttentionLevel::Autopilot => ("AUTOPILOT", "focus-attention-1"),
        AttentionLevel::Monitor => ("MONITOR", "focus-attention-2"),
        AttentionLevel::Guide => ("GUIDE", "focus-attention-3"),
        AttentionLevel::Intervene => ("INTERVENE", "focus-attention-4"),
        AttentionLevel::Takeover => ("TAKEOVER", "focus-attention-5"),
    };
    pill.set_label(label);
    pill.add_css_class(css);
    pill.set_tooltip_text(summary.attention_brief.as_deref());
    pill.set_visible(true);
}

fn refresh_workspace(context: &Rc<AppContext>) {
    let sessions = context.state.borrow().sessions().to_vec();
    let mut idle = 0usize;
    let mut active = 0usize;
    let mut failed = 0usize;

    for session in &sessions {
        if let Some(observation) = context.observations.borrow().get(&session.id) {
            let observed = ObservedActivity {
                active_command: observation.active_command.clone(),
                dominant_process: observation.dominant_process.clone(),
                recent_files: observation.recent_files.clone(),
                work_output_excerpt: observation.work_output_excerpt.clone(),
                idle_seconds: Some(observation.last_change.elapsed().as_secs()),
            };
            let status = build_battle_card(session, &observed).status;
            match status {
                BattleCardStatus::Idle => idle += 1,
                BattleCardStatus::Stopped => active += 1,
                BattleCardStatus::Active
                | BattleCardStatus::Thinking
                | BattleCardStatus::Working => active += 1,
                BattleCardStatus::Blocked => active += 1,
                BattleCardStatus::Failed => failed += 1,
                BattleCardStatus::Complete | BattleCardStatus::Detached => {}
            }
        }
    }

    let is_empty = sessions.is_empty();
    context.empty_state.set_visible(is_empty);
    context.battlefield_panel.set_visible(!is_empty);
    let state = context.state.borrow();
    let _ = (sessions, idle, active, failed, state);
    context.title.set_subtitle("");
}

fn refresh_card_styles(context: &Rc<AppContext>) {
    const FOCUS_RAIL_CARD_WIDTH: i32 = 168;

    let selected = context.state.borrow().selected_session();
    let focused = context.state.borrow().focused_session();
    let focus_mode = focused.is_some();
    let single_card_mode = !focus_mode && context.session_cards.borrow().len() == 1;
    for (session_id, card) in context.session_cards.borrow().iter() {
        card.row.remove_css_class("selected-card");
        card.row.remove_css_class("focused-card");
        card.frame.remove_css_class("single-card");
        if focus_mode && selected == Some(*session_id) {
            card.row.add_css_class("selected-card");
        }
        if focused == Some(*session_id) {
            card.row.add_css_class("focused-card");
        }
        if focus_mode {
            card.row.set_hexpand(false);
            card.frame.set_hexpand(false);
            card.row.set_halign(gtk::Align::Start);
            card.frame.set_halign(gtk::Align::Start);
            card.row.set_width_request(FOCUS_RAIL_CARD_WIDTH);
            card.frame.set_width_request(FOCUS_RAIL_CARD_WIDTH);
        } else {
            card.row.set_hexpand(true);
            card.frame.set_hexpand(true);
            card.row.set_halign(gtk::Align::Fill);
            card.frame.set_halign(gtk::Align::Fill);
            card.row.set_width_request(-1);
            card.frame.set_width_request(-1);
        }
        let chrome_visibility = card_chrome_visibility(
            card_chrome_mode_for_session(context, *session_id),
            focus_mode,
            !card.alert.label().is_empty(),
        );
        card.headline
            .set_visible(chrome_visibility.headline_visible);
        card.alert.set_wrap(focus_mode);
        card.alert.set_single_line_mode(!focus_mode);
        card.alert.set_ellipsize(if focus_mode {
            gtk::pango::EllipsizeMode::None
        } else {
            gtk::pango::EllipsizeMode::End
        });
        let summary = if focus_mode {
            let evidence = context
                .observations
                .borrow()
                .get(session_id)
                .map(|observation| {
                    build_tactical_evidence(
                        context
                            .state
                            .borrow()
                            .session(*session_id)
                            .expect("session should exist"),
                        observation,
                    )
                });
            evidence.and_then(|evidence| current_summary(context, *session_id, &evidence))
        } else {
            None
        };
        let combined_headline = combined_focus_summary_text(
            summary
                .as_ref()
                .and_then(|s| s.headline.as_deref())
                .unwrap_or(""),
            summary.as_ref().and_then(|s| s.attention_brief.as_deref()),
        );
        card.headline.set_label(&combined_headline);
        card.headline.set_vexpand(focus_mode);
        card.headline_row.set_vexpand(focus_mode);
        card.headline_row.set_valign(if focus_mode {
            gtk::Align::Fill
        } else {
            gtk::Align::Start
        });
        card.headline.set_valign(gtk::Align::Start);
        card.headline.set_lines(if focus_mode { 4 } else { 2 });
        apply_attention_pill(
            &card.attention_pill,
            if focus_mode { summary.as_ref() } else { None },
        );
        card.attention_pill
            .set_visible(focus_mode && summary.is_some());
        apply_summary_chrome_visibility(card, chrome_visibility);
        let shows_terminal = battlefield_embeds_terminal(context, *session_id);
        card.bars.set_orientation(if shows_terminal {
            gtk::Orientation::Horizontal
        } else {
            gtk::Orientation::Vertical
        });
        card.bars.set_homogeneous(shows_terminal);
        if shows_terminal {
            card.frame.remove_css_class("scrollback-card");
            card.terminal_slot
                .remove_css_class("scrollback-terminal-hidden");
        } else {
            card.frame.add_css_class("scrollback-card");
            card.terminal_slot
                .add_css_class("scrollback-terminal-hidden");
        }
        if focus_mode {
            card.middle_stack.set_visible_child_name("scrollback");
            card.middle_stack.set_visible(false);
        } else {
            card.middle_stack.set_visible(true);
            card.middle_stack.set_visible_child_name(if shows_terminal {
                "terminal"
            } else {
                "scrollback"
            });
            card.scrollback_band.set_visible(!shows_terminal);
            if single_card_mode {
                card.frame.add_css_class("single-card");
            }
        }
    }
}

fn show_intervention(context: &Rc<AppContext>, session_id: SessionId) {
    context.state.borrow_mut().enter_focus_mode(session_id);
    if let Some(card) = context.session_cards.borrow().get(&session_id) {
        context.cards.select_child(&card.row);
    }
    context.focus.panel.set_visible(true);
    context.content_root.add_css_class("focus-mode");
    context.cards.set_homogeneous(false);
    context.cards.set_halign(gtk::Align::Start);
    context.battlefield_panel.set_vexpand(false);
    context.battlefield_panel.set_height_request(240);
    context
        .battlefield_panel
        .set_hscrollbar_policy(gtk::PolicyType::Automatic);
    update_flowbox_columns(context);
    sync_terminal_parents(context);
    refresh_card_styles(context);
    refresh_focus_panel(context);
    refresh_workspace(context);
    schedule_runtime_size_sync(context);
}

fn show_battlefield(context: &Rc<AppContext>) {
    context.state.borrow_mut().return_to_battlefield();
    context.focus.panel.set_visible(false);
    context.content_root.remove_css_class("focus-mode");
    context.cards.set_homogeneous(true);
    context.cards.set_halign(gtk::Align::Fill);
    context.battlefield_panel.set_vexpand(true);
    context.battlefield_panel.set_height_request(-1);
    context
        .battlefield_panel
        .set_hscrollbar_policy(gtk::PolicyType::Never);
    update_flowbox_columns(context);
    sync_terminal_parents(context);
    refresh_card_styles(context);
    refresh_workspace(context);
    schedule_runtime_size_sync(context);
}

fn refresh_focus_panel(context: &Rc<AppContext>) {
    let Some(session_id) = context.state.borrow().focused_session() else {
        return;
    };
    let state = context.state.borrow();
    let Some(session) = state.session(session_id) else {
        return;
    };
    let observations = context.observations.borrow();
    let Some(observation) = observations.get(&session_id) else {
        return;
    };
    let observed = ObservedActivity {
        active_command: observation.active_command.clone(),
        dominant_process: observation.dominant_process.clone(),
        recent_files: observation.recent_files.clone(),
        work_output_excerpt: observation.work_output_excerpt.clone(),
        idle_seconds: Some(observation.last_change.elapsed().as_secs()),
    };
    let mut card_model = build_battle_card(session, &observed);
    let evidence = build_tactical_evidence(session, observation);
    let live_summary = if should_show_summary(context, session_id) {
        current_summary(context, session_id, &evidence)
    } else {
        None
    };
    if let Some(summary) = live_summary.clone() {
        card_model = apply_tactical_synthesis(card_model, summary);
    }
    let chrome_mode = CardChromeMode::from_summary(live_summary.as_ref());
    let visual_status = if chrome_mode.summarized() {
        card_model.status
    } else {
        BattleCardStatus::Idle
    };

    context
        .focus
        .title
        .set_label(&ui_display_name(session, chrome_mode));
    context.focus.title.set_visible(chrome_mode.summarized());
    apply_battle_status_style(&context.focus.status, visual_status);
    apply_battle_card_surface_style(&context.focus.frame, visual_status);
    context.focus.status.set_label(&status_chip_label(
        card_model.status,
        &card_model.recency_label,
    ));
    context.focus.status.set_visible(chrome_mode.summarized());
    context
        .focus
        .header
        .set_visible(context.focus.title.is_visible() || context.focus.status.is_visible());
    context.focus.headline.set_label(&card_model.headline);
    context
        .focus
        .headline
        .set_label(&combined_focus_summary_text(
            &card_model.headline,
            live_summary
                .as_ref()
                .and_then(|summary| summary.attention_brief.as_deref()),
        ));
    context.focus.headline.set_lines(4);
    context.focus.headline.set_vexpand(true);
    context.focus.headline.set_valign(gtk::Align::Start);
    context
        .focus
        .headline
        .set_visible(chrome_mode.summarized() && !context.focus.headline.label().is_empty());
    apply_focus_attention_pill(&context.focus.attention_pill, live_summary.as_ref());
    context.focus.summary_box.set_visible(
        context.focus.headline.is_visible() || context.focus.attention_pill.is_visible(),
    );
    context.focus.alert.set_label("");
    context.focus.alert.set_visible(false);
    context
        .focus
        .bars
        .set_orientation(gtk::Orientation::Horizontal);
    context.focus.bars.set_visible(false);
    apply_segmented_bar(
        &context.focus.momentum_bar,
        attention_bar_presentation(live_summary.as_ref()).as_ref(),
        live_summary.is_some(),
    );
    apply_segmented_bar(&context.focus.risk_bar, None, false);
}

fn update_flowbox_columns(context: &Rc<AppContext>) {
    let total = context.session_cards.borrow().len();
    if total == 0 {
        return;
    }

    let available_width = context.battlefield_panel.width();
    let columns = battlefield_columns(
        total,
        available_width,
        context.state.borrow().focused_session().is_some(),
    );
    context.cards.set_max_children_per_line(columns);
    context.cards.set_min_children_per_line(columns);
}

fn battlefield_embeds_terminal(context: &Rc<AppContext>, _session_id: SessionId) -> bool {
    if context.state.borrow().focused_session().is_some() {
        return false;
    }

    let total = context.session_cards.borrow().len();
    if total == 0 {
        return false;
    }

    let columns = current_battlefield_columns(context).max(1);
    let available_width = context.battlefield_panel.width();
    let available_height = context.battlefield_panel.height();
    battlefield_can_embed_terminals(total, columns, available_width, available_height)
}

fn current_battlefield_columns(context: &Rc<AppContext>) -> usize {
    let total = context.session_cards.borrow().len();
    if total == 0 {
        return 0;
    }
    context.cards.max_children_per_line().max(1) as usize
}

fn focused_embedded_terminal_session(context: &Rc<AppContext>) -> Option<SessionId> {
    context
        .session_cards
        .borrow()
        .iter()
        .find_map(|(session_id, card)| {
            (battlefield_embeds_terminal(context, *session_id) && card.terminal.has_focus())
                .then_some(*session_id)
        })
}

pub(crate) fn update_nudge_widgets(context: &Rc<AppContext>, session_id: SessionId) {
    if let Some(card) = context.session_cards.borrow().get(&session_id) {
        apply_nudge_pill(&context.nudge_cache.borrow(), session_id, &card.nudge_state);
    }
}

fn apply_summary_chrome_visibility(card: &SessionCardWidgets, visibility: CardChromeVisibility) {
    card.title.set_visible(visibility.title_visible);
    card.headline.set_visible(visibility.headline_visible);
    card.status.set_visible(visibility.status_visible);
    card.header.set_visible(visibility.header_visible);
    card.headline_row
        .set_visible(visibility.headline_visible || visibility.nudge_row_visible);
    card.bars.set_visible(visibility.bars_visible);
    card.nudge_state.set_visible(visibility.nudge_state_visible);
    card.nudge_row.set_visible(visibility.nudge_row_visible);
    card.footer
        .set_visible(visibility.bars_visible || card.recency.is_visible());
}

fn schedule_runtime_size_sync(context: &Rc<AppContext>) {
    sync_runtime_sizes(context);
    let context = context.clone();
    glib::idle_add_local_once(move || {
        sync_runtime_sizes(&context);
    });
}

fn visible_scrollback_line_capacity(height: i32) -> usize {
    layout_visible_scrollback_line_capacity(height)
}

fn repopulate_scrollback_band(
    scrollback_band: &gtk::DrawingArea,
    scrollback_lines: &Rc<RefCell<Vec<String>>>,
    lines: &[String],
) {
    let items = if lines.is_empty() {
        vec![" ".to_string()]
    } else {
        lines.to_vec()
    };
    *scrollback_lines.borrow_mut() = items;
    scrollback_band.queue_draw();
}

fn apply_nudge_pill(
    cache: &BTreeMap<SessionId, NudgeCacheEntry>,
    session_id: SessionId,
    state: &gtk::Label,
) {
    let cooldown_active = cache
        .get(&session_id)
        .and_then(|entry| entry.last_sent)
        .is_some_and(|sent| sent.elapsed() < Duration::from_secs(120));
    let hovered = cache.get(&session_id).is_some_and(|entry| entry.hovered);
    let enabled = cache.get(&session_id).is_some_and(|entry| entry.enabled);
    let presentation = nudge_state_presentation(enabled, cooldown_active, hovered);

    for candidate in [
        "card-control-off",
        "card-control-armed",
        "card-control-nudged",
        "card-control-cooldown",
    ] {
        state.remove_css_class(candidate);
    }
    state.add_css_class(presentation.css_class);
    state.set_label(presentation.label);
    state.set_visible(true);
}

fn sync_terminal_parents(context: &Rc<AppContext>) {
    let focused = context.state.borrow().focused_session();
    for (session_id, card) in context.session_cards.borrow().iter() {
        if focused == Some(*session_id) {
            reparent_widget_to_box(&card.terminal_view, &context.focus.terminal_slot);
            card.terminal.grab_focus();
        } else {
            reparent_widget_to_box(&card.terminal_view, &card.terminal_slot);
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
        card_chrome_visibility, parse_run_mode, summary_refresh_interval, CardChromeMode, RunMode,
    };
    use std::time::Duration;

    #[test]
    fn parses_ssh_run_mode() {
        let mode = parse_run_mode(vec!["--ssh".into(), "user@example.com".into()]).unwrap();
        assert_eq!(
            mode,
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
    fn summary_refresh_interval_starts_fast_and_backs_off() {
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(0)),
            Duration::from_secs(5)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(59)),
            Duration::from_secs(5)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(60)),
            Duration::from_secs(10)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(179)),
            Duration::from_secs(10)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(180)),
            Duration::from_secs(20)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(299)),
            Duration::from_secs(20)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(300)),
            Duration::from_secs(30)
        );
        assert_eq!(
            summary_refresh_interval(Duration::from_secs(900)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn sparse_shell_hides_all_summary_chrome() {
        let visibility = card_chrome_visibility(CardChromeMode::SparseShell, false, false);

        assert!(!visibility.title_visible);
        assert!(!visibility.status_visible);
        assert!(!visibility.header_visible);
        assert!(!visibility.bars_visible);
        assert!(!visibility.nudge_state_visible);
        assert!(!visibility.nudge_row_visible);
    }

    #[test]
    fn summarized_mode_shows_full_card_chrome() {
        let visibility = card_chrome_visibility(CardChromeMode::Summarized, false, true);

        assert!(visibility.title_visible);
        assert!(visibility.status_visible);
        assert!(visibility.header_visible);
        assert!(visibility.bars_visible);
        assert!(visibility.nudge_state_visible);
        assert!(visibility.nudge_row_visible);
    }
}
