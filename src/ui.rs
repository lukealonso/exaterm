use crate::model::{
    SessionId, SessionKind, SessionLaunch, WorkspaceState,
};
use crate::synthesis::{
    name_signature, suggest_name_blocking, summary_signature, summarize_blocking, MismatchLevel,
    MomentumState, NameSuggestion, NamingEvidence, OpenAiNamingConfig, OpenAiSynthesisConfig,
    OperatorAction, ProgressState, RiskPosture, TacticalEvidence, TacticalState, TacticalSynthesis,
};
use crate::supervision::{
    build_battle_card, BattleCardStatus, DeterministicIntentEngine, ObservedActivity, SignalTone,
};
use crate::terminal_stream::TerminalStreamProcessor;
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vte::prelude::*;
use vte4 as vte;

const APP_ID: &str = "io.exaterm.Exaterm";

const DEFAULT_PROXY_ROWS: u16 = 40;
const DEFAULT_PROXY_COLS: u16 = 160;
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

struct SessionObservation {
    last_change: Instant,
    recent_lines: Vec<String>,
    terminal_activity: Vec<String>,
    painted_line: Option<String>,
    active_command: Option<String>,
    dominant_process: Option<String>,
    process_tree_excerpt: Option<String>,
    recent_files: Vec<String>,
    recent_file_activity: BTreeMap<String, Instant>,
    work_output_excerpt: Option<String>,
    file_fingerprints: BTreeMap<PathBuf, (u64, u64)>,
}

impl SessionObservation {
    fn new() -> Self {
        Self {
            last_change: Instant::now(),
            recent_lines: Vec::new(),
            terminal_activity: Vec::new(),
            painted_line: None,
            active_command: None,
            dominant_process: None,
            process_tree_excerpt: None,
            recent_files: Vec::new(),
            recent_file_activity: BTreeMap::new(),
            work_output_excerpt: None,
            file_fingerprints: BTreeMap::new(),
        }
    }
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

struct SessionRuntime {
    resize_target: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    display_resize_target: Option<Arc<Mutex<File>>>,
    events: mpsc::Receiver<RuntimeEvent>,
    last_size: Option<(u16, u16)>,
}

enum RuntimeEvent {
    Stream(StreamRuntimeUpdate),
    Exited(i32),
}

struct StreamRuntimeUpdate {
    semantic_lines: Vec<String>,
    painted_line: Option<String>,
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

struct FocusWidgets {
    panel: gtk::Box,
    frame: gtk::Frame,
    title: gtk::Label,
    status: gtk::Label,
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
    });

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
    content.append(&alert);
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
    terminal.add_css_class("terminal-surface");
    let terminal_view = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .child(&terminal)
        .build();
    terminal_view.add_css_class("terminal-scroll");
    install_terminal_context_menu(context, &terminal_view, session.id);

    SessionCardWidgets {
        row,
        frame,
        title,
        status,
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
    source_session: SessionId,
) {
    let menu_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();
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
        let split_terminal_button = split_terminal_button.clone();
        let popover = popover.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            let count = context.state.borrow().sessions().len();
            split_terminal_button.set_sensitive(matches!(count, 1 | 2 | 4 | 6 | 8 | 12));
            let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            popover.set_pointing_to(Some(&rect));
            popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
    }
    terminal_view.add_controller(right_click);
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
    let runtime = match if direct_pty_mode_enabled() {
        spawn_direct_runtime(terminal, launch, size)
    } else {
        spawn_proxy_runtime(terminal, launch, size)
    } {
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

struct ProxySpawnResult {
    pid: Option<u32>,
    session_runtime: SessionRuntime,
}

fn direct_pty_mode_enabled() -> bool {
    std::env::var("EXATERM_DIRECT_PTY")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
}

fn spawn_direct_runtime(
    terminal: &vte::Terminal,
    launch: &SessionLaunch,
    size: PtySize,
) -> Result<ProxySpawnResult, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|error| format!("failed to create agent pty: {error}"))?;

