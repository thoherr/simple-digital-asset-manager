//! Tag vocabulary file support.
//!
//! Reads `vocabulary.yaml` from the catalog root and flattens the nested tree
//! into pipe-separated tag paths. Used to populate autocomplete with planned
//! tags that haven't been used on any asset yet.

use std::collections::HashSet;
use std::path::Path;

/// Load vocabulary tags from `vocabulary.yaml` in the catalog root.
/// Returns a sorted, deduplicated list of pipe-separated tag paths.
/// Returns an empty list if the file doesn't exist.
pub fn load_vocabulary(catalog_root: &Path) -> Vec<String> {
    let path = catalog_root.join("vocabulary.yaml");
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_vocabulary(&content),
        Err(_) => Vec::new(),
    }
}

/// Parse a vocabulary YAML string into a flat list of pipe-separated tag paths.
///
/// The YAML is a nested tree where:
/// - Map keys are hierarchy nodes
/// - Array values are leaf lists
/// - Null/empty values are leaf nodes
///
/// Example input:
/// ```yaml
/// subject:
///   nature:
///     - landscape
///     - flora
///   animal:
///     - bird
/// person:
/// ```
///
/// Output: `["subject", "subject|nature", "subject|nature|landscape", ...]`
pub fn parse_vocabulary(yaml_str: &str) -> Vec<String> {
    let value: serde_yaml::Value = match serde_yaml::from_str(yaml_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut tags = Vec::new();
    let mut seen = HashSet::new();
    flatten_value(&value, "", &mut tags, &mut seen);
    tags.sort();
    tags
}

fn flatten_value(
    value: &serde_yaml::Value,
    prefix: &str,
    result: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, val) in map {
                if let serde_yaml::Value::String(key_str) = key {
                    let path = if prefix.is_empty() {
                        key_str.clone()
                    } else {
                        format!("{}|{}", prefix, key_str)
                    };
                    if seen.insert(path.clone()) {
                        result.push(path.clone());
                    }
                    flatten_value(val, &path, result, seen);
                }
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                if let serde_yaml::Value::String(s) = item {
                    let path = if prefix.is_empty() {
                        s.clone()
                    } else {
                        format!("{}|{}", prefix, s)
                    };
                    if seen.insert(path.clone()) {
                        result.push(path.clone());
                    }
                }
            }
        }
        serde_yaml::Value::Null => {
            // Empty node — the prefix itself is the tag (already added by parent)
        }
        _ => {}
    }
}

/// Generate the default vocabulary content based on the Tagging Guide.
pub fn default_vocabulary() -> &'static str {
    r#"# Tag vocabulary — planned tag hierarchy for autocomplete
#
# This file defines your tag vocabulary skeleton. MAKI uses it to offer
# autocomplete suggestions for tags you've planned but haven't used yet.
# Edit freely — add, remove, or reorganize categories as your collection grows.
#
# Format: nested YAML tree. Keys are hierarchy nodes, arrays are leaf lists.
# See the Tagging Guide chapter in the manual for detailed recommendations.

subject:
  nature:
    - landscape
    - flora
    - sky
    - water
  animal:
    - mammal
    - bird
    - reptile
    - invertebrate
    - aquatic
    - domestic
  urban:
    - architecture
    - street
    - transport
  person:
    - portrait
    - group
    - activity
  performing arts:
    - concert
    - theatre
    - dance
  event:
    # Generic ceremony / gathering scene types — NOT performances.
    # Concerts / theatre / dance go under subject|performing arts.
    # Specific dated occasions (Jane's Wedding 2025) go under top-level event|.
    - wedding
    - exhibition
    - workshop
    - sports event
    - festival
  object:
    - food
    - instrument
    - other
  concept:
    - travel
    - fashion
    - documentary
    - abstract
  style:
    # Visual era/aesthetic of the subject (not the photographic technique —
    # that's under technique|style). Cross-cuts other subject categories:
    # tag a vintage car as subject|vehicle|car + subject|style|vintage.
    - vintage
    - modern
    - retro
    - rustic
    - industrial
    - classic
  condition:
    # Physical state of the subject
    - abandoned
    - ruined
    - restored
    - weathered
    - pristine
    - under construction
  mood:
    # Emotional quality of the scene
    - dramatic
    - serene
    - playful
    - mysterious
    - melancholic
    - joyful

location:
  # Structure: location > country > region > city > venue
  # Add your locations as you photograph them

person:
  family:
  friend:
  artist:
    - musician
    - actor
    - model
  public figure:
  ensemble:
    - band
    - choir
    - orchestra
    - team

