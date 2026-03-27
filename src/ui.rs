use crate::model::{
    SessionId, SessionLaunch, WorkspaceState,
};
use crate::observation::{
    apply_stream_update, build_naming_evidence, build_nudge_evidence, build_tactical_evidence,
    effective_display_name, refresh_observation as refresh_session_observation,
    scrollback_fragments, SessionObservation,
};
use crate::runtime::{spawn_runtime, terminal_size_hint, RuntimeEvent, SessionRuntime};
use crate::synthesis::{
    name_signature, nudge_signature, suggest_name_blocking, suggest_nudge_blocking,
    summary_signature, summarize_blocking, MismatchLevel, MomentumState, NameSuggestion,
    NamingEvidence, NudgeEvidence, NudgeSuggestion, OpenAiNamingConfig, OpenAiNudgeConfig,
    OpenAiSynthesisConfig, OperatorAction, ProgressState, RiskPosture, TacticalEvidence,
    TacticalState, TacticalSynthesis,
};
use crate::supervision::{
    build_battle_card, BattleCardStatus, DeterministicIntentEngine, ObservedActivity, SignalTone,
};
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use portable_pty::PtySize;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vte::prelude::*;
use vte4 as vte;

const APP_ID: &str = "io.exaterm.Exaterm";

const ESTIMATED_TERMINAL_CELL_WIDTH: i32 = 8;
const ESTIMATED_TERMINAL_CELL_HEIGHT: i32 = 18;
const MIN_EMBEDDED_TERMINAL_COLS: i32 = 80;
const MIN_EMBEDDED_TERMINAL_ROWS: i32 = 24;
const EMBEDDED_TERMINAL_MIN_WIDTH: i32 = (ESTIMATED_TERMINAL_CELL_WIDTH * MIN_EMBEDDED_TERMINAL_COLS) + 72;
const EMBEDDED_TERMINAL_MIN_HEIGHT: i32 = (ESTIMATED_TERMINAL_CELL_HEIGHT * MIN_EMBEDDED_TERMINAL_ROWS) + 96;

#[derive(Clone)]
struct SegmentedBarWidgets {
    frame: gtk::Box,
    reason: gtk::Label,
    segments: Vec<gtk::Box>,
}

#[derive(Clone)]
struct SessionCardWidgets {
    row: gtk::FlowBoxChild,
    frame: gtk::Frame,
    title: gtk::Label,
    status: gtk::Label,
    nudge_row: gtk::Box,
    nudge_state: gtk::Label,
    recency: gtk::Label,
    middle_stack: gtk::Stack,
    scrollback_band: gtk::Box,
    terminal_slot: gtk::Box,
    bars: gtk::Box,
    headline: gtk::Label,
    detail: gtk::Label,
    evidence_one: gtk::Label,
    evidence_two: gtk::Label,
    evidence_three: gtk::Label,
    alert: gtk::Label,
    momentum_bar: SegmentedBarWidgets,
    risk_bar: SegmentedBarWidgets,
    terminal_view: gtk::ScrolledWindow,
    terminal: vte::Terminal,
}

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

struct NudgeCacheEntry {
    enabled: bool,
    hovered: bool,
    completed_signature: Option<String>,
    requested_signature: Option<String>,
    last_nudge: Option<String>,
    last_error: Option<String>,
    last_attempt: Option<Instant>,
    last_sent: Option<Instant>,
    in_flight: bool,
}

