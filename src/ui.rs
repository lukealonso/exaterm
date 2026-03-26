use crate::demo::WorkspaceBlueprint;
use crate::model::{SessionId, SessionKind, SessionLaunch, SessionStatus, WorkspaceState};
use crate::supervision::{
    build_battle_card, BattleCardStatus, DeterministicIntentEngine, IntentSource,
    ObservedActivity,
};
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;
use vte::prelude::*;
use vte4 as vte;

const APP_ID: &str = "io.exaterm.Exaterm";

#[derive(Clone)]
struct SessionCardWidgets {
    row: gtk::FlowBoxChild,
    status: gtk::Label,
    recency: gtk::Label,
    command: gtk::Label,
    intent: gtk::Label,
    reality: gtk::Label,
    files: gtk::Label,
    output: gtk::Label,
    correlation: gtk::Label,
    intervene: gtk::Button,
    terminal: vte::Terminal,
    terminal_page: String,
}

struct SessionObservation {
    last_signature: String,
    last_change: Instant,
    recent_lines: Vec<String>,
    active_command: Option<String>,
    dominant_process: Option<String>,
    recent_files: Vec<String>,
    work_output_excerpt: Option<String>,
}

impl SessionObservation {
    fn new() -> Self {
        Self {
            last_signature: String::new(),
            last_change: Instant::now(),
            recent_lines: Vec::new(),
            active_command: None,
            dominant_process: None,
            recent_files: Vec::new(),
            work_output_excerpt: None,
        }
    }
}

struct InterventionWidgets {
    title: gtk::Label,
    subtitle: gtk::Label,
    terminal_stack: gtk::Stack,
}

struct AppContext {
    state: Rc<RefCell<WorkspaceState>>,
    title: adw::WindowTitle,
    workspace_summary: gtk::Label,
    cards: gtk::FlowBox,
    page_stack: gtk::Stack,
    back_button: gtk::Button,
    intervention: InterventionWidgets,
    session_cards: RefCell<BTreeMap<SessionId, SessionCardWidgets>>,
    observations: RefCell<BTreeMap<SessionId, SessionObservation>>,
    current_intervention: RefCell<Option<SessionId>>,
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

    let cards = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .column_spacing(12)
        .row_spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .homogeneous(true)
        .valign(gtk::Align::Start)
        .build();

