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
use crate::config::PreviewConfig;
use crate::preview::PreviewGenerator;
use crate::query::QueryEngine;

/// Shared application state for the web server.
pub struct AppState {
    catalog_root: PathBuf,
    preview_config: PreviewConfig,
    pub preview_ext: String,
}

impl AppState {
    pub fn new(catalog_root: PathBuf, preview_config: PreviewConfig) -> Self {
        let preview_ext = preview_config.format.extension().to_string();
        Self {
            catalog_root,
            preview_config,
            preview_ext,
        }
    }

    /// Open a fresh catalog connection (each request gets its own).
    /// Uses `open_fast` since migrations run once at server startup.
    pub fn catalog(&self) -> Result<Catalog> {
        Catalog::open_fast(&self.catalog_root)
    }

    /// Create a QueryEngine for this catalog.
    pub fn query_engine(&self) -> QueryEngine {
        QueryEngine::new(&self.catalog_root)
    }

    /// Create a PreviewGenerator for checking preview existence.
    pub fn preview_generator(&self) -> PreviewGenerator {
        PreviewGenerator::new(&self.catalog_root, false, &self.preview_config)
    }
}

fn build_router(state: Arc<AppState>) -> Router {
    let preview_dir = state.catalog_root.join("previews");

    Router::new()
        .route("/", axum::routing::get(routes::browse_page))
        .route("/asset/{id}", axum::routing::get(routes::asset_page))
        .route("/tags", axum::routing::get(routes::tags_page))
        .route("/stats", axum::routing::get(routes::stats_page))
        .route("/api/search", axum::routing::get(routes::search_api))
        .route(
            "/api/asset/{id}/tags",
            axum::routing::post(routes::add_tags),
        )
        .route(
            "/api/asset/{id}/tags/{tag}",
            axum::routing::delete(routes::remove_tag),
        )
        .route(
            "/api/asset/{id}/rating",
            axum::routing::put(routes::set_rating),
        )
        .route(
            "/api/asset/{id}/description",
            axum::routing::put(routes::set_description),
        )
        .route(
            "/api/asset/{id}/name",
            axum::routing::put(routes::set_name),
        )
        .route(
            "/api/asset/{id}/label",
            axum::routing::put(routes::set_label),
        )
        .route(
            "/api/asset/{id}/preview",
            axum::routing::post(routes::generate_preview),
        )
        .route("/api/tags", axum::routing::get(routes::tags_api))
        .route("/api/stats", axum::routing::get(routes::stats_api))
        .route(
            "/api/batch/rating",
            axum::routing::put(routes::batch_set_rating),
        )
        .route(
            "/api/batch/tags",
            axum::routing::post(routes::batch_tags),
        )
        .route(
            "/api/batch/label",
            axum::routing::put(routes::batch_set_label),
        )
        .route(
            "/api/saved-searches",
            axum::routing::get(routes::list_saved_searches)
                .post(routes::create_saved_search),
        )
        .route(
            "/api/saved-searches/{name}",
            axum::routing::delete(routes::delete_saved_search),
        )
        .route("/collections", axum::routing::get(routes::collections_page))
        .route(
            "/api/collections",
            axum::routing::get(routes::list_collections_api)
                .post(routes::create_collection_api),
        )
        .route(
            "/api/batch/collection",
            axum::routing::post(routes::batch_add_to_collection),
        )
        .route("/static/htmx.min.js", axum::routing::get(static_assets::htmx_js))
        .route("/static/style.css", axum::routing::get(static_assets::style_css))
        .nest_service("/preview", ServeDir::new(preview_dir))
        .with_state(state)
}

/// Start the web server.
pub async fn serve(catalog_root: PathBuf, bind: &str, port: u16, preview_config: PreviewConfig) -> Result<()> {
    let state = Arc::new(AppState::new(catalog_root, preview_config));

    // Verify catalog is accessible and run schema migrations once at startup
    Catalog::open(&state.catalog_root)?;

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
