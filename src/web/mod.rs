mod routes;
mod static_assets;
pub mod templates;

use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use axum::Router;
use tower_http::services::ServeDir;

use crate::asset_service::AssetService;
use crate::catalog::Catalog;
use crate::config::PreviewConfig;
use crate::preview::PreviewGenerator;
use crate::query::QueryEngine;

/// A simple connection pool for SQLite. Pre-opens connections with pragmas set
/// so per-request overhead is near zero. Connections are returned to the pool on drop.
pub struct CatalogPool {
    catalog_root: PathBuf,
    pool: Mutex<Vec<Catalog>>,
    capacity: usize,
}

impl CatalogPool {
    /// Create a pool and pre-open `capacity` connections.
    pub fn new(catalog_root: &std::path::Path, capacity: usize) -> Result<Self> {
        let mut conns = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            conns.push(Catalog::open_fast(catalog_root)?);
        }
        Ok(Self {
            catalog_root: catalog_root.to_path_buf(),
            pool: Mutex::new(conns),
            capacity,
        })
    }

    /// Take a connection from the pool (or open a fresh one if pool is empty).
    pub fn get(self: &Arc<Self>) -> Result<PooledCatalog> {
        let conn = {
            let mut pool = self.pool.lock().unwrap();
            pool.pop()
        };
        let catalog = match conn {
            Some(c) => c,
            None => Catalog::open_fast(&self.catalog_root)?,
        };
        Ok(PooledCatalog { pool: Arc::clone(self), catalog: Some(catalog) })
    }

    fn return_conn(&self, catalog: Catalog) {
        let mut pool = self.pool.lock().unwrap();
        if pool.len() < self.capacity {
            pool.push(catalog);
        }
        // else: drop the connection (pool is full)
    }
}

/// RAII wrapper that returns the connection to the pool on drop.
pub struct PooledCatalog {
    pool: Arc<CatalogPool>,
    catalog: Option<Catalog>,
}

impl Deref for PooledCatalog {
    type Target = Catalog;
    fn deref(&self) -> &Catalog {
        self.catalog.as_ref().unwrap()
    }
}

impl DerefMut for PooledCatalog {
    fn deref_mut(&mut self) -> &mut Catalog {
        self.catalog.as_mut().unwrap()
    }
}

impl Drop for PooledCatalog {
    fn drop(&mut self) {
        if let Some(catalog) = self.catalog.take() {
            self.pool.return_conn(catalog);
        }
    }
}

#[cfg(feature = "ai")]
use crate::config::AiConfig;

/// Cached dropdown data for the browse page filter controls.
/// Populated lazily on first access, invalidated by write endpoints.
struct DropdownCacheInner {
    tags: Option<Vec<(String, u64)>>,
    formats: Option<Vec<(String, u64)>>,
    volumes: Option<Vec<(String, String)>>,
    collections: Option<Vec<String>>,
    #[cfg(feature = "ai")]
    people: Option<Vec<(String, String)>>,
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
                #[cfg(feature = "ai")]
                people: None,
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

