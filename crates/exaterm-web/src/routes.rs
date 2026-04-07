use crate::relay::DaemonRelay;
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use include_dir::{include_dir, Dir};
use std::path::PathBuf;
use std::sync::Arc;

static FRONTEND_DIST: Dir = include_dir!("$CARGO_MANIFEST_DIR/frontend/dist");

pub fn build_router(relay: Arc<DaemonRelay>, dev_assets: Option<PathBuf>) -> Router {
    let state = AppState { relay, dev_assets };
    Router::new()
        .route("/", get(index))
        .route("/assets/{*path}", get(static_asset))
        .route("/ws/control", get(crate::websocket::ws_control))
        .route(
            "/ws/stream/{session_id}",
            get(crate::websocket::ws_stream),
        )
        .layer(middleware::from_fn(security_headers))
        .with_state(Arc::new(state))
}

async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // 'unsafe-inline' is required for style-src because xterm.js injects
    // inline styles at runtime and the UI sets element.style properties.
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'"),
    );
    response
}

#[derive(Clone)]
pub struct AppState {
    pub relay: Arc<DaemonRelay>,
    pub dev_assets: Option<PathBuf>,
}

async fn index(state: axum::extract::State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(dir) = &state.dev_assets {
        match tokio::fs::read_to_string(dir.join("index.html")).await {
            Ok(html) => return Html(html).into_response(),
            Err(e) => eprintln!("dev assets: failed to read index.html: {e}"),
        }
    }
    match FRONTEND_DIST.get_file("index.html") {
        Some(file) => Html(file.contents_utf8().unwrap_or_default()).into_response(),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}

async fn static_asset(
    Path(path): Path<String>,
    state: axum::extract::State<Arc<AppState>>,
) -> Response {
    if let Some(dir) = &state.dev_assets {
        let file_path = dir.join(&path);
        // Prevent path traversal — resolved path must be within the asset dir.
        if let Ok(canonical) = file_path.canonicalize() {
            if let Ok(canonical_dir) = dir.canonicalize() {
                if canonical.starts_with(&canonical_dir) {
                    if let Ok(bytes) = tokio::fs::read(&canonical).await {
                        let mime = mime_for_path(&path);
                        return ([(header::CONTENT_TYPE, mime)], bytes).into_response();
                    }
                }
            }
        }
    }
    match FRONTEND_DIST.get_file(&path) {
        Some(file) => {
            let mime = mime_for_path(&path);
            ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn mime_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_for_known_extensions() {
        assert_eq!(mime_for_path("app.js"), "application/javascript");
        assert_eq!(mime_for_path("app.css"), "text/css");
        assert_eq!(mime_for_path("index.html"), "text/html");
        assert_eq!(mime_for_path("icon.svg"), "image/svg+xml");
        assert_eq!(mime_for_path("logo.png"), "image/png");
        assert_eq!(mime_for_path("font.woff2"), "font/woff2");
        assert_eq!(mime_for_path("app.js.map"), "application/json");
    }

    #[test]
    fn mime_for_unknown_extension() {
        assert_eq!(mime_for_path("data.bin"), "application/octet-stream");
        assert_eq!(mime_for_path("noext"), "application/octet-stream");
    }

    #[test]
    fn embedded_dist_contains_index_html() {
        assert!(
            FRONTEND_DIST.get_file("index.html").is_some(),
            "frontend/dist/index.html should be embedded"
        );
    }

    #[test]
    fn embedded_dist_contains_main_js() {
        assert!(
            FRONTEND_DIST.get_file("main.js").is_some(),
            "frontend/dist/main.js should be embedded"
        );
    }

    #[test]
    fn embedded_dist_contains_app_css() {
        assert!(
            FRONTEND_DIST.get_file("app.css").is_some(),
            "frontend/dist/app.css should be embedded"
        );
    }

    #[test]
    fn embedded_dist_contains_main_css() {
        assert!(
            FRONTEND_DIST.get_file("main.css").is_some(),
            "frontend/dist/main.css should be embedded (xterm.js styles)"
        );
    }

    #[test]
    fn embedded_index_html_references_assets() {
        let file = FRONTEND_DIST
            .get_file("index.html")
            .expect("index.html should exist");
        let html = file.contents_utf8().expect("index.html should be utf8");
        assert!(
            html.contains("/assets/main.js"),
            "index.html should reference main.js"
        );
        assert!(
            html.contains("/assets/app.css"),
            "index.html should reference app.css"
        );
        assert!(
            html.contains("/assets/main.css"),
            "index.html should reference main.css"
        );
    }
}
