use crate::demo::WorkspaceBlueprint;
use crate::model::{
    SessionId, SessionKind, SessionLaunch, WorkspaceState,
};
use crate::supervision::{
    build_battle_card, BattleCardStatus, DeterministicIntentEngine, ObservedActivity, SignalTone,
};
use gtk::gdk;
use gtk::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;
use vte::prelude::*;
use vte4 as vte;

const APP_ID: &str = "io.exaterm.Exaterm";

#[derive(Clone)]
struct SessionCardWidgets {
    row: gtk::FlowBoxChild,
    frame: gtk::Frame,
    status: gtk::Label,
    recency: gtk::Label,
    headline: gtk::Label,
    detail: gtk::Label,
    evidence_one: gtk::Label,
    evidence_two: gtk::Label,
    alert: gtk::Label,
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
    file_fingerprints: BTreeMap<PathBuf, (u64, u64)>,
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
            file_fingerprints: BTreeMap::new(),
        }
    }
}

struct FocusWidgets {
    panel: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    terminal_stack: gtk::Stack,
    exit_button: gtk::Button,
}

struct AppContext {
    state: Rc<RefCell<WorkspaceState>>,
    title: adw::WindowTitle,
    workspace_summary: gtk::Label,
    content_root: gtk::Box,
    cards: gtk::FlowBox,
    battlefield_scroller: gtk::ScrolledWindow,
    focus: FocusWidgets,
    session_cards: RefCell<BTreeMap<SessionId, SessionCardWidgets>>,
    observations: RefCell<BTreeMap<SessionId, SessionObservation>>,
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

    let focus_title = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["focus-title".to_string()])
        .build();
    let focus_subtitle = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(vec!["focus-subtitle".to_string()])
        .wrap(true)
        .build();
    let terminal_stack = gtk::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let focus_terminal_frame = gtk::Frame::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&terminal_stack)
        .build();
    focus_terminal_frame.add_css_class("focus-frame");

    let focus_exit_button = gtk::Button::builder()
        .label("Battlefield Only")
        .css_classes(vec!["pill".to_string()])
        .build();

    let focus_header_left = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    focus_header_left.append(&focus_title);
    focus_header_left.append(&focus_subtitle);

    let focus_header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    focus_header.append(&focus_header_left);
    focus_header.append(&focus_exit_button);

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
    focus_panel.append(&focus_header);
    focus_panel.append(&focus_terminal_frame);

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
    header.pack_end(&add_shell_button);

    let content_root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    content_root.add_css_class("battlefield-root");
    content_root.append(&workspace_summary);
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
        state: Rc::new(RefCell::new(WorkspaceState::new())),
        title,
        workspace_summary,
        content_root,
        cards,
        battlefield_scroller,
        focus: FocusWidgets {
            panel: focus_panel,
            title: focus_title,
            subtitle: focus_subtitle,
            terminal_stack,
            exit_button: focus_exit_button,
        },
        session_cards: RefCell::new(BTreeMap::new()),
        observations: RefCell::new(BTreeMap::new()),
    });

    {
        let exit_button = context.focus.exit_button.clone();
        let context = context.clone();
        exit_button.connect_clicked(move |_| show_battlefield(&context));
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
            if key == gdk::Key::Escape && context.state.borrow().focused_session().is_some() {
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
        .focus
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
    let headline = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-headline".to_string()])
        .build();
    let detail = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-detail".to_string()])
        .build();
    let evidence_one = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-evidence".to_string()])
        .build();
    let evidence_two = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-evidence".to_string()])
        .build();
    let alert = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["card-alert".to_string()])
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

    let evidence_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    evidence_box.append(&evidence_one);
    evidence_box.append(&evidence_two);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(14)
        .margin_end(14)
        .build();
    content.append(&header);
    content.append(&headline);
    content.append(&detail);
    content.append(&evidence_box);
    content.append(&alert);

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
            show_intervention(&context, session_id);
        });
        frame.add_controller(click);
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
        frame,
        status,
        recency,
        headline,
        detail,
        evidence_one,
        evidence_two,
        alert,
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
    refresh_focus_panel(context);
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
    observation.recent_files = session
        .launch
        .cwd
        .as_deref()
        .map(|cwd| scan_recent_files(cwd, &mut observation.file_fingerprints))
        .unwrap_or_default();
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
    apply_battle_card_surface_style(&card.frame, card_model.status);
    card.status.set_label(card_model.status.label());
    card.recency.set_label(&card_model.recency_label);
    card.headline.set_label(&card_model.headline);
    card.detail
        .set_label(card_model.primary_detail.as_deref().unwrap_or(""));
    card.detail.set_visible(card_model.primary_detail.is_some());

    let evidence_one = card_model.evidence_fragments.first().map(String::as_str).unwrap_or("");
    let evidence_two = card_model
        .evidence_fragments
        .get(1)
        .map(String::as_str)
        .unwrap_or("");
    card.evidence_one.set_label(evidence_one);
    card.evidence_two.set_label(evidence_two);
    card.evidence_one.set_visible(!evidence_one.is_empty());
    card.evidence_two.set_visible(!evidence_two.is_empty());

    card.alert.set_label(&card_model.alignment.text);
    for css in ["card-signal-calm", "card-signal-watch", "card-signal-alert"] {
        card.alert.remove_css_class(css);
    }
    card.alert.add_css_class(match card_model.alignment.tone {
        SignalTone::Calm => "card-signal-calm",
        SignalTone::Watch => "card-signal-watch",
        SignalTone::Alert => "card-signal-alert",
    });
    card.alert.set_visible(true);
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
    let state = context.state.borrow();
    let subtitle = if let Some(session_id) = state.focused_session() {
        let focus_name = state
            .session(session_id)
            .map(|session| session.launch.name.clone())
            .unwrap_or_else(|| "Session".into());
        format!(
            "Focused terminal: {} · {} sessions · idle {} · failed {}",
            focus_name,
            sessions.len(),
            idle,
            failed
        )
    } else {
        format!(
            "{} sessions · idle {} · working {} · failed {}",
            sessions.len(),
            idle,
            working,
            failed
        )
    };
    context.title.set_subtitle(&subtitle);
}