    let argv_owned = launch.argv();
    let mut builder = CommandBuilder::new(&argv_owned[0]);
    for arg in argv_owned.iter().skip(1) {
        builder.arg(arg);
    }
    if let Some(cwd) = launch.cwd.as_ref() {
        builder.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| format!("failed to spawn command: {error}"))?;
    drop(pair.slave);

    let pid = child.process_id();
    let Some(master_fd) = pair.master.as_raw_fd() else {
        return Err("agent pty master did not expose a file descriptor".into());
    };
    let foreign_fd = unsafe { libc::dup(master_fd) };
    if foreign_fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let master = unsafe { OwnedFd::from_raw_fd(foreign_fd) };
    let pty = vte::Pty::foreign_sync(master, None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    terminal.set_pty(Some(&pty));

    let resize_target = Arc::new(Mutex::new(pair.master));
    let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    spawn_wait_thread(child, event_tx, stop_flag);

    Ok(ProxySpawnResult {
        pid,
        session_runtime: SessionRuntime {
            resize_target,
            display_resize_target: None,
            events: event_rx,
            last_size: Some((size.rows, size.cols)),
        },
    })
}

fn spawn_proxy_runtime(
    terminal: &vte::Terminal,
    launch: &SessionLaunch,
    size: PtySize,
) -> Result<ProxySpawnResult, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size)
        .map_err(|error| format!("failed to create agent pty: {error}"))?;

    let argv_owned = launch.argv();
    let mut builder = CommandBuilder::new(&argv_owned[0]);
    for arg in argv_owned.iter().skip(1) {
        builder.arg(arg);
    }
    if let Some(cwd) = launch.cwd.as_ref() {
        builder.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| format!("failed to spawn command: {error}"))?;
    drop(pair.slave);

    let pid = child.process_id();
    let Some(agent_master_fd) = pair.master.as_raw_fd() else {
        return Err("agent pty master did not expose a file descriptor".into());
    };
    let agent_reader_fd = unsafe { libc::dup(agent_master_fd) };
    let agent_writer_fd = unsafe { libc::dup(agent_master_fd) };
    if agent_reader_fd < 0 || agent_writer_fd < 0 {
        unsafe {
            if agent_reader_fd >= 0 {
                libc::close(agent_reader_fd);
            }
            if agent_writer_fd >= 0 {
                libc::close(agent_writer_fd);
            }
        }
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut agent_reader = unsafe { File::from_raw_fd(agent_reader_fd) };
    let mut agent_writer = unsafe { File::from_raw_fd(agent_writer_fd) };
    let resize_target = Arc::new(Mutex::new(pair.master));
    let (display_pty, mut display_reader, mut display_writer, display_resizer) =
        create_display_pty(size)?;
    terminal.set_pty(Some(&display_pty));

    let (event_tx, event_rx) = mpsc::channel::<RuntimeEvent>();
    let stop_flag = Arc::new(AtomicBool::new(false));

    spawn_proxy_relay_thread(
        &mut agent_reader,
        &mut agent_writer,
        &mut display_reader,
        &mut display_writer,
        event_tx.clone(),
        stop_flag.clone(),
    );
    spawn_wait_thread(child, event_tx, stop_flag);

    Ok(ProxySpawnResult {
        pid,
        session_runtime: SessionRuntime {
            resize_target,
            display_resize_target: Some(Arc::new(Mutex::new(display_resizer))),
            events: event_rx,
            last_size: Some((size.rows, size.cols)),
        },
    })
}

