use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::query::parse_search_query;

/// A saved search (smart album) — a named query that can be re-executed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    /// Search query in the same format as `maki search` (e.g. "type:image tag:landscape rating:4+")
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub query: String,
    /// Sort order (e.g. "date_desc", "name_asc"). Omitted = default (date_desc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    /// Whether this search appears as a chip on the browse page.
    #[serde(default, skip_serializing_if = "is_false")]
    pub favorite: bool,
}

fn is_false(v: &bool) -> bool {
    !v
}

impl SavedSearch {
    /// Convert the stored query into browse-page URL parameters.
    ///
    /// Parses the query string into structured filters and builds separate URL
    /// params so the browse page dropdowns reflect the active filters.
    pub fn to_url_params(&self) -> String {
        let parsed = parse_search_query(&self.query);
        let mut params = Vec::new();

        // Free-text portion
        if let Some(ref text) = parsed.text {
            params.push(format!("q={}", urlencoded(text)));
        }

        // Structured filters extracted from query
        // For now, only use the first element if multiple are present
        if let Some(t) = parsed.asset_types.first() {
            params.push(format!("type={}", urlencoded(t)));
        }
        if let Some(t) = parsed.tags.first() {
            params.push(format!("tag={}", urlencoded(t)));
        }
        if let Some(f) = parsed.formats.first() {
            params.push(format!("format={}", urlencoded(f)));
        }
        if let Some(l) = parsed.color_labels.first() {
            params.push(format!("label={}", urlencoded(l)));
        }

        // Rating: reconstruct the filter string
        if let Some(min) = parsed.rating_min {
            params.push(format!("rating={}%2B", min)); // N+
        } else if let Some(exact) = parsed.rating_exact {
            params.push(format!("rating={}", exact));
        }

        // Sort
        let sort = self.sort.as_deref().unwrap_or("date_desc");
        params.push(format!("sort={}", urlencoded(sort)));

        params.join("&")
    }
}

/// File structure for searches.toml.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedSearchFile {
    #[serde(default, rename = "search")]
    pub searches: Vec<SavedSearch>,
}

const FILENAME: &str = "searches.toml";

/// Load saved searches from the catalog root. Returns empty list if file doesn't exist.
pub fn load(catalog_root: &Path) -> Result<SavedSearchFile> {
    let path = catalog_root.join(FILENAME);
    if path.exists() {
        let contents = std::fs::read_to_string(&path)?;
        let file: SavedSearchFile = toml::from_str(&contents)?;
        Ok(file)
    } else {
        Ok(SavedSearchFile::default())
    }
}

/// Save saved searches to the catalog root. Creates the file if it doesn't exist.
pub fn save(catalog_root: &Path, file: &SavedSearchFile) -> Result<()> {
    let path = catalog_root.join(FILENAME);
    let contents = toml::to_string_pretty(file)?;
    std::fs::write(path, contents)?;
    Ok(())
}

/// Find a saved search by name (case-sensitive).
pub fn find_by_name<'a>(file: &'a SavedSearchFile, name: &str) -> Option<&'a SavedSearch> {
    file.searches.iter().find(|s| s.name == name)
}

/// Minimal percent-encoding for URL parameter values.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '&' => out.push_str("%26"),
            '=' => out.push_str("%3D"),
            '+' => out.push_str("%2B"),
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_toml() {
        let file = SavedSearchFile {
            searches: vec![
                SavedSearch {
                    name: "Landscapes".to_string(),
                    query: "type:image tag:landscape rating:4+".to_string(),
                    sort: Some("name_asc".to_string()),
                    favorite: false,
                },
                SavedSearch {
                    name: "Unrated".to_string(),
                    query: "rating:0".to_string(),
                    sort: None,
                    favorite: false,
                },
            ],
        };

        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: SavedSearchFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(file, parsed);
    }

    #[test]
    fn parse_empty_file() {
        let file: SavedSearchFile = toml::from_str("").unwrap();
        assert!(file.searches.is_empty());
    }

    #[test]
    fn to_url_params_basic() {
        let ss = SavedSearch {
            name: "Test".to_string(),
            query: "type:image tag:landscape rating:4+".to_string(),
            sort: Some("name_asc".to_string()),
            favorite: false,
        };
        let params = ss.to_url_params();
        assert!(params.contains("type=image"));
        assert!(params.contains("tag=landscape"));
        assert!(params.contains("rating=4%2B"));
        assert!(params.contains("sort=name_asc"));
    }

    #[test]
    fn to_url_params_with_text() {
        let ss = SavedSearch {
            name: "Test".to_string(),
            query: "sunset beach type:image".to_string(),
            sort: None,
            favorite: false,
        };
        let params = ss.to_url_params();
        assert!(params.contains("q=sunset%20beach"));
        assert!(params.contains("type=image"));
        assert!(params.contains("sort=date_desc"));
    }

    #[test]
    fn load_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = load(dir.path()).unwrap();
        assert!(file.searches.is_empty());
    }

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let file = SavedSearchFile {
            searches: vec![SavedSearch {
                name: "Test".to_string(),
                query: "type:image".to_string(),
                sort: None,
                favorite: false,
            }],
        };
        save(dir.path(), &file).unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.searches.len(), 1);
        assert_eq!(loaded.searches[0].name, "Test");
    }

    #[test]
    fn find_by_name_found() {
        let file = SavedSearchFile {
            searches: vec![
                SavedSearch {
                    name: "A".to_string(),
                    query: "".to_string(),
                    sort: None,
                    favorite: false,
                },
                SavedSearch {
                    name: "B".to_string(),
                    query: "type:video".to_string(),
                    sort: None,
                    favorite: false,
                },
            ],
        };
        assert_eq!(find_by_name(&file, "B").unwrap().query, "type:video");
        assert!(find_by_name(&file, "C").is_none());
    }

    #[test]
    fn favorite_default_false() {
        let toml_str = r#"
[[search]]
name = "Legacy"
query = "type:image"
"#;
        let file: SavedSearchFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.searches.len(), 1);
        assert!(!file.searches[0].favorite);
    }

    #[test]
    fn favorite_roundtrip() {
        let file = SavedSearchFile {
            searches: vec![
                SavedSearch {
                    name: "Fav".to_string(),
                    query: "type:image".to_string(),
                    sort: None,
                    favorite: true,
                },
                SavedSearch {
                    name: "NotFav".to_string(),
                    query: "type:video".to_string(),
                    sort: None,
                    favorite: false,
                },
            ],
        };
        let toml_str = toml::to_string_pretty(&file).unwrap();
        // favorite = true is serialized, favorite = false is skipped
        assert!(toml_str.contains("favorite = true"));
        assert!(!toml_str.contains("favorite = false"));
        let parsed: SavedSearchFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(file, parsed);
    }
}
