use crate::style::{configure_app_icons, load_css};
use crate::ui::{launch_workspace, APP_ID};
use exaterm_core::config::{
    apply_app_config_environment, load_app_config, save_app_config, AppConfig, RememberedHost,
    TerminalAssistConfig, DEFAULT_OPENAI_BASE_URL, DEFAULT_TERMINAL_ASSIST_MODEL,
};
use exaterm_ui::beachhead::{
    list_local_live_workspaces, list_remote_live_workspaces, LiveWorkspace, ParsedArgs, RunMode,
    WorkspaceArg,
};
use gtk::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
enum LauncherSource {
    Local,
    Remote(String),
}

impl LauncherSource {
    fn run_mode(&self) -> RunMode {
        match self {
            Self::Local => RunMode::Local,
            Self::Remote(target) => RunMode::Ssh {
                target: target.clone(),
            },
        }
    }
}

pub(crate) fn present_launcher(app: &gtk::Application, initial: ParsedArgs) {
    load_css();
    configure_app_icons(APP_ID);

    let config = Rc::new(RefCell::new(load_app_config()));
    apply_app_config_environment(&config.borrow());
    if let RunMode::Ssh { target } = &initial.mode {
        remember_remote_target(&config, target);
    }

    let source = Rc::new(RefCell::new(match initial.mode {
        RunMode::Local => LauncherSource::Local,
        RunMode::Ssh { target } => LauncherSource::Remote(target),
    }));
    let workspaces = Rc::new(RefCell::new(Vec::<LiveWorkspace>::new()));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("exaterm")
        .icon_name(APP_ID)
        .default_width(980)
        .default_height(620)
        .build();
    window.add_css_class("launcher-window");

    let title = adw::WindowTitle::new("Exaterm", "Choose a Host");
    let header = adw::HeaderBar::builder()
        .title_widget(&title)
        .show_end_title_buttons(true)
        .build();
    let settings_button = gtk::Button::builder()
        .label("Settings")
        .tooltip_text("Settings")
        .build();
    settings_button.add_css_class("launcher-secondary-button");
    header.pack_end(&settings_button);
    {
        let window = window.clone();
        let config = config.clone();
        settings_button.connect_clicked(move |_| {
            present_settings_window(&window, &config);
        });
    }

    let headline = gtk::Label::builder()
        .label("Open Workspace")
        .xalign(0.0)
        .css_classes(vec!["launcher-title".to_string()])
        .build();
    let subtitle = gtk::Label::builder()
        .label(
            "Choose where your sessions live, then reconnect to a workspace or start another one.",
        )
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["launcher-subtitle".to_string()])
        .build();

    let source_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .vexpand(true)
        .build();
    source_list.add_css_class("launcher-list");
    populate_source_list(&source_list, &config.borrow());

    let remote_entry = gtk::Entry::builder()
        .placeholder_text("user@host")
        .hexpand(true)
        .build();
    remote_entry.add_css_class("launcher-entry");
    if let LauncherSource::Remote(target) = &*source.borrow() {
        remote_entry.set_text(target);
    }
    let scan_button = gtk::Button::builder().label("Scan").build();
    scan_button.add_css_class("launcher-secondary-button");
    let spinner = gtk::Spinner::builder().visible(false).build();

    let source_status = gtk::Label::builder()
        .label("Local host")
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["launcher-muted".to_string()])
        .build();

    let workspace_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .vexpand(true)
        .build();
    workspace_list.add_css_class("launcher-list");

    let workspace_status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["launcher-muted".to_string()])
        .build();
    let reconnect_button = gtk::Button::builder()
        .label("Reconnect")
        .sensitive(false)
        .build();
    reconnect_button.add_css_class("launcher-primary-button");
    let refresh_button = gtk::Button::builder().label("Refresh").build();
    refresh_button.add_css_class("launcher-secondary-button");

    let new_entry = gtk::Entry::builder()
        .placeholder_text("new-workspace")
        .hexpand(true)
        .build();
    new_entry.add_css_class("launcher-entry");
    let start_button = gtk::Button::builder().label("Start").build();
    start_button.add_css_class("launcher-secondary-button");

    let select_local = {
        let source = source.clone();
        let workspaces = workspaces.clone();
        let workspace_list = workspace_list.clone();
        let workspace_status = workspace_status.clone();
        let reconnect_button = reconnect_button.clone();
        let source_status = source_status.clone();
        move || {
            source.replace(LauncherSource::Local);
            source_status.set_label("Local host");
            let found = list_local_live_workspaces();
            *workspaces.borrow_mut() = found;
            refresh_workspace_list(
                &workspace_list,
                &workspace_status,
                &reconnect_button,
                &workspaces.borrow(),
                "local",
            );
        }
    };
    select_local();

    {
        let reconnect_button = reconnect_button.clone();
        workspace_list.connect_row_selected(move |_, row| {
            reconnect_button.set_sensitive(row.is_some());
        });
    }
    {
        let app = app.clone();
        let window = window.clone();
        let source = source.clone();
        let workspace_list_for_signal = workspace_list.clone();
        let workspace_list = workspace_list.clone();
        let workspaces = workspaces.clone();
        workspace_list_for_signal.connect_row_activated(move |_, _| {
            if let Some(workspace) = selected_workspace(&workspace_list, &workspaces.borrow()) {
                launch_selected_workspace(
                    &app,
                    &window,
                    source.borrow().run_mode(),
                    workspace.id,
                    false,
                );
            }
        });
    }
    {
        let app = app.clone();
        let window = window.clone();
        let source = source.clone();
        let workspace_list = workspace_list.clone();
        let workspaces = workspaces.clone();
        reconnect_button.connect_clicked(move |_| {
            if let Some(workspace) = selected_workspace(&workspace_list, &workspaces.borrow()) {
                launch_selected_workspace(
                    &app,
                    &window,
                    source.borrow().run_mode(),
                    workspace.id,
                    false,
                );
            }
        });
    }
    {
        let app = app.clone();
        let window = window.clone();
        let source = source.clone();
        let entry = new_entry.clone();
        start_button.connect_clicked(move |_| {
            launch_selected_workspace(
                &app,
                &window,
                source.borrow().run_mode(),
                normalized_workspace_id(&entry.text()),
                true,
            );
        });
    }
    {
        let app = app.clone();
        let window = window.clone();
        let source = source.clone();
        let entry = new_entry.clone();
        new_entry.connect_activate(move |_| {
            launch_selected_workspace(
                &app,
                &window,
                source.borrow().run_mode(),
                normalized_workspace_id(&entry.text()),
                true,
            );
        });
    }
    {
        let source = source.clone();
        let workspaces = workspaces.clone();
        let workspace_list = workspace_list.clone();
        let workspace_status = workspace_status.clone();
        let reconnect_button = reconnect_button.clone();
        let source_status = source_status.clone();
        let spinner = spinner.clone();
        refresh_button.connect_clicked(move |_| match &*source.borrow() {
            LauncherSource::Local => {
                source_status.set_label("Local host");
                let found = list_local_live_workspaces();
                *workspaces.borrow_mut() = found;
                refresh_workspace_list(
                    &workspace_list,
                    &workspace_status,
                    &reconnect_button,
                    &workspaces.borrow(),
                    "local",
                );
            }
            LauncherSource::Remote(target) => {
                source_status.set_label(&format!("Remote host: {target}"));
                start_remote_scan(
                    target.clone(),
                    &workspace_list,
                    &workspaces,
                    &workspace_status,
                    &spinner,
                    &reconnect_button,
                );
            }
        });
    }
    {
        let source_list = source_list.clone();
        let config = config.clone();
        let source = source.clone();
        let remote_entry = remote_entry.clone();
        let workspace_list = workspace_list.clone();
        let workspaces = workspaces.clone();
        let workspace_status = workspace_status.clone();
        let reconnect_button = reconnect_button.clone();
        let source_status = source_status.clone();
        let spinner = spinner.clone();
        source_list.connect_row_selected(move |_, row| {
            let Some(row) = row else {
                return;
            };
            let index = row.index();
            if index == 0 {
                source.replace(LauncherSource::Local);
                source_status.set_label("Local host");
                let found = list_local_live_workspaces();
                *workspaces.borrow_mut() = found;
                refresh_workspace_list(
                    &workspace_list,
                    &workspace_status,
                    &reconnect_button,
                    &workspaces.borrow(),
                    "local",
                );
            } else if index > 0 {
                let targets = remembered_targets(&config.borrow());
                let Some(target) = targets.get(index as usize - 1).cloned() else {
                    return;
                };
                remote_entry.set_text(&target);
                source.replace(LauncherSource::Remote(target.clone()));
                source_status.set_label(&format!("Remote host: {target}"));
                start_remote_scan(
                    target,
                    &workspace_list,
                    &workspaces,
                    &workspace_status,
                    &spinner,
                    &reconnect_button,
                );
            }
        });
    }
    {
        let config = config.clone();
        let source_list = source_list.clone();
        let source = source.clone();
        let remote_entry = remote_entry.clone();
        let workspace_list = workspace_list.clone();
        let workspaces = workspaces.clone();
        let workspace_status = workspace_status.clone();
        let reconnect_button = reconnect_button.clone();
        let source_status = source_status.clone();
        let spinner = spinner.clone();
        scan_button.connect_clicked(move |_| {
            let Some(target) = normalized_remote_target(&remote_entry.text()) else {
                workspace_status.set_label("Enter an SSH target first.");
                return;
            };
            remember_remote_target(&config, &target);
            populate_source_list(&source_list, &config.borrow());
            source.replace(LauncherSource::Remote(target.clone()));
            source_status.set_label(&format!("Remote host: {target}"));
            start_remote_scan(
                target,
                &workspace_list,
                &workspaces,
                &workspace_status,
                &spinner,
                &reconnect_button,
            );
        });
    }
    {
        let config = config.clone();
        let source_list = source_list.clone();
        let source = source.clone();
        let remote_entry_for_scan = remote_entry.clone();
        let workspace_list = workspace_list.clone();
        let workspaces = workspaces.clone();
        let workspace_status = workspace_status.clone();
        let reconnect_button = reconnect_button.clone();
        let source_status = source_status.clone();
        let spinner = spinner.clone();
        remote_entry.connect_activate(move |_| {
            let Some(target) = normalized_remote_target(&remote_entry_for_scan.text()) else {
                workspace_status.set_label("Enter an SSH target first.");
                return;
            };
            remember_remote_target(&config, &target);
            populate_source_list(&source_list, &config.borrow());
            source.replace(LauncherSource::Remote(target.clone()));
            source_status.set_label(&format!("Remote host: {target}"));
            start_remote_scan(
                target,
                &workspace_list,
                &workspaces,
                &workspace_status,
                &spinner,
                &reconnect_button,
            );
        });
    }

    if let LauncherSource::Remote(target) = &*source.borrow() {
        start_remote_scan(
            target.clone(),
            &workspace_list,
            &workspaces,
            &workspace_status,
            &spinner,
            &reconnect_button,
        );
    } else if let Some(row) = source_list.row_at_index(0) {
        source_list.select_row(Some(&row));
    }

    let remote_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    remote_row.append(&remote_entry);
    remote_row.append(&scan_button);
    remote_row.append(&spinner);

    let source_panel = launcher_panel("Hosts", "Local or SSH");
    source_panel.append(&source_status);
    source_panel.append(&source_list);
    source_panel.append(&remote_row);

    let workspace_actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    workspace_actions.append(&reconnect_button);
    workspace_actions.append(&refresh_button);

    let new_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    new_row.append(&new_entry);
    new_row.append(&start_button);

    let workspace_panel = launcher_panel("Workspaces", "Live sessions");
    workspace_panel.append(&workspace_status);
    workspace_panel.append(&list_scroller(&workspace_list));
    workspace_panel.append(&workspace_actions);
    workspace_panel.append(&launcher_divider());
    workspace_panel.append(&new_row);

    let panels = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(14)
        .hexpand(true)
        .vexpand(true)
        .build();
    source_panel.set_size_request(300, -1);
    panels.append(&source_panel);
    panels.append(&workspace_panel);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(22)
        .margin_bottom(22)
        .margin_start(22)
        .margin_end(22)
        .hexpand(true)
        .vexpand(true)
        .build();
    content.append(&headline);
    content.append(&subtitle);
    content.append(&panels);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    body.append(&header);
    body.append(&content);
    window.set_content(Some(&body));
    window.present();
}

