mod actions;
mod beachhead;
mod layout;
mod remote;
mod style;
mod supervision;
mod terminal_adapter;
mod ui;
mod widgets;
mod workspace_view;

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--beachhead-daemon") {
        return if exaterm_core::run_local_daemon() == std::process::ExitCode::SUCCESS {
            glib::ExitCode::SUCCESS
        } else {
            glib::ExitCode::from(1)
        };
    }
    ui::run()
}
