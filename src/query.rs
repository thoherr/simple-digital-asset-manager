use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

use crate::catalog::{AssetDetails, Catalog, SearchOptions, SearchRow};
use crate::content_store::ContentStore;
use crate::device_registry::DeviceRegistry;
use crate::metadata_store::MetadataStore;
use crate::models::Asset;
use crate::xmp_reader;

/// Parsed search query with all supported filter prefixes.
#[derive(Debug, Default)]
pub struct ParsedSearch {
    pub text: Option<String>,
    pub asset_type: Option<String>,
    pub tag: Option<String>,
    pub format: Option<String>,
    pub rating_min: Option<u8>,
    pub rating_exact: Option<u8>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso_min: Option<i64>,
    pub iso_max: Option<i64>,
    pub focal_min: Option<f64>,
    pub focal_max: Option<f64>,
    pub f_min: Option<f64>,
    pub f_max: Option<f64>,
    pub width_min: Option<i64>,
    pub height_min: Option<i64>,
    pub meta_filters: Vec<(String, String)>,
    pub orphan: bool,
    pub stale_days: Option<u64>,
    pub missing: bool,
    pub volume_none: bool,
    pub color_label: Option<String>,
    pub collection: Option<String>,
}

impl ParsedSearch {
    /// Convert to `SearchOptions` for passing to catalog search methods.
    pub fn to_search_options(&self) -> SearchOptions<'_> {
        SearchOptions {
            text: self.text.as_deref(),
            asset_type: self.asset_type.as_deref(),
            tag: self.tag.as_deref(),
            format: self.format.as_deref(),
            rating_min: self.rating_min,
            rating_exact: self.rating_exact,
            camera: self.camera.as_deref(),
            lens: self.lens.as_deref(),
            iso_min: self.iso_min,
            iso_max: self.iso_max,
            focal_min: self.focal_min,
            focal_max: self.focal_max,
            f_min: self.f_min,
            f_max: self.f_max,
            width_min: self.width_min,
            height_min: self.height_min,
            meta_filters: self
                .meta_filters
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            orphan: self.orphan,
            stale_days: self.stale_days,
            color_label: self.color_label.as_deref(),
            ..Default::default()
        }
    }
}

