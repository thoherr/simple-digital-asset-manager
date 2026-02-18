mod routes;
mod static_assets;
pub mod templates;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use tower_http::services::ServeDir;

use crate::catalog::Catalog;
use crate::preview::PreviewGenerator;
use crate::query::QueryEngine;

/// Shared application state for the web server.
pub struct AppState {
    catalog_root: PathBuf,
}

impl AppState {
    pub fn new(catalog_root: PathBuf) -> Self {
        Self { catalog_root }
    }

    /// Open a fresh catalog connection (each request gets its own).
    pub fn catalog(&self) -> Result<Catalog> {
        Catalog::open(&self.catalog_root)
    }

    /// Create a QueryEngine for this catalog.
    pub fn query_engine(&self) -> QueryEngine {
        QueryEngine::new(&self.catalog_root)
    }

    /// Create a PreviewGenerator for checking preview existence.
    pub fn preview_generator(&self) -> PreviewGenerator {
        PreviewGenerator::new(&self.catalog_root, false)
    }
}

fn build_router(state: Arc<AppState>) -> Router {
    let preview_dir = state.catalog_root.join("previews");

    Router::new()
        .route("/", axum::routing::get(routes::browse_page))
        .route("/asset/{id}", axum::routing::get(routes::asset_page))
        .route("/api/search", axum::routing::get(routes::search_api))
        .route(
            "/api/asset/{id}/tags",
            axum::routing::post(routes::add_tags),
        )
        .route(
            "/api/asset/{id}/tags/{tag}",
            axum::routing::delete(routes::remove_tag),
        )
        .route("/api/tags", axum::routing::get(routes::tags_api))
        .route("/api/stats", axum::routing::get(routes::stats_api))
        .route("/static/htmx.min.js", axum::routing::get(static_assets::htmx_js))
        .route("/static/style.css", axum::routing::get(static_assets::style_css))
        .nest_service("/preview", ServeDir::new(preview_dir))
        .with_state(state)
}

/// Start the web server.
pub async fn serve(catalog_root: PathBuf, bind: &str, port: u16) -> Result<()> {
    let state = Arc::new(AppState::new(catalog_root));

    // Verify catalog is accessible
    state.catalog()?;

    let app = build_router(state);

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    eprintln!("dam web UI: http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    eprintln!("\nShutting down...");
}