impl SummaryCacheEntry {
    fn new() -> Self {
        Self {
            completed_signature: None,
            requested_signature: None,
            last_summary: None,
            last_error: None,
            last_attempt: None,
            in_flight: false,
        }
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
    fn new() -> Self {
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

struct FocusWidgets {
    panel: gtk::Box,
    frame: gtk::Frame,
    title: gtk::Label,
    status: gtk::Label,
    nudge_row: gtk::Box,
    nudge_state: gtk::Label,
    alert: gtk::Label,
    terminal_slot: gtk::Box,
    bars: gtk::Box,
    momentum_bar: SegmentedBarWidgets,
    risk_bar: SegmentedBarWidgets,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RunMode {
    Local,
    Ssh { target: String },
}

struct AppContext {
    mode: RunMode,
    state: Rc<RefCell<WorkspaceState>>,
    title: adw::WindowTitle,
    empty_state: gtk::Box,
    content_root: gtk::Box,
    cards: gtk::FlowBox,
    battlefield_scroller: gtk::ScrolledWindow,
    focus: FocusWidgets,
    session_cards: RefCell<BTreeMap<SessionId, SessionCardWidgets>>,
    observations: RefCell<BTreeMap<SessionId, SessionObservation>>,
    runtimes: RefCell<BTreeMap<SessionId, SessionRuntime>>,
    summary_worker: Option<SummaryWorker>,
    summary_cache: RefCell<BTreeMap<SessionId, SummaryCacheEntry>>,
    naming_worker: Option<NamingWorker>,
    naming_cache: RefCell<BTreeMap<SessionId, NamingCacheEntry>>,
    nudge_worker: Option<NudgeWorker>,
    nudge_cache: RefCell<BTreeMap<SessionId, NudgeCacheEntry>>,
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
    let app = gtk::Application::builder().application_id(APP_ID).build();
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

fn parse_run_mode(args: impl IntoIterator<Item = String>) -> Result<RunMode, String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None => Ok(RunMode::Local),
        Some("--ssh") => {
            let Some(target) = args.next() else {
                return Err("--ssh requires a target like user@host".into());
            };
            if args.next().is_some() {
                return Err("unexpected extra arguments after --ssh target".into());
            }
            Ok(RunMode::Ssh { target })
        }
        Some(other) => Err(format!("unknown argument: {other}")),
    }
}

fn build_ui(app: &gtk::Application, mode: RunMode) {
    load_css();

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

    let battlefield_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&cards)
        .hexpand(true)
        .vexpand(true)
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
    let focus_status = gtk::Label::builder()
        .xalign(0.5)
        .css_classes(vec!["card-status".to_string(), "battle-active".to_string()])
        .label("Active")
        .build();
    let focus_alert = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .hexpand(true)
        .css_classes(vec!["card-alert".to_string()])
        .build();
    focus_alert.set_halign(gtk::Align::Fill);
    focus_alert.set_single_line_mode(true);
    focus_alert.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let focus_nudge_state = gtk::Label::builder()
        .label("AUTONUDGE OFF")
        .xalign(0.5)
        .css_classes(vec!["card-control-state".to_string(), "card-control-off".to_string()])
        .build();
    focus_nudge_state.set_halign(gtk::Align::End);
    let focus_nudge_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .visible(false)
        .build();
    focus_nudge_row.add_css_class("card-control-row");
    focus_nudge_row.append(&focus_alert);
    focus_nudge_row.append(&focus_nudge_state);
    let focus_terminal_slot = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    focus_terminal_slot.add_css_class("card-terminal-slot");
    let focus_momentum_bar = build_segmented_bar("Momentum");
    let focus_risk_bar = build_segmented_bar("Risk");

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
    focus_bars.append(&focus_momentum_bar.frame);
    focus_bars.append(&focus_risk_bar.frame);

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
    focus_content.append(&focus_nudge_row);
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
        .margin_top(18)
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
    content_root.append(&battlefield_scroller);
    content_root.append(&focus_panel);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&content_root);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Exaterm")
        .default_width(1480)
        .default_height(960)
        .content(&body)
        .build();

    let context = Rc::new(AppContext {
        mode: mode.clone(),
        state: Rc::new(RefCell::new(WorkspaceState::new())),
        title,
        empty_state,
        content_root,
        cards,
        battlefield_scroller,
        focus: FocusWidgets {
            panel: focus_panel,
            frame: focus_frame,
            title: focus_title,
            status: focus_status,
            nudge_row: focus_nudge_row,
            nudge_state: focus_nudge_state,
            alert: focus_alert,
            terminal_slot: focus_terminal_slot,
            bars: focus_bars,
            momentum_bar: focus_momentum_bar,
            risk_bar: focus_risk_bar,
        },
        session_cards: RefCell::new(BTreeMap::new()),
        observations: RefCell::new(BTreeMap::new()),
        runtimes: RefCell::new(BTreeMap::new()),
        summary_worker: spawn_summary_worker(),
        summary_cache: RefCell::new(BTreeMap::new()),
        naming_worker: spawn_naming_worker(),
        naming_cache: RefCell::new(BTreeMap::new()),
        nudge_worker: spawn_nudge_worker(),
        nudge_cache: RefCell::new(BTreeMap::new()),
    });

    install_focus_nudge_pill_interactions(&context, &context.focus.nudge_state);

    {
        let cards = context.cards.clone();
        let context = context.clone();
        cards.connect_selected_children_changed(move |flowbox| {
            let selected = flowbox.selected_children();
            let Some(selected_child) = selected.first() else {
                return;
            };
            let maybe_session = context
                .session_cards
                .borrow()
                .iter()
                .find_map(|(session_id, card)| (card.row == *selected_child).then_some(*session_id));
            if let Some(session_id) = maybe_session {
                if context.state.borrow().focused_session().is_some() {
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

    if visual_gallery_enabled() {
        seed_visual_gallery(&context);
    } else {
        let launch = default_shell_launch(&context, 1);
        append_session_card(&context, launch);
    }

    refresh_runtime_and_cards(&context);
    refresh_workspace(&context);

    window.present();
}

fn default_shell_launch(context: &Rc<AppContext>, number: usize) -> SessionLaunch {
    match &context.mode {
        RunMode::Local => SessionLaunch::user_shell(
            format!("Shell {number}"),
            "Generic command session",
        ),
        RunMode::Ssh { target } => SessionLaunch::ssh_shell(
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
    context.session_cards.borrow_mut().insert(session_id, card.clone());
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
        SessionLaunch::running_stream(
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
        SessionLaunch::planning_stream(
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
        SessionLaunch::blocking_prompt(
            "Agent C",
            "Deploy approval",
            "The deploy script is ready, but this next step will touch production. Proceed with deploy? [y/N]",
        ),
        SessionLaunch::running_stream(
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
        SessionLaunch::planning_stream(
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
        SessionLaunch::planning_stream(
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
    session: &crate::model::SessionRecord,
) -> SessionCardWidgets {
    let title = gtk::Label::builder()
        .label(&session.launch.name)
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
        .css_classes(vec!["card-control-state".to_string(), "card-control-off".to_string()])
        .build();
    nudge_state.set_halign(gtk::Align::End);
    let nudge_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .hexpand(true)
        .halign(gtk::Align::Fill)
        .visible(false)
        .build();
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
        .visible(false)
        .css_classes(vec!["card-headline".to_string()])
        .build();
    let detail = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(vec!["card-detail".to_string()])
        .build();
    let evidence_one = gtk::Label::builder()
        .xalign(0.0)
        .visible(false)
        .css_classes(vec!["card-scrollback-line".to_string()])
        .build();
    evidence_one.set_single_line_mode(true);
    evidence_one.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let evidence_two = gtk::Label::builder()
        .xalign(0.0)
        .visible(false)
        .css_classes(vec!["card-scrollback-line".to_string()])
        .build();
    evidence_two.set_single_line_mode(true);
    evidence_two.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let evidence_three = gtk::Label::builder()
        .xalign(0.0)
        .visible(false)
        .css_classes(vec!["card-scrollback-line".to_string()])
        .build();
    evidence_three.set_single_line_mode(true);
    evidence_three.set_ellipsize(gtk::pango::EllipsizeMode::End);
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
    let momentum_bar = build_segmented_bar("Momentum");
    let risk_bar = build_segmented_bar("Risk");

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

    let scrollback_band = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .vexpand(true)
        .valign(gtk::Align::Fill)
        .build();
    scrollback_band.add_css_class("card-scrollback-band");
    scrollback_band.append(&evidence_one);
    scrollback_band.append(&evidence_two);
    scrollback_band.append(&evidence_three);
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
    content.append(&nudge_row);
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
            context.cards.select_child(&row);
            context.state.borrow_mut().select_session(session_id);
            if context.state.borrow().focused_session() == Some(session_id) {
                show_battlefield(&context);
            } else if battlefield_embeds_terminal(&context, session_id) {
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
    install_terminal_context_menu(context, &terminal_view, &terminal, session.id);
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
        title,
        status,
        nudge_row,
        nudge_state,
        recency,
        middle_stack,
        scrollback_band,
        terminal_slot,
        bars,
        headline,
        detail,
        evidence_one,
        evidence_two,
        evidence_three,
        alert,
        momentum_bar,
        risk_bar,
        terminal_view,
        terminal,
    }
}

fn install_terminal_context_menu(
    context: &Rc<AppContext>,
    terminal_view: &gtk::ScrolledWindow,
    terminal: &vte::Terminal,
    source_session: SessionId,
) {
    let menu_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();
    let copy_button = gtk::Button::builder()
        .label("Copy")
        .halign(gtk::Align::Fill)
        .build();
    copy_button.add_css_class("flat");
    menu_box.append(&copy_button);
    let paste_button = gtk::Button::builder()
        .label("Paste")
        .halign(gtk::Align::Fill)
        .build();
    paste_button.add_css_class("flat");
    menu_box.append(&paste_button);
    let split_terminal_button = gtk::Button::builder()
        .label("Add Terminals")
        .halign(gtk::Align::Fill)
        .build();
    split_terminal_button.add_css_class("flat");
    menu_box.append(&split_terminal_button);

    let popover = gtk::Popover::builder()
        .has_arrow(true)
        .autohide(true)
        .child(&menu_box)
        .build();
    popover.set_parent(terminal_view);

    {
        let popover = popover.clone();
        let terminal = terminal.clone();
        copy_button.connect_clicked(move |_| {
            terminal.copy_clipboard_format(vte::Format::Text);
            popover.popdown();
        });
    }

    {
        let popover = popover.clone();
        let terminal = terminal.clone();
        paste_button.connect_clicked(move |_| {
            terminal.paste_clipboard();
            popover.popdown();
        });
    }

    {
        let context = context.clone();
        let popover = popover.clone();
        split_terminal_button.connect_clicked(move |_| {
            popover.popdown();
            split_terminal_here(&context, source_session);
        });
    }

    let right_click = gtk::GestureClick::new();
    right_click.set_button(3);
    {
        let context = context.clone();
        let terminal = terminal.clone();
        let copy_button = copy_button.clone();
        let split_terminal_button = split_terminal_button.clone();
        let popover = popover.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            let count = context.state.borrow().sessions().len();
            copy_button.set_sensitive(terminal.has_selection());
            split_terminal_button.set_sensitive(matches!(count, 1 | 2 | 4 | 6 | 8 | 12));
            let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    terminal_view.add_controller(right_click);
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

fn install_focus_nudge_pill_interactions(context: &Rc<AppContext>, pill: &gtk::Label) {
    let click = gtk::GestureClick::new();
    click.set_button(1);
    {
        let context = context.clone();
        click.connect_released(move |gesture, _, _, _| {
            if let Some(session_id) = context.state.borrow().focused_session() {
                toggle_auto_nudge(&context, session_id);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
    }
    pill.add_controller(click);

    let motion = gtk::EventControllerMotion::new();
    {
        let context = context.clone();
        motion.connect_enter(move |_, _, _| {
            if let Some(session_id) = context.state.borrow().focused_session() {
                set_auto_nudge_hover(&context, session_id, true);
            }
        });
    }
    {
        let context = context.clone();
        motion.connect_leave(move |_| {
            if let Some(session_id) = context.state.borrow().focused_session() {
                set_auto_nudge_hover(&context, session_id, false);
            }
        });
    }
    pill.add_controller(motion);
}

fn toggle_auto_nudge(context: &Rc<AppContext>, session_id: SessionId) {
    let enabled = {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache
            .entry(session_id)
            .or_insert_with(NudgeCacheEntry::new);
        entry.enabled = !entry.enabled;
        if !entry.enabled {
            entry.in_flight = false;
            entry.requested_signature = None;
            entry.hovered = false;
        }
        entry.enabled
    };
    update_nudge_widgets(context, session_id);
    if enabled {
        refresh_runtime_and_cards(context);
    }
}

fn set_auto_nudge_hover(context: &Rc<AppContext>, session_id: SessionId, hovered: bool) {
    let changed = {
        let mut cache = context.nudge_cache.borrow_mut();
        let entry = cache
            .entry(session_id)
            .or_insert_with(NudgeCacheEntry::new);
        let changed = entry.hovered != hovered;
        entry.hovered = hovered;
        changed
    };
    if changed {
        update_nudge_widgets(context, session_id);
    }
}

fn split_terminal_here(context: &Rc<AppContext>, source_session: SessionId) {
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
    if context.state.borrow().focused_session().is_none() && battlefield_embeds_terminal(context, new_session) {
        if let Some(card) = context.session_cards.borrow().get(&new_session) {
            card.terminal.grab_focus();
        }
    }
    refresh_card_styles(context);
}

fn build_segmented_bar(label: &str) -> SegmentedBarWidgets {
    let caption = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .css_classes(vec!["bar-caption".to_string()])
        .build();
    let bar = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .hexpand(true)
        .build();
    bar.add_css_class("segmented-bar");
    let segments = (0..4)
        .map(|_| {
            let segment = gtk::Box::builder().hexpand(true).build();
            segment.add_css_class("bar-segment");
            bar.append(&segment);
            segment
        })
        .collect::<Vec<_>>();
    let reason = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["bar-reason".to_string()])
        .build();
    let frame = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    frame.add_css_class("bar-widget");
    frame.append(&caption);
    frame.append(&bar);
    frame.append(&reason);
    SegmentedBarWidgets {
        frame,
        reason,
        segments,
    }
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

fn refresh_runtime_and_cards(context: &Rc<AppContext>) {
    drain_summary_results(context);
    drain_naming_results(context);
    drain_nudge_results(context);
    drain_runtime_events(context);
    sync_runtime_sizes(context);
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
                let observation = observations
                    .entry(session_id)
                    .or_insert_with(SessionObservation::new);
                apply_stream_update(observation, update);
            }
            RuntimeEvent::Exited(exit_code) => {
                context.state.borrow_mut().mark_exited(session_id, exit_code);
            }
        }
    }
}

fn sync_runtime_sizes(context: &Rc<AppContext>) {
    let sizes = context
        .session_cards
        .borrow()
        .iter()
        .map(|(session_id, card)| (*session_id, terminal_size_hint(&card.terminal)))
        .collect::<Vec<_>>();

    let mut runtimes = context.runtimes.borrow_mut();
    for (session_id, size) in sizes {
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

fn send_runtime_input_line(
    context: &Rc<AppContext>,
    session_id: SessionId,
    line: &str,
) -> std::io::Result<()> {
    let writer = {
        let runtimes = context.runtimes.borrow();
        runtimes
            .get(&session_id)
            .and_then(|runtime| runtime.input_writer.as_ref().cloned())
    }
    .ok_or_else(|| std::io::Error::other("session runtime input writer missing"))?;

    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    write_runtime_input(&writer, &bytes)
}

fn write_runtime_input(writer: &Arc<Mutex<File>>, bytes: &[u8]) -> std::io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| std::io::Error::other("runtime input writer lock poisoned"))?;
    let mut offset = 0usize;

    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "short write to runtime input",
                ))
            }
            Ok(n) => offset += n,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let mut fds = [libc::pollfd {
                    fd: writer.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                }];
                let poll_result =
                    unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 1000) };
                if poll_result < 0 {
                    let poll_error = std::io::Error::last_os_error();
                    if poll_error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(poll_error);
                }
                if poll_result == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out waiting to write runtime input",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
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

fn refresh_observation(context: &Rc<AppContext>, session: &crate::model::SessionRecord) {
    let remote_mode = matches!(context.mode, RunMode::Ssh { .. });
    let mut observations = context.observations.borrow_mut();
    let observation = observations
        .entry(session.id)
        .or_insert_with(SessionObservation::new);
    refresh_session_observation(observation, session, remote_mode);
}

fn update_battle_card_widgets(context: &Rc<AppContext>, session: &crate::model::SessionRecord) {
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
    let mut card_model = build_battle_card(
        session,
        &observed,
        &observation.recent_lines,
        &DeterministicIntentEngine,
    );
    let evidence = build_tactical_evidence(session, observation);
    maybe_queue_summary(context, session.id, &evidence);
    let naming = build_naming_evidence(session, observation);
    maybe_queue_name(context, session.id, &naming);
    let live_summary = current_summary(context, session.id, &evidence);
    maybe_queue_nudge(context, session, observation, live_summary.as_ref());
    if let Some(summary) = live_summary.clone() {
        card_model = apply_tactical_synthesis(card_model, summary);
    }

    let display_name = effective_display_name(session);
    card.title.set_label(&display_name);
    apply_battle_status_style(&card.status, card_model.status);
    apply_battle_card_surface_style(&card.frame, card_model.status);
    card.status
        .set_label(&status_chip_label(card_model.status, &card_model.recency_label));
    card.recency.set_label("");
    card.recency.set_visible(false);
    card.headline.set_label(&card_model.headline);
    card.detail
        .set_label(card_model.primary_detail.as_deref().unwrap_or(""));
    card.detail.set_visible(card_model.primary_detail.is_some());

    let scrollback = scrollback_fragments(observation);
    let evidence_one = scrollback.first().map(String::as_str).unwrap_or(" ");
    let evidence_two = scrollback.get(1).map(String::as_str).unwrap_or(" ");
    let evidence_three = scrollback.get(2).map(String::as_str).unwrap_or(" ");
    card.evidence_one.set_label(evidence_one);
    card.evidence_two.set_label(evidence_two);
    card.evidence_three.set_label(evidence_three);
    card.evidence_one.set_visible(true);
    card.evidence_two.set_visible(true);
    card.evidence_three.set_visible(true);
    card.scrollback_band.set_visible(true);

    apply_metric_widgets(
        &card,
        live_summary.as_ref(),
        Some(observation.last_change.elapsed().as_secs()),
    );

    let operator_summary = live_summary
        .as_ref()
        .and_then(|summary| summary.terse_operator_summary.as_ref())
        .cloned()
        .unwrap_or_default();
    card.alert.set_label(&operator_summary);
    card.alert.set_visible(!operator_summary.is_empty());
    card.nudge_row.set_visible(true);
    apply_nudge_pill(&context.nudge_cache.borrow(), session.id, &card.nudge_state);
}

fn maybe_queue_summary(context: &Rc<AppContext>, session_id: SessionId, evidence: &TacticalEvidence) {
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

    if entry
        .last_attempt
        .is_some_and(|attempt| attempt.elapsed() < Duration::from_secs(5))
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
    session: &crate::model::SessionRecord,
    observation: &SessionObservation,
    summary: Option<&TacticalSynthesis>,
) {
    if visual_gallery_enabled() {
        return;
    }

    let Some(summary) = summary else {
        return;
    };
    if summary.tactical_state != Some(TacticalState::Idle) {
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
    let entry = cache
        .entry(session.id)
        .or_insert_with(NudgeCacheEntry::new);
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
    evidence: &TacticalEvidence,
) -> Option<TacticalSynthesis> {
    if visual_gallery_enabled() {
        return gallery_mock_summary(context, session_id);
    }

    let signature = summary_signature(evidence);
    let cache = context.summary_cache.borrow();
    let entry = cache.get(&session_id)?;
    if entry.completed_signature.as_deref() == Some(signature.as_str()) {
        return entry.last_summary.clone();
    }
    if entry.in_flight || entry.requested_signature.is_some() {
        return entry.last_summary.clone();
    } else {
        entry.last_summary.clone()
    }
}

fn gallery_mock_summary(context: &Rc<AppContext>, session_id: SessionId) -> Option<TacticalSynthesis> {
    let state = context.state.borrow();
    let session = state.session(session_id)?;
    let name = session.launch.name.as_str();
    Some(match name {
        "Agent A" => TacticalSynthesis {
            tactical_state: Some(TacticalState::Working),
            tactical_state_brief: Some("Narrowing the parser failure with focused reruns".into()),
            progress_state: Some(ProgressState::Verifying),
            progress_state_brief: Some("Focused test reruns are narrowing the failure".into()),
            momentum_state: Some(MomentumState::Steady),
            momentum_state_brief: Some("The loop keeps moving, but the fix is not landed yet".into()),
            operator_action: Some(OperatorAction::Watch),
            operator_action_brief: Some("Let the focused repair loop continue".into()),
            terse_operator_summary: Some("Tight edit-test loop, still failing but converging.".into()),
            headline: None,
            primary_fragment: None,
            supporting_fragments: Vec::new(),
            alignment_fragment: None,
            risk_posture: Some(RiskPosture::Low),
            risk_brief: Some("Normal repair work with no risky behavior visible".into()),
            mismatch_level: MismatchLevel::Low,
            mismatch_brief: Some("Narrative and machine evidence line up".into()),
            intervention_warranted: false,
        },
        "Agent B" => TacticalSynthesis {
            tactical_state: Some(TacticalState::Idle),
            tactical_state_brief: Some("Paused after a clean checkpoint".into()),
            progress_state: Some(ProgressState::WaitingForNudge),
            progress_state_brief: Some("The agent paused after reporting a clean checkpoint".into()),
            momentum_state: Some(MomentumState::Strong),
            momentum_state_brief: Some("The checkpoint landed cleanly before the pause".into()),
            operator_action: Some(OperatorAction::Nudge),
            operator_action_brief: Some("A continue prompt is probably enough".into()),
            terse_operator_summary: Some("Looks done with this pass and waiting for a nudge.".into()),
            headline: None,
            primary_fragment: None,
            supporting_fragments: Vec::new(),
            alignment_fragment: None,
            risk_posture: Some(RiskPosture::Low),
            risk_brief: Some("No risky behavior visible; this looks like a clean pause".into()),
            mismatch_level: MismatchLevel::Low,
            mismatch_brief: Some("The pause matches the visible checkpoint".into()),
            intervention_warranted: false,
        },
        "Agent C" => TacticalSynthesis {
            tactical_state: Some(TacticalState::Blocked),
            tactical_state_brief: Some("Waiting on explicit approval".into()),
            progress_state: Some(ProgressState::Blocked),
            progress_state_brief: Some("The next step cannot proceed without operator input".into()),
            momentum_state: Some(MomentumState::Fragile),
            momentum_state_brief: Some("Forward motion stops at the approval boundary".into()),
            operator_action: Some(OperatorAction::Intervene),
            operator_action_brief: Some("Approval or redirection is required now".into()),
            terse_operator_summary: Some("Hard stop on approval boundary; operator input required.".into()),
            headline: None,
            primary_fragment: None,
            supporting_fragments: Vec::new(),
            alignment_fragment: None,
            risk_posture: Some(RiskPosture::Watch),
            risk_brief: Some("The next step touches production, so operator review matters".into()),
            mismatch_level: MismatchLevel::Low,
            mismatch_brief: Some("The stop is consistent with the stated boundary".into()),
            intervention_warranted: true,
        },
        "Agent D" => TacticalSynthesis {
            tactical_state: Some(TacticalState::Active),
            tactical_state_brief: Some("Retrying the same failing path".into()),
            progress_state: Some(ProgressState::Flailing),
            progress_state_brief: Some("Repeated retries are not producing new evidence".into()),
            momentum_state: Some(MomentumState::Stalled),
            momentum_state_brief: Some("Retries keep looping without narrowing the issue".into()),
            operator_action: Some(OperatorAction::Watch),
            operator_action_brief: Some("Watch closely; step in if the loop repeats again".into()),
            terse_operator_summary: Some("Retry loop is repeating without a decisive new clue.".into()),
            headline: None,
            primary_fragment: None,
            supporting_fragments: Vec::new(),
            alignment_fragment: None,
            risk_posture: Some(RiskPosture::Watch),
            risk_brief: Some("Churn risk is rising because the same failure keeps returning".into()),
            mismatch_level: MismatchLevel::Watch,
            mismatch_brief: Some("The narrative sounds active, but progress is weak".into()),
            intervention_warranted: false,
        },
        "Agent E" => TacticalSynthesis {
            tactical_state: Some(TacticalState::Idle),
            tactical_state_brief: Some("Stable after validation".into()),
            progress_state: Some(ProgressState::ConvergedWaiting),
            progress_state_brief: Some("Repeated steady status suggests the task is parked cleanly".into()),
            momentum_state: Some(MomentumState::Steady),
            momentum_state_brief: Some("Recent momentum is fading after a clean finish".into()),
            operator_action: Some(OperatorAction::None),
            operator_action_brief: Some("No intervention needed unless priorities change".into()),
            terse_operator_summary: Some("Looks stably parked after validation, not suspiciously idle.".into()),
            headline: None,
            primary_fragment: None,
            supporting_fragments: Vec::new(),
            alignment_fragment: None,
            risk_posture: Some(RiskPosture::Low),
            risk_brief: Some("No risky behavior or mismatch is visible".into()),
            mismatch_level: MismatchLevel::Low,
            mismatch_brief: Some("The transcript supports a clean stand-by state".into()),
            intervention_warranted: false,
        },
        "Agent F" => TacticalSynthesis {
            tactical_state: Some(TacticalState::Active),
            tactical_state_brief: Some("Escalating from disk pressure into risky cleanup ideas".into()),
            progress_state: Some(ProgressState::Blocked),
            progress_state_brief: Some("Disk pressure is blocking forward progress".into()),
            momentum_state: Some(MomentumState::Stalled),
            momentum_state_brief: Some("Disk pressure is halting forward motion".into()),
            operator_action: Some(OperatorAction::Intervene),
            operator_action_brief: Some("Step in before cleanup turns destructive".into()),
            terse_operator_summary: Some("Blocked on disk space and drifting toward risky cleanup actions.".into()),
            headline: None,
            primary_fragment: None,
            supporting_fragments: Vec::new(),
            alignment_fragment: None,
            risk_posture: Some(RiskPosture::Extreme),
            risk_brief: Some("Frustration plus destructive cleanup ideas is an extreme-risk combination".into()),
            mismatch_level: MismatchLevel::Watch,
            mismatch_brief: Some("The transcript still matches the disk-pressure problem, but escalation is concerning".into()),
            intervention_warranted: true,
        },
        _ => return None,
    })
}

fn apply_tactical_synthesis(
    mut card_model: crate::supervision::BattleCardViewModel,
    summary: TacticalSynthesis,
) -> crate::supervision::BattleCardViewModel {
    if let Some(tactical_state) = summary.tactical_state {
        card_model.status = match tactical_state {
            TacticalState::Idle => BattleCardStatus::Idle,
            TacticalState::Active => BattleCardStatus::Active,
            TacticalState::Thinking => BattleCardStatus::Thinking,
            TacticalState::Working => BattleCardStatus::Working,
            TacticalState::Blocked => BattleCardStatus::Blocked,
            TacticalState::Failed => BattleCardStatus::Failed,
            TacticalState::Complete => BattleCardStatus::Complete,
            TacticalState::Detached => BattleCardStatus::Detached,
        };
        card_model.recency_label = match card_model.status {
            BattleCardStatus::Idle => card_model.recency_label,
            _ if card_model.recency_label.starts_with("idle ") => "active now".into(),
            _ => card_model.recency_label,
        };
    }

    if !summary.supporting_fragments.is_empty() {
        let mut merged = card_model.evidence_fragments.clone();
        for fragment in &summary.supporting_fragments {
            if !merged
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(fragment))
            {
                merged.push(fragment.clone());
            }
        }
        merged.truncate(2);
        card_model.evidence_fragments = merged;
    }
    let caution_text = match summary.operator_action {
        Some(crate::synthesis::OperatorAction::Intervene) => summary.operator_action_brief.clone(),
        Some(crate::synthesis::OperatorAction::Nudge) => summary.operator_action_brief.clone(),
        _ if matches!(
            summary.risk_posture,
            Some(crate::synthesis::RiskPosture::Watch)
                | Some(crate::synthesis::RiskPosture::High)
                | Some(crate::synthesis::RiskPosture::Extreme)
        ) =>
        {
            summary.risk_brief.clone()
        }
        _ if !matches!(summary.mismatch_level, MismatchLevel::Low) => summary.mismatch_brief.clone(),
        _ => summary.alignment_fragment.clone(),
    };

    if let Some(text) = caution_text {
        card_model.alignment.text = text;
        card_model.alignment.tone = if matches!(
            summary.risk_posture,
            Some(crate::synthesis::RiskPosture::High)
                | Some(crate::synthesis::RiskPosture::Extreme)
        ) || matches!(summary.operator_action, Some(crate::synthesis::OperatorAction::Intervene))
            || matches!(summary.mismatch_level, MismatchLevel::High)
        {
            SignalTone::Alert
        } else if matches!(
            summary.risk_posture,
            Some(crate::synthesis::RiskPosture::Watch)
        ) || matches!(summary.operator_action, Some(crate::synthesis::OperatorAction::Nudge))
            || matches!(summary.mismatch_level, MismatchLevel::Watch)
        {
            SignalTone::Watch
        } else {
            SignalTone::Calm
        };
    } else if matches!(summary.mismatch_level, MismatchLevel::High) {
        card_model.alignment.tone = SignalTone::Alert;
    }

    card_model
}

fn apply_metric_widgets(
    card: &SessionCardWidgets,
    summary: Option<&TacticalSynthesis>,
    idle_seconds: Option<u64>,
) {
    let momentum = momentum_bar_value(summary, idle_seconds);
    apply_segmented_bar(&card.momentum_bar, momentum.as_ref());

    let risk = risk_bar_value(summary);
    apply_segmented_bar(&card.risk_bar, risk.as_ref());
}

fn apply_segmented_bar(
    bar: &SegmentedBarWidgets,
    value: Option<&(usize, SignalTone, Option<String>)>,
) {
    let Some((fill, tone, reason)) = value else {
        bar.frame.set_visible(false);
        bar.reason.set_label("");
        bar.frame.set_tooltip_text(None::<&str>);
        for segment in &bar.segments {
            for css in ["bar-calm", "bar-watch", "bar-alert", "bar-empty"] {
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
        for css in ["bar-calm", "bar-watch", "bar-alert", "bar-empty"] {
            segment.remove_css_class(css);
        }
        if index < *fill {
            segment.add_css_class(match tone {
                SignalTone::Calm => "bar-calm",
                SignalTone::Watch => "bar-watch",
                SignalTone::Alert => "bar-alert",
            });
        } else {
            segment.add_css_class("bar-empty");
        }
    }
}

fn momentum_bar_value(
    summary: Option<&TacticalSynthesis>,
    idle_seconds: Option<u64>,
) -> Option<(usize, SignalTone, Option<String>)> {
    if let Some(summary) = summary {
        let (base_fill, tone) = match summary.momentum_state {
            Some(MomentumState::Strong) => (4usize, SignalTone::Calm),
            Some(MomentumState::Steady) => (3usize, SignalTone::Calm),
            Some(MomentumState::Fragile) => (2usize, SignalTone::Watch),
            Some(MomentumState::Stalled) => (1usize, SignalTone::Alert),
            None => return None,
        };
        let fill = decayed_momentum_fill(base_fill, summary, idle_seconds);
        return Some((fill, tone, summary.momentum_state_brief.clone()));
    }
    None
}

fn decayed_momentum_fill(
    base_fill: usize,
    summary: &TacticalSynthesis,
    idle_seconds: Option<u64>,
) -> usize {
    let mut fill = base_fill;
    let should_decay = matches!(
        summary.tactical_state,
        Some(TacticalState::Idle | TacticalState::Blocked)
    ) || matches!(summary.momentum_state, Some(MomentumState::Stalled));

    if should_decay {
        let seconds = idle_seconds.unwrap_or_default();
        let decay_steps = (seconds / 30) as usize;
        fill = fill.saturating_sub(decay_steps);
    }

    fill.min(4)
}

fn risk_bar_value(summary: Option<&TacticalSynthesis>) -> Option<(usize, SignalTone, Option<String>)> {
    if let Some(summary) = summary {
        if let Some(risk) = summary.risk_posture {
            let hint = summary.risk_brief.clone();
            return Some(match risk {
                crate::synthesis::RiskPosture::Low => (1, SignalTone::Calm, hint),
                crate::synthesis::RiskPosture::Watch => (2, SignalTone::Watch, hint),
                crate::synthesis::RiskPosture::High => (3, SignalTone::Alert, hint),
                crate::synthesis::RiskPosture::Extreme => (4, SignalTone::Alert, hint),
            });
        }
    }
    None
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
            let status = build_battle_card(
                session,
                &observed,
                &observation.recent_lines,
                &DeterministicIntentEngine,
            )
            .status;
            match status {
                BattleCardStatus::Idle => idle += 1,
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
    context.battlefield_scroller.set_visible(!is_empty);
    let state = context.state.borrow();
    let _ = (sessions, idle, active, failed, state);
    context.title.set_subtitle("");
}

fn refresh_card_styles(context: &Rc<AppContext>) {
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
        card.headline.set_visible(!focus_mode);
        card.detail.set_visible(!focus_mode && !card.detail.label().is_empty());
        card.momentum_bar.frame.set_visible(!focus_mode);
        card.risk_bar.frame.set_visible(!focus_mode);
        card.alert.set_wrap(focus_mode);
        card.alert.set_single_line_mode(!focus_mode);
        card.alert.set_ellipsize(if focus_mode {
            gtk::pango::EllipsizeMode::None
        } else {
            gtk::pango::EllipsizeMode::End
        });
        let shows_terminal = battlefield_embeds_terminal(context, *session_id);
        card.bars.set_orientation(if shows_terminal {
            gtk::Orientation::Horizontal
        } else {
            gtk::Orientation::Vertical
        });
        if shows_terminal {
            card.frame.remove_css_class("scrollback-card");
            card.terminal_slot.remove_css_class("scrollback-terminal-hidden");
        } else {
            card.frame.add_css_class("scrollback-card");
            card.terminal_slot.add_css_class("scrollback-terminal-hidden");
        }
        if focus_mode {
            card.middle_stack.set_visible_child_name("scrollback");
            card.middle_stack.set_visible(false);
        } else {
            card.middle_stack.set_visible(true);
            card.middle_stack
                .set_visible_child_name(if shows_terminal { "terminal" } else { "scrollback" });
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
    context.battlefield_scroller.set_vexpand(false);
    context.battlefield_scroller.set_hscrollbar_policy(gtk::PolicyType::Automatic);
    context.battlefield_scroller.set_vscrollbar_policy(gtk::PolicyType::Never);
    context.battlefield_scroller.set_min_content_height(252);
    context.battlefield_scroller.set_max_content_height(300);
    update_flowbox_columns(context);
    sync_terminal_parents(context);
    refresh_card_styles(context);
    refresh_focus_panel(context);
    refresh_workspace(context);
}

fn show_battlefield(context: &Rc<AppContext>) {
    context.state.borrow_mut().return_to_battlefield();
    context.focus.panel.set_visible(false);
    context.content_root.remove_css_class("focus-mode");
    context.battlefield_scroller.set_vexpand(true);
    context.battlefield_scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    context.battlefield_scroller.set_vscrollbar_policy(gtk::PolicyType::Automatic);
    context.battlefield_scroller.set_min_content_height(0);
    context.battlefield_scroller.set_max_content_height(-1);
    update_flowbox_columns(context);
    sync_terminal_parents(context);
    refresh_card_styles(context);
    refresh_workspace(context);
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
    let mut card_model = build_battle_card(
        session,
        &observed,
        &observation.recent_lines,
        &DeterministicIntentEngine,
    );
    let evidence = build_tactical_evidence(session, observation);
    let live_summary = current_summary(context, session_id, &evidence);
    if let Some(summary) = live_summary.clone() {
        card_model = apply_tactical_synthesis(card_model, summary);
    }

    context
        .focus
        .title
        .set_label(&effective_display_name(session));
    apply_battle_status_style(&context.focus.status, card_model.status);
    apply_battle_card_surface_style(&context.focus.frame, card_model.status);
    context
        .focus
        .status
        .set_label(&status_chip_label(card_model.status, &card_model.recency_label));
    let operator_summary = live_summary
        .as_ref()
        .and_then(|summary| summary.terse_operator_summary.as_ref())
        .cloned()
        .unwrap_or_default();
    context.focus.alert.set_label(&operator_summary);
    context.focus.alert.set_visible(!operator_summary.is_empty());
    context.focus.nudge_row.set_visible(true);
    apply_nudge_pill(
        &context.nudge_cache.borrow(),
        session.id,
        &context.focus.nudge_state,
    );
    context.focus.bars.set_orientation(gtk::Orientation::Horizontal);
    apply_segmented_bar(
        &context.focus.momentum_bar,
        momentum_bar_value(live_summary.as_ref(), Some(observation.last_change.elapsed().as_secs()))
            .as_ref(),
    );
    apply_segmented_bar(&context.focus.risk_bar, risk_bar_value(live_summary.as_ref()).as_ref());
}

fn update_flowbox_columns(context: &Rc<AppContext>) {
    let total = context.session_cards.borrow().len();
    if total == 0 {
        return;
    }

    let available_width = context.battlefield_scroller.width();
    let columns = if available_width <= 0 {
        if context.state.borrow().focused_session().is_some() {
            total
        } else if total <= 2 {
            total
        } else if total <= 4 {
            2
        } else if total <= 6 {
            3
        } else {
            4
        }
    } else if context.state.borrow().focused_session().is_some() {
        total
    } else if total == 1 {
        1
    } else if total == 2 {
        if (available_width / 2) >= EMBEDDED_TERMINAL_MIN_WIDTH {
            2
        } else {
            1
        }
    } else if total == 4 {
        2
    } else if total == 6 {
        3
    } else if total <= 4 {
        if available_width >= 1800 {
            total
        } else {
            2
        }
    } else if total == 5 {
        ((available_width as usize) / 420).clamp(3, 5)
    } else {
        ((available_width as usize) / 380).clamp(3, total.min(4))
    } as u32;
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
    let available_width = context.battlefield_scroller.width().max(0);
    let available_height = context.battlefield_scroller.height().max(0);
    let tile_width = if columns > 0 {
        (available_width - ((columns - 1) as i32 * 12) - 24) / columns as i32
    } else {
        0
    };
    let rows = ((total as f32) / (columns as f32)).ceil() as i32;
    let tile_height = if rows > 0 {
        (available_height - ((rows - 1) * 12) - 24) / rows
    } else {
        0
    };

    tile_width >= EMBEDDED_TERMINAL_MIN_WIDTH && tile_height >= EMBEDDED_TERMINAL_MIN_HEIGHT
}

fn current_battlefield_columns(context: &Rc<AppContext>) -> usize {
    let total = context.session_cards.borrow().len();
    if total == 0 {
        return 0;
    }
    context.cards.max_children_per_line().max(1) as usize
}

fn focused_embedded_terminal_session(context: &Rc<AppContext>) -> Option<SessionId> {
    context.session_cards.borrow().iter().find_map(|(session_id, card)| {
        (battlefield_embeds_terminal(context, *session_id) && card.terminal.has_focus())
            .then_some(*session_id)
    })
}

fn update_nudge_widgets(context: &Rc<AppContext>, session_id: SessionId) {
    if let Some(card) = context.session_cards.borrow().get(&session_id) {
        apply_nudge_pill(&context.nudge_cache.borrow(), session_id, &card.nudge_state);
    }
    if context.state.borrow().focused_session() == Some(session_id) {
        apply_nudge_pill(
            &context.nudge_cache.borrow(),
            session_id,
            &context.focus.nudge_state,
        );
    }
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
    let hovered = cache
        .get(&session_id)
        .is_some_and(|entry| entry.hovered);
    let enabled = cache
        .get(&session_id)
        .is_some_and(|entry| entry.enabled);
    let (text, css) = if hovered {
        if enabled {
            ("DISARM AUTONUDGE", "card-control-cooldown")
        } else {
            ("ARM AUTONUDGE", "card-control-off")
        }
    } else if cooldown_active {
        ("AUTONUDGE COOLDOWN", "card-control-cooldown")
    } else if enabled {
        ("AUTONUDGE ARMED", "card-control-armed")
    } else {
        ("AUTONUDGE OFF", "card-control-off")
    };

    for candidate in [
        "card-control-off",
        "card-control-armed",
        "card-control-nudged",
        "card-control-cooldown",
    ] {
        state.remove_css_class(candidate);
    }
    state.add_css_class(css);
    state.set_label(text);
    state.set_visible(true);
}

fn sync_terminal_parents(context: &Rc<AppContext>) {
    let focused = context.state.borrow().focused_session();
    for (session_id, card) in context.session_cards.borrow().iter() {
        if focused == Some(*session_id) {
            reparent_widget_to_box(&card.terminal_view, &context.focus.terminal_slot);
            card.terminal.grab_focus();
        } else if battlefield_embeds_terminal(context, *session_id) {
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

fn apply_battle_status_style(label: &gtk::Label, status: BattleCardStatus) {
    for css in [
        "battle-idle",
        "battle-active",
        "battle-thinking",
        "battle-working",
        "battle-blocked",
        "battle-failed",
        "battle-complete",
        "battle-detached",
    ] {
        label.remove_css_class(css);
    }

    label.add_css_class(match status {
        BattleCardStatus::Idle => "battle-idle",
        BattleCardStatus::Active => "battle-active",
        BattleCardStatus::Thinking => "battle-thinking",
        BattleCardStatus::Working => "battle-working",
        BattleCardStatus::Blocked => "battle-blocked",
        BattleCardStatus::Failed => "battle-failed",
        BattleCardStatus::Complete => "battle-complete",
        BattleCardStatus::Detached => "battle-detached",
    });
}

fn status_chip_label(status: BattleCardStatus, recency_label: &str) -> String {
    if matches!(status, BattleCardStatus::Idle) && recency_label.starts_with("idle ") {
        let seconds = recency_label.trim_start_matches("idle ").trim();
        return format!("IDLE - {seconds}");
    }

    status.label().to_string()
}

fn apply_battle_card_surface_style(frame: &gtk::Frame, status: BattleCardStatus) {
    for css in [
        "card-idle",
        "card-active",
        "card-thinking",
        "card-working",
        "card-blocked",
        "card-failed",
        "card-complete",
        "card-detached",
    ] {
        frame.remove_css_class(css);
    }

    frame.add_css_class(match status {
        BattleCardStatus::Idle => "card-idle",
        BattleCardStatus::Active => "card-active",
        BattleCardStatus::Thinking => "card-thinking",
        BattleCardStatus::Working => "card-working",
        BattleCardStatus::Blocked => "card-blocked",
        BattleCardStatus::Failed => "card-failed",
        BattleCardStatus::Complete => "card-complete",
        BattleCardStatus::Detached => "card-detached",
    });
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "
        window {
            background: #000000;
        }

        flowboxchild {
            padding: 0;
            background: transparent;
            box-shadow: none;
            outline: none;
        }

        flowboxchild:selected {
            background: transparent;
            box-shadow: none;
            outline: none;
        }

        flowboxchild:selected > * {
            box-shadow: none;
        }

        flowboxchild.selected-card > * {
            border-color: rgba(113, 197, 255, 0.98);
            box-shadow: 0 0 0 1px rgba(113, 197, 255, 0.92), 0 22px 44px rgba(13, 92, 151, 0.24);
        }

        .workspace-summary {
            color: rgba(199, 210, 222, 0.9);
            font-size: 13px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .workspace-hint {
            color: rgba(189, 204, 219, 0.74);
            font-size: 12px;
        }

        .empty-state {
            margin-top: 40px;
            margin-bottom: 56px;
        }

        .empty-title {
            color: #f8fafc;
            font-size: 28px;
            font-weight: 800;
        }

        .empty-body {
            color: rgba(198, 211, 225, 0.82);
            font-size: 15px;
            line-height: 1.45;
        }

        .battle-card {
            border-radius: 24px;
            border: 1px solid rgba(163, 175, 194, 0.16);
            background: rgba(10, 18, 28, 0.95);
            box-shadow: 0 24px 46px rgba(0, 0, 0, 0.28);
            min-width: 392px;
            min-height: 220px;
        }

        .battle-card.single-card {
            min-width: 0;
            min-height: 0;
        }

        .battle-card.scrollback-card {
            min-width: 0;
            min-height: 0;
        }

        .card-terminal-slot {
            border-radius: 20px;
            border: 1px solid rgba(120, 136, 158, 0.2);
            background: rgba(7, 13, 20, 0.96);
            min-height: 420px;
            padding: 10px;
        }

        .card-terminal-slot.scrollback-terminal-hidden {
            min-height: 0;
            padding: 0;
            border-color: transparent;
            background: transparent;
        }

        .card-header-row {
            min-height: 34px;
        }

        .card-body-stack {
            margin-top: 2px;
        }

        .card-bottom-stack,
        .card-footer-stack {
            margin-top: 0;
        }

        .card-scrollback-band {
            border-radius: 14px;
            border: 1px solid rgba(173, 188, 204, 0.08);
            background: rgba(8, 14, 22, 0.34);
            padding: 8px 10px;
            min-height: 0;
        }

        .card-scrollback-line {
            color: rgba(202, 214, 227, 0.88);
            font-size: 11px;
            font-family: Monospace;
            line-height: 1.1;
        }

        .card-bars-row {
            margin-top: 0;
        }

        .card-title {
            font-weight: 800;
            font-size: 18px;
            color: #f8fafc;
        }

        .card-subtitle {
            color: rgba(196, 208, 222, 0.66);
            font-size: 12px;
            letter-spacing: 0.04em;
            text-transform: uppercase;
        }

        .card-status {
            font-weight: 800;
            font-size: 10px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            border-radius: 999px;
            padding: 4px 10px;
            border: 1px solid rgba(190, 202, 217, 0.2);
        }

        .card-recency {
            color: rgba(188, 201, 216, 0.88);
            font-size: 12px;
            font-weight: 700;
            letter-spacing: 0.03em;
        }

        .card-headline {
            color: #f8fafc;
            font-weight: 800;
            font-size: 20px;
            line-height: 1.12;
        }

        .card-detail {
            color: rgba(226, 234, 242, 0.94);
            font-size: 15px;
            font-weight: 650;
            line-height: 1.25;
        }

        .card-evidence {
            color: rgba(198, 212, 227, 0.88);
            font-size: 12px;
            font-family: Monospace;
            background: rgba(11, 18, 28, 0.32);
            border-radius: 11px;
            border: 1px solid rgba(173, 188, 204, 0.12);
            padding: 7px 10px;
        }

        .card-alert {
            color: rgba(202, 214, 227, 0.78);
            font-size: 11px;
            font-weight: 600;
            line-height: 1.2;
            margin: 0;
        }

        .card-control-row {
            min-height: 28px;
            margin-top: -2px;
            margin-bottom: -2px;
        }

        .card-control-label {
            color: rgba(203, 214, 226, 0.72);
            font-size: 10px;
            font-weight: 700;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .card-control-state {
            font-size: 10px;
            font-weight: 800;
            letter-spacing: 0.08em;
            text-transform: uppercase;
            border-radius: 999px;
            padding: 4px 10px;
            border: 1px solid rgba(190, 202, 217, 0.16);
        }

        .card-control-off {
            color: rgba(214, 222, 230, 0.84);
            background: rgba(84, 97, 112, 0.18);
            border-color: rgba(163, 175, 194, 0.16);
        }

        .card-control-armed {
            color: #fde68a;
            background: rgba(120, 87, 10, 0.22);
            border-color: rgba(250, 204, 21, 0.22);
        }

        .card-control-nudged {
            color: #86efac;
            background: rgba(17, 88, 51, 0.22);
            border-color: rgba(74, 222, 128, 0.2);
        }

        .card-control-cooldown {
            color: #93c5fd;
            background: rgba(33, 82, 145, 0.22);
            border-color: rgba(96, 165, 250, 0.2);
        }

        .bar-widget {
            border-radius: 12px;
            border: 1px solid rgba(173, 188, 204, 0.08);
            background: rgba(11, 18, 28, 0.18);
            padding: 7px 9px;
        }

        .bar-caption {
            color: rgba(186, 200, 214, 0.62);
            font-size: 10px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .segmented-bar {
            min-height: 8px;
        }

        .bar-segment {
            min-height: 8px;
            border-radius: 999px;
        }

        .bar-empty {
            background: rgba(163, 175, 194, 0.14);
        }

        .bar-calm {
            background: linear-gradient(90deg, rgba(110, 231, 183, 0.88) 0%, rgba(52, 211, 153, 0.92) 100%);
        }

        .bar-watch {
            background: linear-gradient(90deg, rgba(250, 204, 21, 0.88) 0%, rgba(251, 146, 60, 0.92) 100%);
        }

        .bar-alert {
            background: linear-gradient(90deg, rgba(248, 113, 113, 0.9) 0%, rgba(239, 68, 68, 0.94) 100%);
        }

        .bar-reason {
            color: rgba(186, 200, 214, 0.56);
            font-size: 10px;
            line-height: 1.2;
        }

        .focus-title {
            color: #f8fafc;
            font-size: 20px;
            font-weight: 800;
        }

        .focus-subtitle {
            color: rgba(196, 208, 222, 0.78);
            font-size: 14px;
            margin-bottom: 6px;
        }

        .focus-frame {
            border-radius: 24px;
            border: 1px solid rgba(120, 136, 158, 0.2);
            background: rgba(7, 13, 20, 0.96);
            padding: 10px;
        }

        .focus-panel {
            margin-top: 4px;
        }

        .pill {
            border-radius: 999px;
            padding: 6px 14px;
        }

        .pill {
            background: rgba(119, 198, 255, 0.16);
            color: #dbeafe;
        }

        flowboxchild.focused-card > * {
            border-color: rgba(110, 231, 183, 0.92);
            box-shadow: 0 0 0 1px rgba(110, 231, 183, 0.78), 0 20px 38px rgba(7, 88, 57, 0.22);
        }

        .card-idle {
            background: linear-gradient(180deg, rgba(60, 48, 12, 0.98) 0%, rgba(26, 23, 10, 0.97) 100%);
            border-color: rgba(250, 204, 21, 0.42);
        }

        .card-active {
            background: linear-gradient(180deg, rgba(16, 37, 58, 0.98) 0%, rgba(10, 20, 34, 0.97) 100%);
            border-color: rgba(96, 165, 250, 0.34);
        }

        .card-thinking {
            background: linear-gradient(180deg, rgba(10, 49, 32, 0.98) 0%, rgba(10, 25, 18, 0.97) 100%);
            border-color: rgba(74, 222, 128, 0.34);
        }

        .card-working {
            background: linear-gradient(180deg, rgba(10, 49, 32, 0.98) 0%, rgba(10, 25, 18, 0.97) 100%);
            border-color: rgba(74, 222, 128, 0.34);
        }

        .card-blocked {
            background: linear-gradient(180deg, rgba(58, 31, 12, 0.98) 0%, rgba(28, 17, 10, 0.97) 100%);
            border-color: rgba(251, 146, 60, 0.38);
        }

        .card-failed {
            background: linear-gradient(180deg, rgba(61, 20, 24, 0.98) 0%, rgba(30, 12, 16, 0.97) 100%);
            border-color: rgba(248, 113, 113, 0.38);
        }

        .card-complete {
            background: linear-gradient(180deg, rgba(12, 44, 45, 0.98) 0%, rgba(8, 22, 24, 0.97) 100%);
            border-color: rgba(94, 234, 212, 0.34);
        }

        .card-detached {
            background: linear-gradient(180deg, rgba(40, 20, 57, 0.98) 0%, rgba(18, 10, 28, 0.97) 100%);
            border-color: rgba(192, 132, 252, 0.34);
        }

        .battle-idle {
            color: #fde68a;
            background: rgba(120, 87, 10, 0.22);
            border-color: rgba(250, 204, 21, 0.28);
        }

        .battle-active {
            color: #93c5fd;
            background: rgba(33, 82, 145, 0.22);
            border-color: rgba(96, 165, 250, 0.26);
        }

        .battle-thinking {
            color: #86efac;
            background: rgba(17, 88, 51, 0.24);
            border-color: rgba(74, 222, 128, 0.24);
        }

        .battle-working {
            color: #86efac;
            background: rgba(17, 88, 51, 0.24);
            border-color: rgba(74, 222, 128, 0.24);
        }

        .battle-blocked {
            color: #fdba74;
            background: rgba(108, 58, 14, 0.24);
            border-color: rgba(251, 146, 60, 0.24);
        }

        .battle-failed {
            color: #fca5a5;
            background: rgba(114, 28, 35, 0.24);
            border-color: rgba(248, 113, 113, 0.24);
        }

        .battle-complete {
            color: #99f6e4;
            background: rgba(16, 77, 77, 0.22);
            border-color: rgba(94, 234, 212, 0.24);
        }

        .battle-detached {
            color: #e9d5ff;
            background: rgba(74, 34, 112, 0.22);
            border-color: rgba(192, 132, 252, 0.24);
        }

        .focus-mode flowboxchild .battle-card {
            min-width: 176px;
            min-height: 182px;
            border-radius: 18px;
            box-shadow: 0 14px 28px rgba(0, 0, 0, 0.22);
        }

        .focus-mode flowboxchild .card-title {
            font-size: 15px;
        }

        .focus-mode flowboxchild .card-status,
        .focus-mode flowboxchild .card-recency {
            font-size: 10px;
        }

        .focus-mode flowboxchild .card-header-row {
            min-height: 28px;
        }

        .focus-mode flowboxchild .card-bottom-stack {
            margin-top: 0;
        }

        .focus-mode flowboxchild .card-alert {
            color: rgba(206, 217, 229, 0.84);
            font-size: 12px;
            font-weight: 600;
            line-height: 1.3;
            padding: 0;
            background: transparent;
            border-color: transparent;
            min-height: 112px;
            margin-top: 6px;
            margin-bottom: 0;
            margin-left: 0;
            margin-right: 0;
        }

        .focus-mode flowboxchild .card-headline,
        .focus-mode flowboxchild .card-detail,
        .focus-mode flowboxchild .card-scrollback-band,
        .focus-mode flowboxchild .bar-widget {
            display: none;
        }

        terminal {
            border-radius: 18px;
            padding: 12px;
        }
        ",
    );

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display should exist"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

#[cfg(test)]
mod tests {
    use super::{parse_run_mode, RunMode};

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
}