fn refresh_card_styles(context: &Rc<AppContext>) {
    let selected = context.state.borrow().selected_session();
    let focused = context.state.borrow().focused_session();
    for (session_id, card) in context.session_cards.borrow().iter() {
        card.row.remove_css_class("selected-card");
        card.row.remove_css_class("focused-card");
        if selected == Some(*session_id) {
            card.row.add_css_class("selected-card");
        }
        if focused == Some(*session_id) {
            card.row.add_css_class("focused-card");
        }
    }
}

fn show_intervention(context: &Rc<AppContext>, session_id: SessionId) {
    context.state.borrow_mut().enter_focus_mode(session_id);
    if let Some(card) = context.session_cards.borrow().get(&session_id) {
        context.cards.select_child(&card.row);
        context
            .focus
            .terminal_stack
            .set_visible_child_name(&card.terminal_page);
        card.terminal.grab_focus();
    }
    context.focus.panel.set_visible(true);
    context.content_root.add_css_class("focus-mode");
    context.battlefield_scroller.set_vexpand(false);
    context.battlefield_scroller.set_min_content_height(280);
    context.battlefield_scroller.set_max_content_height(340);
    update_flowbox_columns(context);
    refresh_card_styles(context);
    refresh_focus_panel(context);
    refresh_workspace(context);
}

fn show_battlefield(context: &Rc<AppContext>) {
    context.state.borrow_mut().return_to_battlefield();
    context.focus.panel.set_visible(false);
    context.content_root.remove_css_class("focus-mode");
    context.battlefield_scroller.set_vexpand(true);
    context.battlefield_scroller.set_min_content_height(0);
    context.battlefield_scroller.set_max_content_height(-1);
    update_flowbox_columns(context);
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

    context
        .focus
        .title
        .set_label(&format!("{} · Focused Terminal", session.launch.name));
    context.focus.subtitle.set_label(&format!(
        "{} · cards stay visible above; click another card to retarget focus or press Escape to return to battlefield-only mode",
        session.launch.subtitle
    ));
}

