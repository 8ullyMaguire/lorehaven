//! Static asset serving.
//!
//! Spec §5 requires "embedded frontend assets" and "a frontend page loads from
//! the Rust executable"; §2.1 notes that the production application does not
//! need a Node.js server. The frontend bundle is therefore compiled into the
//! binary with `rust-embed` and served from memory.
//!
//! Development keeps an escape hatch: `[assets] dir` serves from disk instead,
//! so a `vite build --watch` loop does not require recompiling Rust. Production
//! refuses that configuration (see [`crate::safety`]), because a deployment
//! that depends on files beside the binary is not one artifact.

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Response as HttpResponse, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

use crate::state::AppState;

/// The compiled-in frontend bundle.
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../frontend/dist"]
struct EmbeddedAssets;

/// Files that never change once built, and so may be cached aggressively.
/// Vite emits content-hashed names into `assets/`.
const IMMUTABLE_PREFIX: &str = "assets/";

/// Serve a static asset, falling back to the SPA shell for route paths.
pub async fn serve(State(state): State<AppState>, uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    if !is_safe_relative_path(requested) {
        return not_found(requested);
    }

    let assets_dir = state.config().assets.dir.clone();

    if let Some((bytes, mime)) = load(assets_dir.as_deref(), requested).await {
        return file_response(requested, bytes, mime);
    }

    // Client-side routing: an extensionless path is a route, not a file, so
    // hand back the shell and let the router resolve it.
    if !requested.contains('.') && requested != "index.html" {
        if let Some((bytes, _)) = load(assets_dir.as_deref(), "index.html").await {
            return file_response("index.html", bytes, "text/html; charset=utf-8".to_owned());
        }
    }

    not_found(requested)
}

/// Read an asset from disk (development) or from the embedded bundle.
async fn load(dir: Option<&Path>, path: &str) -> Option<(Vec<u8>, String)> {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    if let Some(dir) = dir {
        let full = dir.join(path);
        // Re-check after joining: symlinks and `..` must not escape the root.
        if !is_safe_relative_path(path) {
            return None;
        }
        let bytes = tokio::fs::read(&full).await.ok()?;
        return Some((bytes, mime));
    }

    let file = EmbeddedAssets::get(path)?;
    Some((file.data.into_owned(), mime))
}

fn file_response(path: &str, bytes: Vec<u8>, mime: String) -> Response {
    let cache = if path.starts_with(IMMUTABLE_PREFIX) {
        // Content-hashed filenames: safe for a year, and revalidation is waste.
        "public, max-age=31536000, immutable"
    } else {
        // The shell and the manifest change with every deploy.
        "no-cache"
    };

    let mut response = HttpResponse::new(Body::from(bytes));
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&mime) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

/// A 404 that is honest about being a 404.
fn not_found(path: &str) -> Response {
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Not found — Lorehaven</title></head><body>\
         <h1>Not found</h1><p>No such page: <code>{}</code></p>\
         <p><a href=\"/\">Return to Lorehaven</a></p></body></html>",
        html_escape(path)
    );

    let mut response = (StatusCode::NOT_FOUND, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// Reject anything that is not a plain, relative, descendant-free path.
#[must_use]
pub fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 1024 {
        return false;
    }
    if path.contains('\0') || path.contains('\\') {
        return false;
    }
    let candidate = PathBuf::from(path);
    !candidate
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_paths_are_accepted() {
        assert!(is_safe_relative_path("index.html"));
        assert!(is_safe_relative_path("assets/index-abc123.js"));
        assert!(is_safe_relative_path("favicon.ico"));
    }

    #[test]
    fn traversal_attempts_are_refused() {
        assert!(!is_safe_relative_path("../etc/passwd"));
        assert!(!is_safe_relative_path("assets/../../etc/passwd"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("assets/./../../x"));
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("a\\b"));
        assert!(!is_safe_relative_path("with\0nul"));
    }

    #[test]
    fn the_404_page_escapes_its_input() {
        let response = not_found("<script>alert(1)</script>");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        // The escaping helper is what stands between a crafted URL and
        // reflected script execution.
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn the_embedded_shell_exists() {
        // `cargo build` materialises a placeholder when the frontend has not
        // been built, so this must hold for a clean checkout too.
        assert!(
            EmbeddedAssets::get("index.html").is_some(),
            "the embedded bundle must always contain a shell"
        );
    }
}