    pub fn get_formats(&self, catalog: &Catalog) -> Vec<(String, u64)> {
        if let Some(cached) = self.inner.read().unwrap().formats.as_ref() {
            return cached.clone();
        }
        let mut w = self.inner.write().unwrap();
        if let Some(cached) = w.formats.as_ref() {
            return cached.clone();
        }
        let formats = catalog.list_all_format_counts().unwrap_or_default();
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

    #[cfg(feature = "ai")]
    pub fn get_people(&self, catalog: &Catalog) -> Vec<(String, String)> {
        if let Some(cached) = self.inner.read().unwrap().people.as_ref() {
            return cached.clone();
        }
        let mut w = self.inner.write().unwrap();
        if let Some(cached) = w.people.as_ref() {
            return cached.clone();
        }
        let face_store = crate::face_store::FaceStore::new(catalog.conn());
        let people: Vec<(String, String)> = face_store
            .list_people()
            .unwrap_or_default()
            .into_iter()
            .map(|(p, _count)| {
                let display = p.name.unwrap_or_else(|| format!("Unknown ({})", &p.id[..8.min(p.id.len())]));
                (p.id, display)
            })
            .collect();
        w.people = Some(people.clone());
        people
    }

    pub fn invalidate_tags(&self) {
        self.inner.write().unwrap().tags = None;
    }

    pub fn invalidate_collections(&self) {
        self.inner.write().unwrap().collections = None;
    }

    #[cfg(feature = "ai")]
    pub fn invalidate_people(&self) {
        self.inner.write().unwrap().people = None;
    }
}

/// Shared application state for the web server.
pub struct AppState {
    catalog_root: PathBuf,
    catalog_pool: Arc<CatalogPool>,
    preview_config: PreviewConfig,
    pub preview_ext: String,
    pub log_requests: bool,
    pub dropdown_cache: DropdownCache,
    pub dedup_prefer: Option<String>,
    pub smart_on_demand: bool,
    pub per_page: u32,
    pub stroll_neighbors: u32,
    pub stroll_neighbors_max: u32,
    pub stroll_fanout: u32,
    pub stroll_fanout_max: u32,
    pub stroll_discover_pool: u32,
    pub ai_enabled: bool,
    pub vlm_enabled: bool,
    pub verbosity: crate::Verbosity,
    pub default_filter: Option<String>,
    pub vlm_config: crate::config::VlmConfig,
    #[cfg(feature = "ai")]
    pub ai_model: tokio::sync::Mutex<Option<crate::ai::SigLipModel>>,
    #[cfg(feature = "ai")]
    pub ai_label_cache: tokio::sync::RwLock<Option<(Vec<String>, Vec<Vec<f32>>)>>,
    #[cfg(feature = "ai")]
    pub ai_config: AiConfig,
    #[cfg(feature = "ai")]
    pub ai_embedding_index: std::sync::RwLock<Option<crate::embedding_store::EmbeddingIndex>>,
    #[cfg(feature = "ai")]
    pub face_detector: tokio::sync::Mutex<Option<crate::face::FaceDetector>>,
}

impl AppState {
    #[cfg(feature = "ai")]
    pub fn new(catalog_root: PathBuf, preview_config: PreviewConfig, log_requests: bool, dedup_prefer: Option<String>, per_page: u32, stroll_neighbors: u32, stroll_neighbors_max: u32, stroll_fanout: u32, stroll_fanout_max: u32, stroll_discover_pool: u32, ai_config: AiConfig, vlm_config: crate::config::VlmConfig, default_filter: Option<String>, verbosity: crate::Verbosity) -> Self {
        let preview_ext = preview_config.format.extension().to_string();
        let smart_on_demand = preview_config.generate_on_demand;
        let vlm_enabled = check_vlm_at_startup(&vlm_config);
        let catalog_pool = Arc::new(CatalogPool::new(&catalog_root, 4).expect("Failed to open catalog pool"));
        Self {
            catalog_root,
            catalog_pool,
            preview_config,
            preview_ext,
            log_requests,
            dropdown_cache: DropdownCache::new(),
            dedup_prefer,
            smart_on_demand,
            per_page,
            stroll_neighbors,
            stroll_neighbors_max,
            stroll_fanout,
            stroll_fanout_max,
            stroll_discover_pool,
            ai_enabled: true,
            default_filter,
            verbosity,
            vlm_enabled,
            vlm_config,
            ai_model: tokio::sync::Mutex::new(None),
            ai_label_cache: tokio::sync::RwLock::new(None),
            ai_config,
            ai_embedding_index: std::sync::RwLock::new(None),
            face_detector: tokio::sync::Mutex::new(None),
        }
    }

    #[cfg(not(feature = "ai"))]
    pub fn new(catalog_root: PathBuf, preview_config: PreviewConfig, log_requests: bool, dedup_prefer: Option<String>, per_page: u32, stroll_neighbors: u32, stroll_neighbors_max: u32, stroll_fanout: u32, stroll_fanout_max: u32, stroll_discover_pool: u32, vlm_config: crate::config::VlmConfig, default_filter: Option<String>, verbosity: crate::Verbosity) -> Self {
        let preview_ext = preview_config.format.extension().to_string();
        let smart_on_demand = preview_config.generate_on_demand;
        let vlm_enabled = check_vlm_at_startup(&vlm_config);
        let catalog_pool = Arc::new(CatalogPool::new(&catalog_root, 4).expect("Failed to open catalog pool"));
        Self {
            catalog_root,
            catalog_pool,
            preview_config,
            preview_ext,
            log_requests,
            dropdown_cache: DropdownCache::new(),
            dedup_prefer,
            smart_on_demand,
            per_page,
            stroll_neighbors,
            stroll_neighbors_max,
            stroll_fanout,
            stroll_fanout_max,
            stroll_discover_pool,
            ai_enabled: false,
            default_filter,
            verbosity,
            vlm_enabled,
            vlm_config,
        }
    }