technique:
  style:
    - black and white
    - high key
    - low key
    - infrared
  exposure:
    - long exposure
    - double exposure
    - HDR
  lighting:
    - natural light
    - flash
    - studio
    - golden hour
    - blue hour
    - stage lighting
  composition:
    - minimalist
    - symmetry
    - leading lines
  effect:
    - bokeh
    - motion blur
    - silhouette
    - reflection
    - lens flare

project:
  # Project entries are personal — add your projects here

event:
  # Specific occasions: weddings, trips, workshops, named concerts, etc.
  # Name them however you'll remember them — e.g. event|wedding-jane-2025
  # or event|2025|wedding-jane for year-grouped browsing.
  # Separate from subject|event (generic scene types) and
  # subject|performing arts (performance scene types).

color:
  # Dominant image color — opt-in facet. Keep flat, low-cardinality.
  # Note: MAKI also has a 5-value color_label field for editorial workflow;
  # content-color tagging here is for finer distinctions and catalog filtering.
  - red
  - orange
  - yellow
  - green
  - blue
  - purple
  - pink
  - brown
  - black
  - white
  - grey
  - monochrome
  - pastel
  - warm
  - cold
"#
}

/// Build a vocabulary YAML string from a flat list of pipe-separated tags.
/// Groups tags into a nested tree structure.
pub fn tags_to_vocabulary_yaml(tags: &[(String, u64)]) -> String {
    use std::collections::BTreeMap;

    // Build a tree structure
    #[derive(Default)]
    struct Node {
        children: BTreeMap<String, Node>,
    }

    let mut root = Node::default();
    for (tag, _count) in tags {
        let parts: Vec<&str> = tag.split('|').collect();
        let mut current = &mut root;
        for part in parts {
            current = current.children.entry(part.to_string()).or_default();
        }
    }

    fn write_node(node: &Node, indent: usize, output: &mut String) {
        let prefix = "  ".repeat(indent);
        // Separate leaf children (no sub-children) from branch children
        let mut leaves = Vec::new();
        let mut branches = Vec::new();
        for (name, child) in &node.children {
            if child.children.is_empty() {
                leaves.push(name.as_str());
            } else {
                branches.push((name.as_str(), child));
            }
        }

        // Write branches first (as nested maps)
        for (name, child) in &branches {
            output.push_str(&format!("{}{}:\n", prefix, name));
            write_node(child, indent + 1, output);
        }

        // Write leaves as a YAML list
        if !leaves.is_empty() {
            for leaf in &leaves {
                output.push_str(&format!("{}- {}\n", prefix, leaf));
            }
        }
    }

    let mut output = String::from("# Tag vocabulary — exported from catalog\n#\n# Edit this file to plan your tag hierarchy.\n# MAKI uses it for autocomplete suggestions.\n\n");
    write_node(&root, 0, &mut output);
    output
}

/// Build a tab-indented keyword text file from a flat list of pipe-separated tags.
///
/// This is the format accepted by Adobe Lightroom ("Import Keywords") and
/// Capture One ("Import Keywords" → Keyword Text File). Each keyword appears
/// on its own line; hierarchy is expressed by the number of leading tabs.
///
/// Tag names are normalized for the target tools:
/// - XML entities (`&amp;`, `&lt;`, …) are decoded (C1 rejects `&` outright).
/// - Commas and semicolons are replaced with spaces (both tools treat them as
///   keyword delimiters on import).
/// - Runs of whitespace collapse to a single space; leading/trailing space is trimmed.
/// - Tags empty after sanitization are skipped.
///
/// Returns the rendered text plus a list of `(before, after)` pairs for tags
/// whose name changed — callers can surface these so the user knows what to
/// rename in their catalog.
///
/// Example output:
/// ```text
/// location
/// \tGermany
/// \t\tBayern
/// \t\t\tMünchen
/// subject
/// \tnature
/// \t\tlandscape
/// ```
pub fn tags_to_keyword_text(tags: &[(String, u64)]) -> (String, Vec<(String, String)>) {
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Node {
        children: BTreeMap<String, Node>,
    }

    let mut root = Node::default();
    let mut changes: Vec<(String, String)> = Vec::new();
    for (tag, _count) in tags {
        let mut sanitized_parts: Vec<String> = Vec::new();
        let mut part_changed = false;
        for part in tag.split('|') {
            let clean = sanitize_keyword_part(part);
            if clean != part {
                part_changed = true;
            }
            if clean.is_empty() {
                // An empty segment would collapse the hierarchy — skip the whole tag.
                sanitized_parts.clear();
                break;
            }
            sanitized_parts.push(clean);
        }
        if sanitized_parts.is_empty() {
            if !tag.is_empty() {
                changes.push((tag.clone(), String::new()));
            }
            continue;
        }
        let sanitized_tag = sanitized_parts.join("|");
        if part_changed {
            changes.push((tag.clone(), sanitized_tag.clone()));
        }
        let mut current = &mut root;
        for part in sanitized_parts {
            current = current.children.entry(part).or_default();
        }
    }

    fn write_node(node: &Node, depth: usize, output: &mut String) {
        for (name, child) in &node.children {
            for _ in 0..depth {
                output.push('\t');
            }
            output.push_str(name);
            output.push('\n');
            write_node(child, depth + 1, output);
        }
    }

    let mut output = String::new();
    write_node(&root, 0, &mut output);
    (output, changes)
}

