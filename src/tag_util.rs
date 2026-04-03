/// Tag utility functions for hierarchical tag support.
///
/// Internal storage convention:
/// - `|` is the hierarchy separator: `animals|birds|eagles`
/// - `/` is a literal character: `AF Nikkor 85mm f/1.4`
///
/// User-facing input convention (aligned with Lightroom/CaptureOne):
/// - `|` means hierarchy: `animals|birds|eagles`
/// - `>` also means hierarchy: `animals>birds>eagles`
/// - `/` is a literal character (no escaping needed)

/// Convert user-facing tag input to internal storage form.
///
/// - `>` becomes `|` (hierarchy separator, CaptureOne/Lightroom convention)
/// - `|` stays as `|` (already the internal separator)
/// - `/` stays as `/` (literal character)
///
/// # Examples
///
/// ```
/// use maki::tag_util::tag_input_to_storage;
///
/// assert_eq!(tag_input_to_storage("animals|birds"), "animals|birds");
/// assert_eq!(tag_input_to_storage("animals>birds>eagles"), "animals|birds|eagles");
/// assert_eq!(tag_input_to_storage("f/1.4"), "f/1.4");
/// assert_eq!(tag_input_to_storage("plain tag"), "plain tag");
/// ```
pub fn tag_input_to_storage(input: &str) -> String {
    input.replace('>', "|")
}

/// Convert internal storage form to user-facing display form.
///
/// Tags are displayed as stored — `|` is the visible hierarchy separator,
/// matching the Lightroom/CaptureOne convention.
///
/// # Examples
///
/// ```
/// use maki::tag_util::tag_storage_to_display;
///
/// assert_eq!(tag_storage_to_display("animals|birds"), "animals|birds");
/// assert_eq!(tag_storage_to_display("f/1.4"), "f/1.4");
/// assert_eq!(tag_storage_to_display("plain tag"), "plain tag");
/// ```
pub fn tag_storage_to_display(stored: &str) -> String {
    stored.to_string()
}

/// Check if a stored tag is hierarchical (contains `|`).
///
/// # Examples
///
/// ```
/// use maki::tag_util::is_hierarchical;
///
/// assert!(is_hierarchical("animals|birds"));
/// assert!(!is_hierarchical("landscape"));
/// ```
pub fn is_hierarchical(tag: &str) -> bool {
    tag.contains('|')
}

/// Split a stored tag into hierarchy segments on `|`.
///
/// # Examples
///
/// ```
/// use maki::tag_util::split_hierarchy;
///
/// assert_eq!(split_hierarchy("animals|birds|eagles"), vec!["animals", "birds", "eagles"]);
/// assert_eq!(split_hierarchy("landscape"), vec!["landscape"]);
/// ```
pub fn split_hierarchy(tag: &str) -> Vec<&str> {
    tag.split('|').collect()
}

/// Expand a hierarchical tag into all ancestor paths (including itself).
///
/// Given `a|b|c|d`, returns `["a", "a|b", "a|b|c", "a|b|c|d"]`.
/// A flat tag returns just itself: `["landscape"]`.
///
/// # Examples
///
/// ```
/// use maki::tag_util::expand_ancestors;
///
/// assert_eq!(expand_ancestors("person|artist|musician"), vec!["person", "person|artist", "person|artist|musician"]);
/// assert_eq!(expand_ancestors("landscape"), vec!["landscape"]);
/// ```
pub fn expand_ancestors(tag: &str) -> Vec<String> {
    let parts: Vec<&str> = tag.split('|').collect();
    let mut result = Vec::with_capacity(parts.len());
    for i in 1..=parts.len() {
        result.push(parts[..i].join("|"));
    }
    result
}

/// Expand a list of tags, adding all ancestor paths. Deduplicates the result.
///
/// This matches the CaptureOne/Lightroom convention where each hierarchical
/// tag also stores all its ancestor paths as separate tags.
pub fn expand_all_ancestors(tags: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for tag in tags {
        for ancestor in expand_ancestors(tag) {
            if seen.insert(ancestor.clone()) {
                result.push(ancestor);
            }
        }
    }
    result
}

