use axum::http::header;
use axum::response::{IntoResponse, Response};

const HTMX_JS: &[u8] = include_bytes!("static/htmx.min.js");
const STYLE_CSS: &str = include_str!("static/style.css");

// Leaflet + MarkerCluster
const LEAFLET_JS: &[u8] = include_bytes!("static/leaflet.min.js");
const LEAFLET_CSS: &str = include_str!("static/leaflet.css");
const MARKERCLUSTER_JS: &[u8] = include_bytes!("static/leaflet.markercluster.min.js");
const MARKERCLUSTER_CSS: &str = include_str!("static/MarkerCluster.css");
const MARKERCLUSTER_DEFAULT_CSS: &str = include_str!("static/MarkerCluster.Default.css");

// Brand assets
const FAVICON_ICO: &[u8] = include_bytes!("static/favicon.ico");
const MAKI_ICON_SVG: &[u8] = include_bytes!("static/maki-icon.svg");

// Leaflet marker images
const MARKER_ICON: &[u8] = include_bytes!("static/leaflet-images/marker-icon.png");
const MARKER_ICON_2X: &[u8] = include_bytes!("static/leaflet-images/marker-icon-2x.png");
const MARKER_SHADOW: &[u8] = include_bytes!("static/leaflet-images/marker-shadow.png");
const LAYERS_PNG: &[u8] = include_bytes!("static/leaflet-images/layers.png");
const LAYERS_2X: &[u8] = include_bytes!("static/leaflet-images/layers-2x.png");

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

pub async fn leaflet_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        LEAFLET_JS,
    )
        .into_response()
}

pub async fn leaflet_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        LEAFLET_CSS,
    )
        .into_response()
}

pub async fn markercluster_js() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKERCLUSTER_JS,
    )
        .into_response()
}

pub async fn markercluster_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKERCLUSTER_CSS,
    )
        .into_response()
}

pub async fn markercluster_default_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKERCLUSTER_DEFAULT_CSS,
    )
        .into_response()
}

pub async fn favicon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        FAVICON_ICO,
    )
        .into_response()
}

pub async fn maki_icon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MAKI_ICON_SVG,
    )
        .into_response()
}

pub async fn marker_icon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKER_ICON,
    )
        .into_response()
}

pub async fn marker_icon_2x() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKER_ICON_2X,
    )
        .into_response()
}

pub async fn marker_shadow() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        MARKER_SHADOW,
    )
        .into_response()
}

pub async fn layers_png() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        LAYERS_PNG,
    )
        .into_response()
}

pub async fn layers_2x() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        LAYERS_2X,
    )
        .into_response()
}