    /// Get a catalog connection from the pool (returned on drop).
    pub fn catalog(&self) -> Result<PooledCatalog> {
        self.catalog_pool.get()
    }

    /// Create a QueryEngine for this catalog.
    pub fn query_engine(&self) -> QueryEngine {
        QueryEngine::new(&self.catalog_root)
    }

    /// Create a PreviewGenerator for checking preview existence.
    pub fn preview_generator(&self) -> PreviewGenerator {
        PreviewGenerator::new(&self.catalog_root, crate::Verbosity::quiet(), &self.preview_config)
    }

    /// Create an AssetService for dedup and other operations.
    pub fn asset_service(&self) -> AssetService {
        AssetService::new(&self.catalog_root, crate::Verbosity::quiet(), &self.preview_config)
    }
}

fn build_router(state: Arc<AppState>) -> Router {
    let preview_dir = state.catalog_root.join("previews");

    #[allow(unused_mut)]
    let mut router = Router::new()
        .route("/", axum::routing::get(routes::browse_page))
        .route("/asset/{id}", axum::routing::get(routes::asset_page))
        .route("/compare", axum::routing::get(routes::compare_page))
        .route("/tags", axum::routing::get(routes::tags_page))
        .route("/stats", axum::routing::get(routes::stats_page))
        .route("/analytics", axum::routing::get(routes::analytics_page))
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
            "/api/asset/{id}/tags/clear",
            axum::routing::post(routes::clear_tags),
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
            "/api/asset/{id}/date",
            axum::routing::put(routes::set_date),
        )
        .route(
            "/api/asset/{id}/preview",
            axum::routing::post(routes::generate_preview),
        )
        .route(
            "/api/asset/{id}/rotate",
            axum::routing::post(routes::set_rotation),
        )
        .route(
            "/api/asset/{id}/preview-variant",
            axum::routing::post(routes::set_preview_variant),
        )
        .route(
            "/api/asset/{id}/variant-role",
            axum::routing::post(routes::set_variant_role),
        )
        .route(
            "/api/asset/{id}/reimport-metadata",
            axum::routing::post(routes::reimport_metadata),
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
            "/api/batch/group",
            axum::routing::post(routes::batch_group),
        )
        .route(
            "/api/asset/{id}/split",
            axum::routing::post(routes::split_asset),
        )
        .route(
            "/api/batch/stack",
            axum::routing::post(routes::batch_create_stack)
                .delete(routes::batch_unstack),
        )
        .route(
            "/api/batch/delete",
            axum::routing::post(routes::batch_delete),
        )
        .route(
            "/api/asset/{id}/stack-pick",
            axum::routing::put(routes::set_stack_pick),
        )
        .route(
            "/api/asset/{id}/stack",
            axum::routing::delete(routes::dissolve_stack),
        )
        .route(
            "/api/stack/{id}/members",
            axum::routing::get(routes::stack_members_api),
        )
        .route("/duplicates", axum::routing::get(routes::duplicates_page))
        .route("/api/dedup/resolve", axum::routing::post(routes::dedup_resolve_api))
        .route("/api/dedup/location", axum::routing::delete(routes::dedup_remove_location_api))
        .route("/api/calendar", axum::routing::get(routes::calendar_api))
        .route("/api/map", axum::routing::get(routes::map_api))
        .route("/api/facets", axum::routing::get(routes::facets_api))
        .route("/api/page-ids", axum::routing::get(routes::page_ids_api))
        .route("/api/open-location", axum::routing::post(routes::open_location))
        .route("/api/open-terminal", axum::routing::post(routes::open_terminal))
        .route(
            "/api/asset/{id}/writeback",
            axum::routing::post(routes::writeback_asset),
        )
        .route(
            "/api/asset/{id}/vlm-describe",
            axum::routing::post(routes::vlm_describe_asset),
        )
        .route(
            "/api/batch/describe",
            axum::routing::post(routes::batch_vlm_describe),
        )
        .route(
            "/api/batch/export",
            axum::routing::post(routes::export_zip),
        );

