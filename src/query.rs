use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use crate::catalog::{AssetDetails, Catalog, SearchOptions, SearchRow};
use crate::content_store::ContentStore;
use crate::device_registry::DeviceRegistry;
use crate::metadata_store::MetadataStore;
use crate::models::volume::Volume;
use crate::models::recipe::Recipe;
use crate::models::Asset;
use crate::xmp_reader;

/// Parse a flexible date input string into a `DateTime<Utc>`.
///
/// Supported formats:
/// - `YYYY` → Jan 1 of that year, midnight UTC
/// - `YYYY-MM` → 1st of that month, midnight UTC
/// - `YYYY-MM-DD` → midnight UTC on that date
/// - Full ISO 8601 / RFC 3339 (e.g. `2024-06-15T12:30:00Z`) — parsed as-is
///
/// # Examples
///
/// ```
/// use maki::query::parse_date_input;
///
/// let dt = parse_date_input("2026").unwrap();
/// assert_eq!(dt.to_rfc3339(), "2026-01-01T00:00:00+00:00");
///
/// let dt = parse_date_input("2026-03").unwrap();
/// assert_eq!(dt.to_rfc3339(), "2026-03-01T00:00:00+00:00");
///
/// let dt = parse_date_input("2026-03-15").unwrap();
/// assert_eq!(dt.to_rfc3339(), "2026-03-15T00:00:00+00:00");
///
/// assert!(parse_date_input("not-a-date").is_err());
/// ```
pub fn parse_date_input(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim();

    // Try RFC 3339 / ISO 8601 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // YYYY-MM-DD
    if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
    }

    // YYYY-MM
    if let Some((y, m)) = s.split_once('-') {
        if let (Ok(year), Ok(month)) = (y.parse::<i32>(), m.parse::<u32>()) {
            if let Some(nd) = NaiveDate::from_ymd_opt(year, month, 1) {
                return Ok(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
            }
        }
    }

    // YYYY
    if let Ok(year) = s.parse::<i32>() {
        if let Some(nd) = NaiveDate::from_ymd_opt(year, 1, 1) {
            return Ok(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
        }
    }

    anyhow::bail!("Invalid date format: '{s}'. Use YYYY, YYYY-MM, YYYY-MM-DD, or ISO 8601.")
}

/// Parsed search query with all supported filter prefixes.
///
/// Multi-value fields (Vecs) support:
/// - **Repeated filters** = AND: `tag:landscape tag:sunset` (must have both)
/// - **Comma within a value** = OR: `tag:alice,bob` (either tag matches)
/// - **`-` prefix** = negation: `-tag:rejected` excludes matching assets
#[derive(Debug, Default)]
pub struct ParsedSearch {
    pub text: Option<String>,
    pub text_exclude: Vec<String>,
    pub asset_types: Vec<String>,
    pub asset_types_exclude: Vec<String>,
    pub tags: Vec<String>,
    pub tags_exclude: Vec<String>,
    pub formats: Vec<String>,
    pub formats_exclude: Vec<String>,
    pub color_labels: Vec<String>,
    pub color_labels_exclude: Vec<String>,
    pub cameras: Vec<String>,
    pub cameras_exclude: Vec<String>,
    pub lenses: Vec<String>,
    pub lenses_exclude: Vec<String>,
    pub collections: Vec<String>,
    pub collections_exclude: Vec<String>,
    pub path_prefixes: Vec<String>,
    pub path_prefixes_exclude: Vec<String>,
    pub rating: Option<NumericFilter>,
    pub iso: Option<NumericFilter>,
    pub focal: Option<NumericFilter>,
    pub aperture: Option<NumericFilter>,
    pub width: Option<NumericFilter>,
    pub height: Option<NumericFilter>,
    pub copies: Option<NumericFilter>,
    pub variant_count: Option<NumericFilter>,
    pub scattered: Option<NumericFilter>,
    pub scattered_depth: Option<u32>,
    pub face_count: Option<NumericFilter>,
    pub duration: Option<NumericFilter>,
    pub codec: Option<String>,
    pub stale_days: Option<NumericFilter>,
    pub meta_filters: Vec<(String, String)>,
    pub orphan: bool,
    pub orphan_false: bool,
    pub missing: bool,
    pub volumes: Vec<String>,
    pub volumes_exclude: Vec<String>,
    pub volume_none: bool,
    pub date_prefix: Option<String>,
    pub date_from: Option<String>,
    pub date_until: Option<String>,
    pub stacked: Option<bool>,
    pub geo_bbox: Option<(f64, f64, f64, f64)>,  // (south, west, north, east)
    pub has_gps: Option<bool>,
    pub has_faces: Option<bool>,
    pub persons: Vec<String>,
    pub persons_exclude: Vec<String>,
    pub asset_ids: Vec<String>,
    pub has_embed: Option<bool>,
    #[cfg(feature = "ai")]
    pub similar: Option<String>,
    #[cfg(feature = "ai")]
    pub similar_limit: Option<usize>,
    #[cfg(feature = "ai")]
    pub min_sim: Option<f32>,
    #[cfg(feature = "ai")]
    pub text_query: Option<String>,
    #[cfg(feature = "ai")]
    pub text_query_limit: Option<usize>,
}

impl ParsedSearch {
    /// Merge another `ParsedSearch` into this one (AND semantics).
    ///
    /// Vec fields are extended (both must match). Option fields prefer `self`'s
    /// value; the other's value is used only when `self` has `None`.
    /// Bool fields are OR'd (either being true activates the filter).
    pub fn merge_from(&mut self, other: &ParsedSearch) {
        // Vec fields: extend
        self.text_exclude.extend(other.text_exclude.iter().cloned());
        self.asset_types.extend(other.asset_types.iter().cloned());
        self.asset_types_exclude.extend(other.asset_types_exclude.iter().cloned());
        self.tags.extend(other.tags.iter().cloned());
        self.tags_exclude.extend(other.tags_exclude.iter().cloned());
        self.formats.extend(other.formats.iter().cloned());
        self.formats_exclude.extend(other.formats_exclude.iter().cloned());
        self.color_labels.extend(other.color_labels.iter().cloned());
        self.color_labels_exclude.extend(other.color_labels_exclude.iter().cloned());
        self.cameras.extend(other.cameras.iter().cloned());
        self.cameras_exclude.extend(other.cameras_exclude.iter().cloned());
        self.lenses.extend(other.lenses.iter().cloned());
        self.lenses_exclude.extend(other.lenses_exclude.iter().cloned());
        self.collections.extend(other.collections.iter().cloned());
        self.collections_exclude.extend(other.collections_exclude.iter().cloned());
        self.path_prefixes.extend(other.path_prefixes.iter().cloned());
        self.path_prefixes_exclude.extend(other.path_prefixes_exclude.iter().cloned());
        self.volumes.extend(other.volumes.iter().cloned());
        self.volumes_exclude.extend(other.volumes_exclude.iter().cloned());
        self.meta_filters.extend(other.meta_filters.iter().cloned());
        self.persons.extend(other.persons.iter().cloned());
        self.persons_exclude.extend(other.persons_exclude.iter().cloned());
        self.asset_ids.extend(other.asset_ids.iter().cloned());

        // Option fields: prefer self, fall back to other
        if self.text.is_none() { self.text = other.text.clone(); }
        self.rating = NumericFilter::or(&self.rating, &other.rating);
        self.iso = NumericFilter::or(&self.iso, &other.iso);
        self.focal = NumericFilter::or(&self.focal, &other.focal);
        self.aperture = NumericFilter::or(&self.aperture, &other.aperture);
        self.width = NumericFilter::or(&self.width, &other.width);
        self.height = NumericFilter::or(&self.height, &other.height);
        self.copies = NumericFilter::or(&self.copies, &other.copies);
        self.variant_count = NumericFilter::or(&self.variant_count, &other.variant_count);
        self.scattered = NumericFilter::or(&self.scattered, &other.scattered);
        self.face_count = NumericFilter::or(&self.face_count, &other.face_count);
        self.stale_days = NumericFilter::or(&self.stale_days, &other.stale_days);
        if self.date_prefix.is_none() { self.date_prefix = other.date_prefix.clone(); }
        if self.date_from.is_none() { self.date_from = other.date_from.clone(); }
        if self.date_until.is_none() { self.date_until = other.date_until.clone(); }
        if self.stacked.is_none() { self.stacked = other.stacked; }
        if self.geo_bbox.is_none() { self.geo_bbox = other.geo_bbox; }
        if self.has_gps.is_none() { self.has_gps = other.has_gps; }
        if self.has_faces.is_none() { self.has_faces = other.has_faces; }
        if self.has_embed.is_none() { self.has_embed = other.has_embed; }
        #[cfg(feature = "ai")]
        {
            if self.similar.is_none() { self.similar = other.similar.clone(); }
            if self.similar_limit.is_none() { self.similar_limit = other.similar_limit; }
            if self.min_sim.is_none() { self.min_sim = other.min_sim; }
            if self.text_query.is_none() { self.text_query = other.text_query.clone(); }
            if self.text_query_limit.is_none() { self.text_query_limit = other.text_query_limit; }
        }

        // Bool fields: OR
        self.orphan = self.orphan || other.orphan;
        self.orphan_false = self.orphan_false || other.orphan_false;
        self.missing = self.missing || other.missing;
        self.volume_none = self.volume_none || other.volume_none;
    }

    /// Convert to `SearchOptions` for passing to catalog search methods.
    pub fn to_search_options(&self) -> SearchOptions<'_> {
        SearchOptions {
            asset_ids: &self.asset_ids,
            text: self.text.as_deref(),
            text_exclude: &self.text_exclude,
            asset_types: &self.asset_types,
            asset_types_exclude: &self.asset_types_exclude,
            tags: &self.tags,
            tags_exclude: &self.tags_exclude,
            formats: &self.formats,
            formats_exclude: &self.formats_exclude,
            color_labels: &self.color_labels,
            color_labels_exclude: &self.color_labels_exclude,
            cameras: &self.cameras,
            cameras_exclude: &self.cameras_exclude,
            lenses: &self.lenses,
            lenses_exclude: &self.lenses_exclude,
            collections: &self.collections,
            collections_exclude: &self.collections_exclude,
            path_prefixes: &self.path_prefixes,
            path_prefixes_exclude: &self.path_prefixes_exclude,
            rating: self.rating.clone(),
            iso: self.iso.clone(),
            focal: self.focal.clone(),
            aperture: self.aperture.clone(),
            width: self.width.clone(),
            height: self.height.clone(),
            copies: self.copies.clone(),
            variant_count: self.variant_count.clone(),
            scattered: self.scattered.clone(),
            scattered_depth: self.scattered_depth,
            face_count: self.face_count.clone(),
            duration: self.duration.clone(),
            codec: self.codec.clone(),
            stale_days: self.stale_days.clone(),
            meta_filters: self
                .meta_filters
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            orphan: self.orphan,
            orphan_false: self.orphan_false,
            date_prefix: self.date_prefix.as_deref(),
            date_from: self.date_from.as_deref(),
            date_until: self.date_until.as_deref(),
            stacked_filter: self.stacked,
            geo_bbox: self.geo_bbox,
            has_gps: self.has_gps,
            has_faces: self.has_faces,
            has_embed: self.has_embed,
            ..Default::default()
        }
    }
}