fn update_flowbox_columns(context: &Rc<AppContext>) {
    let total = context.session_cards.borrow().len();
    let columns = if context.state.borrow().focused_session().is_some() {
        total.clamp(1, 4)
    } else if total <= 4 {
        2
    } else if total <= 6 {
        3
    } else {
        4
    } as u32;
    context.cards.set_max_children_per_line(columns);
    context.cards.set_min_children_per_line(columns);
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
        SessionKind::PlanningStream => None,
        SessionKind::BlockingPrompt => Some("Waiting on approval prompt".into()),
        SessionKind::RunningStream => Some("Long-running tool activity".into()),
        SessionKind::FailingTask => Some("Task exited after failure".into()),
    }
}

fn read_dominant_process_hint(pid: u32) -> Option<String> {
    let process_tree = crate::procfs::format_process_tree(pid).ok()?;
    let mut lines = process_tree.lines().filter(|line| !line.trim().is_empty());
    let candidate = lines.nth(1).or_else(|| process_tree.lines().next())?;
    let simplified = candidate
        .split(" pid=")
        .next()
        .unwrap_or(candidate)
        .replace("  ", " ")
        .trim()
        .to_string();
    let lowered = simplified.to_ascii_lowercase();
    if lowered.starts_with("bash ")
        || lowered.starts_with("bash[")
        || lowered == "bash"
        || lowered.starts_with("sleep ")
        || lowered.starts_with("sleep[")
    {
        return None;
    }
    Some(simplified)
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

fn apply_battle_card_surface_style(frame: &gtk::Frame, status: BattleCardStatus) {
    for css in [
        "card-idle",
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
            font-size: 13px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .battle-card {
            border-radius: 22px;
            border: 1px solid rgba(163, 175, 194, 0.18);
            background: rgba(10, 18, 28, 0.94);
            box-shadow: 0 22px 42px rgba(0, 0, 0, 0.28);
            min-width: 392px;
            min-height: 230px;
        }

        .card-title {
            font-weight: 800;
            font-size: 17px;
            color: #f8fafc;
        }

        .card-subtitle {
            color: rgba(196, 208, 222, 0.72);
            font-size: 13px;
        }

        .card-status {
            font-weight: 800;
            font-size: 11px;
            letter-spacing: 0.08em;
            text-transform: uppercase;
        }

        .card-recency {
            color: rgba(176, 190, 206, 0.8);
            font-size: 12px;
        }

        .card-headline {
            color: #f8fafc;
            font-weight: 800;
            font-size: 20px;
        }

        .card-detail {
            color: rgba(221, 229, 238, 0.92);
            font-size: 14px;
        }

        .card-evidence {
            color: rgba(185, 201, 218, 0.88);
            font-size: 13px;
        }

        .card-alert {
            font-size: 13px;
            font-weight: 700;
        }

        .card-signal-calm {
            color: #bbf7d0;
        }

        .card-signal-watch {
            color: #fde68a;
        }

        .card-signal-alert {
            color: #fecaca;
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
        }

        .battle-thinking {
            color: #dbe7f5;
        }

        .battle-working {
            color: #86efac;
        }

        .battle-blocked {
            color: #fdba74;
        }

        .battle-failed {
            color: #fca5a5;
        }

        .battle-complete {
            color: #99f6e4;
        }

        .battle-detached {
            color: #e9d5ff;
        }

        .focus-mode .battle-card {
            min-width: 286px;
            min-height: 186px;
            border-radius: 18px;
            box-shadow: 0 14px 28px rgba(0, 0, 0, 0.22);
        }

        .focus-mode .card-title {
            font-size: 15px;
        }

        .focus-mode .card-subtitle,
        .focus-mode .card-status,
        .focus-mode .card-recency {
            font-size: 11px;
        }

        .focus-mode .card-headline {
            font-size: 15px;
        }

        .focus-mode .card-detail,
        .focus-mode .card-evidence,
        .focus-mode .card-alert {
            font-size: 12px;
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