    #[cfg(feature = "ai")]
    {
        let faces_dir = state.catalog_root.join("faces");
        router = router
            .route(
                "/api/asset/{id}/suggest-tags",
                axum::routing::post(routes::suggest_tags),
            )
            .route(
                "/api/asset/{id}/similar",
                axum::routing::post(routes::find_similar),
            )
            .route(
                "/api/batch/auto-tag",
                axum::routing::post(routes::batch_auto_tag),
            )
            .route(
                "/api/asset/{id}/faces",
                axum::routing::get(routes::asset_faces),
            )
            .route(
                "/api/asset/{id}/detect-faces",
                axum::routing::post(routes::detect_faces_for_asset),
            )
            .route(
                "/api/batch/detect-faces",
                axum::routing::post(routes::batch_detect_faces),
            )
            .route(
                "/api/faces/{face_id}/assign",
                axum::routing::put(routes::assign_face),
            )
            .route(
                "/api/faces/{face_id}/unassign",
                axum::routing::delete(routes::unassign_face_api),
            )
            .route(
                "/api/faces/{face_id}",
                axum::routing::delete(routes::delete_face_api),
            )
            .route("/people", axum::routing::get(routes::people_page))
            .route("/api/people", axum::routing::get(routes::list_people_api).post(routes::create_person_api))
            .route(
                "/api/people/{id}/name",
                axum::routing::put(routes::name_person_api),
            )
            .route(
                "/api/people/{id}/merge",
                axum::routing::post(routes::merge_person_api),
            )
            .route(
                "/api/people/{id}",
                axum::routing::delete(routes::delete_person_api),
            )
            .route(
                "/api/faces/cluster",
                axum::routing::post(routes::cluster_faces_api),
            )
            .route("/stroll", axum::routing::get(routes::stroll_page))
            .route("/api/stroll/neighbors", axum::routing::get(routes::stroll_neighbors_api))
            .nest_service("/face", ServeDir::new(faces_dir));
    }

    router
        .route("/favicon.ico", axum::routing::get(static_assets::favicon))
        .route("/static/favicon.ico", axum::routing::get(static_assets::favicon))
        .route("/static/maki-icon.svg", axum::routing::get(static_assets::maki_icon))
        .route("/static/htmx.min.js", axum::routing::get(static_assets::htmx_js))
        .route("/static/style.css", axum::routing::get(static_assets::style_css))
        .route("/static/leaflet.min.js", axum::routing::get(static_assets::leaflet_js))
        .route("/static/leaflet.css", axum::routing::get(static_assets::leaflet_css))
        .route("/static/leaflet.markercluster.min.js", axum::routing::get(static_assets::markercluster_js))
        .route("/static/MarkerCluster.css", axum::routing::get(static_assets::markercluster_css))
        .route("/static/MarkerCluster.Default.css", axum::routing::get(static_assets::markercluster_default_css))
        .route("/static/images/marker-icon.png", axum::routing::get(static_assets::marker_icon))
        .route("/static/images/marker-icon-2x.png", axum::routing::get(static_assets::marker_icon_2x))
        .route("/static/images/marker-shadow.png", axum::routing::get(static_assets::marker_shadow))
        .route("/static/images/layers.png", axum::routing::get(static_assets::layers_png))
        .route("/static/images/layers-2x.png", axum::routing::get(static_assets::layers_2x))
        .nest_service("/preview", ServeDir::new(preview_dir))
        .route("/smart-preview/{prefix}/{file}", axum::routing::get(routes::serve_smart_preview))
        .layer(axum::middleware::from_fn_with_state(state.clone(), log_request))
        .with_state(state)
}

async fn log_request(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_preview = req.uri().path().starts_with("/preview/")
        || req.uri().path().starts_with("/smart-preview/")
        || req.uri().path().starts_with("/face/");
    let log = state.log_requests;
    let method = if log { Some(req.method().clone()) } else { None };
    let uri = if log { Some(req.uri().clone()) } else { None };
    let start = if log { Some(std::time::Instant::now()) } else { None };

    let mut response = next.run(req).await;

    // Previews can change (rotation/regeneration) — tell browsers to always revalidate.
    // ServeDir sets Last-Modified, so unchanged files get fast 304 Not Modified.
    if is_preview {
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
    }

    if let (Some(method), Some(uri), Some(start)) = (method, uri, start) {
        eprintln!("{method} {uri} -> {} ({:.1?})", response.status(), start.elapsed());
    }
    response
}