fn present_settings_window(parent: &adw::ApplicationWindow, config: &Rc<RefCell<AppConfig>>) {
    let current = config.borrow().clone().normalized();
    let window = gtk::Window::builder()
        .title("Exaterm Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .resizable(false)
        .build();
    window.add_css_class("launcher-window");

    let title = gtk::Label::builder()
        .label("Settings")
        .xalign(0.0)
        .css_classes(vec!["launcher-title".to_string()])
        .build();
    let subtitle = gtk::Label::builder()
        .label("Configure terminal behavior and Ctrl-K assist before opening a workspace.")
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["launcher-subtitle".to_string()])
        .build();

    let api_key_entry = gtk::Entry::builder()
        .placeholder_text("OpenAI API key")
        .hexpand(true)
        .build();
    api_key_entry.add_css_class("launcher-entry");
    api_key_entry.set_visibility(false);
    api_key_entry.set_input_purpose(gtk::InputPurpose::Password);
    api_key_entry.set_text(&current.terminal_assist.openai_api_key);

    let base_url_entry = gtk::Entry::builder()
        .placeholder_text(DEFAULT_OPENAI_BASE_URL)
        .hexpand(true)
        .build();
    base_url_entry.add_css_class("launcher-entry");
    base_url_entry.set_text(&current.terminal_assist.openai_base_url);

    let model_entry = gtk::Entry::builder()
        .placeholder_text(DEFAULT_TERMINAL_ASSIST_MODEL)
        .hexpand(true)
        .build();
    model_entry.add_css_class("launcher-entry");
    model_entry.set_text(&current.terminal_assist.model);

    let audible_bell_switch = gtk::Switch::builder()
        .active(current.terminal.audible_bell)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();

    let status = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(vec!["launcher-muted".to_string()])
        .build();

    let terminal_fields = launcher_panel("Terminal", "Behavior");
    terminal_fields.append(&settings_switch_row("Audible bell", &audible_bell_switch));

    let fields = launcher_panel("Ctrl-K Assist", "OpenAI");
    fields.append(&settings_row("API Key", &api_key_entry));
    fields.append(&settings_row("Base URL", &base_url_entry));
    fields.append(&settings_row("Model", &model_entry));
    fields.append(&status);

    let cancel_button = gtk::Button::builder().label("Cancel").build();
    cancel_button.add_css_class("launcher-secondary-button");
    let save_button = gtk::Button::builder().label("Save").build();
    save_button.add_css_class("launcher-primary-button");

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .build();
    actions.append(&cancel_button);
    actions.append(&save_button);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(16)
        .margin_top(22)
        .margin_bottom(22)
        .margin_start(22)
        .margin_end(22)
        .build();
    content.append(&title);
    content.append(&subtitle);
    content.append(&terminal_fields);
    content.append(&fields);
    content.append(&actions);
    window.set_child(Some(&content));

    {
        let window = window.clone();
        cancel_button.connect_clicked(move |_| {
            window.close();
        });
    }
    {
        let config = config.clone();
        let window = window.clone();
        let status = status.clone();
        save_button.connect_clicked(move |_| {
            let mut next = config.borrow().clone();
            next.terminal.audible_bell = audible_bell_switch.is_active();
            next.terminal_assist = TerminalAssistConfig {
                openai_api_key: api_key_entry.text().trim().to_string(),
                openai_base_url: base_url_entry.text().trim().to_string(),
                model: model_entry.text().trim().to_string(),
            }
            .normalized();
            let next = next.normalized();
            match save_app_config(&next) {
                Ok(()) => {
                    apply_app_config_environment(&next);
                    config.replace(next);
                    window.close();
                }
                Err(error) => {
                    status.set_label(&error);
                }
            }
        });
    }

    window.present();
}