/// Normalize a single hierarchy segment for keyword-text export:
/// decode XML entities, replace separators that break Lightroom/Capture One
/// import (`,` and `;`), collapse whitespace, trim.
fn sanitize_keyword_part(part: &str) -> String {
    let decoded = unescape_xml_entities(part);
    let stripped: String = decoded
        .chars()
        .map(|c| match c {
            ',' | ';' => ' ',
            // Keep real spaces but neutralize control chars
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode common XML/HTML entities (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`,
/// plus numeric `&#NN;` / `&#xHH;`) into their literal characters. Used when
/// exporting tag names to external tools that don't understand XML escapes.
/// Falls back to the input on failure — never lossy.
fn unescape_xml_entities(s: &str) -> String {
    match quick_xml::escape::unescape(s) {
        Ok(cow) => cow.into_owned(),
        Err(_) => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nested_tree() {
        let yaml = r#"
subject:
  nature:
    - landscape
    - flora
  animal:
    - bird
"#;
        let tags = parse_vocabulary(yaml);
        assert!(tags.contains(&"subject".to_string()));
        assert!(tags.contains(&"subject|nature".to_string()));
        assert!(tags.contains(&"subject|nature|landscape".to_string()));
        assert!(tags.contains(&"subject|nature|flora".to_string()));
        assert!(tags.contains(&"subject|animal".to_string()));
        assert!(tags.contains(&"subject|animal|bird".to_string()));
    }

    #[test]
    fn parse_empty_nodes() {
        let yaml = r#"
person:
  family:
  friend:
"#;
        let tags = parse_vocabulary(yaml);
        assert!(tags.contains(&"person".to_string()));
        assert!(tags.contains(&"person|family".to_string()));
        assert!(tags.contains(&"person|friend".to_string()));
    }

    #[test]
    fn parse_flat_root() {
        let yaml = r#"
location:
technique:
"#;
        let tags = parse_vocabulary(yaml);
        assert!(tags.contains(&"location".to_string()));
        assert!(tags.contains(&"technique".to_string()));
    }

    #[test]
    fn parse_empty_string() {
        let tags = parse_vocabulary("");
        assert!(tags.is_empty());
    }

    #[test]
    fn parse_invalid_yaml() {
        let tags = parse_vocabulary(":::invalid");
        assert!(tags.is_empty());
    }

    #[test]
    fn default_vocabulary_parses() {
        let tags = parse_vocabulary(default_vocabulary());
        assert!(tags.contains(&"subject".to_string()));
        assert!(tags.contains(&"subject|nature|landscape".to_string()));
        assert!(tags.contains(&"technique|lighting|stage lighting".to_string()));
        assert!(tags.contains(&"person|artist|musician".to_string()));
        assert!(tags.len() > 50, "default vocabulary should have many entries, got {}", tags.len());
    }

    #[test]
    fn default_vocabulary_includes_all_top_level_facets() {
        let tags = parse_vocabulary(default_vocabulary());
        for facet in ["subject", "location", "person", "technique", "project", "event", "color"] {
            assert!(
                tags.contains(&facet.to_string()),
                "default vocabulary missing top-level facet `{facet}`",
            );
        }
    }

    #[test]
    fn default_vocabulary_includes_color_leaves() {
        let tags = parse_vocabulary(default_vocabulary());
        for leaf in ["color|red", "color|monochrome", "color|warm"] {
            assert!(tags.contains(&leaf.to_string()), "missing {leaf}");
        }
    }

    #[test]
    fn keyword_text_flat_tags() {
        let tags = vec![
            ("red".to_string(), 1),
            ("blue".to_string(), 2),
            ("green".to_string(), 3),
        ];
        let (text, changes) = tags_to_keyword_text(&tags);
        // BTreeMap sorts alphabetically
        assert_eq!(text, "blue\ngreen\nred\n");
        assert!(changes.is_empty());
    }

    #[test]
    fn keyword_text_nested() {
        let tags = vec![
            ("location|Germany|Bayern|München".to_string(), 5),
            ("location|Germany|Bayern|Gelting".to_string(), 2),
            ("location|France|Paris".to_string(), 1),
        ];
        let (text, changes) = tags_to_keyword_text(&tags);
        let expected = "location\n\tFrance\n\t\tParis\n\tGermany\n\t\tBayern\n\t\t\tGelting\n\t\t\tMünchen\n";
        assert_eq!(text, expected);
        assert!(changes.is_empty());
    }

    #[test]
    fn keyword_text_deduplicates_shared_branches() {
        // Two tags sharing a common ancestor should only emit each branch once
        let tags = vec![
            ("subject|nature|landscape".to_string(), 1),
            ("subject|nature|flora".to_string(), 1),
        ];
        let (text, _) = tags_to_keyword_text(&tags);
        // "subject" and "nature" should appear exactly once each
        assert_eq!(text.matches("subject\n").count(), 1);
        assert_eq!(text.matches("\tnature\n").count(), 1);
        assert!(text.contains("\t\tflora\n"));
        assert!(text.contains("\t\tlandscape\n"));
    }

    #[test]
    fn keyword_text_deep_hierarchy() {
        let tags = vec![("a|b|c|d|e".to_string(), 1)];
        let (text, _) = tags_to_keyword_text(&tags);
        assert_eq!(text, "a\n\tb\n\t\tc\n\t\t\td\n\t\t\t\te\n");
    }

    #[test]
    fn keyword_text_empty() {
        let tags: Vec<(String, u64)> = vec![];
        let (text, changes) = tags_to_keyword_text(&tags);
        assert_eq!(text, "");
        assert!(changes.is_empty());
    }

    #[test]
    fn keyword_text_decodes_xml_entities() {
        // Catalog tags that originated from externally-authored XMP files may
        // contain literal `&amp;` — Capture One rejects these on import.
        let tags = vec![
            ("person|artist|Conny K. &amp; The Boosters".to_string(), 3),
            ("subject|M&amp;M".to_string(), 1),
            ("subject|black &lt;and&gt; white".to_string(), 1),
            ("subject|&#x41;&#65;".to_string(), 1), // &#x41; = A, &#65; = A
        ];
        let (text, changes) = tags_to_keyword_text(&tags);
        assert!(text.contains("Conny K. & The Boosters"), "got:\n{text}");
        assert!(!text.contains("&amp;"), "still has &amp; in:\n{text}");
        assert!(text.contains("M&M"));
        assert!(text.contains("black <and> white"));
        assert!(text.contains("\tAA\n"));
        assert_eq!(changes.len(), 4, "should report 4 sanitized tags");
    }

    #[test]
    fn keyword_text_replaces_commas_and_semicolons() {
        // Commas and semicolons are treated as keyword delimiters by
        // Lightroom and Capture One on import — they must be sanitized.
        let tags = vec![
            ("color|red, gold, white, black".to_string(), 1),
            ("subject|red; orange; blue".to_string(), 1),
            ("plain|no-change".to_string(), 1),
        ];
        let (text, changes) = tags_to_keyword_text(&tags);
        assert!(!text.contains(','), "comma leaked into output:\n{text}");
        assert!(!text.contains(';'), "semicolon leaked into output:\n{text}");
        assert!(text.contains("red gold white black"), "got:\n{text}");
        assert!(text.contains("red orange blue"), "got:\n{text}");
        // Two tags changed, one didn't
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().any(|(b, _)| b.contains("red, gold")));
        assert!(changes.iter().any(|(b, _)| b.contains("red; orange")));
    }

    #[test]
    fn keyword_text_skips_empty_after_sanitize() {
        // A tag that collapses to empty (e.g. pure control chars) is skipped
        // rather than emitted as a bare hierarchy separator.
        let tags = vec![
            ("keep".to_string(), 1),
            (",,,".to_string(), 1),
            ("subject|,,,".to_string(), 1),
        ];
        let (text, changes) = tags_to_keyword_text(&tags);
        assert_eq!(text, "keep\n");
        // Both skipped tags should be reported as changes (with empty replacement)
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn keyword_text_no_comments_no_header() {
        // Unlike the YAML variant, the text format must not emit comment lines —
        // Lightroom/Capture One would import `#` lines as literal keywords.
        let tags = vec![("location".to_string(), 1)];
        let (text, _) = tags_to_keyword_text(&tags);
        assert!(!text.contains('#'), "keyword text must not contain comments");
        assert_eq!(text, "location\n");
    }

    #[test]
    fn sorted_output() {
        let yaml = r#"
z_last:
a_first:
  - beta
  - alpha
"#;
        let tags = parse_vocabulary(yaml);
        let mut sorted = tags.clone();
        sorted.sort();
        assert_eq!(tags, sorted, "output should be sorted");
    }
}