/// Quick non-blocking check if the VLM endpoint is reachable at startup.
/// Validates that the configured model is available on the server.
fn check_vlm_at_startup(vlm_config: &crate::config::VlmConfig) -> bool {
    match crate::vlm::check_endpoint_status(&vlm_config.endpoint, 5, crate::Verbosity::quiet()) {
        Ok(status) => {
            eprintln!("VLM: {}", status.message);
            if !status.available_models.is_empty() {
                let configured = &vlm_config.model;
                match crate::vlm::find_matching_model(configured, &status.available_models) {
                    Some(matched) if matched == *configured => {
                        eprintln!("VLM: using model {configured}");
                    }
                    Some(matched) => {
                        eprintln!("VLM: using model {matched} (matched from \"{configured}\")");
                    }
                    None => {
                        eprintln!(
                            "VLM: warning: configured model \"{configured}\" not found on server"
                        );
                        eprintln!(
                            "VLM: available models: {}",
                            status.available_models.join(", ")
                        );
                        eprintln!(
                            "VLM: pull it with `ollama pull {configured}` or set [vlm] model in maki.toml"
                        );
                    }
                }
            }
            true
        }
        Err(_) => {
            eprintln!("VLM: not available at {} (describe buttons hidden)", vlm_config.endpoint);
            false
        }
    }
}

/// Start the web server.
#[cfg(feature = "ai")]
pub async fn serve(catalog_root: PathBuf, bind: &str, port: u16, preview_config: PreviewConfig, log: bool, dedup_prefer: Option<String>, per_page: u32, stroll_neighbors: u32, stroll_neighbors_max: u32, stroll_fanout: u32, stroll_fanout_max: u32, stroll_discover_pool: u32, ai_config: AiConfig, vlm_config: crate::config::VlmConfig, default_filter: Option<String>, verbosity: crate::Verbosity) -> Result<()> {
    let state = Arc::new(AppState::new(catalog_root, preview_config, log, dedup_prefer, per_page, stroll_neighbors, stroll_neighbors_max, stroll_fanout, stroll_fanout_max, stroll_discover_pool, ai_config, vlm_config, default_filter, verbosity));

    // Verify catalog is accessible and warm dropdown caches
    {
        let catalog = Catalog::open(&state.catalog_root)?;
        state.dropdown_cache.get_tags(&catalog);
        state.dropdown_cache.get_formats(&catalog);
        state.dropdown_cache.get_volumes(&catalog);
        state.dropdown_cache.get_collections(&catalog);
        #[cfg(feature = "ai")]
        state.dropdown_cache.get_people(&catalog);
    }

    let app = build_router(state);

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    eprintln!("MAKI {} web UI: http://{addr}", env!("CARGO_PKG_VERSION"));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Start the web server.
#[cfg(not(feature = "ai"))]
pub async fn serve(catalog_root: PathBuf, bind: &str, port: u16, preview_config: PreviewConfig, log: bool, dedup_prefer: Option<String>, per_page: u32, stroll_neighbors: u32, stroll_neighbors_max: u32, stroll_fanout: u32, stroll_fanout_max: u32, stroll_discover_pool: u32, vlm_config: crate::config::VlmConfig, default_filter: Option<String>, verbosity: crate::Verbosity) -> Result<()> {
    let state = Arc::new(AppState::new(catalog_root, preview_config, log, dedup_prefer, per_page, stroll_neighbors, stroll_neighbors_max, stroll_fanout, stroll_fanout_max, stroll_discover_pool, vlm_config, default_filter, verbosity));

    // Verify catalog is accessible and warm dropdown caches
    {
        let catalog = Catalog::open(&state.catalog_root)?;
        state.dropdown_cache.get_tags(&catalog);
        state.dropdown_cache.get_formats(&catalog);
        state.dropdown_cache.get_volumes(&catalog);
        state.dropdown_cache.get_collections(&catalog);
    }

    let app = build_router(state);

    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    eprintln!("MAKI {} web UI: http://{addr}", env!("CARGO_PKG_VERSION"));

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
