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
    - festival
    - exhibition
    - wedding
    - workshop
    - sports event
  object:
    - food
    - instrument
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

project:
  # Project entries are personal — add your projects here
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