    let battlefield_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&cards)
        .hexpand(true)
        .vexpand(true)
        .build();

    let workspace_summary = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["workspace-summary".to_string()])
        .margin_top(10)
        .margin_start(18)
        .margin_end(18)
        .margin_bottom(4)
        .build();

    let battlefield_page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    battlefield_page.append(&workspace_summary);
    battlefield_page.append(&battlefield_scroller);

    let intervention_title = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["intervention-title".to_string()])
        .build();
    let intervention_subtitle = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["intervention-subtitle".to_string()])
        .build();
    let terminal_stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let intervention_terminal_frame = gtk::Frame::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&terminal_stack)
        .build();
    intervention_terminal_frame.add_css_class("intervention-frame");

    let intervention_page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .hexpand(true)
        .vexpand(true)
        .build();
    intervention_page.append(&intervention_title);
    intervention_page.append(&intervention_subtitle);
    intervention_page.append(&intervention_terminal_frame);

    let page_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .vexpand(true)
        .build();
    page_stack.add_titled(&battlefield_page, Some("battlefield"), "Battlefield");
    page_stack.add_titled(&intervention_page, Some("intervention"), "Intervention");
    page_stack.set_visible_child_name("battlefield");

    let back_button = gtk::Button::builder()
        .label("Back")
        .css_classes(vec!["pill".to_string()])
        .visible(false)
        .build();
    let add_shell_button = gtk::Button::builder()
        .label("Add Shell")
        .css_classes(vec!["pill".to_string()])
        .tooltip_text("Open a new generic command session")
        .build();

    let title = adw::WindowTitle::new("Exaterm", "Battlefield view");
    let header = adw::HeaderBar::builder()
        .title_widget(&title)
        .show_end_title_buttons(true)
        .build();
    header.pack_start(&back_button);
    header.pack_end(&add_shell_button);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&page_stack);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Exaterm")
        .default_width(1480)
        .default_height(960)
        .content(&body)
        .build();

    let context = Rc::new(AppContext {
        state: Rc::new(RefCell::new(WorkspaceState::new())),
        title,
        workspace_summary,
        cards,
        page_stack,
        back_button,
        intervention: InterventionWidgets {
            title: intervention_title,
            subtitle: intervention_subtitle,
            terminal_stack,
        },
        session_cards: RefCell::new(BTreeMap::new()),
        observations: RefCell::new(BTreeMap::new()),
        current_intervention: RefCell::new(None),
    });

    {
        let back_button = context.back_button.clone();
        let context = context.clone();
        back_button.connect_clicked(move |_| show_battlefield(&context));
    }

    {
        let context = context.clone();
        add_shell_button.connect_clicked(move |_| {
            let number = context.state.borrow().sessions().len() + 1;
            let launch = WorkspaceBlueprint::add_shell(number);
            append_session_card(&context, launch);
            refresh_workspace(&context);
        });
    }

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
                context.state.borrow_mut().select_session(session_id);
                refresh_card_styles(&context);
            }
        });
    }

    {
        let context = context.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape && *context.current_intervention.borrow() != None {
                show_battlefield(&context);
                return glib::Propagation::Stop;
            }

            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                let selected_session = context.state.borrow().selected_session();
                if let Some(session_id) = selected_session {
                    show_intervention(&context, session_id);
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

    for launch in WorkspaceBlueprint::demo().sessions {
        append_session_card(&context, launch);
    }
    refresh_runtime_and_cards(&context);
    refresh_workspace(&context);

    window.present();
}

fn append_session_card(context: &Rc<AppContext>, launch: SessionLaunch) {
    let session_id = context.state.borrow_mut().add_session(launch);
    let session = context
        .state
        .borrow()
        .session(session_id)
        .cloned()
        .expect("new session should exist");

    let card = build_battle_card_widgets(context, &session);
    context
        .intervention
        .terminal_stack
        .add_named(&card.terminal, Some(&card.terminal_page));
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
    spawn_session(context, session_id, &session.launch, &card.terminal);
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
    let subtitle = gtk::Label::builder()
        .label(&session.launch.subtitle)
        .xalign(0.0)
        .css_classes(vec!["card-subtitle".to_string()])
        .build();
    let status = gtk::Label::builder()
        .label("Thinking")
        .css_classes(vec!["card-status".to_string(), "battle-thinking".to_string()])
        .build();
    let recency = gtk::Label::builder()
        .label("recency unknown")
        .xalign(1.0)
        .css_classes(vec!["card-recency".to_string()])
        .build();
    let command = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-command".to_string()])
        .build();
    let intent = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-line".to_string()])
        .build();
    let reality = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-line".to_string()])
        .build();
    let files = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-line".to_string()])
        .build();
    let output = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-line".to_string()])
        .build();
    let correlation = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-correlation".to_string()])
        .build();
    let intervene = gtk::Button::builder()
        .label("Intervene")
        .css_classes(vec!["intervene-button".to_string()])
        .tooltip_text("Promote this session into the real terminal")
        .build();

    let header_left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    header_left.append(&title);
    header_left.append(&subtitle);

    let header_right = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::End)
        .build();
    header_right.append(&status);
    header_right.append(&recency);

    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    header.append(&header_left);
    header.append(&header_right);

    let action_hint = gtk::Label::builder()
        .label("Click card to select · Enter or Intervene for real terminal")
        .xalign(0.0)
        .css_classes(vec!["card-action-hint".to_string()])
        .hexpand(true)
        .build();

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    actions.append(&action_hint);
    actions.append(&intervene);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(14)
        .margin_end(14)
        .build();
    content.append(&header);
    content.append(&command);
    content.append(&intent);
    content.append(&reality);
    content.append(&files);
    content.append(&output);
    content.append(&correlation);
    content.append(&actions);

    let frame = gtk::Frame::builder().child(&content).build();
    frame.add_css_class("battle-card");

    let row = gtk::FlowBoxChild::builder().child(&frame).build();
    row.set_focusable(true);

    {
        let context = context.clone();
        let row = row.clone();
        let session_id = session.id;
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            context.cards.select_child(&row);
            context.state.borrow_mut().select_session(session_id);
            refresh_card_styles(&context);
        });
        frame.add_controller(click);
    }

    {
        let context = context.clone();
        let session_id = session.id;
        intervene.connect_clicked(move |_| show_intervention(&context, session_id));
    }

    let terminal = vte::Terminal::builder()
        .scroll_on_output(true)
        .scroll_on_keystroke(true)
        .input_enabled(true)
        .hexpand(true)
        .vexpand(true)
        .build();
    terminal.set_scrollback_lines(20_000);
    terminal.set_size_request(1180, 760);
    terminal.add_css_class("terminal-surface");

    SessionCardWidgets {
        row,
        status,
        recency,
        command,
        intent,
        reality,
        files,
        output,
        correlation,
        intervene,
        terminal,
        terminal_page: format!("session-{}", session.id.0),
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
            refresh_runtime_and_cards(&context);
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
                    Ok(pid) => context.state.borrow_mut().mark_spawned(session_id, pid.0 as u32),
                    Err(error) => {
                        eprintln!("failed to spawn session {session_id:?}: {error}");
                        context.state.borrow_mut().mark_exited(session_id, -1);
                    }
                }
                refresh_runtime_and_cards(&context);
            },
        );
    }
}

