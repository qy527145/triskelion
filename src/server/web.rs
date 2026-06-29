//! 内置 Web UI：把 `web/dist` 编译进二进制并作为静态资源/ SPA 提供。

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// SPA 首页。
pub async fn index() -> Response {
    serve("index.html")
}

/// 兜底处理：静态资源命中则返回文件，未知非 API 路径回退到 index.html（前端路由）。
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // 未匹配的 API 路径返回 JSON 404，避免回退成 HTML。
    if path.starts_with("v1/") || path == "healthz" {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    serve(path)
}

fn serve(path: &str) -> Response {
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(file) = Assets::get(path) {
        let mime = file.metadata.mimetype();
        return ([(header::CONTENT_TYPE, mime)], file.data.into_owned()).into_response();
    }
    // SPA 回退。
    match Assets::get("index.html") {
        Some(file) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            file.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "web ui not built").into_response(),
    }
}
