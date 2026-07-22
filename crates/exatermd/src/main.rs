fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--mcp-stdio-bridge")) {
        let Some(socket_path) = args.next() else {
            eprintln!("usage: exatermd --mcp-stdio-bridge <socket-path>");
            return std::process::ExitCode::from(2);
        };
        return exaterm_core::run_mcp_stdio_bridge(std::path::Path::new(&socket_path));
    }
    exaterm_core::run_local_daemon()
}