fn settings_row(label: &str, entry: &gtk::Entry) -> gtk::Box {
    let label = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .width_chars(10)
        .css_classes(vec!["launcher-row-title".to_string()])
        .build();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.append(&label);
    row.append(entry);
    row
}

fn settings_switch_row(label: &str, switch: &gtk::Switch) -> gtk::Box {
    let label = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["launcher-row-title".to_string()])
        .build();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.append(&label);
    row.append(switch);
    row
}

fn start_remote_scan(
    target: String,
    workspace_list: &gtk::ListBox,
    workspaces: &Rc<RefCell<Vec<LiveWorkspace>>>,
    status: &gtk::Label,
    spinner: &gtk::Spinner,
    reconnect_button: &gtk::Button,
) {
    status.set_label("Scanning remote host workspaces...");
    spinner.set_visible(true);
    spinner.start();
    reconnect_button.set_sensitive(false);
    populate_workspace_list(workspace_list, &[]);
    workspaces.borrow_mut().clear();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = list_remote_live_workspaces(&target);
        let _ = tx.send((target, result));
    });

    let workspace_list = workspace_list.clone();
    let workspaces = workspaces.clone();
    let status = status.clone();
    let spinner = spinner.clone();
    let reconnect_button = reconnect_button.clone();
    glib::timeout_add_local(Duration::from_millis(80), move || match rx.try_recv() {
        Ok((target, result)) => {
            spinner.stop();
            spinner.set_visible(false);
            match result {
                Ok(found) => {
                    let count = found.len();
                    *workspaces.borrow_mut() = found;
                    refresh_workspace_list(
                        &workspace_list,
                        &status,
                        &reconnect_button,
                        &workspaces.borrow(),
                        &target,
                    );
                    reconnect_button.set_sensitive(count > 0);
                }
                Err(error) => {
                    status.set_label(&format!("Could not scan {target}: {error}"));
                    populate_workspace_list(&workspace_list, &[]);
                    workspaces.borrow_mut().clear();
                }
            }
            glib::ControlFlow::Break
        }
        Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(mpsc::TryRecvError::Disconnected) => {
            spinner.stop();
            spinner.set_visible(false);
            status.set_label("Remote scan stopped unexpectedly.");
            glib::ControlFlow::Break
        }
    });
}

