mod routes;
mod static_assets;
pub mod templates;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use axum::Router;
use tower_http::services::ServeDir;

use crate::asset_service::AssetService;
use crate::catalog::Catalog;
use crate::config::PreviewConfig;
use crate::preview::PreviewGenerator;
use crate::query::QueryEngine;

/// Cached dropdown data for the browse page filter controls.
/// Populated lazily on first access, invalidated by write endpoints.
struct DropdownCacheInner {
    tags: Option<Vec<(String, u64)>>,
    formats: Option<Vec<String>>,
    volumes: Option<Vec<(String, String)>>,
    collections: Option<Vec<String>>,
}

pub struct DropdownCache {
    inner: RwLock<DropdownCacheInner>,
}

impl DropdownCache {
    fn new() -> Self {
        Self {
            inner: RwLock::new(DropdownCacheInner {
                tags: None,
                formats: None,
                volumes: None,
                collections: None,
            }),
        }
    }

    pub fn get_tags(&self, catalog: &Catalog) -> Vec<(String, u64)> {
        if let Some(cached) = self.inner.read().unwrap().tags.as_ref() {
            return cached.clone();
        }
        let mut w = self.inner.write().unwrap();
        if let Some(cached) = w.tags.as_ref() {
            return cached.clone();
        }
        let tags = catalog.list_all_tags().unwrap_or_default();
        w.tags = Some(tags.clone());
        tags
    }

    pub fn get_formats(&self, catalog: &Catalog) -> Vec<String> {
        if let Some(cached) = self.inner.read().unwrap().formats.as_ref() {
            return cached.clone();
        }
        let mut w = self.inner.write().unwrap();
        if let Some(cached) = w.formats.as_ref() {
            return cached.clone();
        }
        let formats = catalog.list_all_formats().unwrap_or_default();
        w.formats = Some(formats.clone());
        formats
    }

    pub fn get_volumes(&self, catalog: &Catalog) -> Vec<(String, String)> {
        if let Some(cached) = self.inner.read().unwrap().volumes.as_ref() {
            return cached.clone();
        }
        let mut w = self.inner.write().unwrap();
        if let Some(cached) = w.volumes.as_ref() {
            return cached.clone();
        }
        let volumes = catalog.list_volumes().unwrap_or_default();
        w.volumes = Some(volumes.clone());
        volumes
    }

    pub fn get_collections(&self, catalog: &Catalog) -> Vec<String> {
        if let Some(cached) = self.inner.read().unwrap().collections.as_ref() {
            return cached.clone();
        }
        let mut w = self.inner.write().unwrap();
        if let Some(cached) = w.collections.as_ref() {
            return cached.clone();
        }
        let col_store = crate::collection::CollectionStore::new(catalog.conn());
        let collections: Vec<String> = col_store
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.name)
            .collect();
        w.collections = Some(collections.clone());
        collections
    }

    pub fn invalidate_tags(&self) {
        self.inner.write().unwrap().tags = None;
    }

    pub fn invalidate_collections(&self) {
        self.inner.write().unwrap().collections = None;
    }
}

/// Shared application state for the web server.
pub struct AppState {
    catalog_root: PathBuf,
    preview_config: PreviewConfig,
    pub preview_ext: String,
    pub log_requests: bool,
    pub dropdown_cache: DropdownCache,
    pub dedup_prefer: Option<String>,
}

impl AppState {
    pub fn new(catalog_root: PathBuf, preview_config: PreviewConfig, log_requests: bool, dedup_prefer: Option<String>) -> Self {
        let preview_ext = preview_config.format.extension().to_string();
        Self {
            catalog_root,
            preview_config,
            preview_ext,
            log_requests,
            dropdown_cache: DropdownCache::new(),
            dedup_prefer,
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

    /// Create an AssetService for dedup and other operations.
    pub fn asset_service(&self) -> AssetService {
        AssetService::new(&self.catalog_root, false, &self.preview_config)
    }
}

fn build_router(state: Arc<AppState>) -> Router {
    let preview_dir = state.catalog_root.join("previews");
    let smart_preview_dir = state.catalog_root.join("smart-previews");

    Router::new()
        .route("/", axum::routing::get(routes::browse_page))
        .route("/asset/{id}", axum::routing::get(routes::asset_page))
        .route("/compare", axum::routing::get(routes::compare_page))
        .route("/tags", axum::routing::get(routes::tags_page))
        .route("/stats", axum::routing::get(routes::stats_page))
        .route("/backup", axum::routing::get(routes::backup_page))
        .route("/api/search", axum::routing::get(routes::search_api))
        .route(
            "/api/asset/{id}/tags",
            axum::routing::post(routes::add_tags),
        )
        .route(
            "/api/asset/{id}/tags",
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
        .route(
            "/api/asset/{id}/smart-preview",
            axum::routing::post(routes::generate_smart_preview),
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
        .route(
            "/api/saved-searches/{name}/favorite",
            axum::routing::put(routes::toggle_saved_search_favorite),
        )
        .route(
            "/api/saved-searches/{name}/rename",
            axum::routing::put(routes::rename_saved_search),
        )
        .route(
            "/saved-searches",
            axum::routing::get(routes::saved_searches_page),
        )
        .route("/collections", axum::routing::get(routes::collections_page))
        .route(
            "/api/collections",
            axum::routing::get(routes::list_collections_api)
                .post(routes::create_collection_api),
        )
        .route(
            "/api/batch/collection",
            axum::routing::post(routes::batch_add_to_collection)
                .delete(routes::batch_remove_from_collection),
        )
        .route(
            "/api/batch/auto-group",
            axum::routing::post(routes::batch_auto_group),
        )
        .route(
            "/api/batch/stack",
            axum::routing::post(routes::batch_create_stack)
                .delete(routes::batch_unstack),
        )
        .route(
            "/api/asset/{id}/stack-pick",
            axum::routing::put(routes::set_stack_pick),
        )
        .route(
            "/api/asset/{id}/stack",
            axum::routing::delete(routes::dissolve_stack),
        )
        .route("/duplicates", axum::routing::get(routes::duplicates_page))
        .route("/api/dedup/resolve", axum::routing::post(routes::dedup_resolve_api))
        .route("/api/dedup/location", axum::routing::delete(routes::dedup_remove_location_api))
        .route("/api/calendar", axum::routing::get(routes::calendar_api))
        .route("/static/htmx.min.js", axum::routing::get(static_assets::htmx_js))
        .route("/static/style.css", axum::routing::get(static_assets::style_css))
        .nest_service("/preview", ServeDir::new(preview_dir))
        .nest_service("/smart-preview", ServeDir::new(smart_preview_dir))
        .layer(axum::middleware::from_fn_with_state(state.clone(), log_request))
        .with_state(state)
}

async fn log_request(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !state.log_requests {
        return next.run(req).await;
    }
    let method = req.method().clone();
    let uri = req.uri().clone();
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    eprintln!("{method} {uri} -> {} ({:.1?})", response.status(), start.elapsed());
    response
}

/// Start the web server.
pub async fn serve(catalog_root: PathBuf, bind: &str, port: u16, preview_config: PreviewConfig, log: bool, dedup_prefer: Option<String>) -> Result<()> {
    let state = Arc::new(AppState::new(catalog_root, preview_config, log, dedup_prefer));

    // Verify catalog is accessible and run schema migrations once at startup
    Catalog::open(&state.catalog_root)?;

    let app = build_router(state);

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    eprintln!("dam v{} web UI: http://{addr}", env!("CARGO_PKG_VERSION"));

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
