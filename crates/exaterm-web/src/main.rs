mod relay;
mod routes;
mod websocket;

use relay::DaemonRelay;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let opts = parse_args();
    let relay = Arc::new(DaemonRelay::start());
    if !opts.bind.ip().is_loopback() {
        eprintln!("WARNING: binding to non-localhost address {}. The web UI has no authentication — anyone who can reach this address gets full terminal access.", opts.bind.ip());
    }
    if opts.dev_assets.is_some() {
        eprintln!("dev mode: serving assets from filesystem");
    }
    let router = routes::build_router(relay, opts.dev_assets);
    let listener = tokio::net::TcpListener::bind(opts.bind)
        .await
        .unwrap_or_else(|e| {
            eprintln!("failed to bind {}: {e}", opts.bind);
            std::process::exit(1);
        });
    eprintln!("exaterm-web listening on http://{}", opts.bind);
    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

struct Opts {
    bind: SocketAddr,
    dev_assets: Option<PathBuf>,
}

fn parse_args() -> Opts {
    let mut host = "127.0.0.1".to_string();
    let mut port = 9800u16;
    let mut dev_assets: Option<PathBuf> = None;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    port = value.parse().expect("invalid --port value");
                }
            }
            "--bind" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    host = value.clone();
                }
            }
            "--no-embed" => {
                dev_assets = Some(PathBuf::from(
                    concat!(env!("CARGO_MANIFEST_DIR"), "/frontend/dist"),
                ));
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let bind: SocketAddr = format!("{host}:{port}")
        .parse()
        .expect("invalid bind address");
    Opts { bind, dev_assets }
}