fn launcher_panel(title: &str, subtitle: &str) -> gtk::Box {
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(vec!["launcher-panel-title".to_string()])
        .build();
    let subtitle = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .css_classes(vec!["launcher-muted".to_string()])
        .build();
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.add_css_class("launcher-panel");
    panel.append(&title);
    panel.append(&subtitle);
    panel
}

fn list_scroller(list: &gtk::ListBox) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(list)
        .build()
}

fn launcher_divider() -> gtk::Separator {
    gtk::Separator::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build()
}

fn populate_source_list(list: &gtk::ListBox, config: &AppConfig) {
    clear_listbox(list);
    let local = gtk::ListBoxRow::new();
    local.add_css_class("launcher-row");
    local.set_child(Some(&launcher_row_content(
        "Local machine",
        "This computer",
    )));
    list.append(&local);

    for target in remembered_targets(config) {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("launcher-row");
        row.set_child(Some(&launcher_row_content(&target, "Remote Linux host")));
        list.append(&row);
    }

    if let Some(row) = list.row_at_index(0) {
        list.select_row(Some(&row));
    }
}

fn refresh_workspace_list(
    list: &gtk::ListBox,
    status: &gtk::Label,
    reconnect_button: &gtk::Button,
    workspaces: &[LiveWorkspace],
    source_label: &str,
) {
    populate_workspace_list(list, workspaces);
    update_workspace_status(status, workspaces.len(), source_label);
    reconnect_button.set_sensitive(!workspaces.is_empty());
}

