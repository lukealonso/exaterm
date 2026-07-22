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
    let args = std::env::args_os().collect::<Vec<_>>();
    if args.get(1).map(|arg| arg.as_os_str()) == Some(std::ffi::OsStr::new("--mcp-stdio-bridge")) {
        let Some(socket_path) = args.get(2) else {
            eprintln!("usage: exaterm-gtk --mcp-stdio-bridge <socket-path>");
            return glib::ExitCode::from(2);
        };
        return if exaterm_core::run_mcp_stdio_bridge(std::path::Path::new(socket_path))
            == std::process::ExitCode::SUCCESS
        {
            glib::ExitCode::SUCCESS
        } else {
            glib::ExitCode::from(1)
        };
    }
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