/// Tokenize a search query respecting double-quoted values.
///
/// Splits on whitespace, but `prefix:"multi word value"` stays as a single token
/// with quotes stripped from the value. Unquoted tokens work as before.
///
/// Examples:
///   `tag:"Fools Theater" rating:4+` → `["tag:Fools Theater", "rating:4+"]`
///   `tag:landscape type:image`      → `["tag:landscape", "type:image"]`
///   `hello world`                   → `["hello", "world"]`
fn tokenize_query(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = query.chars().peekable();

    while chars.peek().is_some() {
        // Skip whitespace
        while chars.peek().map_or(false, |c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let mut token = String::new();
        let mut in_quotes = false;

        while let Some(&c) = chars.peek() {
            if in_quotes {
                chars.next();
                if c == '"' {
                    in_quotes = false;
                } else {
                    token.push(c);
                }
            } else if c == '"' {
                chars.next();
                in_quotes = true;
            } else if c.is_whitespace() {
                break;
            } else {
                chars.next();
                token.push(c);
            }
        }

        if !token.is_empty() {
            tokens.push(token);
        }
    }

    tokens
}

/// Parse a search query string into structured filters.
///
/// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`, `rating:3+`,
/// `camera:fuji`, `lens:56mm`, `iso:3200`, `iso:100-800`, `focal:50`, `focal:35-70`,
/// `f:2.8`, `f:1.4-2.8`, `width:4000+`, `height:2000+`, `meta:key=value`.
/// Values with spaces can be quoted: `tag:"Fools Theater"`, `camera:"Canon EOS R5"`.
/// Remaining tokens are joined as free-text search.
///
/// # Examples
///
/// ```
/// use maki::query::{parse_search_query, NumericFilter};
///
/// let p = parse_search_query("tag:sunset type:image rating:3+");
/// assert_eq!(p.tags, vec!["sunset"]);
/// assert_eq!(p.asset_types, vec!["image"]);
/// assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
///
/// // Negation with - prefix
/// let p = parse_search_query("-tag:rejected");
/// assert_eq!(p.tags_exclude, vec!["rejected"]);
///
/// // Quoted values with spaces
/// let p = parse_search_query("tag:\"Fools Theater\" camera:\"Canon EOS R5\"");
/// assert_eq!(p.tags, vec!["Fools Theater"]);
/// assert_eq!(p.cameras, vec!["Canon EOS R5"]);
///
/// // Rating range
/// let p = parse_search_query("rating:3-5");
/// assert_eq!(p.rating, Some(NumericFilter::Range(3.0, 5.0)));
///
/// // Free text (unrecognized tokens)
/// let p = parse_search_query("sunset beach");
/// assert_eq!(p.text, Some("sunset beach".to_string()));
/// ```
pub fn parse_search_query(query: &str) -> ParsedSearch {
    let mut parsed = ParsedSearch::default();
    let mut text_parts = Vec::new();

    for token in tokenize_query(query) {
        // Detect negation prefix
        let (negated, token_body) = if token.starts_with('-') && token.len() > 1 && token.as_bytes()[1] != b'-' {
            (true, &token[1..])
        } else {
            (false, token.as_str())
        };

        if let Some(value) = token_body.strip_prefix("id:") {
            parsed.asset_ids.push(value.to_string());
        } else if let Some(value) = token_body.strip_prefix("type:") {
            if negated {
                parsed.asset_types_exclude.push(value.to_string());
            } else {
                parsed.asset_types.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("tag:") {
            if negated {
                parsed.tags_exclude.push(value.to_string());
            } else {
                parsed.tags.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("format:") {
            if negated {
                parsed.formats_exclude.push(value.to_string());
            } else {
                parsed.formats.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("rating:") {
            parsed.rating = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("camera:") {
            if negated {
                parsed.cameras_exclude.push(value.to_string());
            } else {
                parsed.cameras.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("lens:") {
            if negated {
                parsed.lenses_exclude.push(value.to_string());
            } else {
                parsed.lenses.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("iso:") {
            parsed.iso = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("focal:") {
            parsed.focal = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("f:") {
            parsed.aperture = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("width:") {
            parsed.width = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("height:") {
            parsed.height = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("meta:") {
            if let Some((key, val)) = value.split_once('=') {
                parsed.meta_filters.push((key.to_string(), val.to_string()));
            }
        } else if token_body == "orphan:true" {
            parsed.orphan = true;
        } else if token_body == "orphan:false" {
            parsed.orphan_false = true;
        } else if token_body == "missing:true" {
            parsed.missing = true;
        } else if let Some(value) = token_body.strip_prefix("stale:") {
            parsed.stale_days = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("volume:") {
            if value == "none" {
                parsed.volume_none = true;
            } else if negated {
                parsed.volumes_exclude.push(value.to_string());
            } else {
                parsed.volumes.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("label:") {
            if negated {
                parsed.color_labels_exclude.push(value.to_string());
            } else {
                parsed.color_labels.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("collection:") {
            if negated {
                parsed.collections_exclude.push(value.to_string());
            } else {
                parsed.collections.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("path:") {
            if negated {
                parsed.path_prefixes_exclude.push(value.to_string());
            } else {
                parsed.path_prefixes.push(value.to_string());
            }
        } else if let Some(value) = token_body.strip_prefix("copies:") {
            parsed.copies = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("variants:") {
            parsed.variant_count = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("scattered:") {
            // Support scattered:N+/D syntax where /D is the path depth
            if let Some((num_part, depth_part)) = value.rsplit_once('/') {
                parsed.scattered = parse_numeric_filter(num_part);
                parsed.scattered_depth = depth_part.parse::<u32>().ok();
            } else {
                parsed.scattered = parse_numeric_filter(value);
            }
        } else if let Some(value) = token_body.strip_prefix("date:") {
            parsed.date_prefix = Some(value.to_string());
        } else if let Some(value) = token_body.strip_prefix("dateFrom:") {
            parsed.date_from = Some(value.to_string());
        } else if let Some(value) = token_body.strip_prefix("dateUntil:") {
            parsed.date_until = Some(value.to_string());
        } else if token_body == "stacked:true" {
            parsed.stacked = Some(true);
        } else if token_body == "stacked:false" {
            parsed.stacked = Some(false);
        } else if let Some(value) = token_body.strip_prefix("geo:") {
            if value == "any" {
                parsed.has_gps = Some(true);
            } else if value == "none" {
                parsed.has_gps = Some(false);
            } else {
                // Try lat,lng,radius_km or south,west,north,east
                let parts: Vec<f64> = value.split(',').filter_map(|s| s.parse().ok()).collect();
                if parts.len() == 3 {
                    // geo:lat,lng,radius_km → bounding box
                    let lat = parts[0];
                    let lng = parts[1];
                    let r = parts[2];
                    let dlat = r / 111.0;
                    let dlng = r / (111.0 * lat.to_radians().cos());
                    parsed.geo_bbox = Some((lat - dlat, lng - dlng, lat + dlat, lng + dlng));
                } else if parts.len() == 4 {
                    // geo:south,west,north,east
                    parsed.geo_bbox = Some((parts[0], parts[1], parts[2], parts[3]));
                }
            }
        } else if let Some(value) = token_body.strip_prefix("duration:") {
            parsed.duration = parse_numeric_filter(value);
        } else if let Some(value) = token_body.strip_prefix("codec:") {
            parsed.codec = Some(value.to_string());
        } else if let Some(value) = token_body.strip_prefix("faces:") {
            if value == "any" {
                parsed.has_faces = Some(true);
            } else if value == "none" {
                parsed.has_faces = Some(false);
            } else {
                parsed.face_count = parse_numeric_filter(value);
            }
        } else if let Some(value) = token_body.strip_prefix("embed:") {
            if value == "any" || value == "true" {
                parsed.has_embed = Some(true);
            } else if value == "none" || value == "false" {
                parsed.has_embed = Some(false);
            }
        } else if let Some(value) = token_body.strip_prefix("person:") {
            if negated {
                parsed.persons_exclude.push(value.to_string());
            } else {
                parsed.persons.push(value.to_string());
            }
        } else if let Some(_value) = token_body.strip_prefix("similar:") {
            #[cfg(feature = "ai")]
            {
                // similar:<asset-id> or similar:<asset-id>:<limit>
                if let Some((id, limit_str)) = _value.rsplit_once(':') {
                    if let Ok(limit) = limit_str.parse::<usize>() {
                        parsed.similar = Some(id.to_string());
                        parsed.similar_limit = Some(limit);
                    } else {
                        // Not a valid limit, treat entire value as asset ID
                        parsed.similar = Some(_value.to_string());
                    }
                } else {
                    parsed.similar = Some(_value.to_string());
                }
            }
        } else if let Some(_value) = token_body.strip_prefix("min_sim:") {
            #[cfg(feature = "ai")]
            {
                if let Ok(v) = _value.parse::<f32>() {
                    parsed.min_sim = Some(v.clamp(0.0, 100.0));
                }
            }
        } else if let Some(_value) = token_body.strip_prefix("text:") {
            #[cfg(feature = "ai")]
            {
                if !_value.is_empty() {
                    // text:"query":limit or text:query:limit or text:"query" or text:query
                    // Check if the value ends with :<number> after the query part
                    if let Some((query_part, limit_str)) = _value.rsplit_once(':') {
                        if let Ok(limit) = limit_str.parse::<usize>() {
                            if !query_part.is_empty() {
                                parsed.text_query = Some(query_part.to_string());
                                parsed.text_query_limit = Some(limit);
                            }
                        } else {
                            parsed.text_query = Some(_value.to_string());
                        }
                    } else {
                        parsed.text_query = Some(_value.to_string());
                    }
                }
            }
        } else if negated {
            // Negated free text: -word
            text_parts.push(token_body.to_string());
            // Actually this should go to text_exclude
            text_parts.pop();
            parsed.text_exclude.push(token_body.to_string());
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
/// Unified numeric filter supporting exact, minimum, range, and OR values.
///
/// All numeric search filters (rating, iso, focal, f, width, height, copies,
/// variants, scattered, face_count) use this type for consistent syntax:
/// `x` (exact), `x+` (minimum), `x-y` (range), `x,y` (OR), `x,y+` (combined).
#[derive(Debug, Clone, PartialEq)]
pub enum NumericFilter {
    /// Exactly this value
    Exact(f64),
    /// This value or more
    Min(f64),
    /// Between min and max (inclusive)
    Range(f64, f64),
    /// Any of these exact values
    Values(Vec<f64>),
    /// Any of these exact values OR >= min
    ValuesOrMin { values: Vec<f64>, min: f64 },
}

impl NumericFilter {
    /// Merge another filter (from default_filter) if self is None.
    pub fn or(a: &Option<Self>, b: &Option<Self>) -> Option<Self> {
        a.clone().or_else(|| b.clone())
    }
}

/// Parse a numeric filter value string into a NumericFilter.
///
/// # Examples
///
/// ```
/// use maki::query::parse_numeric_filter;
///
/// assert_eq!(parse_numeric_filter("3"), Some(maki::query::NumericFilter::Exact(3.0)));
/// assert_eq!(parse_numeric_filter("3+"), Some(maki::query::NumericFilter::Min(3.0)));
/// assert_eq!(parse_numeric_filter("3-5"), Some(maki::query::NumericFilter::Range(3.0, 5.0)));
/// assert_eq!(parse_numeric_filter("2,4"), Some(maki::query::NumericFilter::Values(vec![2.0, 4.0])));
/// ```
pub fn parse_numeric_filter(value: &str) -> Option<NumericFilter> {
    if value.contains(',') {
        let mut values = Vec::new();
        let mut min = None;
        for part in value.split(',') {
            let part = part.trim();
            if let Some(num_str) = part.strip_suffix('+') {
                if let Ok(n) = num_str.parse::<f64>() {
                    min = Some(n);
                }
            } else if part.contains('-') {
                if let Some((lo, hi)) = part.split_once('-') {
                    if let (Ok(a), Ok(b)) = (lo.parse::<f64>(), hi.parse::<f64>()) {
                        // Range inside comma list: return as range
                        return Some(NumericFilter::Range(a, b));
                    }
                }
            } else if let Ok(n) = part.parse::<f64>() {
                values.push(n);
            }
        }
        if let Some(m) = min {
            if values.is_empty() {
                Some(NumericFilter::Min(m))
            } else {
                Some(NumericFilter::ValuesOrMin { values, min: m })
            }
        } else if values.len() == 1 {
            Some(NumericFilter::Exact(values[0]))
        } else if !values.is_empty() {
            Some(NumericFilter::Values(values))
        } else {
            None
        }
    } else if let Some(num_str) = value.strip_suffix('+') {
        num_str.parse::<f64>().ok().map(NumericFilter::Min)
    } else if value.contains('-') {
        let (lo, hi) = value.split_once('-')?;
        let a = lo.parse::<f64>().ok()?;
        let b = hi.parse::<f64>().ok()?;
        Some(NumericFilter::Range(a, b))
    } else {
        value.parse::<f64>().ok().map(NumericFilter::Exact)
    }
}

/// Check if `short` is a prefix-match for `long` with a separator boundary.
///
/// Returns true if `short == long` (exact match) or if `long` starts with `short`
/// and the character immediately following in `long` is non-alphanumeric.
/// This prevents `DSC_001` from matching `DSC_0010` while allowing
/// `DSC_001` to match `DSC_001-Edit` or `DSC_001_v2`.
fn stem_prefix_matches(short: &str, long: &str) -> bool {
    if short == long {
        return true;
    }
    if !long.starts_with(short) {
        return false;
    }
    // The character right after the prefix must be a non-alphanumeric separator
    match long[short.len()..].chars().next() {
        Some(c) => !c.is_alphanumeric(),
        None => true,
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

/// Result of a split operation.
#[derive(Debug, serde::Serialize)]
pub struct SplitResult {
    /// The source asset ID (that lost variants).
    pub source_id: String,
    /// New assets created from the extracted variants.
    pub new_assets: Vec<NewSplitAsset>,
}

/// Info about one newly created asset from a split.
#[derive(Debug, serde::Serialize)]
pub struct NewSplitAsset {
    pub asset_id: String,
    pub variant_hash: String,
    pub original_filename: String,
}

/// One stem group found by `auto_group`.
#[derive(Debug, serde::Serialize)]
pub struct StemGroupEntry {
    pub stem: String,
    pub target_id: String,
    pub asset_ids: Vec<String>,
    pub donor_count: usize,
}

/// Result of an auto-group operation.
#[derive(Debug, serde::Serialize)]
pub struct AutoGroupResult {
    pub groups: Vec<StemGroupEntry>,
    pub total_donors_merged: usize,
    pub total_variants_moved: usize,
    pub dry_run: bool,
}

/// Result of converting tags into stacks via `stack_from_tag`.
#[derive(Debug, serde::Serialize)]
pub struct FromTagResult {
    pub tags_matched: u32,
    pub tags_skipped: u32,
    pub stacks_created: u32,
    pub assets_stacked: u32,
    pub assets_skipped: u32,
    pub tags_removed: u32,
    pub dry_run: bool,
    pub details: Vec<FromTagDetail>,
}

/// One matched tag in a `stack_from_tag` operation.
#[derive(Debug, serde::Serialize)]
pub struct FromTagDetail {
    pub tag: String,
    pub assets_found: u32,
    pub assets_stacked: u32,
    pub assets_skipped: u32,
    pub stack_id: Option<String>,
}

/// Fields to edit on an asset. `None` = no change, `Some(None)` = clear, `Some(Some(x))` = set.
pub struct EditFields {
    pub name: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub rating: Option<Option<u8>>,
    pub color_label: Option<Option<String>>,
    /// `None` = no change, `Some(Some(dt))` = set to dt, `Some(None)` = reset to now.
    pub created_at: Option<Option<DateTime<Utc>>>,
}

/// Result of an edit operation.
#[derive(Debug, serde::Serialize)]
pub struct EditResult {
    pub asset_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub rating: Option<u8>,
    pub color_label: Option<String>,
    pub created_at: String,
}

/// Result of a tag add/remove operation.
pub struct TagResult {
    /// Tags that were actually added or removed.
    pub changed: Vec<String>,
    /// The full set of tags after the operation.
    pub current_tags: Vec<String>,
}

/// Result of a `maki tag-rename` operation.
#[derive(Debug, Default, serde::Serialize)]
pub struct TagRenameResult {
    pub dry_run: bool,
    pub matched: usize,
    pub renamed: usize,
    pub skipped: usize,
}

/// Result of a `maki writeback` operation.
#[derive(Debug, Default, serde::Serialize)]
pub struct WritebackResult {
    /// Number of XMP files written (or that would be written in dry-run).
    pub written: u32,
    /// Number of recipes skipped (volume offline or file missing).
    pub skipped: u32,
    /// Number of recipes that failed.
    pub failed: u32,
    /// Error messages.
    pub errors: Vec<String>,
    /// Whether this was a dry run.
    pub dry_run: bool,
}

/// Resolve and normalize a `path:` filter value for search.
///
/// When `cwd` is provided (CLI context):
/// - `~` or `~/...` is expanded to the user's home directory
/// - `./...` or `../...` is resolved relative to `cwd`
///
/// After resolution, if the path is absolute and matches a volume mount point
/// (longest prefix match), returns (volume-relative path, Some(volume_id)).
/// Otherwise returns (path, None) unchanged.
pub fn normalize_path_for_search(
    path: &str,
    volumes: &[Volume],
    cwd: Option<&std::path::Path>,
) -> (String, Option<String>) {
    // Step 1: Expand ~ and resolve ./ ../ when cwd is available
    let resolved = if let Some(cwd) = cwd {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"));
        if path == "~" {
            home.map(|h| h.to_string())
                .unwrap_or_else(|_| path.to_string())
        } else if let Some(rest) = path.strip_prefix("~/") {
            home.map(|h| std::path::PathBuf::from(h).join(rest).to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string())
        } else if path.starts_with("./") || path.starts_with("../") {
            let joined = cwd.join(path);
            // Clean the path components (handle ./ and ../) without requiring
            // the path to exist on disk (unlike canonicalize)
            clean_path(&joined)
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    // Step 2: If absolute, try to match a volume mount point
    // On Windows, canonicalized paths have \\?\ prefix — strip it for matching
    #[cfg(windows)]
    let resolved = resolved.strip_prefix(r"\\?\").unwrap_or(&resolved).to_string();
    let p = std::path::Path::new(&resolved);
    if !p.is_absolute() {
        return (resolved, None);
    }

    let mut best: Option<&Volume> = None;
    let mut best_len = 0;

    for v in volumes {
        // On Windows, volume mount points may also have \\?\ prefix
        #[cfg(windows)]
        let mount = std::path::PathBuf::from(
            v.mount_point.to_string_lossy().strip_prefix(r"\\?\").unwrap_or(&v.mount_point.to_string_lossy())
        );
        #[cfg(unix)]
        let mount = &v.mount_point;
        if p.starts_with(&mount) {
            let len = mount.as_os_str().len();
            if len > best_len {
                best = Some(v);
                best_len = len;
            }
        }
    }

    match best {
        Some(vol) => {
            // Use the same \\?\-stripped mount for strip_prefix
            #[cfg(windows)]
            let mount = std::path::PathBuf::from(
                vol.mount_point.to_string_lossy().strip_prefix(r"\\?\").unwrap_or(&vol.mount_point.to_string_lossy())
            );
            #[cfg(unix)]
            let mount = &vol.mount_point;
            let relative = p
                .strip_prefix(&mount)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (relative, Some(vol.id.to_string()))
        }
        None => (resolved, None),
    }
}

/// Logically clean a path by resolving `.` and `..` components without
/// touching the filesystem (unlike `canonicalize` which requires the path to exist).
fn clean_path(path: &std::path::Path) -> String {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {} // skip .
            std::path::Component::ParentDir => {
                parts.pop(); // go up
            }
            other => parts.push(other.as_os_str()),
        }
    }
    let result: std::path::PathBuf = parts.iter().collect();
    // Normalize to forward slashes for cross-platform consistency
    result.to_string_lossy().replace('\\', "/")
}

/// Search and filter assets via the SQLite catalog.
/// Shared context for batch operations — avoids reopening catalog, device
/// registry, and content store per asset.
pub struct BatchContext {
    pub catalog: Catalog,
    pub meta_store: MetadataStore,
    pub online_volumes: HashMap<uuid::Uuid, PathBuf>,
    pub content_store: ContentStore,
}

pub struct QueryEngine {
    catalog_root: PathBuf,
    default_filter: Option<String>,
}

impl QueryEngine {
    pub fn new(catalog_root: &Path) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
            default_filter: None,
        }
    }

    /// Check if XMP writeback is enabled in maki.toml.
    fn is_writeback_enabled(&self) -> bool {
        crate::config::CatalogConfig::load(&self.catalog_root)
            .map(|c| c.writeback.enabled)
            .unwrap_or(false)
    }

    /// Create a QueryEngine with a default filter from config.
    pub fn with_default_filter(catalog_root: &Path, default_filter: Option<String>) -> Self {
        Self {
            catalog_root: catalog_root.to_path_buf(),
            default_filter,
        }
    }

    /// Search assets by a free-text query string.
    ///
    /// Supports prefix filters: `type:image`, `tag:landscape`, `format:jpg`, `rating:3+`,
    /// `camera:fuji`, `lens:56mm`, `iso:3200`, `focal:50`, `f:2.8`, `width:4000+`,
    /// `height:2000+`, `meta:key=value`.
    /// Remaining tokens are joined as free-text search against name/filename/description/metadata.
    pub fn search(&self, query: &str) -> Result<Vec<SearchRow>> {
        let mut parsed = parse_search_query(query);

        // Apply default filter from config (AND semantics)
        if let Some(ref df) = self.default_filter {
            if !df.is_empty() {
                let default_parsed = parse_search_query(df);
                parsed.merge_from(&default_parsed);
            }
        }

        // Normalize path prefixes: ~, ./, ../, /absolute → volume-relative + volume filter
        let path_volume_id: Option<String>;
        if !parsed.path_prefixes.is_empty() {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;
            let cwd = std::env::current_dir().ok();
            // Normalize the first path prefix (CLI context)
            let (normalized, vol_id) = normalize_path_for_search(
                &parsed.path_prefixes[0],
                &volumes,
                cwd.as_deref(),
            );
            parsed.path_prefixes[0] = normalized;
            path_volume_id = vol_id;
        } else {
            path_volume_id = None;
        }

        let mut opts = SearchOptions {
            per_page: u32::MAX,
            ..parsed.to_search_options()
        };

        if let Some(ref vid) = path_volume_id {
            opts.volume = Some(vid);
        }

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

        // Pre-compute collection asset IDs (include)
        let collection_ids;
        if !parsed.collections.is_empty() {
            let store = crate::collection::CollectionStore::new(catalog.conn());
            // OR across all collection entries, then intersect
            let mut all_ids = HashSet::new();
            for col_entry in &parsed.collections {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_asset_ids = Some(&collection_ids);
        }

        // Pre-compute collection exclude IDs
        let collection_exclude_ids;
        if !parsed.collections_exclude.is_empty() {
            let store = crate::collection::CollectionStore::new(catalog.conn());
            let mut all_ids = HashSet::new();
            for col_entry in &parsed.collections_exclude {
                for col_name in col_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Ok(ids) = store.asset_ids_for_collection(col_name) {
                        all_ids.extend(ids);
                    }
                }
            }
            collection_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
            opts.collection_exclude_ids = Some(&collection_exclude_ids);
        }

        // Pre-compute person asset IDs (include)
        let person_ids;
        if !parsed.persons.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for person_entry in &parsed.persons {
                    for person_name in person_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(person_name) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_ids = all_ids.into_iter().collect::<Vec<_>>();
                opts.person_asset_ids = Some(&person_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_ids = Vec::new();
                opts.person_asset_ids = Some(&person_ids);
            }
        }

        // Pre-compute person exclude IDs
        let person_exclude_ids;
        if !parsed.persons_exclude.is_empty() {
            #[cfg(feature = "ai")]
            {
                let face_store = crate::face_store::FaceStore::new(catalog.conn());
                let mut all_ids = std::collections::HashSet::new();
                for person_entry in &parsed.persons_exclude {
                    for person_name in person_entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                        if let Ok(ids) = face_store.find_person_asset_ids(person_name) {
                            all_ids.extend(ids);
                        }
                    }
                }
                person_exclude_ids = all_ids.into_iter().collect::<Vec<_>>();
                opts.person_exclude_ids = Some(&person_exclude_ids);
            }
            #[cfg(not(feature = "ai"))]
            {
                person_exclude_ids = Vec::new();
                opts.person_exclude_ids = Some(&person_exclude_ids);
            }
        }

        // Pre-compute similar asset IDs from embedding similarity search
        #[cfg(feature = "ai")]
        let similar_ids;
        #[cfg(feature = "ai")]
        if let Some(ref similar_ref) = parsed.similar {
            let full_id = catalog
                .resolve_asset_id(similar_ref)?
                .ok_or_else(|| anyhow::anyhow!("No asset found matching '{similar_ref}'"))?;
            let config = crate::config::CatalogConfig::load(&self.catalog_root).unwrap_or_default();
            let model_id = &config.ai.model;
            let emb_store = crate::embedding_store::EmbeddingStore::new(catalog.conn());
            let query_emb = emb_store
                .get(&full_id, model_id)?
                .ok_or_else(|| anyhow::anyhow!(
                    "No embedding found for asset '{similar_ref}'. Run `maki embed --asset {full_id}` first."
                ))?;
            let limit = parsed.similar_limit.unwrap_or(40);
            let dim = query_emb.len();
            let index = crate::embedding_store::EmbeddingIndex::load(catalog.conn(), model_id, dim)?;
            let results = index.search(&query_emb, limit.saturating_sub(1), Some(&full_id));
            // min_sim is specified as percentage 0-100, convert to 0.0-1.0
            let min_sim = parsed.min_sim.unwrap_or(0.0) / 100.0;
            // Include the source asset itself
            similar_ids = std::iter::once(full_id.clone())
                .chain(results.into_iter()
                    .filter(|(_id, score)| *score >= min_sim)
                    .map(|(id, _score)| id))
                .collect::<Vec<_>>();
            opts.similar_asset_ids = Some(&similar_ids);
        }

        // Pre-compute text search asset IDs from text-to-image embedding similarity
        #[cfg(feature = "ai")]
        let text_query_ids;
        #[cfg(feature = "ai")]
        if let Some(ref text_q) = parsed.text_query {
            let config = crate::config::CatalogConfig::load(&self.catalog_root).unwrap_or_default();
            let model_id = &config.ai.model;
            let spec = crate::ai::get_model_spec(model_id)
                .ok_or_else(|| anyhow::anyhow!("Unknown AI model: {model_id}"))?;

            // Resolve model directory
            let model_dir_str = &config.ai.model_dir;
            let model_base = if model_dir_str.starts_with("~/") {
                let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))
                    .map_err(|_| anyhow::anyhow!("Cannot determine home directory"))?;
                std::path::PathBuf::from(home).join(&model_dir_str[2..])
            } else {
                std::path::PathBuf::from(model_dir_str)
            };
            let model_dir = model_base.join(model_id);

            // Load model and encode the text query
            let mut model = crate::ai::SigLipModel::load_with_provider(
                &model_dir, model_id, crate::Verbosity::quiet(), &config.ai.execution_provider,
            )?;
            let query_emb = model.encode_texts(&[text_q.clone()])?;
            let query_emb = &query_emb[0];

            // Search embedding index
            let limit = parsed.text_query_limit.unwrap_or(config.ai.text_limit);
            let index = crate::embedding_store::EmbeddingIndex::load(
                catalog.conn(), model_id, spec.embedding_dim,
            )?;
            let results = index.search(query_emb, limit, None);
            text_query_ids = results.into_iter().map(|(id, _score)| id).collect::<Vec<_>>();
            opts.text_search_ids = Some(&text_query_ids);
        }

        // Resolve volume labels to volume IDs, and handle volume:none
        let resolved_volume_ids;
        let resolved_volume_exclude_ids;
        let online_vol_ids;
        if !parsed.volumes.is_empty() || !parsed.volumes_exclude.is_empty() || parsed.volume_none {
            let registry = DeviceRegistry::new(&self.catalog_root);
            let volumes = registry.list()?;

            resolved_volume_ids = Self::resolve_volume_labels(&parsed.volumes, &volumes)?;
            if !resolved_volume_ids.is_empty() {
                opts.volume_ids = &resolved_volume_ids;
            }

            resolved_volume_exclude_ids = Self::resolve_volume_labels(&parsed.volumes_exclude, &volumes)?;
            if !resolved_volume_exclude_ids.is_empty() {
                opts.volume_ids_exclude = &resolved_volume_exclude_ids;
            }

            if parsed.volume_none {
                online_vol_ids = volumes
                    .iter()
                    .filter(|v| v.is_online)
                    .map(|v| v.id.to_string())
                    .collect::<Vec<_>>();
                opts.no_online_locations = Some(&online_vol_ids);
            }
        }

        catalog.search_paginated(&opts)
    }

    /// Resolve volume label strings to volume UUIDs.
    ///
    /// Each entry may be comma-separated for OR semantics (e.g. "Photos,Archive").
    /// Label matching is case-insensitive.
    fn resolve_volume_labels(labels: &[String], volumes: &[Volume]) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in labels {
            for label in entry.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                let vol = volumes.iter().find(|v| v.label.eq_ignore_ascii_case(label))
                    .ok_or_else(|| anyhow::anyhow!("Unknown volume: '{label}'"))?;
                ids.push(vol.id.to_string());
            }
        }
        Ok(ids)
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

    /// Resolve a scope (query, single asset, explicit IDs) to a set of asset IDs.
    ///
    /// This is the standard way to turn the CLI's scope options into a concrete
    /// set of assets to process. Returns `None` if no scope was specified (caller
    /// should process everything). Returns `Some(set)` to filter by membership.
    pub fn resolve_scope(
        &self,
        query: Option<&str>,
        asset: Option<&str>,
        asset_ids: &[String],
    ) -> Result<Option<HashSet<String>>> {
        // Explicit asset ID list (from shell variable expansion)
        if !asset_ids.is_empty() {
            let catalog = Catalog::open(&self.catalog_root)?;
            let mut ids = HashSet::new();
            for raw_id in asset_ids {
                let full_id = catalog
                    .resolve_asset_id(raw_id)?
                    .ok_or_else(|| anyhow::anyhow!("No asset found matching '{raw_id}'"))?;
                ids.insert(full_id);
            }
            return Ok(Some(ids));
        }
        // Single asset ID
        if let Some(prefix) = asset {
            let catalog = Catalog::open(&self.catalog_root)?;
            let full_id = catalog
                .resolve_asset_id(prefix)?
                .ok_or_else(|| anyhow::anyhow!("No asset found matching '{prefix}'"))?;
            return Ok(Some(HashSet::from([full_id])));
        }
        // Search query
        if let Some(q) = query {
            let rows = self.search(q)?;
            let ids: HashSet<String> = rows.into_iter().map(|r| r.asset_id).collect();
            return Ok(Some(ids));
        }
        // No scope — process everything
        Ok(None)
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

        // Check if the target asset has RAW variants (for smart role assignment)
        let target_has_raw = target.variants.iter().any(|v| {
            crate::asset_service::is_raw_extension(&v.format)
        });

        for donor in &donors {
            for variant in &donor.variants {
                let mut moved_variant = variant.clone();
                moved_variant.asset_id = target_id;
                // Smart role assignment: in RAW+non-RAW groups, non-RAW donors
                // become Export (processed output). Otherwise Alternate.
                if moved_variant.role == crate::models::VariantRole::Original {
                    if target_has_raw && !crate::asset_service::is_raw_extension(&moved_variant.format) {
                        moved_variant.role = crate::models::VariantRole::Export;
                    } else {
                        moved_variant.role = crate::models::VariantRole::Alternate;
                    }
                }
                target.variants.push(moved_variant);
                variants_moved += 1;
            }
            for tag in &donor.tags {
                if all_tags.insert(tag.clone()) {
                    target.tags.push(tag.clone());
                }
            }
            for recipe in &donor.recipes {
                target.recipes.push(recipe.clone());
            }
            // Keep the highest rating across target and donors
            if let Some(donor_rating) = donor.rating {
                target.rating = Some(target.rating.map_or(donor_rating, |r| r.max(donor_rating)));
            }
            // Keep first non-None color label
            if target.color_label.is_none() && donor.color_label.is_some() {
                target.color_label.clone_from(&donor.color_label);
            }
            // Keep first non-None description
            if target.description.is_none() && donor.description.is_some() {
                target.description.clone_from(&donor.description);
            }
        }

        // Step 5: Save target sidecar and update catalog
        store.save(&target)?;
        catalog.insert_asset(&target)?;

        // Step 6: Update variant rows in catalog and clean up donors
        for donor in &donors {
            let donor_id = donor.id.to_string();

            for variant in &donor.variants {
                catalog.update_variant_asset_id(
                    &variant.content_hash,
                    &target_id.to_string(),
                )?;
                // Re-role originals to exports in the catalog too
                if variant.role == crate::models::VariantRole::Original {
                    catalog.update_variant_role(&variant.content_hash, "alternate")?;
                }
            }

            // Clean up FK-referencing rows before deleting the donor asset.
            // variants, collection_assets, and faces reference assets(id).
            let _ = catalog.delete_collection_memberships_for_asset(&donor_id);
            let _ = catalog.delete_recipes_for_asset(&donor_id);
            let _ = catalog.delete_file_locations_for_asset(&donor_id);
            let _ = catalog.delete_variants_for_asset(&donor_id);
            // faces table references assets(id) via FK
            let _ = catalog.conn().execute(
                "DELETE FROM faces WHERE asset_id = ?1",
                rusqlite::params![donor_id],
            );

            store.delete(donor.id)?;
            catalog.delete_asset(&donor_id)?;
        }

        let donors_removed = donors.len();

        Ok(GroupResult {
            target_id: target_id.to_string(),
            variants_moved,
            donors_removed,
        })
    }

    /// Group assets by their IDs into a single asset.
    ///
    /// If `target_id` is provided, that asset becomes the merge target
    /// (must be one of the `asset_ids`). Otherwise the oldest asset wins.
    pub fn group_by_asset_ids(
        &self,
        asset_ids: &[String],
        target_id: Option<&str>,
    ) -> Result<GroupResult> {
        if asset_ids.len() < 2 {
            anyhow::bail!("Need at least 2 assets to group");
        }

        if let Some(tid) = target_id {
            if !asset_ids.iter().any(|id| id == tid) {
                anyhow::bail!("Target asset '{}' is not in the selected assets", tid);
            }
        }

        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);

        // Deduplicate
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            asset_ids
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        };

        if unique_ids.len() < 2 {
            return Ok(GroupResult {
                target_id: unique_ids.into_iter().next().unwrap(),
                variants_moved: 0,
                donors_removed: 0,
            });
        }

        // Load all assets from sidecar
        let mut assets: Vec<crate::models::Asset> = unique_ids
            .iter()
            .map(|id| {
                let uuid: uuid::Uuid = id.parse()?;
                store.load(uuid)
            })
            .collect::<Result<_>>()?;

        // Pick target: explicit or oldest
        let target_idx = if let Some(tid) = target_id {
            assets
                .iter()
                .position(|a| a.id.to_string() == tid)
                .unwrap()
        } else {
            assets.sort_by_key(|a| a.created_at);
            0
        };

        let mut target = assets.remove(target_idx);
        let target_uuid = target.id;
        let donors = assets;

        // Merge variants, tags, recipes from donors into target
        let mut variants_moved = 0;
        let mut all_tags: HashSet<String> = target.tags.iter().cloned().collect();

        let target_has_raw = target.variants.iter().any(|v| {
            crate::asset_service::is_raw_extension(&v.format)
        });

        for donor in &donors {
            for variant in &donor.variants {
                let mut moved_variant = variant.clone();
                moved_variant.asset_id = target_uuid;
                if moved_variant.role == crate::models::VariantRole::Original {
                    if target_has_raw && !crate::asset_service::is_raw_extension(&moved_variant.format) {
                        moved_variant.role = crate::models::VariantRole::Export;
                    } else {
                        moved_variant.role = crate::models::VariantRole::Alternate;
                    }
                }
                target.variants.push(moved_variant);
                variants_moved += 1;
            }
            for tag in &donor.tags {
                if all_tags.insert(tag.clone()) {
                    target.tags.push(tag.clone());
                }
            }
            for recipe in &donor.recipes {
                target.recipes.push(recipe.clone());
            }
            // Keep the highest rating across target and donors
            if let Some(donor_rating) = donor.rating {
                target.rating = Some(target.rating.map_or(donor_rating, |r| r.max(donor_rating)));
            }
            // Keep first non-None color label
            if target.color_label.is_none() && donor.color_label.is_some() {
                target.color_label.clone_from(&donor.color_label);
            }
            // Keep first non-None description
            if target.description.is_none() && donor.description.is_some() {
                target.description.clone_from(&donor.description);
            }
        }

        // Save target and update catalog
        store.save(&target)?;
        catalog.insert_asset(&target)?;

        for donor in &donors {
            let donor_id = donor.id.to_string();

            for variant in &donor.variants {
                catalog.update_variant_asset_id(
                    &variant.content_hash,
                    &target_uuid.to_string(),
                )?;
                if variant.role == crate::models::VariantRole::Original {
                    catalog.update_variant_role(&variant.content_hash, "alternate")?;
                }
            }

            let _ = catalog.delete_collection_memberships_for_asset(&donor_id);
            let _ = catalog.delete_recipes_for_asset(&donor_id);
            let _ = catalog.delete_file_locations_for_asset(&donor_id);
            let _ = catalog.delete_variants_for_asset(&donor_id);
            let _ = catalog.conn().execute(
                "DELETE FROM faces WHERE asset_id = ?1",
                rusqlite::params![donor_id],
            );

            store.delete(donor.id)?;
            catalog.delete_asset(&donor_id)?;
        }

        Ok(GroupResult {
            target_id: target_uuid.to_string(),
            variants_moved,
            donors_removed: donors.len(),
        })
    }

    /// Split variants out of an asset into new standalone assets.
    ///
    /// Each extracted variant becomes a separate asset with role `Original`.
    /// Tags, rating, color_label, and description are inherited from the source.
    /// Recipes attached to extracted variants move with them.
    pub fn split(&self, asset_id: &str, variant_hashes: &[String]) -> Result<SplitResult> {
        if variant_hashes.is_empty() {
            anyhow::bail!("No variant hashes provided");
        }

        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);

        // Resolve asset ID (supports prefix matching)
        let full_id = catalog
            .resolve_asset_id(asset_id)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id}'"))?;
        let source_uuid: uuid::Uuid = full_id.parse()?;
        let mut source = store.load(source_uuid)?;

        // Validate: all hashes belong to this asset
        let source_hashes: HashSet<&str> =
            source.variants.iter().map(|v| v.content_hash.as_str()).collect();
        for hash in variant_hashes {
            if !source_hashes.contains(hash.as_str()) {
                anyhow::bail!(
                    "Variant '{}' does not belong to asset '{}'",
                    hash,
                    &full_id[..8]
                );
            }
        }

        // Refuse to extract all variants — at least one must remain
        let extract_set: HashSet<&str> = variant_hashes.iter().map(|h| h.as_str()).collect();
        if extract_set.len() >= source.variants.len() {
            anyhow::bail!("Cannot extract all variants — at least one must remain");
        }

        let mut new_assets_info = Vec::new();

        // For each variant to extract, create a new asset
        for hash in variant_hashes {
            // Find and remove the variant from source
            let idx = source
                .variants
                .iter()
                .position(|v| v.content_hash == *hash)
                .ok_or_else(|| anyhow::anyhow!("Variant '{}' not found", hash))?;
            let mut variant = source.variants.remove(idx);

            // Create new asset ID deterministically from variant hash
            let new_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, hash.as_bytes());
            variant.asset_id = new_uuid;
            variant.role = crate::models::VariantRole::Original;

            // Move recipes that belong to this variant
            let mut moved_recipes = Vec::new();
            source.recipes.retain(|r| {
                if r.variant_hash == *hash {
                    moved_recipes.push(r.clone());
                    false
                } else {
                    true
                }
            });

            // Determine asset type from the variant's format
            let asset_type = crate::asset_service::determine_asset_type(&variant.format);

            let original_filename = variant.original_filename.clone();

            let new_asset = crate::models::Asset {
                id: new_uuid,
                name: None,
                created_at: source.created_at,
                asset_type,
                tags: source.tags.clone(),
                description: source.description.clone(),
                rating: source.rating,
                color_label: source.color_label.clone(),
                preview_rotation: None,
                preview_variant: None,
                variants: vec![variant.clone()],
                recipes: moved_recipes,
            };

            // Save new asset sidecar and insert into catalog
            store.save(&new_asset)?;
            catalog.insert_asset(&new_asset)?;

            // Update variant's asset_id and role in catalog
            catalog.update_variant_asset_id(&variant.content_hash, &new_uuid.to_string())?;
            catalog.update_variant_role(&variant.content_hash, "original")?;

            new_assets_info.push(NewSplitAsset {
                asset_id: new_uuid.to_string(),
                variant_hash: variant.content_hash.clone(),
                original_filename,
            });
        }

        // Save updated source asset (with extracted variants removed)
        store.save(&source)?;
        catalog.insert_asset(&source)?;

        Ok(SplitResult {
            source_id: full_id,
            new_assets: new_assets_info,
        })
    }

    /// Auto-group assets by filename stem using fuzzy prefix matching.
    ///
    /// Two stems match if the shorter is a prefix of the longer and the next
    /// character in the longer string is non-alphanumeric (a separator like
    /// `-`, `_`, ` `, `(`, etc.). This handles the common case where export
    /// tools append suffixes to the original filename:
    /// `Z91_8561.ARW` → `Z91_8561-1-HighRes-(c)_2025_Name.tif`.
    ///
    /// Picks the best target per group (RAW preferred, then oldest) and merges.
    pub fn auto_group(&self, asset_ids: &[String], dry_run: bool) -> Result<AutoGroupResult> {
        let catalog = Catalog::open(&self.catalog_root)?;

        // Deduplicate input IDs
        let unique_ids: Vec<String> = {
            let mut seen = HashSet::new();
            asset_ids
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect()
        };

        // Load details for each asset and extract stem
        struct StemEntry {
            stem: String,
            asset_id: String,
            details: crate::catalog::AssetDetails,
        }
        let mut entries: Vec<StemEntry> = Vec::new();
        for id in &unique_ids {
            let details = match catalog.load_asset_details(id)? {
                Some(d) => d,
                None => continue,
            };
            let stem = if let Some(v) = details.variants.first() {
                std::path::Path::new(&v.original_filename)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_uppercase())
                    .unwrap_or_default()
            } else {
                continue;
            };
            if stem.is_empty() {
                continue;
            }
            entries.push(StemEntry { stem, asset_id: id.clone(), details });
        }

        // Sort by stem length (shortest first) for prefix resolution
        entries.sort_by_key(|e| e.stem.len());

        // Resolve each stem to its root (shortest valid prefix-match)
        let mut roots: Vec<String> = Vec::new();
        let mut stem_to_root: HashMap<String, String> = HashMap::new();

        for entry in &entries {
            let stem = &entry.stem;
            if stem_to_root.contains_key(stem) {
                // Another asset with the same stem already resolved
                continue;
            }
            let mut found_root = None;
            for root in &roots {
                if stem_prefix_matches(root, stem) {
                    found_root = Some(root.clone());
                    break; // first (shortest) root wins
                }
            }
            match found_root {
                Some(root) => {
                    stem_to_root.insert(stem.clone(), root);
                }
                None => {
                    roots.push(stem.clone());
                    stem_to_root.insert(stem.clone(), stem.clone());
                }
            }
        }

        // Group assets by resolved root stem
        let mut group_map: HashMap<String, Vec<(String, crate::catalog::AssetDetails)>> =
            HashMap::new();
        for entry in entries {
            let root = stem_to_root.get(&entry.stem).unwrap();
            group_map
                .entry(root.clone())
                .or_default()
                .push((entry.asset_id, entry.details));
        }

        // Filter to groups with >1 distinct asset and merge
        let mut groups = Vec::new();
        let mut total_donors_merged = 0;
        let mut total_variants_moved = 0;

        for (root_stem, mut entries) in group_map {
            if entries.len() < 2 {
                continue;
            }

            // Sort: prefer asset with RAW variant, then oldest by created_at
            entries.sort_by(|a, b| {
                let a_raw = a.1.variants.iter().any(|v| {
                    crate::asset_service::is_raw_extension(&v.format)
                });
                let b_raw = b.1.variants.iter().any(|v| {
                    crate::asset_service::is_raw_extension(&v.format)
                });
                b_raw.cmp(&a_raw).then_with(|| a.1.created_at.cmp(&b.1.created_at))
            });

            let target_id = entries[0].0.clone();
            let all_ids: Vec<String> = entries.iter().map(|e| e.0.clone()).collect();
            let donor_count = entries.len() - 1;

            if !dry_run {
                let all_hashes: Vec<String> = entries
                    .iter()
                    .flat_map(|e| e.1.variants.iter().map(|v| v.content_hash.clone()))
                    .collect();
                let result = self.group(&all_hashes)?;
                total_variants_moved += result.variants_moved;
                total_donors_merged += result.donors_removed;
            } else {
                let donor_variants: usize = entries[1..]
                    .iter()
                    .map(|e| e.1.variants.len())
                    .sum();
                total_variants_moved += donor_variants;
                total_donors_merged += donor_count;
            }

            groups.push(StemGroupEntry {
                stem: root_stem,
                target_id,
                asset_ids: all_ids,
                donor_count,
            });
        }

        // Sort groups by stem for deterministic output
        groups.sort_by(|a, b| a.stem.cmp(&b.stem));

        Ok(AutoGroupResult {
            groups,
            total_donors_merged,
            total_variants_moved,
            dry_run,
        })
    }

    /// Add or remove tags on an asset. Updates both sidecar YAML and SQLite catalog.
    pub fn tag(&self, asset_id_prefix: &str, tags: &[String], remove: bool) -> Result<TagResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);
        let online = Self::load_online_volumes(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let ctx = BatchContext { catalog, meta_store: store, online_volumes: online, content_store };
        self.tag_inner(&ctx, asset_id_prefix, tags, remove)
    }

    fn tag_inner(&self, ctx: &BatchContext, asset_id_prefix: &str, tags: &[String], remove: bool) -> Result<TagResult> {
        let full_id = ctx.catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let mut asset = ctx.meta_store.load(uuid)?;

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

        ctx.meta_store.save(&asset)?;
        ctx.catalog.insert_asset(&asset)?;

        if !changed.is_empty() {
            let (to_add, to_remove) = if remove {
                (Vec::new(), changed.clone())
            } else {
                (changed.clone(), Vec::new())
            };
            if self.is_writeback_enabled() {
                self.write_back_tags_to_xmp_inner(&mut asset, &to_add, &to_remove, &ctx.catalog, &ctx.meta_store, &ctx.online_volumes, &ctx.content_store);
            }
        }

        Ok(TagResult {
            changed,
            current_tags: asset.tags.clone(),
        })
    }

    /// Rename a tag across all assets that have it.
    pub fn tag_rename(
        &self,
        old_tag: &str,
        new_tag: &str,
        apply: bool,
        mut on_asset: impl FnMut(&str, bool),
    ) -> Result<TagRenameResult> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);
        let online = Self::load_online_volumes(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);

        let matches = catalog.assets_with_exact_tag(old_tag)?;
        let mut renamed = 0usize;
        let mut skipped = 0usize;

        for (asset_id, _stack_id) in &matches {
            let uuid: uuid::Uuid = asset_id.parse()?;
            let mut asset = store.load(uuid)?;

            // Replace old tag with new tag
            let had_old = asset.tags.contains(&old_tag.to_string());
            let has_new = asset.tags.contains(&new_tag.to_string());
            if !had_old {
                skipped += 1;
                continue;
            }

            let name = asset.name.clone().unwrap_or_else(|| asset_id[..8.min(asset_id.len())].to_string());
            on_asset(&name, true);

            if apply {
                // Collect ancestor components of the new tag that are now redundant.
                // E.g., renaming "Munich" to "location|Germany|Bavaria|Munich" makes
                // standalone "Germany", "Bavaria", "location" tags redundant because
                // hierarchical search matches ancestors automatically.
                let new_parts: Vec<&str> = new_tag.split('|').collect();
                let mut redundant = vec![old_tag.to_string()];
                if new_parts.len() > 1 {
                    // Build all ancestor paths: "location", "location|Germany", etc.
                    let mut ancestors = Vec::new();
                    for i in 1..new_parts.len() {
                        ancestors.push(new_parts[..i].join("|"));
                    }
                    // Also check bare component names (e.g., "Germany" without prefix)
                    for part in &new_parts[..new_parts.len() - 1] {
                        if !part.is_empty() {
                            ancestors.push(part.to_string());
                        }
                    }
                    for ancestor in &ancestors {
                        if asset.tags.contains(ancestor) && !redundant.contains(ancestor) {
                            redundant.push(ancestor.clone());
                        }
                    }
                }

                asset.tags.retain(|t| !redundant.contains(t));
                if !has_new {
                    asset.tags.push(new_tag.to_string());
                }
                store.save(&asset)?;
                catalog.insert_asset(&asset)?;

                // Writeback: remove old + redundant tags, add new tag in XMP
                if self.is_writeback_enabled() {
                    let to_add = if has_new { vec![] } else { vec![new_tag.to_string()] };
                    self.write_back_tags_to_xmp_inner(
                        &mut asset, &to_add, &redundant,
                        &catalog, &store, &online, &content_store,
                    );
                }
            }
            renamed += 1;
        }

        Ok(TagRenameResult {
            dry_run: !apply,
            matched: matches.len(),
            renamed,
            skipped,
        })
    }

    /// Clear asset-level metadata and re-extract from variant source files (XMP recipes + embedded XMP).
    /// Returns the updated tags list.
    pub fn reimport_metadata(&self, asset_id_prefix: &str) -> Result<Vec<String>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);

        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let mut asset = store.load(uuid)?;

        // Clear asset-level metadata that comes from XMP sources
        asset.tags.clear();
        asset.description = None;
        asset.rating = None;
        asset.color_label = None;

        // Build volume lookup (id string -> Volume)
        let volumes = registry.list().unwrap_or_default();
        let vol_map: HashMap<String, &crate::models::volume::Volume> =
            volumes.iter().map(|v| (v.id.to_string(), v)).collect();

        // Re-extract from XMP recipe files
        let recipes = catalog.list_recipes_for_asset(&full_id)?;
        for (_recipe_id, _content_hash, variant_hash, relative_path, volume_id) in &recipes {
            let vol = match vol_map.get(volume_id) {
                Some(v) if v.is_online => *v,
                _ => continue,
            };
            let ext = std::path::Path::new(relative_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !ext.eq_ignore_ascii_case("xmp") {
                continue;
            }
            let full_path = vol.mount_point.join(relative_path);
            if full_path.exists() {
                let xmp = crate::xmp_reader::extract(&full_path);
                crate::asset_service::apply_xmp_data_pub(&xmp, &mut asset, variant_hash);
            }
        }

        // Re-extract from embedded XMP in JPEG/TIFF media files
        let locations = catalog.list_file_locations_for_asset(&full_id)?;
        for (content_hash, relative_path, volume_id) in &locations {
            let vol = match vol_map.get(volume_id) {
                Some(v) if v.is_online => *v,
                _ => continue,
            };
            let ext = std::path::Path::new(relative_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !matches!(ext.to_lowercase().as_str(), "jpg" | "jpeg" | "tif" | "tiff") {
                continue;
            }
            let full_path = vol.mount_point.join(relative_path);
            if full_path.exists() {
                let embedded_xmp = crate::embedded_xmp::extract_embedded_xmp(&full_path);
                if !embedded_xmp.keywords.is_empty()
                    || embedded_xmp.description.is_some()
                    || !embedded_xmp.source_metadata.is_empty()
                {
                    crate::asset_service::apply_xmp_data_pub(&embedded_xmp, &mut asset, content_hash);
                }
            }
        }

        store.save(&asset)?;
        catalog.insert_asset(&asset)?;

        Ok(asset.tags.clone())
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

        if let Some(date) = &fields.created_at {
            match date {
                Some(dt) => asset.created_at = *dt,
                None => asset.created_at = Utc::now(),
            }
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
            created_at: asset.created_at.to_rfc3339(),
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

    /// Set the date on an asset. Updates both sidecar YAML and SQLite catalog.
    /// No XMP write-back needed — date has no XMP equivalent in our workflow.
    /// Returns the new date as an RFC 3339 string.
    pub fn set_date(&self, asset_id_prefix: &str, date: DateTime<Utc>) -> Result<String> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.created_at = date;
        store.save(&asset)?;
        catalog.update_asset_created_at(&full_id, &date)?;

        Ok(date.to_rfc3339())
    }

    /// Set the rating on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Also writes back the rating to any `.xmp` recipe files on disk.
    /// Returns the new rating value.
    pub fn set_rating(&self, asset_id_prefix: &str, rating: Option<u8>) -> Result<Option<u8>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);
        let online = Self::load_online_volumes(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let ctx = BatchContext { catalog, meta_store: store, online_volumes: online, content_store };
        self.set_rating_inner(&ctx, asset_id_prefix, rating)
    }

    fn set_rating_inner(&self, ctx: &BatchContext, asset_id_prefix: &str, rating: Option<u8>) -> Result<Option<u8>> {
        let full_id = ctx.catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let mut asset = ctx.meta_store.load(uuid)?;

        asset.rating = rating;
        ctx.meta_store.save(&asset)?;
        ctx.catalog.update_asset_rating(&full_id, rating)?;

        self.write_back_rating_to_xmp_inner(&mut asset, rating, &ctx.catalog, &ctx.meta_store, &ctx.online_volumes, &ctx.content_store);

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
        if !self.is_writeback_enabled() { return; }
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP write-back: {e}");
                return;
            }
        };
        let online: HashMap<uuid::Uuid, PathBuf> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.clone()))
            .collect();
        let content_store = ContentStore::new(&self.catalog_root);
        self.write_back_rating_to_xmp_inner(asset, rating, catalog, store, &online, &content_store);
    }

    fn write_back_rating_to_xmp_inner(
        &self,
        asset: &mut Asset,
        rating: Option<u8>,
        catalog: &Catalog,
        store: &MetadataStore,
        online: &HashMap<uuid::Uuid, PathBuf>,
        content_store: &ContentStore,
    ) {
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
                Some(mp) => mp.as_path(),
                None => {
                    Self::mark_recipe_pending(recipe, catalog);
                    sidecar_dirty = true;
                    continue;
                }
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                Self::mark_recipe_pending(recipe, catalog);
                sidecar_dirty = true;
                continue;
            }

            match xmp_reader::update_rating(&full_path, rating) {
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
                            if recipe.pending_writeback {
                                Self::clear_recipe_pending(recipe, catalog);
                            }
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {
                    if recipe.pending_writeback {
                        Self::clear_recipe_pending(recipe, catalog);
                        sidecar_dirty = true;
                    }
                }
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
    fn write_back_tags_to_xmp_inner(
        &self,
        asset: &mut Asset,
        tags_to_add: &[String],
        tags_to_remove: &[String],
        catalog: &Catalog,
        store: &MetadataStore,
        online: &HashMap<uuid::Uuid, PathBuf>,
        content_store: &ContentStore,
    ) {
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
                Some(mp) => mp.as_path(),
                None => {
                    Self::mark_recipe_pending(recipe, catalog);
                    sidecar_dirty = true;
                    continue;
                }
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                Self::mark_recipe_pending(recipe, catalog);
                sidecar_dirty = true;
                continue;
            }

            let dc_add: Vec<String> = tags_to_add.iter().map(|t| t.replace('|', "/")).collect();
            let dc_remove: Vec<String> =
                tags_to_remove.iter().map(|t| t.replace('|', "/")).collect();
            let changed_dc = match xmp_reader::update_tags(&full_path, &dc_add, &dc_remove)
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Warning: could not write tags to {}: {e}",
                        full_path.display()
                    );
                    false
                }
            };
            let changed_lr =
                match xmp_reader::update_hierarchical_subjects(&full_path, tags_to_add, tags_to_remove)
                {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!(
                            "Warning: could not write hierarchical subjects to {}: {e}",
                            full_path.display()
                        );
                        false
                    }
                };
            if changed_dc || changed_lr {
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
                        if recipe.pending_writeback {
                            Self::clear_recipe_pending(recipe, catalog);
                        }
                        sidecar_dirty = true;
                    }
                    Err(e) => {
                        eprintln!("Warning: could not re-hash XMP file: {e}");
                    }
                }
            } else if recipe.pending_writeback {
                Self::clear_recipe_pending(recipe, catalog);
                sidecar_dirty = true;
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
        let store = MetadataStore::new(&self.catalog_root);
        let online = Self::load_online_volumes(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        let ctx = BatchContext { catalog, meta_store: store, online_volumes: online, content_store };
        self.set_color_label_inner(&ctx, asset_id_prefix, label)
    }

    fn set_color_label_inner(&self, ctx: &BatchContext, asset_id_prefix: &str, label: Option<String>) -> Result<Option<String>> {
        let full_id = ctx.catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let mut asset = ctx.meta_store.load(uuid)?;

        asset.color_label = label.clone();
        ctx.meta_store.save(&asset)?;
        ctx.catalog.update_asset_color_label(&full_id, label.as_deref())?;

        self.write_back_label_to_xmp_inner(&mut asset, label.as_deref(), &ctx.catalog, &ctx.meta_store, &ctx.online_volumes, &ctx.content_store);

        Ok(label)
    }

    /// Set the preview rotation override on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Returns the new rotation value.
    pub fn set_preview_rotation(
        &self,
        asset_id_prefix: &str,
        rotation: Option<u16>,
    ) -> Result<Option<u16>> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.preview_rotation = rotation;
        store.save(&asset)?;
        catalog.update_asset_preview_rotation(&full_id, rotation)?;

        Ok(rotation)
    }

    /// Set the preview variant override on an asset. Updates both sidecar YAML and SQLite catalog.
    /// Pass `None` to clear the override and revert to algorithmic selection.
    pub fn set_preview_variant(
        &self,
        asset_id_prefix: &str,
        content_hash: Option<&str>,
    ) -> Result<()> {
        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        // Validate that the content_hash belongs to this asset
        if let Some(hash) = content_hash {
            let details = self.show(&full_id)?;
            if !details.variants.iter().any(|v| v.content_hash == hash) {
                anyhow::bail!("Variant {hash} does not belong to asset {full_id}");
            }
        }

        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        asset.preview_variant = content_hash.map(|s| s.to_string());
        store.save(&asset)?;
        catalog.update_asset_preview_variant(&full_id, content_hash)?;

        Ok(())
    }

    /// Change a variant's role. Updates both sidecar YAML and SQLite catalog.
    /// Also updates denormalized columns (primary_format, best_variant_hash).
    pub fn set_variant_role(
        &self,
        asset_id_prefix: &str,
        variant_hash: &str,
        role: &str,
    ) -> Result<()> {
        let valid_roles = ["original", "alternate", "processed", "export", "sidecar"];
        let role_lower = role.to_lowercase();
        if !valid_roles.contains(&role_lower.as_str()) {
            anyhow::bail!(
                "Invalid role '{role}'. Valid roles: {}",
                valid_roles.join(", ")
            );
        }

        let catalog = Catalog::open(&self.catalog_root)?;
        let full_id = catalog
            .resolve_asset_id(asset_id_prefix)?
            .ok_or_else(|| anyhow::anyhow!("No asset found matching '{asset_id_prefix}'"))?;

        // Verify variant belongs to this asset
        let details = self.show(&full_id)?;
        if !details.variants.iter().any(|v| v.content_hash == variant_hash) {
            anyhow::bail!("Variant {variant_hash} does not belong to asset {full_id}");
        }

        // Update sidecar
        let uuid: uuid::Uuid = full_id.parse()?;
        let store = MetadataStore::new(&self.catalog_root);
        let mut asset = store.load(uuid)?;

        let variant_role = match role_lower.as_str() {
            "original" => crate::models::VariantRole::Original,
            "alternate" => crate::models::VariantRole::Alternate,
            "processed" => crate::models::VariantRole::Processed,
            "export" => crate::models::VariantRole::Export,
            "sidecar" => crate::models::VariantRole::Sidecar,
            _ => unreachable!(),
        };

        if let Some(v) = asset.variants.iter_mut().find(|v| v.content_hash == variant_hash) {
            v.role = variant_role;
        }

        store.save(&asset)?;

        // Update catalog
        catalog.update_variant_role(variant_hash, &role_lower)?;
        catalog.update_denormalized_variant_columns(&asset)?;

        Ok(())
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
        if !self.is_writeback_enabled() { return; }
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
                None => {
                    Self::mark_recipe_pending(recipe, catalog);
                    sidecar_dirty = true;
                    continue;
                }
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                Self::mark_recipe_pending(recipe, catalog);
                sidecar_dirty = true;
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
                            if recipe.pending_writeback {
                                Self::clear_recipe_pending(recipe, catalog);
                            }
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {
                    if recipe.pending_writeback {
                        Self::clear_recipe_pending(recipe, catalog);
                        sidecar_dirty = true;
                    }
                }
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
        if !self.is_writeback_enabled() { return; }
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = match registry.list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Warning: could not load volumes for XMP label write-back: {e}");
                return;
            }
        };
        let online: HashMap<uuid::Uuid, PathBuf> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.clone()))
            .collect();
        let content_store = ContentStore::new(&self.catalog_root);
        self.write_back_label_to_xmp_inner(asset, label, catalog, store, &online, &content_store);
    }

    fn write_back_label_to_xmp_inner(
        &self,
        asset: &mut Asset,
        label: Option<&str>,
        catalog: &Catalog,
        store: &MetadataStore,
        online: &HashMap<uuid::Uuid, PathBuf>,
        content_store: &ContentStore,
    ) {
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
                Some(mp) => mp.as_path(),
                None => {
                    Self::mark_recipe_pending(recipe, catalog);
                    sidecar_dirty = true;
                    continue;
                }
            };

            let full_path = mount_point.join(&recipe.location.relative_path);
            if !full_path.exists() {
                Self::mark_recipe_pending(recipe, catalog);
                sidecar_dirty = true;
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
                            if recipe.pending_writeback {
                                Self::clear_recipe_pending(recipe, catalog);
                            }
                            sidecar_dirty = true;
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash XMP file: {e}");
                        }
                    }
                }
                Ok(false) => {
                    if recipe.pending_writeback {
                        Self::clear_recipe_pending(recipe, catalog);
                        sidecar_dirty = true;
                    }
                }
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

    /// Write back pending metadata changes to XMP recipe files.
    ///
    /// For each recipe with `pending_writeback=1`, reads the current asset metadata
    /// (rating, label, tags, description) and writes all four fields to the XMP file.
    /// Clears the pending flag on success.
    ///
    /// `all=true` writes back all XMP recipes regardless of pending flag.
    pub fn writeback(
        &self,
        volume_filter: Option<&str>,
        asset_filter: Option<&str>,
        asset_id_set: Option<&HashSet<String>>,
        all: bool,
        dry_run: bool,
        log: bool,
        callback: Option<&dyn Fn(&str, &str)>,
    ) -> Result<WritebackResult> {
        if !dry_run && !self.is_writeback_enabled() {
            anyhow::bail!(
                "XMP writeback is disabled. To enable, add to maki.toml:\n\n  \
                 [writeback]\n  enabled = true\n\n  \
                 Warning: this will modify .xmp recipe files on your volumes."
            );
        }
        let catalog = Catalog::open(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);
        let registry = DeviceRegistry::new(&self.catalog_root);
        let volumes = registry.list()?;
        let online: HashMap<uuid::Uuid, PathBuf> = volumes
            .iter()
            .filter(|v| v.is_online)
            .map(|v| (v.id, v.mount_point.clone()))
            .collect();
        let content_store = ContentStore::new(&self.catalog_root);

        // Resolve volume filter to volume ID
        let volume_id_filter: Option<String> = if let Some(label) = volume_filter {
            let vol = volumes.iter().find(|v| v.label == label)
                .ok_or_else(|| anyhow::anyhow!("Unknown volume: {label}"))?;
            Some(vol.id.to_string())
        } else {
            None
        };

        // Collect recipes to process
        let pending_recipes: Vec<(String, String, String, String)> = if all {
            // All XMP recipes (optionally filtered by volume)
            let sql = if volume_id_filter.is_some() {
                "SELECT r.id, v.asset_id, r.volume_id, r.relative_path \
                 FROM recipes r \
                 JOIN variants v ON r.variant_hash = v.content_hash \
                 WHERE r.volume_id = ?1 AND LOWER(r.relative_path) LIKE '%.xmp'"
            } else {
                "SELECT r.id, v.asset_id, r.volume_id, r.relative_path \
                 FROM recipes r \
                 JOIN variants v ON r.variant_hash = v.content_hash \
                 WHERE LOWER(r.relative_path) LIKE '%.xmp'"
            };
            let mut stmt = catalog.conn().prepare(sql)?;
            let params: Vec<Box<dyn rusqlite::types::ToSql>> = if let Some(ref vid) = volume_id_filter {
                vec![Box::new(vid.clone())]
            } else {
                vec![]
            };
            let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            let mut result = Vec::new();
            for row in rows { result.push(row?); }
            result
        } else {
            catalog.list_pending_writeback_recipes(volume_id_filter.as_deref())?
        };

        self.writeback_process(pending_recipes, &catalog, &store, &online, &content_store, asset_filter, asset_id_set, dry_run, log, callback)
    }

    /// Process a list of recipes for writeback. Each tuple is (recipe_id, asset_id, volume_id, relative_path).
    pub fn writeback_process(
        &self,
        recipes: Vec<(String, String, String, String)>,
        catalog: &Catalog,
        store: &MetadataStore,
        online: &HashMap<uuid::Uuid, PathBuf>,
        content_store: &ContentStore,
        asset_filter: Option<&str>,
        asset_id_set: Option<&HashSet<String>>,
        dry_run: bool,
        log: bool,
        callback: Option<&dyn Fn(&str, &str)>,
    ) -> Result<WritebackResult> {
        if !dry_run && !self.is_writeback_enabled() {
            anyhow::bail!(
                "XMP writeback is disabled. To enable, add to maki.toml:\n\n  \
                 [writeback]\n  enabled = true\n\n  \
                 Warning: this will modify .xmp recipe files on your volumes."
            );
        }
        let mut result = WritebackResult::default();
        result.dry_run = dry_run;

        // Group by asset_id
        let mut by_asset: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        for (recipe_id, asset_id, volume_id, rel_path) in recipes {
            if let Some(prefix) = asset_filter {
                if !asset_id.starts_with(prefix) {
                    continue;
                }
            }
            if let Some(set) = asset_id_set {
                if !set.contains(&asset_id) {
                    continue;
                }
            }
            by_asset.entry(asset_id).or_default().push((recipe_id, volume_id, rel_path));
        }

        for (asset_id, recipe_entries) in &by_asset {
            let asset_uuid: uuid::Uuid = match asset_id.parse() {
                Ok(u) => u,
                Err(_) => {
                    result.errors.push(format!("Invalid asset ID: {asset_id}"));
                    result.failed += recipe_entries.len() as u32;
                    continue;
                }
            };
            let mut asset = match store.load(asset_uuid) {
                Ok(a) => a,
                Err(e) => {
                    result.errors.push(format!("Could not load asset {asset_id}: {e}"));
                    result.failed += recipe_entries.len() as u32;
                    continue;
                }
            };

            for (recipe_id, volume_id, rel_path) in recipe_entries {
                let vol_uuid: uuid::Uuid = match volume_id.parse() {
                    Ok(u) => u,
                    Err(_) => {
                        result.errors.push(format!("Invalid volume ID: {volume_id}"));
                        result.failed += 1;
                        continue;
                    }
                };

                let mount_point = match online.get(&vol_uuid) {
                    Some(mp) => mp.as_path(),
                    None => {
                        result.skipped += 1;
                        if log {
                            eprintln!("{rel_path} — skipped (volume offline)");
                        }
                        continue;
                    }
                };

                let full_path = mount_point.join(rel_path);
                if !full_path.exists() {
                    result.skipped += 1;
                    if log {
                        eprintln!("{rel_path} — skipped (file missing)");
                    }
                    continue;
                }

                if dry_run {
                    result.written += 1;
                    if log {
                        eprintln!("{rel_path} — would write back");
                    }
                    if let Some(cb) = callback {
                        cb(rel_path, "would write back");
                    }
                    continue;
                }

                // Write all four metadata fields
                let mut file_changed = false;

                if let Ok(true) = xmp_reader::update_rating(&full_path, asset.rating) {
                    file_changed = true;
                }
                if let Ok(true) = xmp_reader::update_label(
                    &full_path,
                    asset.color_label.as_deref(),
                ) {
                    file_changed = true;
                }
                if let Ok(true) = xmp_reader::update_description(
                    &full_path,
                    asset.description.as_deref(),
                ) {
                    file_changed = true;
                }
                // Tags: write the full current tag set as additions (no removals)
                let dc_tags: Vec<String> = asset.tags.iter().map(|t: &String| t.replace('|', "/")).collect();
                if !dc_tags.is_empty() {
                    if let Ok(true) = xmp_reader::update_tags(&full_path, &dc_tags, &[]) {
                        file_changed = true;
                    }
                    let _ = xmp_reader::update_hierarchical_subjects(&full_path, &asset.tags, &[]);
                }

                if file_changed {
                    match content_store.hash_file(&full_path) {
                        Ok(new_hash) => {
                            let _ = catalog.update_recipe_content_hash(recipe_id, &new_hash);
                            // Update the in-memory recipe too
                            if let Some(r) = asset.recipes.iter_mut().find(|r| r.id.to_string() == *recipe_id) {
                                r.content_hash = new_hash;
                                r.pending_writeback = false;
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: could not re-hash {}: {e}", full_path.display());
                        }
                    }
                }

                let _ = catalog.clear_pending_writeback(recipe_id);
                result.written += 1;
                if log {
                    eprintln!("{rel_path} — written");
                }
                if let Some(cb) = callback {
                    cb(rel_path, "written");
                }
            }

            // Save sidecar with cleared pending flags
            if !dry_run {
                // Clear pending_writeback on all processed recipes
                for r in &mut asset.recipes {
                    if recipe_entries.iter().any(|(rid, _, _)| r.id.to_string() == *rid) {
                        r.pending_writeback = false;
                    }
                }
                if let Err(e) = store.save(&asset) {
                    eprintln!("Warning: could not save sidecar for {asset_id}: {e}");
                }
            }
        }

        Ok(result)
    }

    /// Convert tags matching a pattern into stacks.
    /// Pattern uses `{}` as a wildcard placeholder (e.g. `"Aperture Stack {}"`).
    /// Default is report-only (dry run); pass `apply=true` to execute.
    pub fn stack_from_tag(
        &self,
        pattern: &str,
        remove_tags: bool,
        apply: bool,
        log: bool,
    ) -> Result<FromTagResult> {
        if !pattern.contains("{}") {
            anyhow::bail!("Pattern must contain '{{}}' as a wildcard placeholder");
        }

        // Build regex: escape metacharacters, replace {} with (.+), anchor
        let escaped = regex::escape(pattern).replace("\\{\\}", "(.+)");
        let re = regex::Regex::new(&format!("^{escaped}$"))?;

        let catalog = Catalog::open(&self.catalog_root)?;
        let all_tags = catalog.list_all_tags()?;

        let mut matching_tags: Vec<String> = all_tags
            .iter()
            .filter(|(tag, _)| re.is_match(tag))
            .map(|(tag, _)| tag.clone())
            .collect();
        matching_tags.sort();

        let mut result = FromTagResult {
            tags_matched: matching_tags.len() as u32,
            tags_skipped: 0,
            stacks_created: 0,
            assets_stacked: 0,
            assets_skipped: 0,
            tags_removed: 0,
            dry_run: !apply,
            details: Vec::new(),
        };

        let store = crate::stack::StackStore::new(catalog.conn());

        for tag in &matching_tags {
            let assets = catalog.assets_with_exact_tag(tag)?;
            let total_found = assets.len() as u32;

            // Partition into stacked and unstacked
            let (already_stacked, unstacked): (Vec<_>, Vec<_>) =
                assets.into_iter().partition(|(_, stack_id)| stack_id.is_some());

            let skipped = already_stacked.len() as u32;

            if unstacked.len() < 2 {
                result.tags_skipped += 1;
                if log {
                    eprintln!(
                        "{} — skipped ({} unstacked, {} already stacked)",
                        tag,
                        unstacked.len(),
                        skipped
                    );
                }
                result.details.push(FromTagDetail {
                    tag: tag.clone(),
                    assets_found: total_found,
                    assets_stacked: 0,
                    assets_skipped: skipped,
                    stack_id: None,
                });
                continue;
            }

            let unstacked_ids: Vec<String> = unstacked.into_iter().map(|(id, _)| id).collect();
            let stacked_count = unstacked_ids.len() as u32;
            let mut stack_id_str = None;

            if apply {
                let stack = store.create(&unstacked_ids)?;
                stack_id_str = Some(stack.id.to_string());

                if remove_tags {
                    for id in &unstacked_ids {
                        let _ = self.tag(id, &[tag.clone()], true);
                        result.tags_removed += 1;
                    }
                }
            }

            result.stacks_created += 1;
            result.assets_stacked += stacked_count;
            result.assets_skipped += skipped;

            if log {
                let action = if apply { "stacked" } else { "would stack" };
                eprintln!(
                    "{} — {} {} assets (skipped {})",
                    tag, action, stacked_count, skipped
                );
            }

            result.details.push(FromTagDetail {
                tag: tag.clone(),
                assets_found: total_found,
                assets_stacked: stacked_count,
                assets_skipped: skipped,
                stack_id: stack_id_str,
            });
        }

        if apply {
            let yaml = store.export_all()?;
            crate::stack::save_yaml(&self.catalog_root, &yaml)?;
        }

        Ok(result)
    }

    // --- Batch methods (shared catalog/registry/content_store) ---

    /// Load online volume mount points. Returns empty map if no volumes registered.
    fn load_online_volumes(catalog_root: &Path) -> HashMap<uuid::Uuid, PathBuf> {
        let registry = DeviceRegistry::new(catalog_root);
        match registry.list() {
            Ok(volumes) => volumes
                .iter()
                .filter(|v| v.is_online)
                .map(|v| (v.id, v.mount_point.clone()))
                .collect(),
            Err(_) => HashMap::new(),
        }
    }

    /// Mark a recipe as pending write-back in SQLite and set the flag on the struct.
    /// The caller must save the sidecar YAML.
    fn mark_recipe_pending(recipe: &mut Recipe, catalog: &Catalog) {
        if !recipe.pending_writeback {
            recipe.pending_writeback = true;
            let _ = catalog.mark_pending_writeback(&recipe.id.to_string());
        }
    }

    /// Clear pending write-back flag after successful XMP write.
    /// The caller must save the sidecar YAML.
    fn clear_recipe_pending(recipe: &mut Recipe, catalog: &Catalog) {
        if recipe.pending_writeback {
            recipe.pending_writeback = false;
            let _ = catalog.clear_pending_writeback(&recipe.id.to_string());
        }
    }

    /// Create a `BatchContext` using `open_fast()` for use by batch web handlers.
    fn batch_context_fast(&self) -> Result<BatchContext> {
        let catalog = Catalog::open_fast(&self.catalog_root)?;
        let store = MetadataStore::new(&self.catalog_root);
        let online = Self::load_online_volumes(&self.catalog_root);
        let content_store = ContentStore::new(&self.catalog_root);
        Ok(BatchContext { catalog, meta_store: store, online_volumes: online, content_store })
    }

    /// Tag multiple assets using a single shared catalog connection.
    pub fn batch_tag(&self, asset_ids: &[String], tags: &[String], remove: bool) -> Vec<Result<TagResult>> {
        let ctx = match self.batch_context_fast() {
            Ok(c) => c,
            Err(e) => return asset_ids.iter().map(|_| Err(anyhow::anyhow!("{e:#}"))).collect(),
        };
        asset_ids.iter().map(|id| self.tag_inner(&ctx, id, tags, remove)).collect()
    }

    /// Set rating on multiple assets using a single shared catalog connection.
    pub fn batch_set_rating(&self, asset_ids: &[String], rating: Option<u8>) -> Vec<Result<Option<u8>>> {
        let ctx = match self.batch_context_fast() {
            Ok(c) => c,
            Err(e) => return asset_ids.iter().map(|_| Err(anyhow::anyhow!("{e:#}"))).collect(),
        };
        asset_ids.iter().map(|id| self.set_rating_inner(&ctx, id, rating)).collect()
    }

    /// Set color label on multiple assets using a single shared catalog connection.
    pub fn batch_set_color_label(&self, asset_ids: &[String], label: Option<String>) -> Vec<Result<Option<String>>> {
        let ctx = match self.batch_context_fast() {
            Ok(c) => c,
            Err(e) => return asset_ids.iter().map(|_| Err(anyhow::anyhow!("{e:#}"))).collect(),
        };
        asset_ids.iter().map(|id| self.set_color_label_inner(&ctx, id, label.clone())).collect()
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

        // Original variant keeps its role, donor variant becomes alternate
        let original = details.variants.iter().find(|v| v.content_hash == "sha256:hash1").unwrap();
        assert_eq!(original.role, "original");
        let moved = details.variants.iter().find(|v| v.content_hash == "sha256:hash2").unwrap();
        assert_eq!(moved.role, "alternate");

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
    fn group_merges_best_rating() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        // Asset 1: rating 3, no label, no description
        let mut asset1 = Asset::new(AssetType::Image, "sha256:rate1");
        asset1.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
        asset1.rating = Some(3);
        let v1 = Variant {
            content_hash: "sha256:rate1".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "arw".to_string(),
            file_size: 25_000_000,
            original_filename: "DSC_001.ARW".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(v1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&v1).unwrap();
        store.save(&asset1).unwrap();

        // Asset 2: rating 5, color label Red, description "Great shot"
        let mut asset2 = Asset::new(AssetType::Image, "sha256:rate2");
        asset2.rating = Some(5);
        asset2.color_label = Some("Red".to_string());
        asset2.description = Some("Great shot".to_string());
        let v2 = Variant {
            content_hash: "sha256:rate2".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5_000_000,
            original_filename: "DSC_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(v2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&v2).unwrap();
        store.save(&asset2).unwrap();

        let id1 = asset1.id.to_string();
        let engine = QueryEngine::new(dir.path());
        engine.group(&["sha256:rate1".to_string(), "sha256:rate2".to_string()]).unwrap();

        let details = engine.show(&id1).unwrap();
        // Highest rating wins
        assert_eq!(details.rating, Some(5));
        // First non-None color label
        assert_eq!(details.color_label.as_deref(), Some("Red"));
        // First non-None description
        assert_eq!(details.description.as_deref(), Some("Great shot"));
    }

    #[test]
    fn group_keeps_target_metadata_when_higher() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        // Asset 1 (target, older): rating 5, label Blue, description "Target desc"
        let mut asset1 = Asset::new(AssetType::Image, "sha256:keep1");
        asset1.created_at = chrono::Utc::now() - chrono::Duration::hours(2);
        asset1.rating = Some(5);
        asset1.color_label = Some("Blue".to_string());
        asset1.description = Some("Target desc".to_string());
        let v1 = Variant {
            content_hash: "sha256:keep1".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "arw".to_string(),
            file_size: 25_000_000,
            original_filename: "IMG_001.ARW".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(v1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&v1).unwrap();
        store.save(&asset1).unwrap();

        // Asset 2 (donor): rating 2, label Red, description "Donor desc"
        let mut asset2 = Asset::new(AssetType::Image, "sha256:keep2");
        asset2.rating = Some(2);
        asset2.color_label = Some("Red".to_string());
        asset2.description = Some("Donor desc".to_string());
        let v2 = Variant {
            content_hash: "sha256:keep2".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 5_000_000,
            original_filename: "IMG_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(v2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&v2).unwrap();
        store.save(&asset2).unwrap();

        let id1 = asset1.id.to_string();
        let engine = QueryEngine::new(dir.path());
        engine.group(&["sha256:keep1".to_string(), "sha256:keep2".to_string()]).unwrap();

        let details = engine.show(&id1).unwrap();
        // Target had higher rating — keep it
        assert_eq!(details.rating, Some(5));
        // Target already had label — keep it
        assert_eq!(details.color_label.as_deref(), Some("Blue"));
        // Target already had description — keep it
        assert_eq!(details.description.as_deref(), Some("Target desc"));
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
        assert_eq!(p.cameras, vec!["fuji"]);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_lens_filter() {
        let p = parse_search_query("lens:56mm");
        assert_eq!(p.lenses, vec!["56mm"]);
    }

    #[test]
    fn parse_iso_exact() {
        let p = parse_search_query("iso:3200");
        assert_eq!(p.iso, Some(NumericFilter::Exact(3200.0)));
    }

    #[test]
    fn parse_iso_min() {
        let p = parse_search_query("iso:3200+");
        assert_eq!(p.iso, Some(NumericFilter::Min(3200.0)));
    }

    #[test]
    fn parse_iso_range() {
        let p = parse_search_query("iso:100-800");
        assert_eq!(p.iso, Some(NumericFilter::Range(100.0, 800.0)));
    }

    #[test]
    fn parse_focal_exact() {
        let p = parse_search_query("focal:50");
        assert_eq!(p.focal, Some(NumericFilter::Exact(50.0)));
    }

    #[test]
    fn parse_focal_range() {
        let p = parse_search_query("focal:35-70");
        assert_eq!(p.focal, Some(NumericFilter::Range(35.0, 70.0)));
    }

    #[test]
    fn parse_f_exact() {
        let p = parse_search_query("f:2.8");
        assert_eq!(p.aperture, Some(NumericFilter::Exact(2.8)));
    }

    #[test]
    fn parse_f_min() {
        let p = parse_search_query("f:2.8+");
        assert_eq!(p.aperture, Some(NumericFilter::Min(2.8)));
    }

    #[test]
    fn parse_f_range() {
        let p = parse_search_query("f:1.4-2.8");
        assert_eq!(p.aperture, Some(NumericFilter::Range(1.4, 2.8)));
    }

    #[test]
    fn parse_width_min() {
        let p = parse_search_query("width:4000+");
        assert_eq!(p.width, Some(NumericFilter::Min(4000.0)));
    }

    #[test]
    fn parse_height_min() {
        let p = parse_search_query("height:2000+");
        assert_eq!(p.height, Some(NumericFilter::Min(2000.0)));
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
        assert_eq!(p.cameras, vec!["fuji"]);
        assert_eq!(p.iso, Some(NumericFilter::Exact(400.0)));
        assert_eq!(p.text.as_deref(), Some("sunset landscape"));
    }

    #[test]
    fn parse_existing_filters_still_work() {
        let p = parse_search_query("type:image tag:nature format:jpg rating:3+");
        assert_eq!(p.asset_types, vec!["image"]);
        assert_eq!(p.tags, vec!["nature"]);
        assert_eq!(p.formats, vec!["jpg"]);
        assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
    }

    #[test]
    fn parse_quoted_tag_with_spaces() {
        let p = parse_search_query(r#"tag:"Fools Theater" rating:4+"#);
        assert_eq!(p.tags, vec!["Fools Theater"]);
        assert_eq!(p.rating, Some(NumericFilter::Min(4.0)));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_quoted_camera_and_lens() {
        let p = parse_search_query(r#"camera:"Canon EOS R5" lens:"RF 50mm f/1.2""#);
        assert_eq!(p.cameras, vec!["Canon EOS R5"]);
        assert_eq!(p.lenses, vec!["RF 50mm f/1.2"]);
    }

    #[test]
    fn parse_quoted_label() {
        let p = parse_search_query(r#"label:"light blue" type:image"#);
        assert_eq!(p.color_labels, vec!["light blue"]);
        assert_eq!(p.asset_types, vec!["image"]);
    }

    #[test]
    fn parse_quoted_collection() {
        let p = parse_search_query(r#"collection:"My Favorites""#);
        assert_eq!(p.collections, vec!["My Favorites"]);
    }

    #[test]
    fn parse_mixed_quoted_and_unquoted() {
        let p = parse_search_query(r#"sunset tag:"Fools Theater" rating:5"#);
        assert_eq!(p.tags, vec!["Fools Theater"]);
        assert_eq!(p.rating, Some(NumericFilter::Exact(5.0)));
        assert_eq!(p.text.as_deref(), Some("sunset"));
    }

    #[test]
    fn tokenize_basic() {
        assert_eq!(tokenize_query("hello world"), vec!["hello", "world"]);
        assert_eq!(tokenize_query(r#"tag:"two words""#), vec!["tag:two words"]);
        assert_eq!(
            tokenize_query(r#"tag:"a b" rating:3+"#),
            vec!["tag:a b", "rating:3+"]
        );
        // Unmatched quote: consumes rest of input
        assert_eq!(tokenize_query(r#"tag:"open"#), vec!["tag:open"]);
        // Empty input
        assert!(tokenize_query("").is_empty());
        assert!(tokenize_query("   ").is_empty());
    }

    #[test]
    fn parse_orphan_filter() {
        let p = parse_search_query("orphan:true");
        assert!(p.orphan);
        assert!(!p.orphan_false);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_orphan_false_filter() {
        let p = parse_search_query("orphan:false");
        assert!(!p.orphan);
        assert!(p.orphan_false);
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
        assert_eq!(p.stale_days, Some(NumericFilter::Exact(30.0)));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_stale_filter_zero() {
        let p = parse_search_query("stale:0");
        assert_eq!(p.stale_days, Some(NumericFilter::Exact(0.0)));
    }

    #[test]
    fn parse_volume_none_filter() {
        let p = parse_search_query("volume:none");
        assert!(p.volume_none);
        assert!(p.volumes.is_empty());
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_volume_label_filter() {
        let p = parse_search_query("volume:Photos");
        assert!(!p.volume_none);
        assert_eq!(p.volumes, vec!["Photos"]);
        assert!(p.volumes_exclude.is_empty());
    }

    #[test]
    fn parse_volume_label_negated() {
        let p = parse_search_query("-volume:Archive");
        assert_eq!(p.volumes_exclude, vec!["Archive"]);
        assert!(p.volumes.is_empty());
    }

    #[test]
    fn parse_volume_label_comma_or() {
        let p = parse_search_query("volume:Photos,Archive");
        assert_eq!(p.volumes, vec!["Photos,Archive"]);
    }

    #[test]
    fn parse_volume_label_with_other_filters() {
        let p = parse_search_query("volume:Working type:image rating:3+");
        assert_eq!(p.volumes, vec!["Working"]);
        assert_eq!(p.asset_types, vec!["image"]);
        assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
    }

    #[test]
    fn parse_volume_quoted_label() {
        let p = parse_search_query("volume:\"External SSD\" type:image");
        assert_eq!(p.volumes, vec!["External SSD"]);
        assert_eq!(p.asset_types, vec!["image"]);
    }

    #[test]
    fn parse_location_health_combined() {
        let p = parse_search_query("orphan:true stale:7 tag:landscape");
        assert!(p.orphan);
        assert_eq!(p.stale_days, Some(NumericFilter::Exact(7.0)));
        assert_eq!(p.tags, vec!["landscape"]);
        assert!(!p.missing);
        assert!(!p.volume_none);
    }

    #[test]
    fn parse_label_filter() {
        let p = parse_search_query("label:Red");
        assert_eq!(p.color_labels, vec!["Red"]);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_label_with_other_filters() {
        let p = parse_search_query("label:Blue tag:landscape sunset");
        assert_eq!(p.color_labels, vec!["Blue"]);
        assert_eq!(p.tags, vec!["landscape"]);
        assert_eq!(p.text.as_deref(), Some("sunset"));
    }

    #[test]
    fn parse_path_filter() {
        let p = parse_search_query("path:Capture/2026-02-22");
        assert_eq!(p.path_prefixes, vec!["Capture/2026-02-22"]);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_path_filter_quoted() {
        let p = parse_search_query(r#"path:"Photos/My Trip""#);
        assert_eq!(p.path_prefixes, vec!["Photos/My Trip"]);
    }

    #[test]
    fn parse_path_with_other_filters() {
        let p = parse_search_query("path:Capture/2026 rating:3+ tag:landscape");
        assert_eq!(p.path_prefixes, vec!["Capture/2026"]);
        assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
        assert_eq!(p.tags, vec!["landscape"]);
        assert!(p.text.is_none());
    }

    // ── copies filter parse tests ─────────────────────────────────

    #[test]
    fn parse_copies_exact() {
        let p = parse_search_query("copies:2");
        assert_eq!(p.copies, Some(NumericFilter::Exact(2.0)));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_copies_min() {
        let p = parse_search_query("copies:2+");
        assert_eq!(p.copies, Some(NumericFilter::Min(2.0)));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_copies_with_other_filters() {
        let p = parse_search_query("copies:3+ rating:4+ tag:landscape");
        assert_eq!(p.copies, Some(NumericFilter::Min(3.0)));
        assert_eq!(p.rating, Some(NumericFilter::Min(4.0)));
        assert_eq!(p.tags, vec!["landscape"]);
    }

    // ── variants filter parse tests ─────────────────────────────────

    #[test]
    fn parse_variants_exact() {
        let p = parse_search_query("variants:3");
        assert_eq!(p.variant_count, Some(NumericFilter::Exact(3.0)));
    }

    #[test]
    fn parse_variants_min() {
        let p = parse_search_query("variants:3+");
        assert_eq!(p.variant_count, Some(NumericFilter::Min(3.0)));
    }

    #[test]
    fn parse_variants_with_other_filters() {
        let p = parse_search_query("variants:5+ tag:landscape");
        assert_eq!(p.variant_count, Some(NumericFilter::Min(5.0)));
        assert_eq!(p.tags, vec!["landscape"]);
    }

    // ── scattered filter parse tests ─────────────────────────────────

    #[test]
    fn parse_scattered() {
        let p = parse_search_query("scattered:2");
        assert_eq!(p.scattered, Some(NumericFilter::Exact(2.0)));
    }

    #[test]
    fn parse_scattered_with_variants() {
        let p = parse_search_query("scattered:2 variants:3+");
        assert_eq!(p.scattered, Some(NumericFilter::Exact(2.0)));
        assert_eq!(p.variant_count, Some(NumericFilter::Min(3.0)));
    }

    #[test]
    fn parse_scattered_with_plus_suffix() {
        let p = parse_search_query("scattered:2+");
        assert_eq!(p.scattered, Some(NumericFilter::Min(2.0)));
        assert_eq!(p.scattered_depth, None);
    }

    #[test]
    fn parse_scattered_with_depth() {
        let p = parse_search_query("scattered:2+/3");
        assert_eq!(p.scattered, Some(NumericFilter::Min(2.0)));
        assert_eq!(p.scattered_depth, Some(3));
    }

    #[test]
    fn parse_scattered_exact_with_depth() {
        let p = parse_search_query("scattered:2/1");
        assert_eq!(p.scattered, Some(NumericFilter::Exact(2.0)));
        assert_eq!(p.scattered_depth, Some(1));
    }

    // ── duration filter parse tests ──────────────────────────────────

    #[test]
    fn parse_duration_exact() {
        let p = parse_search_query("duration:60");
        assert_eq!(p.duration, Some(NumericFilter::Exact(60.0)));
    }

    #[test]
    fn parse_duration_min() {
        let p = parse_search_query("duration:30+");
        assert_eq!(p.duration, Some(NumericFilter::Min(30.0)));
    }

    #[test]
    fn parse_duration_range() {
        let p = parse_search_query("duration:10-120");
        assert_eq!(p.duration, Some(NumericFilter::Range(10.0, 120.0)));
    }

    #[test]
    fn parse_duration_with_type() {
        let p = parse_search_query("duration:60+ type:video");
        assert_eq!(p.duration, Some(NumericFilter::Min(60.0)));
        assert_eq!(p.asset_types, vec!["video"]);
    }

    // ── codec filter parse tests ──────────────────────────────────────

    #[test]
    fn parse_codec() {
        let p = parse_search_query("codec:h264");
        assert_eq!(p.codec, Some("h264".to_string()));
    }

    #[test]
    fn parse_codec_with_duration() {
        let p = parse_search_query("codec:hevc duration:60+");
        assert_eq!(p.codec, Some("hevc".to_string()));
        assert_eq!(p.duration, Some(NumericFilter::Min(60.0)));
    }

    #[test]
    fn parse_rating_comma_separated() {
        // rating:4,5 → exact values 4 and 5
        let p = parse_search_query("rating:4,5");
        assert_eq!(p.rating, Some(NumericFilter::Values(vec![4.0, 5.0])));
    }

    #[test]
    fn parse_rating_comma_with_min() {
        // rating:2,4+ → exact 2 OR minimum 4
        let p = parse_search_query("rating:2,4+");
        assert_eq!(p.rating, Some(NumericFilter::ValuesOrMin { values: vec![2.0], min: 4.0 }));
    }

    #[test]
    fn parse_rating_comma_multiple() {
        let p = parse_search_query("rating:1,3,5");
        assert_eq!(p.rating, Some(NumericFilter::Values(vec![1.0, 3.0, 5.0])));
    }

    #[test]
    fn parse_rating_range() {
        let p = parse_search_query("rating:3-5");
        assert_eq!(p.rating, Some(NumericFilter::Range(3.0, 5.0)));
    }

    #[test]
    fn parse_rating_range_low() {
        let p = parse_search_query("rating:1-2");
        assert_eq!(p.rating, Some(NumericFilter::Range(1.0, 2.0)));
    }

    // ── date filter parse tests ─────────────────────────────────────

    #[test]
    fn parse_date_prefix_day() {
        let p = parse_search_query("date:2026-02-25");
        assert_eq!(p.date_prefix.as_deref(), Some("2026-02-25"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_date_prefix_month() {
        let p = parse_search_query("date:2026-02");
        assert_eq!(p.date_prefix.as_deref(), Some("2026-02"));
    }

    #[test]
    fn parse_date_prefix_year() {
        let p = parse_search_query("date:2026");
        assert_eq!(p.date_prefix.as_deref(), Some("2026"));
    }

    #[test]
    fn parse_date_from() {
        let p = parse_search_query("dateFrom:2026-01-15");
        assert_eq!(p.date_from.as_deref(), Some("2026-01-15"));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_date_until() {
        let p = parse_search_query("dateUntil:2026-02-28");
        assert_eq!(p.date_until.as_deref(), Some("2026-02-28"));
    }

    #[test]
    fn parse_date_range_combined() {
        let p = parse_search_query("dateFrom:2026-01-01 dateUntil:2026-12-31 tag:landscape");
        assert_eq!(p.date_from.as_deref(), Some("2026-01-01"));
        assert_eq!(p.date_until.as_deref(), Some("2026-12-31"));
        assert_eq!(p.tags, vec!["landscape"]);
    }

    #[test]
    fn parse_geo_any() {
        let p = parse_search_query("geo:any");
        assert_eq!(p.has_gps, Some(true));
        assert!(p.geo_bbox.is_none());
    }

    #[test]
    fn parse_geo_none() {
        let p = parse_search_query("geo:none");
        assert_eq!(p.has_gps, Some(false));
    }

    #[test]
    fn parse_geo_lat_lng_radius() {
        let p = parse_search_query("geo:52.5,13.4,10");
        assert!(p.geo_bbox.is_some());
        let (s, w, n, e) = p.geo_bbox.unwrap();
        assert!((s - (52.5 - 10.0/111.0)).abs() < 0.01);
        assert!(n > s);
        assert!(e > w);
        assert!(w < 13.4);
        assert!(e > 13.4);
    }

    #[test]
    fn parse_geo_bbox() {
        let p = parse_search_query("geo:50,10,55,15");
        assert!(p.geo_bbox.is_some());
        let (s, w, n, e) = p.geo_bbox.unwrap();
        assert!((s - 50.0).abs() < 0.001);
        assert!((w - 10.0).abs() < 0.001);
        assert!((n - 55.0).abs() < 0.001);
        assert!((e - 15.0).abs() < 0.001);
    }

    #[test]
    fn parse_embed_any() {
        let p = parse_search_query("embed:any");
        assert_eq!(p.has_embed, Some(true));
    }

    #[test]
    fn parse_embed_true() {
        let p = parse_search_query("embed:true");
        assert_eq!(p.has_embed, Some(true));
    }

    #[test]
    fn parse_embed_none() {
        let p = parse_search_query("embed:none");
        assert_eq!(p.has_embed, Some(false));
    }

    #[test]
    fn parse_embed_false() {
        let p = parse_search_query("embed:false");
        assert_eq!(p.has_embed, Some(false));
    }

    // ── negation and OR parse tests ────────────────────────────────

    #[test]
    fn parse_negated_tag() {
        let p = parse_search_query("-tag:rejected");
        assert!(p.tags.is_empty());
        assert_eq!(p.tags_exclude, vec!["rejected"]);
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_negated_format() {
        let p = parse_search_query("-format:xmp");
        assert!(p.formats.is_empty());
        assert_eq!(p.formats_exclude, vec!["xmp"]);
    }

    #[test]
    fn parse_negated_type() {
        let p = parse_search_query("-type:other");
        assert!(p.asset_types.is_empty());
        assert_eq!(p.asset_types_exclude, vec!["other"]);
    }

    #[test]
    fn parse_negated_label() {
        let p = parse_search_query("-label:Red");
        assert!(p.color_labels.is_empty());
        assert_eq!(p.color_labels_exclude, vec!["Red"]);
    }

    #[test]
    fn parse_negated_camera() {
        let p = parse_search_query("-camera:phone");
        assert!(p.cameras.is_empty());
        assert_eq!(p.cameras_exclude, vec!["phone"]);
    }

    #[test]
    fn parse_negated_lens() {
        let p = parse_search_query("-lens:kit");
        assert!(p.lenses.is_empty());
        assert_eq!(p.lenses_exclude, vec!["kit"]);
    }

    #[test]
    fn parse_negated_collection() {
        let p = parse_search_query("-collection:Rejects");
        assert!(p.collections.is_empty());
        assert_eq!(p.collections_exclude, vec!["Rejects"]);
    }

    #[test]
    fn parse_negated_path() {
        let p = parse_search_query("-path:Trash");
        assert!(p.path_prefixes.is_empty());
        assert_eq!(p.path_prefixes_exclude, vec!["Trash"]);
    }

    #[test]
    fn parse_negated_text() {
        let p = parse_search_query("sunset -boring");
        assert_eq!(p.text.as_deref(), Some("sunset"));
        assert_eq!(p.text_exclude, vec!["boring"]);
    }

    #[test]
    fn parse_negated_quoted_tag() {
        let p = parse_search_query(r#"-tag:"Fools Theater""#);
        assert_eq!(p.tags_exclude, vec!["Fools Theater"]);
        assert!(p.tags.is_empty());
    }

    #[test]
    fn parse_comma_or_format() {
        let p = parse_search_query("format:nef,cr3");
        assert_eq!(p.formats, vec!["nef,cr3"]);
    }

    #[test]
    fn parse_comma_or_tag() {
        let p = parse_search_query("tag:alice,bob");
        assert_eq!(p.tags, vec!["alice,bob"]);
    }

    #[test]
    fn parse_comma_or_type() {
        let p = parse_search_query("type:image,video");
        assert_eq!(p.asset_types, vec!["image,video"]);
    }

    #[test]
    fn parse_repeated_tags_are_and() {
        let p = parse_search_query("tag:landscape tag:sunset");
        assert_eq!(p.tags, vec!["landscape", "sunset"]);
    }

    #[test]
    fn parse_combined_negation_or_and() {
        let p = parse_search_query("tag:alice,bob tag:portrait -tag:rejected -type:other");
        assert_eq!(p.tags, vec!["alice,bob", "portrait"]);
        assert_eq!(p.tags_exclude, vec!["rejected"]);
        assert_eq!(p.asset_types_exclude, vec!["other"]);
    }

    #[test]
    fn parse_negation_does_not_affect_rating() {
        // Rating doesn't support negation — the `-` is ignored
        let p = parse_search_query("-rating:3+");
        assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
        assert!(p.text.is_none());
    }

    #[test]
    fn parse_negation_with_all_filters() {
        let p = parse_search_query("tag:keep -tag:reject format:nef,cr3 -format:xmp label:Red -label:Blue camera:nikon -camera:phone");
        assert_eq!(p.tags, vec!["keep"]);
        assert_eq!(p.tags_exclude, vec!["reject"]);
        assert_eq!(p.formats, vec!["nef,cr3"]);
        assert_eq!(p.formats_exclude, vec!["xmp"]);
        assert_eq!(p.color_labels, vec!["Red"]);
        assert_eq!(p.color_labels_exclude, vec!["Blue"]);
        assert_eq!(p.cameras, vec!["nikon"]);
        assert_eq!(p.cameras_exclude, vec!["phone"]);
    }

    #[test]
    fn parse_multiple_text_excludes() {
        let p = parse_search_query("sunset -boring -blurry");
        assert_eq!(p.text.as_deref(), Some("sunset"));
        assert_eq!(p.text_exclude, vec!["boring", "blurry"]);
    }

    #[test]
    #[cfg(feature = "ai")]
    fn parse_similar_basic() {
        let p = parse_search_query("similar:abc12345");
        assert_eq!(p.similar.as_deref(), Some("abc12345"));
        assert!(p.similar_limit.is_none());
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_similar_with_limit() {
        let p = parse_search_query("similar:abc12345:50");
        assert_eq!(p.similar.as_deref(), Some("abc12345"));
        assert_eq!(p.similar_limit, Some(50));
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_similar_with_other_filters() {
        let p = parse_search_query("similar:abc12345 rating:3+ tag:landscape");
        assert_eq!(p.similar.as_deref(), Some("abc12345"));
        assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
        assert_eq!(p.tags, vec!["landscape"]);
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_similar_uuid_like() {
        let p = parse_search_query("similar:550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(p.similar.as_deref(), Some("550e8400-e29b-41d4-a716-446655440000"));
        assert!(p.similar_limit.is_none());
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_similar_uuid_with_limit() {
        let p = parse_search_query("similar:550e8400-e29b-41d4-a716-446655440000:10");
        assert_eq!(p.similar.as_deref(), Some("550e8400-e29b-41d4-a716-446655440000"));
        assert_eq!(p.similar_limit, Some(10));
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_min_sim() {
        let p = parse_search_query("similar:abc123 min_sim:90");
        assert_eq!(p.similar.as_deref(), Some("abc123"));
        assert_eq!(p.min_sim, Some(90.0));
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_min_sim_without_similar() {
        let p = parse_search_query("min_sim:85");
        assert_eq!(p.min_sim, Some(85.0));
        assert!(p.similar.is_none());
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_text_query_basic() {
        let p = parse_search_query("text:sunset");
        assert_eq!(p.text_query.as_deref(), Some("sunset"));
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_text_query_quoted() {
        let p = parse_search_query("text:\"sunset on the beach\"");
        assert_eq!(p.text_query.as_deref(), Some("sunset on the beach"));
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_text_query_with_other_filters() {
        let p = parse_search_query("text:\"colorful flowers\" rating:3+ type:image");
        assert_eq!(p.text_query.as_deref(), Some("colorful flowers"));
        assert_eq!(p.rating, Some(NumericFilter::Min(3.0)));
        assert_eq!(p.asset_types, vec!["image".to_string()]);
    }

    #[cfg(feature = "ai")]
    #[test]
    fn parse_text_query_empty_ignored() {
        let p = parse_search_query("text:");
        assert!(p.text_query.is_none());
    }

    #[test]
    fn parse_double_dash_not_negated() {
        // A token starting with `--` should not be treated as negation
        let p = parse_search_query("--help");
        assert_eq!(p.text.as_deref(), Some("--help"));
        assert!(p.text_exclude.is_empty());
    }

    // ── group recipe preservation tests ──────────────────────────────

    #[test]
    fn group_preserves_recipes() {
        use crate::models::{Recipe, RecipeType};
        use crate::models::volume::FileLocation;

        let (dir, hash1, hash2, id1, id2) = setup_group_env();

        // Add a recipe to the donor (asset2)
        let store = MetadataStore::new(dir.path());
        let uuid2: uuid::Uuid = id2.parse().unwrap();
        let mut asset2 = store.load(uuid2).unwrap();
        asset2.recipes.push(Recipe {
            id: uuid::Uuid::new_v4(),
            variant_hash: "sha256:hash2".to_string(),
            software: "Adobe/CaptureOne".to_string(),
            recipe_type: RecipeType::Sidecar,
            content_hash: "sha256:recipe_hash".to_string(),
            location: FileLocation {
                volume_id: uuid::Uuid::nil(),
                relative_path: "DSC_001.xmp".into(),
                verified_at: None,
            },
            pending_writeback: false,
        });
        store.save(&asset2).unwrap();

        let engine = QueryEngine::new(dir.path());
        engine.group(&[hash1, hash2]).unwrap();

        // Verify recipe is on the target sidecar
        let uuid1: uuid::Uuid = id1.parse().unwrap();
        let target = store.load(uuid1).unwrap();
        assert_eq!(target.recipes.len(), 1);
        assert_eq!(target.recipes[0].variant_hash, "sha256:hash2");
    }

    // ── auto_group tests ─────────────────────────────────────────────

    #[test]
    fn auto_group_merges_same_stem() {
        let (dir, _, _, id1, id2) = setup_group_env();
        // Both assets have variants with stem DSC_001 (ARW and JPG)
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .auto_group(&[id1.clone(), id2.clone()], false)
            .unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.total_donors_merged, 1);
        assert!(!result.dry_run);

        // RAW asset (id1) should be the target
        assert_eq!(result.groups[0].target_id, id1);

        // Only one asset should remain
        let details = engine.show(&id1).unwrap();
        assert_eq!(details.variants.len(), 2);
        assert!(engine.show(&id2).is_err());
    }

    #[test]
    fn auto_group_dry_run_does_not_modify() {
        let (dir, _, _, id1, id2) = setup_group_env();
        let engine = QueryEngine::new(dir.path());

        let result = engine
            .auto_group(&[id1.clone(), id2.clone()], true)
            .unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.total_donors_merged, 1);
        assert!(result.dry_run);

        // Both assets should still exist
        assert!(engine.show(&id1).is_ok());
        assert!(engine.show(&id2).is_ok());
    }

    #[test]
    fn auto_group_different_stems_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let mut asset1 = Asset::new(AssetType::Image, "sha256:aaa");
        let v1 = Variant {
            content_hash: "sha256:aaa".to_string(),
            asset_id: asset1.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 1000,
            original_filename: "IMG_001.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset1.variants.push(v1.clone());
        catalog.insert_asset(&asset1).unwrap();
        catalog.insert_variant(&v1).unwrap();
        store.save(&asset1).unwrap();

        let mut asset2 = Asset::new(AssetType::Image, "sha256:bbb");
        let v2 = Variant {
            content_hash: "sha256:bbb".to_string(),
            asset_id: asset2.id,
            role: VariantRole::Original,
            format: "jpg".to_string(),
            file_size: 2000,
            original_filename: "IMG_002.JPG".to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset2.variants.push(v2.clone());
        catalog.insert_asset(&asset2).unwrap();
        catalog.insert_variant(&v2).unwrap();
        store.save(&asset2).unwrap();

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(
                &[asset1.id.to_string(), asset2.id.to_string()],
                false,
            )
            .unwrap();

        assert!(result.groups.is_empty());
        assert_eq!(result.total_donors_merged, 0);
    }

    // ── stem_prefix_matches tests ────────────────────────────────────

    #[test]
    fn stem_prefix_exact_match() {
        assert!(stem_prefix_matches("DSC_001", "DSC_001"));
    }

    #[test]
    fn stem_prefix_separator_dash() {
        assert!(stem_prefix_matches("Z91_8561", "Z91_8561-1-HIGHRES"));
    }

    #[test]
    fn stem_prefix_separator_underscore() {
        assert!(stem_prefix_matches("DSC_001", "DSC_001_V2"));
    }

    #[test]
    fn stem_prefix_separator_space() {
        assert!(stem_prefix_matches("DSC_001", "DSC_001 (1)"));
    }

    #[test]
    fn stem_prefix_separator_paren() {
        assert!(stem_prefix_matches("IMG_1234", "IMG_1234(EDIT)"));
    }

    #[test]
    fn stem_prefix_rejects_digit_continuation() {
        // DSC_001 should NOT match DSC_0010 (different shot number)
        assert!(!stem_prefix_matches("DSC_001", "DSC_0010"));
    }

    #[test]
    fn stem_prefix_rejects_letter_continuation() {
        assert!(!stem_prefix_matches("IMG", "IMAGES"));
    }

    #[test]
    fn stem_prefix_no_match() {
        assert!(!stem_prefix_matches("DSC_001", "IMG_001"));
    }

    // ── fuzzy auto_group tests ───────────────────────────────────────

    /// Helper: create a single-variant asset in the catalog/sidecar.
    fn create_asset_with_filename(
        catalog: &Catalog,
        store: &MetadataStore,
        hash: &str,
        filename: &str,
        format: &str,
    ) -> String {
        let mut asset = Asset::new(AssetType::Image, hash);
        let v = Variant {
            content_hash: hash.to_string(),
            asset_id: asset.id,
            role: VariantRole::Original,
            format: format.to_string(),
            file_size: 1000,
            original_filename: filename.to_string(),
            source_metadata: Default::default(),
            locations: vec![],
        };
        asset.variants.push(v.clone());
        catalog.insert_asset(&asset).unwrap();
        catalog.insert_variant(&v).unwrap();
        store.save(&asset).unwrap();
        asset.id.to_string()
    }

    #[test]
    fn auto_group_fuzzy_prefix_match() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let id_raw = create_asset_with_filename(
            &catalog, &store, "sha256:raw1", "Z91_8561.ARW", "arw",
        );
        let id_export = create_asset_with_filename(
            &catalog, &store, "sha256:exp1",
            "Z91_8561-1-HighRes-(c)_2025_Thomas Herrmann.TIF", "tif",
        );

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(&[id_raw.clone(), id_export.clone()], false)
            .unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.total_donors_merged, 1);
        // RAW asset should be the target
        assert_eq!(result.groups[0].target_id, id_raw);
    }

    #[test]
    fn auto_group_fuzzy_rejects_numeric_continuation() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let id1 = create_asset_with_filename(
            &catalog, &store, "sha256:f1", "DSC_001.ARW", "arw",
        );
        let id2 = create_asset_with_filename(
            &catalog, &store, "sha256:f2", "DSC_0010.JPG", "jpg",
        );

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(&[id1, id2], false)
            .unwrap();

        // Should NOT match — these are different shots
        assert!(result.groups.is_empty());
    }

    #[test]
    fn auto_group_fuzzy_chain_resolves_to_shortest_root() {
        let dir = tempfile::tempdir().unwrap();
        let catalog_root = dir.path();
        let catalog = Catalog::open(catalog_root).unwrap();
        catalog.initialize().unwrap();
        let store = MetadataStore::new(catalog_root);

        let id_raw = create_asset_with_filename(
            &catalog, &store, "sha256:c1", "Z91_8561.ARW", "arw",
        );
        let id_v1 = create_asset_with_filename(
            &catalog, &store, "sha256:c2", "Z91_8561-1.JPG", "jpg",
        );
        let id_v2 = create_asset_with_filename(
            &catalog, &store, "sha256:c3",
            "Z91_8561-1-HighRes.TIF", "tif",
        );

        let engine = QueryEngine::new(catalog_root);
        let result = engine
            .auto_group(&[id_raw.clone(), id_v1.clone(), id_v2.clone()], false)
            .unwrap();

        // All three should be in one group
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].asset_ids.len(), 3);
        assert_eq!(result.total_donors_merged, 2);
        // RAW asset should be the target
        assert_eq!(result.groups[0].target_id, id_raw);
    }

    // ── normalize_path_for_search tests ────────────────────────────

    use crate::models::volume::{Volume, VolumeType};

    fn make_volume(label: &str, mount: &str) -> Volume {
        Volume {
            id: uuid::Uuid::new_v4(),
            label: label.to_string(),
            mount_point: std::path::PathBuf::from(mount),
            volume_type: VolumeType::External,
            purpose: None,
            is_online: true,
        }
    }

    #[test]
    #[cfg(unix)]
    fn normalize_absolute_path_matching_volume() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search(
            "/Volumes/Photos/Capture/2026", &[vol.clone()], None,
        );
        assert_eq!(rel, "Capture/2026");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_absolute_path_no_match() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search("/mnt/other/data", &[vol], None);
        assert_eq!(rel, "/mnt/other/data");
        assert!(vid.is_none());
    }

    #[test]
    fn normalize_relative_path_unchanged() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search("Capture/2026", &[vol], None);
        assert_eq!(rel, "Capture/2026");
        assert!(vid.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn normalize_picks_longest_mount_point() {
        let vol_parent = make_volume("Root", "/Volumes");
        let vol_child = make_volume("Photos", "/Volumes/Photos");
        let volumes = vec![vol_parent, vol_child.clone()];
        let (rel, vid) = normalize_path_for_search(
            "/Volumes/Photos/Capture/2026", &volumes, None,
        );
        assert_eq!(rel, "Capture/2026");
        assert_eq!(vid, Some(vol_child.id.to_string()));
    }

    #[test]
    #[cfg(unix)]
    fn normalize_tilde_expands_to_home() {
        let home = std::env::var("HOME").unwrap();
        let vol = make_volume("Home", &home);
        let cwd = std::path::Path::new("/tmp");

        let (rel, vid) = normalize_path_for_search(
            "~/Photos/2026", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Photos/2026");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    #[cfg(unix)]
    fn normalize_tilde_alone() {
        let home = std::env::var("HOME").unwrap();
        let vol = make_volume("Home", &home);
        let cwd = std::path::Path::new("/tmp");

        let (rel, vid) = normalize_path_for_search("~", &[vol.clone()], Some(cwd));
        assert_eq!(rel, "");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_tilde_without_cwd_unchanged() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let (rel, vid) = normalize_path_for_search("~/Photos", &[vol], None);
        assert_eq!(rel, "~/Photos");
        assert!(vid.is_none());
    }

    #[test]
    #[cfg(unix)]
    fn normalize_dot_slash_resolves_relative_to_cwd() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let cwd = std::path::Path::new("/Volumes/Photos/Capture");

        let (rel, vid) = normalize_path_for_search(
            "./2026-02-22", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Capture/2026-02-22");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    #[cfg(unix)]
    fn normalize_dotdot_resolves_relative_to_cwd() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let cwd = std::path::Path::new("/Volumes/Photos/Capture/2026");

        let (rel, vid) = normalize_path_for_search(
            "../2025", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Capture/2025");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    fn normalize_plain_relative_unchanged_even_with_cwd() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let cwd = std::path::Path::new("/Volumes/Photos/Capture");

        let (rel, vid) = normalize_path_for_search(
            "Capture/2026", &[vol], Some(cwd),
        );
        // Plain relative paths stay as volume-relative prefix matches
        assert_eq!(rel, "Capture/2026");
        assert!(vid.is_none());
    }

    // ── Windows path normalization tests ────────────────────────────

    #[test]
    #[cfg(windows)]
    fn normalize_windows_absolute_path_matching_volume() {
        let vol = make_volume("Photos", r"D:\Photos");
        let (rel, vid) = normalize_path_for_search(
            r"D:\Photos\Capture\2026", &[vol.clone()], None,
        );
        assert_eq!(rel, "Capture/2026");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    #[cfg(windows)]
    fn normalize_windows_picks_longest_mount_point() {
        let vol_parent = make_volume("Drive", r"D:\");
        let vol_child = make_volume("Photos", r"D:\Photos");
        let (rel, vid) = normalize_path_for_search(
            r"D:\Photos\Capture\2026", &[vol_parent, vol_child.clone()], None,
        );
        assert_eq!(rel, "Capture/2026");
        assert_eq!(vid, Some(vol_child.id.to_string()));
    }

    #[test]
    #[cfg(windows)]
    fn normalize_windows_tilde_expands_to_userprofile() {
        let home = std::env::var("USERPROFILE").unwrap();
        let vol = make_volume("Home", &home);
        let cwd = std::path::Path::new(r"C:\Temp");

        let (rel, vid) = normalize_path_for_search(
            "~/Photos/2026", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Photos/2026");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    #[cfg(windows)]
    fn normalize_windows_dot_slash_resolves_relative_to_cwd() {
        let vol = make_volume("Photos", r"D:\Photos");
        let cwd = std::path::Path::new(r"D:\Photos\Capture");

        let (rel, vid) = normalize_path_for_search(
            "./2026-02-22", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Capture/2026-02-22");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    #[test]
    #[cfg(windows)]
    fn normalize_windows_dotdot_resolves_relative_to_cwd() {
        let vol = make_volume("Photos", r"D:\Photos");
        let cwd = std::path::Path::new(r"D:\Photos\Capture\2026");

        let (rel, vid) = normalize_path_for_search(
            "../2025", &[vol.clone()], Some(cwd),
        );
        assert_eq!(rel, "Capture/2025");
        assert_eq!(vid, Some(vol.id.to_string()));
    }

    // -- from-tag pattern matching tests --

    fn build_from_tag_regex(pattern: &str) -> regex::Regex {
        let escaped = regex::escape(pattern).replace("\\{\\}", "(.+)");
        regex::Regex::new(&format!("^{escaped}$")).unwrap()
    }

    #[test]
    fn from_tag_pattern_matches_aperture_stack() {
        let re = build_from_tag_regex("Aperture Stack {}");
        assert!(re.is_match("Aperture Stack 1"));
        assert!(re.is_match("Aperture Stack 1234"));
        assert!(re.is_match("Aperture Stack abc"));
        assert!(re.is_match("Aperture Stack 1 extra")); // wildcard captures "1 extra"
        assert!(!re.is_match("Aperture Stack")); // empty wildcard = no match
        assert!(!re.is_match("Other Tag"));
    }

    #[test]
    fn from_tag_pattern_matches_prefix_wildcard() {
        let re = build_from_tag_regex("shoot-{}");
        assert!(re.is_match("shoot-A"));
        assert!(re.is_match("shoot-paris-01"));
        assert!(!re.is_match("shoot-")); // empty wildcard
        assert!(!re.is_match("xshoot-A")); // prefix mismatch
    }

    #[test]
    fn from_tag_pattern_escapes_regex_metacharacters() {
        let re = build_from_tag_regex("Group (A) {}");
        assert!(re.is_match("Group (A) 1"));
        assert!(!re.is_match("Group A 1")); // literal parens required
    }

    #[test]
    fn from_tag_pattern_middle_wildcard() {
        let re = build_from_tag_regex("pre-{}-post");
        assert!(re.is_match("pre-X-post"));
        assert!(re.is_match("pre-hello world-post"));
        assert!(!re.is_match("pre--post")); // empty wildcard
    }

    // ── resolve_volume_labels tests ────────────────────────────

    #[test]
    fn resolve_volume_labels_single() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let id = vol.id.to_string();
        let result = QueryEngine::resolve_volume_labels(
            &["Photos".to_string()], &[vol],
        ).unwrap();
        assert_eq!(result, vec![id]);
    }

    #[test]
    fn resolve_volume_labels_case_insensitive() {
        let vol = make_volume("ScreenSaver", "/path");
        let id = vol.id.to_string();
        let result = QueryEngine::resolve_volume_labels(
            &["screensaver".to_string()], &[vol],
        ).unwrap();
        assert_eq!(result, vec![id]);
    }

    #[test]
    fn resolve_volume_labels_comma_or() {
        let vol1 = make_volume("Photos", "/Volumes/Photos");
        let vol2 = make_volume("Archive", "/Volumes/Archive");
        let id1 = vol1.id.to_string();
        let id2 = vol2.id.to_string();
        let result = QueryEngine::resolve_volume_labels(
            &["Photos,Archive".to_string()], &[vol1, vol2],
        ).unwrap();
        assert_eq!(result, vec![id1, id2]);
    }

    #[test]
    fn resolve_volume_labels_unknown_errors() {
        let vol = make_volume("Photos", "/Volumes/Photos");
        let result = QueryEngine::resolve_volume_labels(
            &["Nonexistent".to_string()], &[vol],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown volume"));
    }

    #[test]
    fn merge_from_combines_vec_fields() {
        let mut base = parse_search_query("tag:sunset");
        let default = parse_search_query("-tag:rest rating:1+");
        base.merge_from(&default);
        assert_eq!(base.tags, vec!["sunset".to_string()]);
        assert_eq!(base.tags_exclude, vec!["rest".to_string()]);
        assert_eq!(base.rating, Some(NumericFilter::Min(1.0)));
    }

    #[test]
    fn merge_from_prefers_self_options() {
        let mut base = parse_search_query("rating:3+");
        let default = parse_search_query("rating:1+");
        base.merge_from(&default);
        // Self's rating takes priority
        assert_eq!(base.rating, Some(NumericFilter::Min(3.0)));
    }

    #[test]
    fn merge_from_empty_base() {
        let mut base = ParsedSearch::default();
        let default = parse_search_query("-tag:rest type:image");
        base.merge_from(&default);
        assert_eq!(base.tags_exclude, vec!["rest".to_string()]);
        assert_eq!(base.asset_types, vec!["image".to_string()]);
    }
}