/// Parse a search query string into structured filters.
///
/// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`, `rating:3+`,
/// `camera:fuji`, `lens:56mm`, `iso:3200`, `iso:100-800`, `focal:50`, `focal:35-70`,
/// `f:2.8`, `f:1.4-2.8`, `width:4000+`, `height:2000+`, `meta:key=value`.
/// Remaining tokens are joined as free-text search.
pub fn parse_search_query(query: &str) -> ParsedSearch {
    let mut parsed = ParsedSearch::default();
    let mut text_parts = Vec::new();

    for token in query.split_whitespace() {
        if let Some(value) = token.strip_prefix("type:") {
            parsed.asset_type = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("tag:") {
            parsed.tag = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("format:") {
            parsed.format = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("rating:") {
            if let Some(num_str) = value.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<u8>() {
                    parsed.rating_min = Some(n);
                }
            } else if let Ok(n) = value.parse::<u8>() {
                parsed.rating_exact = Some(n);
            }
        } else if let Some(value) = token.strip_prefix("camera:") {
            parsed.camera = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("lens:") {
            parsed.lens = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("iso:") {
            parse_int_range(value, &mut parsed.iso_min, &mut parsed.iso_max);
        } else if let Some(value) = token.strip_prefix("focal:") {
            parse_float_range(value, &mut parsed.focal_min, &mut parsed.focal_max);
        } else if let Some(value) = token.strip_prefix("f:") {
            parse_float_range(value, &mut parsed.f_min, &mut parsed.f_max);
        } else if let Some(value) = token.strip_prefix("width:") {
            if let Some(num_str) = value.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<i64>() {
                    parsed.width_min = Some(n);
                }
            } else if let Ok(n) = value.parse::<i64>() {
                parsed.width_min = Some(n);
            }
        } else if let Some(value) = token.strip_prefix("height:") {
            if let Some(num_str) = value.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<i64>() {
                    parsed.height_min = Some(n);
                }
            } else if let Ok(n) = value.parse::<i64>() {
                parsed.height_min = Some(n);
            }
        } else if let Some(value) = token.strip_prefix("meta:") {
            if let Some((key, val)) = value.split_once('=') {
                parsed.meta_filters.push((key.to_string(), val.to_string()));
            }
        } else if token == "orphan:true" {
            parsed.orphan = true;
        } else if token == "missing:true" {
            parsed.missing = true;
        } else if let Some(value) = token.strip_prefix("stale:") {
            if let Ok(days) = value.parse::<u64>() {
                parsed.stale_days = Some(days);
            }
        } else if token == "volume:none" {
            parsed.volume_none = true;
        } else if let Some(value) = token.strip_prefix("label:") {
            parsed.color_label = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("collection:") {
            parsed.collection = Some(value.to_string());
        } else {
            text_parts.push(token);
        }
    }

    if !text_parts.is_empty() {
        parsed.text = Some(text_parts.join(" "));
    }

    parsed
}

/// Parse an integer range value: "3200" (exact), "3200+" (min), "100-800" (range).
fn parse_int_range(value: &str, min: &mut Option<i64>, max: &mut Option<i64>) {
    if let Some(num_str) = value.strip_suffix('+') {
        if let Ok(n) = num_str.parse::<i64>() {
            *min = Some(n);
        }
    } else if let Some((lo, hi)) = value.split_once('-') {
        if let (Ok(lo_n), Ok(hi_n)) = (lo.parse::<i64>(), hi.parse::<i64>()) {
            *min = Some(lo_n);
            *max = Some(hi_n);
        }
    } else if let Ok(n) = value.parse::<i64>() {
        *min = Some(n);
        *max = Some(n);
    }
}

/// Parse a float range value: "2.8" (exact), "2.8+" (min), "1.4-2.8" (range).
fn parse_float_range(value: &str, min: &mut Option<f64>, max: &mut Option<f64>) {
    if let Some(num_str) = value.strip_suffix('+') {
        if let Ok(n) = num_str.parse::<f64>() {
            *min = Some(n);
        }
    } else if let Some((lo, hi)) = value.split_once('-') {
        if let (Ok(lo_n), Ok(hi_n)) = (lo.parse::<f64>(), hi.parse::<f64>()) {
            *min = Some(lo_n);
            *max = Some(hi_n);
        }
    } else if let Ok(n) = value.parse::<f64>() {
        *min = Some(n);
        *max = Some(n);
    }
}

/// Result of a group operation.
#[derive(Debug)]
pub struct GroupResult {
    /// The asset ID that all variants were merged into.
    pub target_id: String,
    /// Number of variants moved from donor assets.
    pub variants_moved: usize,
    /// Number of donor assets removed.
    pub donors_removed: usize,
}

/// Fields to edit on an asset. `None` = no change, `Some(None)` = clear, `Some(Some(x))` = set.
pub struct EditFields {
    pub name: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub rating: Option<Option<u8>>,
    pub color_label: Option<Option<String>>,
}

/// Result of an edit operation.
#[derive(Debug, serde::Serialize)]
pub struct EditResult {
    pub asset_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
}

/// Result of a tag add/remove operation.
pub struct TagResult {
    /// Tags that were actually added or removed.
    pub changed: Vec<String>,
    /// The full set of tags after the operation.
    pub current_tags: Vec<String>,
}

/// Search and filter assets via the SQLite catalog.
pub struct QueryEngine {
    catalog_root: std::path::PathBuf,
}

impl QueryEngine {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
        }
    }

    /// Search assets by a free-text query string.
    ///
    /// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`, `rating:3+`,
    /// `camera:fuji`, `lens:56mm`, `iso:3200`, `focal:50`, `f:2.8`, `width:4000+`,
    /// `height:2000+`, `meta:key=value`.
    /// Remaining tokens are joined as free-text search against name/filename/description/metadata.
    pub fn search(&self, query: &str) -> Result<Vec<SearchRow>> {
        let parsed = parse_search_query(query);
        let mut opts = SearchOptions {
            per_page: u32::MAX,
            ..parsed.to_search_options()
        };
        let catalog = Catalog::open(&self.catalog_root)?;

        // Pre-compute missing asset IDs if needed (requires disk I/O)
        let missing_ids;
        if parsed.missing {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;
            let online: HashMap<String, std::path::PathBuf> = volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| (v.id.to_string(), v.mount_point.clone()))
                .collect();
            let all_locs = catalog.list_all_locations_with_assets()?;
            let mut ids = HashSet::new();
            for (asset_id, volume_id, relative_path) in &all_locs {
                if let Some(mount) = online.get(volume_id) {
                    if !mount.join(relative_path).exists() {
                        ids.insert(asset_id.clone());
                    }
                }
            }
            missing_ids = ids.into_iter().collect::<Vec<_>>();
            opts.missing_asset_ids = Some(&missing_ids);
        }

        // Pre-compute collection asset IDs
        let collection_ids;
        if let Some(ref col_name) = parsed.collection {
            let store = crate::collection::CollectionStore::new(catalog.conn());
            collection_ids = store.asset_ids_for_collection(col_name)?;
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Pre-compute online volume IDs for volume:none
        let online_vol_ids;
        if parsed.volume_none {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;
            online_vol_ids = volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| v.id.to_string())
                .collect::<Vec<_>>();
            opts.no_online_locations = Some(&online_vol_ids);
        }

        catalog.search_paginated(&opts)
    }

    /// Look up a single asset by its full ID or a unique prefix.
    pub fn show(&self, asset_id_prefix: &str) -> Result<AssetDetails> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;
        catalog
            .load_asset_details(&full_id)?
            .ok_or_else(|| anyhow::anyhow!("Asset '{full_id}' not found in catalog"))
    }

    /// Group variants (identified by content hashes) into a single asset.
    ///
    /// Picks the oldest asset as the target, moves all other variants into it,
    /// merges tags, and deletes donor assets.
    pub fn group(&self, variant_hashes: &[String]) -> Result<GroupResult> {
        if variant_hashes.is_empty() {
            anyhow::bail!("No variant hashes provided");
        }

        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);

        // Step 1: Look up owning asset for each hash
        let mut asset_ids = Vec::new();
        for hash in variant_hashes {
            let asset_id = catalog
                .find_asset_id_by_variant(hash)?
                .ok_or_else(|| anyhow::anyhow!("No variant found with hash '{hash}'"))?;
            asset_ids.push(asset_id);
        }

        // Step 2: Collect unique asset IDs
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            asset_ids
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        };

        if unique_ids.len() == 1 {
            return Ok(GroupResult {
                target_id: unique_ids.into_iter().next().unwrap(),
                variants_moved: 0,
                donors_removed: 0,
            });
        }

        // Step 3: Load all assets from sidecar, pick oldest as target
        let mut assets: Vec<crate::models::Asset> = unique_ids
            .iter()
            .map(|id| {
                let uuid: uuid::Uuid = id.parse()?;
                store.load(uuid)
            })
            .collect::<Result<_>>()?;

        assets.sort_by_key(|a| a.created_at);
        let target_id = assets[0].id;
        let mut target = assets.remove(0);
        let donors = assets; // remaining are donors

        // Step 4: Merge variants and tags from donors into target
        let mut variants_moved = 0;
        let existing_tags: HashSet<String> = target.tags.iter().cloned().collect();
        let mut all_tags = existing_tags;

        for donor in &donors {
            for variant in &donor.variants {
                let mut moved_variant = variant.clone();
                moved_variant.asset_id = target_id;
                target.variants.push(moved_variant);
                variants_moved += 1;
            }
            for tag in &donor.tags {
                if all_tags.insert(tag.clone()) {
                    target.tags.push(tag.clone());
                }
            }
        }

        // Step 5: Save target sidecar and update catalog
        store.save(&target)?;
        catalog.insert_asset(&target)?;

        // Step 6: Update variant rows in catalog and clean up donors
        for donor in &donors {
            for variant in &donor.variants {
                catalog.update_variant_asset_id(
                    &variant.content_hash,
                    &target_id.to_string(),
                )?;
            }
            store.delete(donor.id)?;
            catalog.delete_asset(&donor.id.to_string())?;
        }

        let donors_removed = donors.len();

        Ok(GroupResult {
            target_id: target_id.to_string(),
            variants_moved,
            donors_removed,
        })
    }

    /// Add or remove tags on an asset. Updates both sidecar YAML and SQLite catalog.
    pub fn tag(&self, asset_id_prefix: &str, tags: &[String], remove: bool) -> Result<TagResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        let changed;
        if remove {
            let to_remove: std::collections::HashSet<&str> =
                tags.iter().map(|s| s.as_str()).collect();
            let mut actually_removed = Vec::new();
            asset.tags.retain(|t| {
                if to_remove.contains(t.as_str()) {
                    actually_removed.push(t.clone());
                    false
                } else {
                    true
                }
            });
            changed = actually_removed;
        } else {
            let existing: std::collections::HashSet<String> =
                asset.tags.iter().cloned().collect();
            let mut added = Vec::new();
            for tag in tags {
                if !existing.contains(tag) {
                    asset.tags.push(tag.clone());
                    added.push(tag.clone());
                }
            }
            changed = added;
        }

        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        if !changed.is_empty() {
            let (to_add, to_remove) = if remove {
                (Vec::new(), changed.clone())
            } else {
                (changed.clone(), Vec::new())
            };
            self.write_back_tags_to_xmp(&mut asset, &to_add, &to_remove, &catalog, &store);
        }

        Ok(TagResult {
            changed,
            current_tags: asset.tags.clone(),
        })
    }

    /// Edit asset metadata (name, description, rating). Updates both sidecar YAML and SQLite.
    pub fn edit(&self, asset_id_prefix: &str, fields: EditFields) -> Result<EditResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        if let Some(name) = &fields.name {
            asset.name = name.clone();
        }
        if let Some(description) = &fields.description {
            // Normalize empty string to None (clear)
            asset.description = description
                .as_ref()
                .filter(|s| !s.is_empty())
                .cloned();
        }
        let rating_changed = fields.rating.is_some();
        if let Some(rating) = &fields.rating {
            asset.rating = *rating;
        }
        let label_changed = fields.color_label.is_some();
        if let Some(label) = &fields.color_label {
            asset.color_label = label.clone();
        }

        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        if rating_changed {
            let rating = asset.rating;
            self.write_back_rating_to_xmp(&mut asset, rating, &catalog, &store);
        }

        if fields.description.is_some() {
            let desc = asset.description.clone();
            self.write_back_description_to_xmp(&mut asset, desc.as_deref(), &catalog, &store);
        }

        if label_changed {
            let label = asset.color_label.clone();
            self.write_back_label_to_xmp(&mut asset, label.as_deref(), &catalog, &store);
        }

        Ok(EditResult {
            asset_id: full_id,
            name: asset.name,
            description: asset.description,
            rating: asset.rating,
            color_label: asset.color_label,
        })
    }

    /// Set the name on an asset. Updates both sidecar YAML and SQLite catalog.
    /// No XMP write-back needed — name has no XMP equivalent.
    /// Returns the new name value.
    pub fn set_name(
        &self,
        asset_id_prefix: &str,
        name: Option<String>,
    ) -> Result<Option<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.name = name;
        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        Ok(asset.name)
    }

    /// Set the rating on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the rating to any `.xmp` recipe files on disk.
    /// Returns the new rating value.
    pub fn set_rating(&self, asset_id_prefix: &str, rating: Option<u8>) -> Result<Option<u8>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.rating = rating;
        store.save(&asset)?;
        catalog.update_asset_rating(&full_id, rating)?;

        self.write_back_rating_to_xmp(&mut asset, rating, &catalog, &store);

        Ok(rating)
    }

    /// Write back a rating change to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, updates the `xmp:Rating` value,
    /// re-hashes the file, and updates the recipe's content hash in catalog and sidecar.
    /// Silently skips offline volumes and missing files.
    fn write_back_rating_to_xmp(
        &self,
        asset: &mut Asset,
        rating: Option<u8>,
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue, // volume offline
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_rating(&full_path, rating) {
                Ok(true) => {
                    // File was modified — re-hash and update catalog
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {} // no change needed
                Err(e) => {
                    eprintln!(
                        "Warning: could not write rating to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP write-back: {e}");
            }
        }
    }

    /// Write back tag add/remove operations to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, applies the same delta (add/remove)
    /// to the `dc:subject` keyword list, re-hashes, and updates the recipe's content
    /// hash in catalog and sidecar. Silently skips offline volumes and missing files.
    fn write_back_tags_to_xmp(
        &self,
        asset: &mut Asset,
        tags_to_add: &[String],
        tags_to_remove: &[String],
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP tag write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue,
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_tags(&full_path, tags_to_add, tags_to_remove) {
                Ok(true) => {
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!(
                        "Warning: could not write tags to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP tag write-back: {e}");
            }
        }
    }

    /// Set the color label on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the label to any `.xmp` recipe files on disk.
    /// Returns the new label value.
    pub fn set_color_label(&self, asset_id_prefix: &str, label: Option<String>) -> Result<Option<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.color_label = label.clone();
        store.save(&asset)?;
        catalog.update_asset_color_label(&full_id, label.as_deref())?;

        self.write_back_label_to_xmp(&mut asset, label.as_deref(), &catalog, &store);

        Ok(label)
    }

    /// Set the description on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the description to any `.xmp` recipe files on disk.
    /// Returns the new description value.
    pub fn set_description(
        &self,
        asset_id_prefix: &str,
        description: Option<String>,
    ) -> Result<Option<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.description = description.clone();
        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        self.write_back_description_to_xmp(&mut asset, description.as_deref(), &catalog, &store);

        Ok(asset.description)
    }

    /// Write back a description change to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, updates the `dc:description` value,
    /// re-hashes the file, and updates the recipe's content hash in catalog and sidecar.
    /// Silently skips offline volumes and missing files.
    fn write_back_description_to_xmp(
        &self,
        asset: &mut Asset,
        description: Option<&str>,
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP description write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue,
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_description(&full_path, description) {
                Ok(true) => {
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!(
                        "Warning: could not write description to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP description write-back: {e}");
            }
        }
    }

    /// Write back a color label change to `.xmp` recipe files on disk.
    ///
    /// For each XMP recipe on an online volume, updates the `xmp:Label` value,
    /// re-hashes the file, and updates the recipe's content hash in catalog and sidecar.
    /// Silently skips offline volumes and missing files.
    fn write_back_label_to_xmp(
        &self,
        asset: &mut Asset,
        label: Option<&str>,
        catalog: &Catalog,
        store: &MetadataStore,
    ) {
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP label write-back: {e}");
                return;
            }
        };

        let online: HashMap<uuid::Uuid, &std::path::Path> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.as_path()))
            .collect();

        let content_store = ContentStore::new(&self.catalog_root);
        let mut sidecar_dirty = false;

        for recipe in &mut asset.recipes {
            let ext = recipe
                .location
                .relative_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "xmp" {
                continue;
            }

            let mount_point = match online.get(&recipe.location.volume_id) {
                Some(mp) => *mp,
                None => continue,
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                continue;
            }

            match xmp_reader::update_label(&full_path, label) {
                Ok(true) => {
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            if let Err(e) = catalog.update_recipe_content_hash(
                                &recipe.id.to_string(),
                                &new_hash,
                            ) {
                                eprintln!(
                                    "Warning: could not update recipe hash in catalog: {e}"
                                );
                            }
                            recipe.content_hash = new_hash;
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    eprintln!(
                        "Warning: could not write label to {}: {e}",
                        full_path.display()
                    );
                }
            }
        }

        if sidecar_dirty {
            if let Err(e) = store.save(asset) {
                eprintln!("Warning: could not save sidecar after XMP label write-back: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::models::{Asset, AssetType};

    /// Set up a temp catalog with one asset and its sidecar, returning (dir, asset_id).
    fn setup_tag_env() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();

        // Init catalog
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();

        // Create and persist an asset
        let mut asset = Asset::new(AssetType::Image, "sha256:tag_env");
        asset.tags = vec!["existing".to_string()];
        catalog.insert_asset(&asset).unwrap();

        let store = MetadataStore::new(catalog_root);
        store.save(&asset).unwrap();

        (dir, asset.id.to_string())
    }

    use crate::models::{Variant, VariantRole};

    /// Set up a temp catalog with two assets, each with one variant, for group tests.
    /// Returns (dir, hash1, hash2, asset_id1, asset_id2).
    fn setup_group_env() -> (tempfile::TempDir, String, String, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();

        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        // Create first asset (older)
        let mut asset1 = Asset::new(AssetType::Image, "sha256:hash1");
        asset1.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
        asset1.tags = vec!["landscape".to_string()];
        let variant1 = Variant {
            content_hash: "sha256:hash1".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "arw".to_string(),
            file_size: 25_000_000,
            original_filename: "DSC_001.ARW".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(variant1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&variant1).unwrap();
        store.save(&asset1).unwrap();

        // Create second asset (newer)
        let mut asset2 = Asset::new(AssetType::Image, "sha256:hash2");
        asset2.tags = vec!["nature".to_string()];
        let variant2 = Variant {
            content_hash: "sha256:hash2".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5_000_000,
            original_filename: "DSC_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(variant2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&variant2).unwrap();
        store.save(&asset2).unwrap();

        let id1 = asset1.id.to_string();
        let id2 = asset2.id.to_string();
        (dir, "sha256:hash1".to_string(), "sha256:hash2".to_string(), id1, id2)
    }

    #[test]
    fn group_two_variants_from_two_assets() {
        let (dir, hash1, hash2, id1, id2) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&[hash1, hash2]).unwrap();

        // Target should be the older asset (asset1)
        assert_eq!(result.target_id, id1);
        assert_eq!(result.variants_moved, 1);
        assert_eq!(result.donors_removed, 1);

        // Target should now have both variants
        let details = engine.show(&id1).unwrap();
        assert_eq!(details.variants.len(), 2);

        // Donor should be gone
        assert!(engine.show(&id2).is_err());
    }

    #[test]
    fn group_already_same_asset_is_noop() {
        let (dir, hash1, _, id1, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&[hash1.clone(), hash1]).unwrap();

        assert_eq!(result.target_id, id1);
        assert_eq!(result.variants_moved, 0);
        assert_eq!(result.donors_removed, 0);
    }

    #[test]
    fn group_nonexistent_hash_errors() {
        let (dir, _, _, _, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.group(&["sha256:bogus".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No variant found"));
    }

    #[test]
    fn group_merges_tags() {
        let (dir, hash1, hash2, id1, _) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        engine.group(&[hash1, hash2]).unwrap();

        let details = engine.show(&id1).unwrap();
        assert!(details.tags.contains(&"landscape".to_string()));
        assert!(details.tags.contains(&"nature".to_string()));
    }

    #[test]
    fn tag_add_new() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .tag(&id, &["landscape".to_string(), "nature".to_string()], false)
            .unwrap();

        assert_eq!(result.changed, vec!["landscape", "nature"]);
        assert_eq!(result.current_tags, vec!["existing", "landscape", "nature"]);
    }

    #[test]
    fn tag_add_duplicate_is_noop() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["existing".to_string()], false).unwrap();

        assert!(result.changed.is_empty());
        assert_eq!(result.current_tags, vec!["existing"]);
    }

    #[test]
    fn tag_remove_existing() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["existing".to_string()], true).unwrap();

        assert_eq!(result.changed, vec!["existing"]);
        assert!(result.current_tags.is_empty());
    }

    #[test]
    fn tag_remove_nonexistent_is_noop() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine.tag(&id, &["nope".to_string()], true).unwrap();

        assert!(result.changed.is_empty());
        assert_eq!(result.current_tags, vec!["existing"]);
    }

    #[test]
    fn tag_persists_to_sidecar_and_catalog() {
        let (dir, id) = setup_tag_env();
        let engine = QueryEngine::new(dir.path());

        engine.tag(&id, &["new_tag".to_string()], false).unwrap();

        // Verify sidecar
        let uuid: uuid::Uuid = id.parse().unwrap();
        let store = MetadataStore::new(dir.path());
        let asset = store.load(uuid).unwrap();
        assert!(asset.tags.contains(&"new_tag".to_string()));

        // Verify catalog
        let details = engine.show(&id).unwrap();
        assert!(details.tags.contains(&"new_tag".to_string()));
    }

    // ── parse_search_query tests ──────────────────────────────────

    #[test]
    fn parse_camera_filter() {
        let p = parse_search_query("camera:fuji");
        assert_eq!(p.camera.as_deref(), Some("fuji"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_lens_filter() {
        let p = parse_search_query("lens:56mm");
        assert_eq!(p.lens.as_deref(), Some("56mm"));
    }

    #[test]
    fn parse_iso_exact() {
        let p = parse_search_query("iso:3200");
        assert_eq!(p.iso_min, Some(3200));
        assert_eq!(p.iso_max, Some(3200));
    }

    #[test]
    fn parse_iso_min() {
        let p = parse_search_query("iso:3200+");
        assert_eq!(p.iso_min, Some(3200));
        assert!(p.iso_max.is_none());
    }

    #[test]
    fn parse_iso_range() {
        let p = parse_search_query("iso:100-800");
        assert_eq!(p.iso_min, Some(100));
        assert_eq!(p.iso_max, Some(800));
    }

    #[test]
    fn parse_focal_exact() {
        let p = parse_search_query("focal:50");
        assert!((p.focal_min.unwrap() - 50.0).abs() < 0.01);
        assert!((p.focal_max.unwrap() - 50.0).abs() < 0.01);
    }

    #[test]
    fn parse_focal_range() {
        let p = parse_search_query("focal:35-70");
        assert!((p.focal_min.unwrap() - 35.0).abs() < 0.01);
        assert!((p.focal_max.unwrap() - 70.0).abs() < 0.01);
    }

    #[test]
    fn parse_f_exact() {
        let p = parse_search_query("f:2.8");
        assert!((p.f_min.unwrap() - 2.8).abs() < 0.01);
        assert!((p.f_max.unwrap() - 2.8).abs() < 0.01);
    }

    #[test]
    fn parse_f_min() {
        let p = parse_search_query("f:2.8+");
        assert!((p.f_min.unwrap() - 2.8).abs() < 0.01);
        assert!(p.f_max.is_none());
    }

    #[test]
    fn parse_f_range() {
        let p = parse_search_query("f:1.4-2.8");
        assert!((p.f_min.unwrap() - 1.4).abs() < 0.01);
        assert!((p.f_max.unwrap() - 2.8).abs() < 0.01);
    }

    #[test]
    fn parse_width_min() {
        let p = parse_search_query("width:4000+");
        assert_eq!(p.width_min, Some(4000));
    }

    #[test]
    fn parse_height_min() {
        let p = parse_search_query("height:2000+");
        assert_eq!(p.height_min, Some(2000));
    }

    #[test]
    fn parse_meta_filter() {
        let p = parse_search_query("meta:label=Red");
        assert_eq!(p.meta_filters.len(), 1);
        assert_eq!(p.meta_filters[0].0, "label");
        assert_eq!(p.meta_filters[0].1, "Red");
    }

    #[test]
    fn parse_mixed_filters_with_text() {
        let p = parse_search_query("camera:fuji sunset iso:400 landscape");
        assert_eq!(p.camera.as_deref(), Some("fuji"));
        assert_eq!(p.iso_min, Some(400));
        assert_eq!(p.iso_max, Some(400));
        assert_eq!(p.text.as_deref(), Some("sunset landscape"));
    }

    #[test]
    fn parse_existing_filters_still_work() {
        let p = parse_search_query("type:image tag:nature format:jpg rating:3+");
        assert_eq!(p.asset_type.as_deref(), Some("image"));
        assert_eq!(p.tag.as_deref(), Some("nature"));
        assert_eq!(p.format.as_deref(), Some("jpg"));
        assert_eq!(p.rating_min, Some(3));
        assert!(p.rating_exact.is_none());
    }

    #[test]
    fn parse_orphan_filter() {
        let p = parse_search_query("orphan:true");
        assert!(p.orphan);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_missing_filter() {
        let p = parse_search_query("missing:true");
        assert!(p.missing);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_stale_filter() {
        let p = parse_search_query("stale:30");
        assert_eq!(p.stale_days, Some(30));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_stale_filter_zero() {
        let p = parse_search_query("stale:0");
        assert_eq!(p.stale_days, Some(0));
    }

    #[test]
    fn parse_volume_none_filter() {
        let p = parse_search_query("volume:none");
        assert!(p.volume_none);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_location_health_combined() {
        let p = parse_search_query("orphan:true stale:7 tag:landscape");
        assert!(p.orphan);
        assert_eq!(p.stale_days, Some(7));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
        assert!(!p.missing);
        assert!(!p.volume_none);
    }

    #[test]
    fn parse_label_filter() {
        let p = parse_search_query("label:Red");
        assert_eq!(p.color_label.as_deref(), Some("Red"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_label_with_other_filters() {
        let p = parse_search_query("label:Blue tag:landscape sunset");
        assert_eq!(p.color_label.as_deref(), Some("Blue"));
        assert_eq!(p.tag.as_deref(), Some("landscape"));
        assert_eq!(p.text.as_deref(), Some("sunset"));
    }
}