fn populate_workspace_list(list: &gtk::ListBox, workspaces: &[LiveWorkspace]) {
    clear_listbox(list);
    for workspace in workspaces {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("launcher-row");
        let detail_text = workspace
            .id
            .as_ref()
            .map(|id| format!("workspace {id}"))
            .unwrap_or_else(|| "default namespace".into());
        row.set_child(Some(&launcher_row_content(&workspace.label, &detail_text)));
        list.append(&row);
    }
    if let Some(row) = list.row_at_index(0) {
        list.select_row(Some(&row));
    }
}

fn launcher_row_content(title: &str, detail: &str) -> gtk::Box {
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["launcher-row-title".to_string()])
        .build();
    let detail = gtk::Label::builder()
        .label(detail)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(vec!["launcher-row-detail".to_string()])
        .build();
    let row_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(10)
        .build();
    row_box.append(&title);
    row_box.append(&detail);
    row_box
}

fn clear_listbox(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn selected_workspace(list: &gtk::ListBox, workspaces: &[LiveWorkspace]) -> Option<LiveWorkspace> {
    let row = list.selected_row()?;
    let index = row.index();
    if index < 0 {
        return None;
    }
    workspaces.get(index as usize).cloned()
}

fn update_workspace_status(label: &gtk::Label, count: usize, source: &str) {
    let text = match count {
        0 => format!("No live workspaces on {source}."),
        1 => format!("1 live workspace on {source}."),
        count => format!("{count} live workspaces on {source}."),
    };
    label.set_label(&text);
}

fn launch_selected_workspace(
    app: &gtk::Application,
    window: &adw::ApplicationWindow,
    mode: RunMode,
    workspace_id: Option<String>,
    create_new: bool,
) {
    let workspace = workspace_id.map(|id| {
        if create_new {
            WorkspaceArg::New(id)
        } else {
            WorkspaceArg::Resume(id)
        }
    });
    window.close();
    launch_workspace(app, ParsedArgs { mode, workspace });
}

fn normalized_workspace_id(value: &glib::GString) -> Option<String> {
    let mut last_was_dash = false;
    let normalized = value
        .trim()
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                last_was_dash = false;
                Some(ch)
            } else if ch.is_whitespace() && !last_was_dash {
                last_was_dash = true;
                Some('-')
            } else {
                None
            }
        })
        .take(64)
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalized_remote_target(value: &glib::GString) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn remembered_targets(config: &AppConfig) -> Vec<String> {
    let mut hosts = config.hosts.clone();
    hosts.sort_by(|a, b| b.last_used_secs.cmp(&a.last_used_secs));
    hosts.into_iter().map(|host| host.target).collect()
}

fn remember_remote_target(config: &Rc<RefCell<AppConfig>>, target: &str) {
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    {
        let mut config = config.borrow_mut();
        config.hosts.retain(|host| host.target != target);
        config.hosts.insert(
            0,
            RememberedHost {
                target: target.to_string(),
                last_used_secs: unix_now_secs(),
            },
        );
        config.hosts.truncate(8);
        let _ = save_app_config(&*config);
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