fn refresh_runtime_and_cards(context: &Rc<AppContext>) {
    let sessions = context.state.borrow().sessions().to_vec();
    for session in &sessions {
        refresh_observation(context, session);
    }
    for session in &sessions {
        update_battle_card_widgets(context, session);
    }
    refresh_workspace(context);
    refresh_card_styles(context);
    refresh_intervention_header(context);
}

fn refresh_observation(context: &Rc<AppContext>, session: &crate::model::SessionRecord) {
    let Some(card) = context.session_cards.borrow().get(&session.id).cloned() else {
        return;
    };

    let current_lines = terminal_snapshot_lines(&card.terminal);
    let current_signature = current_lines.join("\n");

    let process_hint = session
        .pid
        .and_then(read_dominant_process_hint)
        .or_else(|| launch_command_hint(&session.launch));

    let output_excerpt = current_lines
        .iter()
        .rev()
        .find(|line| is_meaningful_output_line(line))
        .cloned();

    let mut observations = context.observations.borrow_mut();
    let observation = observations
        .entry(session.id)
        .or_insert_with(SessionObservation::new);

    if !current_signature.is_empty() && current_signature != observation.last_signature {
        observation.last_signature = current_signature;
        observation.last_change = Instant::now();
        observation.recent_lines = current_lines;
    } else if observation.recent_lines.is_empty() && !current_lines.is_empty() {
        observation.recent_lines = current_lines;
    }

    observation.dominant_process = process_hint.clone();
    observation.active_command =
        launch_command_hint(&session.launch).or_else(|| observation.dominant_process.clone());
    observation.work_output_excerpt = output_excerpt;
    observation.recent_files.clear();
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
    let card_model = build_battle_card(
        session,
        &observed,
        &observation.recent_lines,
        &DeterministicIntentEngine,
    );

    apply_battle_status_style(&card.status, card_model.status);
    card.status.set_label(card_model.status.label());
    card.recency.set_label(&card_model.recency_label);
    card.command
        .set_label(&header_command_label(&observed, session.status));
    card.intent.set_label(&format!(
        "{}: {}",
        match card_model.intent.as_ref().map(|intent| intent.source) {
            Some(IntentSource::Stated) => "Intent",
            Some(IntentSource::Inferred) => "Inferred",
            None => "Intent",
        },
        card_model
            .intent
            .as_ref()
            .map(|intent| intent.text.as_str())
            .unwrap_or("No recent visible intent")
    ));
    card.reality.set_label(&card_model.observed_summary);

    let has_files = card_model.file_summary.is_some();
    card.files
        .set_label(card_model.file_summary.as_deref().unwrap_or("Files: no recent file evidence"));
    card.files.set_visible(has_files);

    let has_output = card_model.output_summary.is_some();
    card.output
        .set_label(card_model.output_summary.as_deref().unwrap_or("Output: no high-signal excerpt yet"));
    card.output.set_visible(has_output || matches!(card_model.status, BattleCardStatus::Failed));

    card.correlation.set_label(&card_model.correlation.narrative);
    if card_model.correlation.suspicious_mismatch {
        card.correlation.add_css_class("correlation-alert");
    } else {
        card.correlation.remove_css_class("correlation-alert");
    }

    card.intervene
        .set_label(if *context.current_intervention.borrow() == Some(session.id) {
            "In Terminal"
        } else {
            "Intervene"
        });
}