/// Check if removing a tag would leave any other descendant that keeps the
/// ancestor alive. Returns the list of ancestor tags that should also be removed.
pub fn orphaned_ancestors(tag_to_remove: &str, all_tags: &[String]) -> Vec<String> {
    let ancestors = expand_ancestors(tag_to_remove);
    let tag_lower = tag_to_remove.to_lowercase();
    let mut orphaned = Vec::new();

    let ancestors_lower: std::collections::HashSet<String> = ancestors.iter()
        .map(|a| a.to_lowercase())
        .collect();

    // For each ancestor (excluding the tag itself), check if any OTHER tag
    // in the asset starts with that ancestor prefix — but don't count
    // tags that are themselves ancestors of the removed tag
    for ancestor in &ancestors[..ancestors.len().saturating_sub(1)] {
        let prefix = format!("{}|", ancestor.to_lowercase());
        let has_other_descendant = all_tags.iter().any(|t| {
            let tl = t.to_lowercase();
            tl.starts_with(&prefix) && !ancestors_lower.contains(&tl)
        });
        if !has_other_descendant {
            orphaned.push(ancestor.clone());
        }
    }
    orphaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_to_storage_pipe() {
        assert_eq!(tag_input_to_storage("animals|birds|eagles"), "animals|birds|eagles");
    }

    #[test]
    fn input_to_storage_greater_than() {
        assert_eq!(tag_input_to_storage("animals>birds>eagles"), "animals|birds|eagles");
    }

    #[test]
    fn input_to_storage_slash_is_literal() {
        assert_eq!(tag_input_to_storage("AF Nikkor 85mm f/1.4"), "AF Nikkor 85mm f/1.4");
    }

    #[test]
    fn input_to_storage_no_special() {
        assert_eq!(tag_input_to_storage("landscape"), "landscape");
    }

    #[test]
    fn storage_to_display_hierarchy() {
        assert_eq!(tag_storage_to_display("animals|birds|eagles"), "animals|birds|eagles");
    }

    #[test]
    fn storage_to_display_literal_slash() {
        assert_eq!(tag_storage_to_display("AF Nikkor 85mm f/1.4"), "AF Nikkor 85mm f/1.4");
    }

    #[test]
    fn storage_to_display_no_special() {
        assert_eq!(tag_storage_to_display("landscape"), "landscape");
    }

    #[test]
    fn round_trip_pipe() {
        let input = "animals|birds|eagles";
        let stored = tag_input_to_storage(input);
        let displayed = tag_storage_to_display(&stored);
        assert_eq!(displayed, input);
    }

    #[test]
    fn round_trip_greater_than() {
        // > on input becomes | in storage and display
        let stored = tag_input_to_storage("animals>birds>eagles");
        assert_eq!(stored, "animals|birds|eagles");
        let displayed = tag_storage_to_display(&stored);
        assert_eq!(displayed, "animals|birds|eagles");
    }

    #[test]
    fn is_hierarchical_test() {
        assert!(is_hierarchical("animals|birds"));
        assert!(!is_hierarchical("landscape"));
        assert!(!is_hierarchical("f/1.4"));
    }

    #[test]
    fn split_hierarchy_test() {
        assert_eq!(split_hierarchy("animals|birds|eagles"), vec!["animals", "birds", "eagles"]);
        assert_eq!(split_hierarchy("landscape"), vec!["landscape"]);
        assert_eq!(split_hierarchy("f/1.4"), vec!["f/1.4"]);
    }

    #[test]
    fn expand_ancestors_hierarchical() {
        assert_eq!(
            expand_ancestors("person|artist|musician"),
            vec!["person", "person|artist", "person|artist|musician"]
        );
    }

    #[test]
    fn expand_ancestors_flat() {
        assert_eq!(expand_ancestors("landscape"), vec!["landscape"]);
    }

    #[test]
    fn expand_all_deduplicates() {
        let tags = vec![
            "person|artist|musician|Peter".to_string(),
            "person|artist|musician|Alice".to_string(),
        ];
        let expanded = expand_all_ancestors(&tags);
        // "person", "person|artist", "person|artist|musician" appear once each
        assert_eq!(expanded.iter().filter(|t| t.as_str() == "person").count(), 1);
        assert_eq!(expanded.iter().filter(|t| t.as_str() == "person|artist").count(), 1);
        assert_eq!(expanded.iter().filter(|t| t.as_str() == "person|artist|musician").count(), 1);
        assert!(expanded.contains(&"person|artist|musician|Peter".to_string()));
        assert!(expanded.contains(&"person|artist|musician|Alice".to_string()));
    }

    #[test]
    fn orphaned_ancestors_with_sibling() {
        let tags = vec![
            "person|artist|musician|Peter".to_string(),
            "person|artist|musician|Alice".to_string(),
            "person|artist|musician".to_string(),
            "person|artist".to_string(),
            "person".to_string(),
        ];
        // Removing Peter: musician, artist, person all have Alice keeping them alive
        let orphaned = orphaned_ancestors("person|artist|musician|Peter", &tags);
        assert!(orphaned.is_empty());
    }

    #[test]
    fn orphaned_ancestors_without_sibling() {
        let tags = vec![
            "location|Germany|Bayern|München".to_string(),
            "location|Germany|Bayern".to_string(),
            "location|Germany".to_string(),
            "location".to_string(),
        ];
        // Removing München: no other descendant under location|Germany|Bayern
        let orphaned = orphaned_ancestors("location|Germany|Bayern|München", &tags);
        assert!(orphaned.contains(&"location|Germany|Bayern".to_string()));
        assert!(orphaned.contains(&"location|Germany".to_string()));
        assert!(orphaned.contains(&"location".to_string()));
    }
}
