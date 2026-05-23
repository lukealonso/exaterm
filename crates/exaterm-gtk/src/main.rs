#[cfg(target_os = "linux")]
mod beachhead;
#[cfg(target_os = "linux")]
mod launcher;
#[cfg(target_os = "linux")]
mod style;
#[cfg(target_os = "linux")]
mod terminal_adapter;
#[cfg(target_os = "linux")]
mod terminal_images;
#[cfg(target_os = "linux")]
mod ui;
#[cfg(target_os = "linux")]
mod widgets;

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("exaterm-gtk is only supported on Linux");
}
