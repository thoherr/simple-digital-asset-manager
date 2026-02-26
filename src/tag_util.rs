/// Tag utility functions for hierarchical tag support.
///
/// Internal convention:
/// - `|` is the hierarchy separator: `animals|birds|eagles`
/// - `/` in a stored tag is a literal slash: `AF Nikkor 85mm f/1.4`
///
/// User-facing convention:
/// - `/` means hierarchy: `animals/birds/eagles` → stored as `animals|birds|eagles`
/// - `\/` means literal slash: `f\/1.4` → stored as `f/1.4`

/// Convert user-facing tag input to internal storage form.
///
/// - `\/` (escaped slash) becomes literal `/` in storage
/// - Unescaped `/` becomes `|` (hierarchy separator)
pub fn tag_input_to_storage(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'/') {
            result.push('/');
            chars.next();
        } else if c == '/' {
            result.push('|');
        } else {
            result.push(c);
        }
    }
    result
}

/// Convert internal storage form to user-facing display form.
///
/// - `|` becomes `/` (hierarchy separator shown as slash)
/// - Literal `/` becomes `\/` (escaped for round-trip clarity)
pub fn tag_storage_to_display(stored: &str) -> String {
    let mut result = String::with_capacity(stored.len() + 4);
    for c in stored.chars() {
        match c {
            '|' => result.push('/'),
            '/' => {
                result.push('\\');
                result.push('/');
            }
            _ => result.push(c),
        }
    }
    result
}

/// Check if a stored tag is hierarchical (contains `|`).
pub fn is_hierarchical(tag: &str) -> bool {
    tag.contains('|')
}

/// Split a stored tag into hierarchy segments on `|`.
pub fn split_hierarchy(tag: &str) -> Vec<&str> {
    tag.split('|').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_to_storage_hierarchy() {
        assert_eq!(tag_input_to_storage("animals/birds/eagles"), "animals|birds|eagles");
    }

    #[test]
    fn input_to_storage_escaped_slash() {
        assert_eq!(tag_input_to_storage(r"AF Nikkor 85mm f\/1.4"), "AF Nikkor 85mm f/1.4");
    }

    #[test]
    fn input_to_storage_mixed() {
        assert_eq!(
            tag_input_to_storage(r"gear\/lenses/nikkor"),
            "gear/lenses|nikkor"
        );
    }

    #[test]
    fn input_to_storage_no_slashes() {
        assert_eq!(tag_input_to_storage("landscape"), "landscape");
    }

    #[test]
    fn storage_to_display_hierarchy() {
        assert_eq!(tag_storage_to_display("animals|birds|eagles"), "animals/birds/eagles");
    }

    #[test]
    fn storage_to_display_literal_slash() {
        assert_eq!(tag_storage_to_display("AF Nikkor 85mm f/1.4"), r"AF Nikkor 85mm f\/1.4");
    }

    #[test]
    fn storage_to_display_mixed() {
        assert_eq!(tag_storage_to_display("gear/lenses|nikkor"), r"gear\/lenses/nikkor");
    }

    #[test]
    fn storage_to_display_no_special() {
        assert_eq!(tag_storage_to_display("landscape"), "landscape");
    }

    #[test]
    fn round_trip() {
        let inputs = [
            "animals/birds/eagles",
            r"AF Nikkor 85mm f\/1.4",
            r"gear\/lenses/nikkor",
            "landscape",
        ];
        for input in inputs {
            let stored = tag_input_to_storage(input);
            let displayed = tag_storage_to_display(&stored);
            assert_eq!(displayed, input, "round-trip failed for: {input}");
        }
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
