use axum::http::header;
use axum::response::{IntoResponse, Response};

const HTMX_JS: &[u8] = include_bytes!("static/htmx.min.js");
const STYLE_CSS: &str = include_str!("static/style.css");

pub async fn htmx_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        HTMX_JS,
    )
        .into_response()
}

pub async fn style_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        STYLE_CSS,
    )
        .into_response()
}