fn spawn_proxy_relay_thread(
    agent_reader: &mut File,
    agent_writer: &mut File,
    display_reader: &mut File,
    display_writer: &mut File,
    event_tx: mpsc::Sender<RuntimeEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    const RELAY_BUF_SIZE: usize = 16 * 1024;
    let mut agent_reader = agent_reader
        .try_clone()
        .expect("agent reader clone should succeed");
    let mut agent_writer = agent_writer
        .try_clone()
        .expect("agent writer clone should succeed");
    let mut display_reader = display_reader
        .try_clone()
        .expect("display reader clone should succeed");
    let mut display_writer = display_writer
        .try_clone()
        .expect("display slave writer clone should succeed");

    set_nonblocking(agent_reader.as_raw_fd()).expect("agent reader should support nonblocking");
    set_nonblocking(agent_writer.as_raw_fd()).expect("agent writer should support nonblocking");
    set_nonblocking(display_reader.as_raw_fd()).expect("display reader should support nonblocking");
    set_nonblocking(display_writer.as_raw_fd()).expect("display writer should support nonblocking");

    thread::spawn(move || {
        let mut processor = TerminalStreamProcessor::default();
        let mut to_display = Vec::<u8>::with_capacity(RELAY_BUF_SIZE);
        let mut to_agent = Vec::<u8>::with_capacity(RELAY_BUF_SIZE);
        let mut scratch = [0u8; 8192];

        loop {
            let mut fds = [
                libc::pollfd {
                    fd: display_reader.as_raw_fd(),
                    events: if to_agent.len() < RELAY_BUF_SIZE {
                        libc::POLLIN
                    } else {
                        0
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: display_writer.as_raw_fd(),
                    events: if to_display.is_empty() {
                        0
                    } else {
                        libc::POLLOUT
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: agent_reader.as_raw_fd(),
                    events: if to_display.len() < RELAY_BUF_SIZE {
                        libc::POLLIN
                    } else {
                        0
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: agent_writer.as_raw_fd(),
                    events: if to_agent.is_empty() {
                        0
                    } else {
                        libc::POLLOUT
                    },
                    revents: 0,
                },
            ];

            let poll_result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if poll_result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }

            if (fds[0].revents | fds[1].revents | fds[2].revents | fds[3].revents)
                & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)
                != 0
            {
                break;
            }

            if fds[0].revents & libc::POLLIN != 0 {
                let remaining = RELAY_BUF_SIZE.saturating_sub(to_agent.len());
                let read_len = remaining.min(scratch.len());
                match display_reader.read(&mut scratch[..read_len]) {
                    Ok(0) => break,
                    Ok(n) => to_agent.extend_from_slice(&scratch[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }

            if fds[2].revents & libc::POLLIN != 0 {
                let remaining = RELAY_BUF_SIZE.saturating_sub(to_display.len());
                let read_len = remaining.min(scratch.len());
                match agent_reader.read(&mut scratch[..read_len]) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &scratch[..n];
                        to_display.extend_from_slice(chunk);
                        let update = processor.ingest(chunk);
                        if !update.is_empty() || !chunk.is_empty() {
                            let _ = event_tx.send(RuntimeEvent::Stream(StreamRuntimeUpdate {
                                semantic_lines: update.semantic_lines,
                                painted_line: update.painted_line,
                            }));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(_) => break,
                }
            }

            if fds[1].revents & libc::POLLOUT != 0 && !to_display.is_empty() {
                match display_writer.write(&to_display) {
                    Ok(0) => break,
                    Ok(n) => consume_relay_buffer(&mut to_display, n),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }

            if fds[3].revents & libc::POLLOUT != 0 && !to_agent.is_empty() {
                match agent_writer.write(&to_agent) {
                    Ok(0) => break,
                    Ok(n) => consume_relay_buffer(&mut to_agent, n),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
        }
        stop_flag.store(true, Ordering::Relaxed);
    });
}

fn consume_relay_buffer(buffer: &mut Vec<u8>, amount: usize) {
    if amount == 0 || amount > buffer.len() {
        return;
    }
    buffer.drain(0..amount);
}

fn create_display_pty(size: PtySize) -> Result<(vte::Pty, File, File, File), String> {
    let mut master_fd = -1;
    let mut slave_fd = -1;
    let mut winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.pixel_width,
        ws_ypixel: size.pixel_height,
    };
    let result = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut winsize,
        )
    };
    if result != 0 {
        return Err(format!(
            "failed to create display pty: {}",
            std::io::Error::last_os_error()
        ));
    }

    if let Err(error) = set_raw_display_slave(slave_fd) {
        unsafe {
            libc::close(master_fd);
            libc::close(slave_fd);
        }
        return Err(format!("failed to configure display pty: {error}"));
    }

    let reader_fd = unsafe { libc::dup(slave_fd) };
    let writer_fd = unsafe { libc::dup(slave_fd) };
    let resize_fd = unsafe { libc::dup(master_fd) };
    if reader_fd < 0 || writer_fd < 0 || resize_fd < 0 {
        unsafe {
            if reader_fd >= 0 {
                libc::close(reader_fd);
            }
            if writer_fd >= 0 {
                libc::close(writer_fd);
            }
            if resize_fd >= 0 {
                libc::close(resize_fd);
            }
            libc::close(master_fd);
            libc::close(slave_fd);
        }
        return Err(std::io::Error::last_os_error().to_string());
    }

    unsafe {
        libc::close(slave_fd);
    }

    let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
    let reader = unsafe { File::from_raw_fd(reader_fd) };
    let writer = unsafe { File::from_raw_fd(writer_fd) };
    let resizer = unsafe { File::from_raw_fd(resize_fd) };
    let pty = vte::Pty::foreign_sync(master, None::<&gio::Cancellable>)
        .map_err(|error| error.to_string())?;
    Ok((pty, reader, writer, resizer))
}

fn set_raw_display_slave(fd: i32) -> std::io::Result<()> {
    let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        libc::cfmakeraw(&mut termios);
    }
    termios.c_cc[libc::VMIN] = 1;
    termios.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn set_nonblocking(fd: i32) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn spawn_wait_thread(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    event_tx: mpsc::Sender<RuntimeEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let exit_code = child
            .wait()
            .map(|status| status.exit_code() as i32)
            .unwrap_or(-1);
        stop_flag.store(true, Ordering::Relaxed);
        let _ = event_tx.send(RuntimeEvent::Exited(exit_code));
    });
}

fn terminal_size_hint(terminal: &vte::Terminal) -> PtySize {
    let rows = match terminal.row_count() {
        rows if rows > 0 => rows as u16,
        _ => DEFAULT_PROXY_ROWS,
    };
    let cols = match terminal.column_count() {
        cols if cols > 0 => cols as u16,
        _ => DEFAULT_PROXY_COLS,
    };
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
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

fn refresh_runtime_and_cards(context: &Rc<AppContext>) {
    drain_summary_results(context);
    drain_naming_results(context);
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
                append_recent_lines(&mut observation.recent_lines, &update.semantic_lines);
                append_terminal_activity(&mut observation.terminal_activity, &update.semantic_lines);
                if let Some(painted_line) = update.painted_line {
                    let changed = observation.painted_line.as_ref() != Some(&painted_line);
                    observation.painted_line = Some(painted_line);
                    if changed {
                        observation.last_change = Instant::now();
                    }
                } else if !update.semantic_lines.is_empty() && observation.painted_line.is_none() {
                    observation.last_change = Instant::now();
                }
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
    let dominant_process = if remote_mode {
        None
    } else {
        session.pid.and_then(read_dominant_process_hint)
    };
    let process_tree_excerpt = if remote_mode {
        None
    } else {
        session.pid.and_then(read_process_tree_hint)
    };

    let mut observations = context.observations.borrow_mut();
    let observation = observations
        .entry(session.id)
        .or_insert_with(SessionObservation::new);
    let active_command = infer_active_command_from_lines(&observation.recent_lines)
        .or(dominant_process.clone())
        .or_else(|| launch_command_hint(&session.launch));
    observation.dominant_process = dominant_process;
    observation.active_command = active_command;
    observation.process_tree_excerpt = process_tree_excerpt;
    observation.work_output_excerpt = observation.painted_line.clone().or_else(|| {
        observation
            .recent_lines
            .iter()
            .rev()
            .find(|line| is_meaningful_output_line(line))
            .cloned()
    });
    let changed_files = if remote_mode {
        Vec::new()
    } else {
        session
            .launch
            .cwd
            .as_deref()
            .map(|cwd| scan_recent_files(cwd, &mut observation.file_fingerprints))
            .unwrap_or_default()
    };
    let now = Instant::now();
    for file in changed_files {
        observation.recent_file_activity.insert(file, now);
    }
    observation
        .recent_file_activity
        .retain(|_, seen_at| seen_at.elapsed() <= Duration::from_secs(12));
    let mut recent_files = observation
        .recent_file_activity
        .iter()
        .map(|(path, seen_at)| (path.clone(), *seen_at))
        .collect::<Vec<_>>();
    recent_files.sort_by_key(|(_, seen_at)| std::cmp::Reverse(*seen_at));
    observation.recent_files = recent_files
        .into_iter()
        .map(|(path, _)| path)
        .take(2)
        .collect();
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
    let evidence = build_tactical_evidence(session, observation, &card_model);
    maybe_queue_summary(context, session.id, &evidence);
    let naming = build_naming_evidence(session, observation);
    maybe_queue_name(context, session.id, &naming);
    let live_summary = current_summary(context, session.id, &evidence);
    if let Some(summary) = live_summary.clone() {
        card_model = apply_tactical_synthesis(card_model, summary);
    }

    let display_name = effective_display_name(session, observation);
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
}

fn build_tactical_evidence(
    session: &crate::model::SessionRecord,
    observation: &SessionObservation,
    _card_model: &crate::supervision::BattleCardViewModel,
) -> TacticalEvidence {
    TacticalEvidence {
        session_name: effective_display_name(session, observation),
        task_label: session.launch.subtitle.clone(),
        dominant_process: observation.dominant_process.clone(),
        process_tree_excerpt: observation.process_tree_excerpt.clone(),
        recent_files: observation.recent_files.clone(),
        work_output_excerpt: observation.painted_line.clone(),
        idle_seconds: Some(observation.last_change.elapsed().as_secs()),
        recent_terminal_activity: synthesis_terminal_activity(observation),
        recent_events: session
            .events
            .iter()
            .rev()
            .filter(|event| is_runtime_event(&event.summary))
            .take(4)
            .map(|event| event.summary.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
    }
}

fn build_naming_evidence(
    session: &crate::model::SessionRecord,
    observation: &SessionObservation,
) -> NamingEvidence {
    NamingEvidence {
        current_name: session.display_name.clone().unwrap_or_default(),
        recent_terminal_history: naming_terminal_history(observation),
    }
}

fn synthesis_terminal_activity(observation: &SessionObservation) -> Vec<String> {
    let mut entries = Vec::new();

    for line in observation
        .recent_lines
        .iter()
        .rev()
        .take(10)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            entries.push(format!("[line] {trimmed}"));
        }
    }

    if let Some(painted) = observation.painted_line.as_deref() {
        let trimmed = painted.trim();
        if !trimmed.is_empty() {
            entries.push(format!("[most recent updated line] {trimmed}"));
        }
    }

    entries
}

fn naming_terminal_history(observation: &SessionObservation) -> Vec<String> {
    observation
        .terminal_activity
        .iter()
        .rev()
        .take(80)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn scrollback_fragments(observation: &SessionObservation) -> Vec<String> {
    observation
        .recent_lines
        .iter()
        .rev()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
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
            momentum: 0.62,
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
            momentum: 0.82,
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
            momentum: 0.58,
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
            momentum: 0.41,
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
            momentum: 0.86,
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
            momentum: 0.37,
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
        let tone = match summary.momentum_state {
            Some(MomentumState::Strong) => SignalTone::Calm,
            Some(MomentumState::Steady) | Some(MomentumState::Fragile) => SignalTone::Watch,
            Some(MomentumState::Stalled) => SignalTone::Alert,
            None => return None,
        };
        let effective = decayed_momentum(summary, idle_seconds);
        let fill = (effective * 4.0).ceil() as usize;
        return Some((fill.min(4), tone, summary.momentum_state_brief.clone()));
    }
    None
}

fn decayed_momentum(summary: &TacticalSynthesis, idle_seconds: Option<u64>) -> f32 {
    let mut effective = summary.momentum.clamp(0.0, 1.0);
    let should_decay = matches!(
        summary.tactical_state,
        Some(TacticalState::Idle | TacticalState::Blocked)
    ) || matches!(summary.momentum_state, Some(MomentumState::Stalled));

    if should_decay {
        let seconds = idle_seconds.unwrap_or_default() as f32;
        let decay = (1.0 - (seconds / 90.0)).clamp(0.0, 1.0);
        effective *= decay;
    }

    effective.clamp(0.0, 1.0)
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
    let evidence = build_tactical_evidence(session, observation, &card_model);
    let live_summary = current_summary(context, session_id, &evidence);
    if let Some(summary) = live_summary.clone() {
        card_model = apply_tactical_synthesis(card_model, summary);
    }

    context
        .focus
        .title
        .set_label(&effective_display_name(session, observation));
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

fn scan_recent_files(root: &Path, fingerprints: &mut BTreeMap<PathBuf, (u64, u64)>) -> Vec<String> {
    let mut current = BTreeMap::new();
    let mut changed = Vec::new();
    collect_file_changes(root, root, fingerprints, &mut current, &mut changed);
    *fingerprints = current;
    changed.truncate(2);
    changed
}

fn collect_file_changes(
    root: &Path,
    path: &Path,
    previous: &BTreeMap<PathBuf, (u64, u64)>,
    current: &mut BTreeMap<PathBuf, (u64, u64)>,
    changed: &mut Vec<String>,
) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        if metadata.is_dir() {
            collect_file_changes(root, &entry_path, previous, current, changed);
            continue;
        }

        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let signature = (modified, metadata.len());
        current.insert(entry_path.clone(), signature);

        let changed_now = previous
            .get(&entry_path)
            .map(|existing| *existing != signature)
            .unwrap_or(true);

        if changed_now {
            if let Ok(relative) = entry_path.strip_prefix(root) {
                changed.push(relative.display().to_string());
            }
        }
    }
}

fn append_recent_lines(recent_lines: &mut Vec<String>, candidate_lines: &[String]) {
    for line in candidate_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if recent_lines
            .last()
            .is_some_and(|existing| existing == trimmed)
        {
            continue;
        }
        recent_lines.push(trimmed.to_string());
    }

    const MAX_RECENT_LINES_WINDOW: usize = 24;
    if recent_lines.len() > MAX_RECENT_LINES_WINDOW {
        let extra = recent_lines.len() - MAX_RECENT_LINES_WINDOW;
        recent_lines.drain(0..extra);
    }
}

