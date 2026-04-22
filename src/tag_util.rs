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

/// Count the "leaf" tags in a list — tags that are not an ancestor path
/// of any other tag in the same list.
///
/// MAKI auto-expands each hierarchical tag to include all its ancestor
/// paths (matching the Lightroom/CaptureOne convention). So for an asset
/// with a single deliberate tag `subject|nature|landscape`, the stored
/// list is `[subject, subject|nature, subject|nature|landscape]`. The
/// *leaf count* here is 1 — the number of tags the user actually
/// intended to apply. That's the quantity most users mean when they ask
/// "how many tags does this asset have?" and the one most useful for
/// catalogue restructuring ("find assets with 0 tags", "find assets
/// with more than 10 tags").
///
/// Comparison is case-insensitive to match the rest of MAKI's tag
/// semantics. An asset with only a single standalone tag (no hierarchy)
/// has leaf count 1.
///
/// # Examples
///
/// ```
/// use maki::tag_util::leaf_tag_count;
///
/// // Single leaf with its auto-expanded ancestors — one intentional tag.
/// assert_eq!(leaf_tag_count(&[
///     "subject".to_string(),
///     "subject|nature".to_string(),
///     "subject|nature|landscape".to_string(),
/// ]), 1);
///
/// // Two distinct leaves plus shared ancestors.
/// assert_eq!(leaf_tag_count(&[
///     "subject".to_string(),
///     "subject|nature".to_string(),
///     "subject|nature|landscape".to_string(),
///     "subject|nature|forest".to_string(),
/// ]), 2);
///
/// // Flat tags are all leaves.
/// assert_eq!(leaf_tag_count(&[
///     "sunset".to_string(),
///     "concert".to_string(),
/// ]), 2);
///
/// // Empty list → 0.
/// assert_eq!(leaf_tag_count(&[]), 0);
/// ```
pub fn leaf_tag_count(tags: &[String]) -> u32 {
    // A tag T is a leaf iff no *other* tag O satisfies O.starts_with(T + '|').
    // Case-insensitive to match tag search semantics.
    let lowered: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    let mut count: u32 = 0;
    for (i, t_lower) in lowered.iter().enumerate() {
        let prefix = format!("{}|", t_lower);
        let has_descendant = lowered
            .iter()
            .enumerate()
            .any(|(j, o)| j != i && o.starts_with(&prefix));
        if !has_descendant {
            count += 1;
        }
    }
    count
}

/// Check if removing a tag would leave any other descendant that keeps the
/// ancestor alive. Returns the list of ancestor tags that should also be removed.
pub fn orphaned_ancestors(tag_to_remove: &str, all_tags: &[String]) -> Vec<String> {
    let ancestors = expand_ancestors(tag_to_remove);
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

    #[test]
    fn leaf_count_empty_and_singleton() {
        assert_eq!(leaf_tag_count(&[]), 0);
        assert_eq!(leaf_tag_count(&["sunset".to_string()]), 1);
    }

    #[test]
    fn leaf_count_deep_single_hierarchy() {
        // One intentional tag + its 3 ancestors = 1 leaf.
        let tags = vec![
            "a".to_string(),
            "a|b".to_string(),
            "a|b|c".to_string(),
            "a|b|c|d".to_string(),
        ];
        assert_eq!(leaf_tag_count(&tags), 1);
    }

    #[test]
    fn leaf_count_two_branches_share_ancestor() {
        let tags = vec![
            "subject".to_string(),
            "subject|nature".to_string(),
            "subject|nature|landscape".to_string(),
            "subject|nature|forest".to_string(),
        ];
        // Leaves are `landscape` and `forest`; `subject` and `subject|nature` have descendants.
        assert_eq!(leaf_tag_count(&tags), 2);
    }

    #[test]
    fn leaf_count_mixed_flat_and_hierarchical() {
        let tags = vec![
            "subject".to_string(),
            "subject|nature".to_string(),
            "subject|nature|landscape".to_string(),
            "sunset".to_string(),
            "golden-hour".to_string(),
        ];
        // Leaves: `landscape`, `sunset`, `golden-hour`.
        assert_eq!(leaf_tag_count(&tags), 3);
    }

    #[test]
    fn leaf_count_case_insensitive() {
        // A user who typed the same hierarchical tag in mixed case at
        // different levels shouldn't double-count. `SUBJECT` is the
        // ancestor of `subject|nature` even though the case differs.
        let tags = vec![
            "SUBJECT".to_string(),
            "subject|nature".to_string(),
        ];
        assert_eq!(leaf_tag_count(&tags), 1);
    }

    #[test]
    fn leaf_count_prefix_collision_not_counted_as_ancestor() {
        // `foo` and `foobar` share the first three characters but neither
        // is the ancestor of the other — the separator is `|`. Both leaves.
        let tags = vec!["foo".to_string(), "foobar".to_string()];
        assert_eq!(leaf_tag_count(&tags), 2);
    }

    #[test]
    fn leaf_count_does_not_double_count_duplicates() {
        // Shouldn't happen in practice (insert_asset dedupes) but guard
        // against it: two identical tags shouldn't claim to be each
        // other's ancestor either. Both non-descendants → both counted.
        let tags = vec!["a".to_string(), "a".to_string()];
        assert_eq!(leaf_tag_count(&tags), 2);
    }
}