fn refresh_workspace(context: &Rc<AppContext>) {
    let sessions = context.state.borrow().sessions().to_vec();
    let mut idle = 0usize;
    let mut thinking = 0usize;
    let mut working = 0usize;
    let mut blocked = 0usize;
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
                BattleCardStatus::Thinking => thinking += 1,
                BattleCardStatus::Working => working += 1,
                BattleCardStatus::Blocked => blocked += 1,
                BattleCardStatus::Failed => failed += 1,
                BattleCardStatus::Complete | BattleCardStatus::Detached => {}
            }
        }
    }

    context.workspace_summary.set_label(&format!(
        "Idle {idle} · Thinking {thinking} · Working {working} · Blocked {blocked} · Failed {failed}"
    ));
    context.title.set_subtitle(&format!(
        "{} sessions · idle {} · working {} · failed {}",
        sessions.len(),
        idle,
        working,
        failed
    ));
}

fn refresh_card_styles(context: &Rc<AppContext>) {
    let selected = context.state.borrow().selected_session();
    for (session_id, card) in context.session_cards.borrow().iter() {
        card.row.remove_css_class("selected-card");
        if selected == Some(*session_id) {
            card.row.add_css_class("selected-card");
        }
    }
}

fn show_intervention(context: &Rc<AppContext>, session_id: SessionId) {
    context.state.borrow_mut().activate_session(session_id);
    *context.current_intervention.borrow_mut() = Some(session_id);
    if let Some(card) = context.session_cards.borrow().get(&session_id) {
        context
            .intervention
            .terminal_stack
            .set_visible_child_name(&card.terminal_page);
        card.terminal.grab_focus();
    }
    context.page_stack.set_visible_child_name("intervention");
    context.back_button.set_visible(true);
    refresh_card_styles(context);
    refresh_intervention_header(context);
}

fn show_battlefield(context: &Rc<AppContext>) {
    *context.current_intervention.borrow_mut() = None;
    context.page_stack.set_visible_child_name("battlefield");
    context.back_button.set_visible(false);
    refresh_workspace(context);
}

fn refresh_intervention_header(context: &Rc<AppContext>) {
    let Some(session_id) = *context.current_intervention.borrow() else {
        return;
    };
    let state = context.state.borrow();
    let Some(session) = state.session(session_id) else {
        return;
    };

    context
        .intervention
        .title
        .set_label(&format!("{} · Native Terminal", session.launch.name));
    context.intervention.subtitle.set_label(&format!(
        "{} · real terminal intervention stays one step from the battlefield",
        session.launch.subtitle
    ));
}

fn update_flowbox_columns(context: &Rc<AppContext>) {
    let total = context.session_cards.borrow().len();
    let columns = if total <= 4 {
        2
    } else if total <= 6 {
        3
    } else {
        4
    };
    context.cards.set_max_children_per_line(columns);
    context.cards.set_min_children_per_line(columns);
}