fn append_terminal_activity(activity: &mut Vec<String>, candidate_lines: &[String]) {
    let timestamp = timestamp_now_label();

    if candidate_lines.is_empty() {
        return;
    }

    let trailing_payloads = activity
        .iter()
        .rev()
        .take(candidate_lines.len())
        .map(|entry| activity_payload(entry).to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    if trailing_payloads == candidate_lines {
        return;
    }

    for line in candidate_lines {
        let entry = format!("[{timestamp}] {}", line.trim());
        activity.push(entry);
    }

    const MAX_ACTIVITY_LINES: usize = 160;
    if activity.len() > MAX_ACTIVITY_LINES {
        let extra = activity.len() - MAX_ACTIVITY_LINES;
        activity.drain(0..extra);
    }
}

fn activity_payload(entry: &str) -> &str {
    entry
        .split_once("] ")
        .map(|(_, payload)| payload)
        .unwrap_or(entry)
}

fn effective_display_name(
    session: &crate::model::SessionRecord,
    _observation: &SessionObservation,
) -> String {
    session
        .display_name
        .clone()
        .unwrap_or_else(|| "New Session".into())
}

fn timestamp_now_label() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|dt| dt.format("%H:%M:%S").ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "now".into())
}

fn is_runtime_event(summary: &str) -> bool {
    !matches!(
        summary,
        "Entered focused terminal view"
            | "Returned to battlefield view"
            | "Probe opened"
            | "Probe closed"
            | "Probe pinned for ongoing watch"
            | "Probe returned to peek mode"
    ) && !summary.starts_with("Probe switched to ")
}

