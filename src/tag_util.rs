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
}