fn terminal_snapshot_lines(terminal: &vte::Terminal) -> Vec<String> {
    let rows = terminal.row_count();
    let cols = terminal.column_count();
    if rows <= 0 || cols <= 0 {
        return Vec::new();
    }

    let (text, _) = terminal.text_range_format(vte::Format::Text, 0, 0, rows - 1, cols - 1);
    text.map(|text| {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

fn launch_command_hint(launch: &SessionLaunch) -> Option<String> {
    match launch.kind {
        SessionKind::WaitingShell => Some("Interactive shell ready".into()),
        SessionKind::BlockingPrompt => Some("Waiting on approval prompt".into()),
        SessionKind::RunningStream => Some("Long-running tool activity".into()),
        SessionKind::FailingTask => Some("Task exited after failure".into()),
    }
}

fn read_dominant_process_hint(pid: u32) -> Option<String> {
    let process_tree = crate::procfs::format_process_tree(pid).ok()?;
    let mut lines = process_tree.lines().filter(|line| !line.trim().is_empty());
    let candidate = lines.nth(1).or_else(|| process_tree.lines().next())?;
    Some(candidate.trim().replace("  ", " "))
}

fn header_command_label(observed: &ObservedActivity, session_status: SessionStatus) -> String {
    if let Some(command) = observed.active_command.as_ref() {
        return command.clone();
    }
    if let Some(process) = observed.dominant_process.as_ref() {
        return process.clone();
    }
    match session_status {
        SessionStatus::Blocked => "Awaiting explicit operator input".into(),
        SessionStatus::Failed(code) => format!("Last command exited with code {code}"),
        SessionStatus::Complete => "Main activity completed".into(),
        _ => "No active command classified yet".into(),
    }
}

fn is_meaningful_output_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    !line.starts_with("bash-")
        && !line.starts_with('$')
        && !lower.starts_with("intent:")
        && !lower.starts_with("now ")
        && !lower.starts_with("i'm ")
        && !lower.starts_with("i am ")
}

fn apply_battle_status_style(label: &gtk::Label, status: BattleCardStatus) {
    for css in [
        "battle-idle",
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
        BattleCardStatus::Thinking => "battle-thinking",
        BattleCardStatus::Working => "battle-working",
        BattleCardStatus::Blocked => "battle-blocked",
        BattleCardStatus::Failed => "battle-failed",
        BattleCardStatus::Complete => "battle-complete",
        BattleCardStatus::Detached => "battle-detached",
    });
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "
        window {
            background: linear-gradient(180deg, #07111b 0%, #0c1827 100%);
        }

        flowboxchild {
            padding: 0;
        }

        flowboxchild.selected-card > * {
            border-color: rgba(113, 197, 255, 0.98);
            box-shadow: 0 0 0 1px rgba(113, 197, 255, 0.92), 0 22px 44px rgba(13, 92, 151, 0.24);
        }

        .workspace-summary {
            color: rgba(199, 210, 222, 0.9);
            font-size: 12px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .battle-card {
            border-radius: 22px;
            border: 1px solid rgba(163, 175, 194, 0.18);
            background: rgba(10, 18, 28, 0.96);
            box-shadow: 0 22px 42px rgba(0, 0, 0, 0.28);
            min-width: 350px;
            min-height: 228px;
        }

        .card-title {
            font-weight: 800;
            font-size: 15px;
            color: #f8fafc;
        }

        .card-subtitle {
            color: rgba(196, 208, 222, 0.72);
            font-size: 12px;
        }

        .card-status {
            border-radius: 999px;
            padding: 4px 12px;
            font-weight: 800;
            font-size: 11px;
        }

        .card-recency {
            color: rgba(176, 190, 206, 0.8);
            font-size: 11px;
        }

        .card-command {
            color: #dbeafe;
            font-weight: 700;
            font-size: 12px;
        }

        .card-line {
            color: rgba(225, 232, 240, 0.88);
            font-size: 12px;
        }

        .card-correlation {
            color: rgba(147, 197, 253, 0.9);
            font-size: 12px;
        }

        .card-correlation.correlation-alert {
            color: #fca5a5;
        }

        .card-action-hint {
            color: rgba(148, 163, 184, 0.82);
            font-size: 11px;
            letter-spacing: 0.04em;
        }

        .intervention-title {
            color: #f8fafc;
            font-size: 18px;
            font-weight: 800;
        }

        .intervention-subtitle {
            color: rgba(196, 208, 222, 0.78);
            font-size: 12px;
            margin-bottom: 6px;
        }

        .intervention-frame {
            border-radius: 24px;
            border: 1px solid rgba(120, 136, 158, 0.2);
            background: rgba(7, 13, 20, 0.96);
            padding: 10px;
        }

        .pill, .intervene-button {
            border-radius: 999px;
            padding: 6px 14px;
        }

        .pill {
            background: rgba(119, 198, 255, 0.16);
            color: #dbeafe;
        }

        .intervene-button {
            background: rgba(144, 230, 189, 0.18);
            color: #d1fae5;
        }

        .battle-idle {
            background: rgba(251, 191, 36, 0.18);
            color: #fde68a;
        }

        .battle-thinking {
            background: rgba(148, 163, 184, 0.16);
            color: #e2e8f0;
        }

        .battle-working {
            background: rgba(74, 222, 128, 0.18);
            color: #86efac;
        }

        .battle-blocked {
            background: rgba(249, 115, 22, 0.18);
            color: #fdba74;
        }

        .battle-failed {
            background: rgba(248, 113, 113, 0.18);
            color: #fca5a5;
        }

        .battle-complete {
            background: rgba(94, 234, 212, 0.16);
            color: #99f6e4;
        }

        .battle-detached {
            background: rgba(192, 132, 252, 0.18);
            color: #e9d5ff;
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