fn launch_command_hint(launch: &SessionLaunch) -> Option<String> {
    match launch.kind {
        SessionKind::WaitingShell => Some("Interactive shell ready".into()),
        SessionKind::PlanningStream => None,
        SessionKind::BlockingPrompt => Some("Waiting on approval prompt".into()),
        SessionKind::RunningStream => Some("cargo test parser".into()),
        SessionKind::FailingTask => Some("Task exited after failure".into()),
    }
}

fn infer_active_command_from_lines(lines: &[String]) -> Option<String> {
    lines.iter().rev().find_map(|line| {
        let trimmed = line.trim();
        if let Some(command) = trimmed.strip_prefix("$ ") {
            let command = command.trim();
            return (!command.is_empty()).then(|| command.to_string());
        }
        None
    })
}

fn read_dominant_process_hint(pid: u32) -> Option<String> {
    crate::procfs::dominant_child_command(pid)
        .ok()
        .flatten()
        .map(|command| command.replace("  ", " ").trim().to_string())
}

fn read_process_tree_hint(pid: u32) -> Option<String> {
    crate::procfs::format_process_tree(pid).ok().map(|tree| {
        tree.lines()
            .take(4)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" | ")
    }).filter(|tree| !tree.is_empty())
}

fn is_meaningful_output_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    !line.starts_with("bash-")
        && !line.starts_with('$')
        && !lower.starts_with("intent:")
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
            color: rgba(186, 200, 214, 0.56);
            font-size: 10px;
            font-weight: 500;
            line-height: 1.2;
            margin-top: -4px;
            margin-bottom: -6px;
            margin-left: 6px;
            margin-right: 6px;
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
            background: linear-gradient(180deg, rgba(19, 32, 55, 0.98) 0%, rgba(11, 21, 36, 0.97) 100%);
            border-color: rgba(125, 151, 183, 0.3);
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
            color: #dbe7f5;
            background: rgba(74, 96, 126, 0.22);
            border-color: rgba(148, 163, 184, 0.24);
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
    use super::{
        append_recent_lines, effective_display_name, naming_terminal_history, parse_run_mode,
        synthesis_terminal_activity, RunMode, SessionObservation,
    };
    use crate::model::{SessionId, SessionKind, SessionLaunch, SessionRecord, SessionStatus};

    #[test]
    fn recent_lines_accumulate_semantic_output_without_duplicates() {
        let mut recent = vec!["first".to_string()];
        append_recent_lines(
            &mut recent,
            &["first".to_string(), "second".to_string(), "second".to_string()],
        );
        assert_eq!(recent, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn synthesis_activity_contains_line_window_and_most_recent_updated_line() {
        let mut observation = SessionObservation::new();
        observation.recent_lines = vec![
            "• Ran cargo test".to_string(),
            "test result: ok".to_string(),
        ];
        observation.painted_line = Some("Working 7".to_string());

        assert_eq!(
            synthesis_terminal_activity(&observation),
            vec![
                "[line] • Ran cargo test".to_string(),
                "[line] test result: ok".to_string(),
                "[most recent updated line] Working 7".to_string(),
            ]
        );
    }

    #[test]
    fn naming_history_uses_large_timestamped_window() {
        let mut observation = SessionObservation::new();
        observation.terminal_activity = (0..100)
            .map(|index| format!("[09:41:{index:02}] line {index}"))
            .collect();

        let history = naming_terminal_history(&observation);
        assert_eq!(history.len(), 80);
        assert_eq!(history.first().map(String::as_str), Some("[09:41:20] line 20"));
        assert_eq!(history.last().map(String::as_str), Some("[09:41:99] line 99"));
    }

    #[test]
    fn effective_display_name_prefers_override_then_new_session() {
        let launch = SessionLaunch {
            name: "Shell 1".into(),
            subtitle: "Generic command session".into(),
            program: "/bin/bash".into(),
            args: vec!["-il".into()],
            cwd: None,
            kind: SessionKind::WaitingShell,
        };
        let session = SessionRecord {
            id: SessionId(1),
            launch,
            display_name: None,
            status: SessionStatus::Launching,
            pid: None,
            events: Vec::new(),
        };
        let observation = SessionObservation::new();
        assert_eq!(effective_display_name(&session, &observation), "New Session");

        let mut named_session = session.clone();
        named_session.display_name = Some("Parser Review".into());
        assert_eq!(
            effective_display_name(&named_session, &observation),
            "Parser Review"
        );
    }

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
