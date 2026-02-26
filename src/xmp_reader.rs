use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;

/// Extracted metadata from an XMP sidecar file.
pub struct XmpData {
    /// Keywords from `dc:subject`.
    pub keywords: Vec<String>,
    /// Hierarchical keywords from `lr:hierarchicalSubject` (pipe-separated in XMP, stored with `/`).
    pub hierarchical_keywords: Vec<String>,
    /// Description from `dc:description`.
    pub description: Option<String>,
    /// Additional metadata: rating, label, creator, copyright.
    pub source_metadata: HashMap<String, String>,
}

impl XmpData {
    pub(crate) fn empty() -> Self {
        Self {
            keywords: Vec::new(),
            hierarchical_keywords: Vec::new(),
            description: None,
            source_metadata: HashMap::new(),
        }
    }
}

/// Which RDF container we're currently inside.
#[derive(Debug, Clone, PartialEq)]
enum Context {
    None,
    SubjectBag,
    HierarchicalBag,
    DescriptionAlt,
    CreatorContainer,
    RightsAlt,
}

/// Return the local name of an XML tag (strip namespace prefix).
fn local_name(tag: &[u8]) -> Vec<u8> {
    match tag.iter().position(|&b| b == b':') {
        Some(pos) => tag[pos + 1..].to_vec(),
        None => tag.to_vec(),
    }
}

/// Extract XMP metadata from a file. Infallible — returns empty data on any error.
pub fn extract(path: &Path) -> XmpData {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return XmpData::empty(),
    };
    parse_xmp(&content)
}

/// Update the `xmp:Rating` value in an XMP file on disk.
///
/// Uses string-based find/replace to preserve all other XMP content byte-for-byte.
/// Returns `Ok(true)` if the file was modified, `Ok(false)` if no change was needed.
/// Rating of `None` or `Some(0)` writes `"0"` (XMP convention for "no rating").
pub fn update_rating(path: &Path, rating: Option<u8>) -> Result<bool> {
    let content = std::fs::read_to_string(path)?;
    let rating_str = match rating {
        Some(r) if r > 0 => r.to_string(),
        _ => "0".to_string(),
    };

    let modified = update_rating_in_string(&content, &rating_str);

    if modified == content {
        return Ok(false);
    }

    std::fs::write(path, &modified)?;
    Ok(true)
}

/// Apply a rating update to an XMP string, returning the modified string.
fn update_rating_in_string(content: &str, rating_str: &str) -> String {
    // Try attribute form: xmp:Rating="..."
    let attr_re = Regex::new(r#"xmp:Rating="[^"]*""#).unwrap();
    if attr_re.is_match(content) {
        return attr_re
            .replace(content, format!(r#"xmp:Rating="{rating_str}""#))
            .into_owned();
    }

    // Try element form: <xmp:Rating>...</xmp:Rating>
    let elem_re = Regex::new(r"<xmp:Rating>[^<]*</xmp:Rating>").unwrap();
    if elem_re.is_match(content) {
        return elem_re
            .replace(
                content,
                format!("<xmp:Rating>{rating_str}</xmp:Rating>"),
            )
            .into_owned();
    }

    // Neither form found — inject attribute if rating > 0
    if rating_str == "0" {
        return content.to_string();
    }

    // Inject xmp:Rating attribute into the first rdf:Description element
    let desc_re = Regex::new(r"(<rdf:Description\b)").unwrap();
    if desc_re.is_match(content) {
        return desc_re
            .replace(
                content,
                format!(r#"${{1}} xmp:Rating="{rating_str}""#),
            )
            .into_owned();
    }

    // No rdf:Description found — can't inject, return unchanged
    content.to_string()
}

/// Update the `dc:subject` keywords in an XMP file on disk.
///
/// Applies delta operations: adds `tags_to_add` and removes `tags_to_remove`
/// from the existing `dc:subject` / `rdf:Bag` keyword list.
/// Preserves tags in the XMP that are not mentioned in either list.
/// Returns `Ok(true)` if the file was modified, `Ok(false)` if no change was needed.
pub fn update_tags(path: &Path, tags_to_add: &[String], tags_to_remove: &[String]) -> Result<bool> {
    if tags_to_add.is_empty() && tags_to_remove.is_empty() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(path)?;
    let modified = update_tags_in_string(&content, tags_to_add, tags_to_remove);
    if modified == content {
        return Ok(false);
    }
    std::fs::write(path, &modified)?;
    Ok(true)
}

/// Apply tag add/remove operations to an XMP string, returning the modified string.
fn update_tags_in_string(content: &str, tags_to_add: &[String], tags_to_remove: &[String]) -> String {
    let remove_set: HashSet<&str> = tags_to_remove.iter().map(|s| s.as_str()).collect();

    // Match existing dc:subject block with rdf:Bag
    let subject_re =
        Regex::new(r"(?s)([ \t]*)<dc:subject>\s*<rdf:Bag>(.*?)</rdf:Bag>\s*</dc:subject>")
            .unwrap();
    let li_re = Regex::new(r"<rdf:li>([^<]*)</rdf:li>").unwrap();

    if let Some(caps) = subject_re.captures(content) {
        let full_match = caps.get(0).unwrap();
        let indent = caps.get(1).unwrap().as_str();
        let bag_content = caps.get(2).unwrap().as_str();

        // Parse existing tags
        let mut tags: Vec<String> = li_re
            .captures_iter(bag_content)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .collect();

        // Apply removals
        tags.retain(|t| !remove_set.contains(t.as_str()));

        // Apply additions (deduplicated)
        for tag in tags_to_add {
            if !tags.iter().any(|t| t == tag) {
                tags.push(tag.clone());
            }
        }

        if tags.is_empty() {
            // Remove the entire dc:subject block including the preceding newline
            let start = full_match.start();
            let end = full_match.end();
            let trim_start = if content[..start].ends_with('\n') {
                start - 1
            } else {
                start
            };
            return format!("{}{}", &content[..trim_start], &content[end..]);
        }

        // Rebuild with same indentation
        let bag_indent = format!("{} ", indent);
        let li_indent = format!("{}  ", indent);
        let mut block = format!("{}<dc:subject>\n{}<rdf:Bag>\n", indent, bag_indent);
        for tag in &tags {
            block.push_str(&format!("{}<rdf:li>{}</rdf:li>\n", li_indent, xml_escape(tag)));
        }
        block.push_str(&format!("{}</rdf:Bag>\n{}</dc:subject>", bag_indent, indent));

        return format!(
            "{}{}{}",
            &content[..full_match.start()],
            block,
            &content[full_match.end()..]
        );
    }

    // No existing dc:subject — only proceed if we have tags to add
    if tags_to_add.is_empty() {
        return content.to_string();
    }

    // Ensure xmlns:dc namespace is declared
    let mut content = content.to_string();
    if !content.contains("xmlns:dc") {
        let desc_re = Regex::new(r#"(<rdf:Description\b)"#).unwrap();
        if desc_re.is_match(&content) {
            content = desc_re
                .replace(
                    &content,
                    r#"${1} xmlns:dc="http://purl.org/dc/elements/1.1/""#,
                )
                .into_owned();
        }
    }

    // Try to inject before </rdf:Description>
    let close_re = Regex::new(r"([ \t]*)</rdf:Description>").unwrap();
    if let Some(caps) = close_re.captures(&content) {
        let m = caps.get(0).unwrap();
        let desc_indent = caps.get(1).unwrap().as_str();
        let indent = format!("{} ", desc_indent);
        let bag_indent = format!("{}  ", desc_indent);
        let li_indent = format!("{}   ", desc_indent);

        let mut block = format!("{}<dc:subject>\n{}<rdf:Bag>\n", indent, bag_indent);
        for tag in tags_to_add {
            block.push_str(&format!("{}<rdf:li>{}</rdf:li>\n", li_indent, xml_escape(tag)));
        }
        block.push_str(&format!(
            "{}</rdf:Bag>\n{}</dc:subject>\n",
            bag_indent, indent
        ));

        return format!("{}{}{}", &content[..m.start()], block, &content[m.start()..]);
    }

    // Try to handle self-closing rdf:Description: convert /> to > and append
    let self_close_re =
        Regex::new(r"(?s)([ \t]*)<rdf:Description\b([^>]*?)/>").unwrap();
    if let Some(caps) = self_close_re.captures(&content) {
        let m = caps.get(0).unwrap();
        let desc_indent = caps.get(1).unwrap().as_str();
        let attrs = caps.get(2).unwrap().as_str();
        let indent = format!("{} ", desc_indent);
        let bag_indent = format!("{}  ", desc_indent);
        let li_indent = format!("{}   ", desc_indent);

        let mut block = format!("{}<rdf:Description{}>\n", desc_indent, attrs);
        block.push_str(&format!("{}<dc:subject>\n{}<rdf:Bag>\n", indent, bag_indent));
        for tag in tags_to_add {
            block.push_str(&format!("{}<rdf:li>{}</rdf:li>\n", li_indent, xml_escape(tag)));
        }
        block.push_str(&format!(
            "{}</rdf:Bag>\n{}</dc:subject>\n{}</rdf:Description>",
            bag_indent, indent, desc_indent
        ));

        return format!("{}{}{}", &content[..m.start()], block, &content[m.end()..]);
    }

    content
}

/// Update the `lr:hierarchicalSubject` keywords in an XMP file on disk.
///
/// Only processes hierarchical tags (containing `/`). Flat tags are ignored.
/// Converts `/` to `|` for XMP storage format.
/// Returns `Ok(true)` if the file was modified, `Ok(false)` if no change was needed.
pub fn update_hierarchical_subjects(
    path: &Path,
    tags_to_add: &[String],
    tags_to_remove: &[String],
) -> Result<bool> {
    // Filter to only hierarchical tags (containing `|` — the internal hierarchy separator)
    let hier_add: Vec<String> = tags_to_add
        .iter()
        .filter(|t| t.contains('|'))
        .cloned()
        .collect();
    let hier_remove: Vec<String> = tags_to_remove
        .iter()
        .filter(|t| t.contains('|'))
        .cloned()
        .collect();

    if hier_add.is_empty() && hier_remove.is_empty() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(path)?;
    let modified = update_hierarchical_in_string(&content, &hier_add, &hier_remove);
    if modified == content {
        return Ok(false);
    }
    std::fs::write(path, &modified)?;
    Ok(true)
}

/// Apply hierarchical subject add/remove operations to an XMP string.
/// Tags use pipe-separated format (e.g., `animals|birds|eagles`).
fn update_hierarchical_in_string(
    content: &str,
    hier_to_add: &[String],
    hier_to_remove: &[String],
) -> String {
    let remove_set: HashSet<&str> = hier_to_remove.iter().map(|s| s.as_str()).collect();

    // Match existing lr:hierarchicalSubject block with rdf:Bag
    let subject_re = Regex::new(
        r"(?s)([ \t]*)<lr:hierarchicalSubject>\s*<rdf:Bag>(.*?)</rdf:Bag>\s*</lr:hierarchicalSubject>",
    )
    .unwrap();
    let li_re = Regex::new(r"<rdf:li>([^<]*)</rdf:li>").unwrap();

    if let Some(caps) = subject_re.captures(content) {
        let full_match = caps.get(0).unwrap();
        let indent = caps.get(1).unwrap().as_str();
        let bag_content = caps.get(2).unwrap().as_str();

        let mut tags: Vec<String> = li_re
            .captures_iter(bag_content)
            .map(|c| c.get(1).unwrap().as_str().to_string())
            .collect();

        tags.retain(|t| !remove_set.contains(t.as_str()));

        for tag in hier_to_add {
            if !tags.iter().any(|t| t == tag) {
                tags.push(tag.clone());
            }
        }

        if tags.is_empty() {
            let start = full_match.start();
            let end = full_match.end();
            let trim_start = if content[..start].ends_with('\n') {
                start - 1
            } else {
                start
            };
            return format!("{}{}", &content[..trim_start], &content[end..]);
        }

        let bag_indent = format!("{} ", indent);
        let li_indent = format!("{}  ", indent);
        let mut block = format!(
            "{}<lr:hierarchicalSubject>\n{}<rdf:Bag>\n",
            indent, bag_indent
        );
        for tag in &tags {
            block.push_str(&format!("{}<rdf:li>{}</rdf:li>\n", li_indent, xml_escape(tag)));
        }
        block.push_str(&format!(
            "{}</rdf:Bag>\n{}</lr:hierarchicalSubject>",
            bag_indent, indent
        ));

        return format!(
            "{}{}{}",
            &content[..full_match.start()],
            block,
            &content[full_match.end()..]
        );
    }

    // No existing lr:hierarchicalSubject — only proceed if we have tags to add
    if hier_to_add.is_empty() {
        return content.to_string();
    }

    // Ensure xmlns:lr namespace is declared
    let mut content = content.to_string();
    if !content.contains("xmlns:lr") {
        let desc_re = Regex::new(r#"(<rdf:Description\b)"#).unwrap();
        if desc_re.is_match(&content) {
            content = desc_re
                .replace(
                    &content,
                    r#"${1} xmlns:lr="http://ns.adobe.com/lightroom/1.0/""#,
                )
                .into_owned();
        }
    }

    // Try to inject before </rdf:Description>
    let close_re = Regex::new(r"([ \t]*)</rdf:Description>").unwrap();
    if let Some(caps) = close_re.captures(&content) {
        let m = caps.get(0).unwrap();
        let desc_indent = caps.get(1).unwrap().as_str();
        let indent = format!("{} ", desc_indent);
        let bag_indent = format!("{}  ", desc_indent);
        let li_indent = format!("{}   ", desc_indent);

        let mut block = format!(
            "{}<lr:hierarchicalSubject>\n{}<rdf:Bag>\n",
            indent, bag_indent
        );
        for tag in hier_to_add {
            block.push_str(&format!("{}<rdf:li>{}</rdf:li>\n", li_indent, xml_escape(tag)));
        }
        block.push_str(&format!(
            "{}</rdf:Bag>\n{}</lr:hierarchicalSubject>\n",
            bag_indent, indent
        ));

        return format!("{}{}{}", &content[..m.start()], block, &content[m.start()..]);
    }

    // Try self-closing rdf:Description
    let self_close_re = Regex::new(r"(?s)([ \t]*)<rdf:Description\b([^>]*?)/>").unwrap();
    if let Some(caps) = self_close_re.captures(&content) {
        let m = caps.get(0).unwrap();
        let desc_indent = caps.get(1).unwrap().as_str();
        let attrs = caps.get(2).unwrap().as_str();
        let indent = format!("{} ", desc_indent);
        let bag_indent = format!("{}  ", desc_indent);
        let li_indent = format!("{}   ", desc_indent);

        let mut block = format!("{}<rdf:Description{}>\n", desc_indent, attrs);
        block.push_str(&format!(
            "{}<lr:hierarchicalSubject>\n{}<rdf:Bag>\n",
            indent, bag_indent
        ));
        for tag in hier_to_add {
            block.push_str(&format!("{}<rdf:li>{}</rdf:li>\n", li_indent, xml_escape(tag)));
        }
        block.push_str(&format!(
            "{}</rdf:Bag>\n{}</lr:hierarchicalSubject>\n{}</rdf:Description>",
            bag_indent, indent, desc_indent
        ));

        return format!("{}{}{}", &content[..m.start()], block, &content[m.end()..]);
    }

    content
}

/// Update the `dc:description` in an XMP file on disk.
///
/// Uses string-based find/replace to preserve all other XMP content byte-for-byte.
/// Returns `Ok(true)` if the file was modified, `Ok(false)` if no change was needed.
/// `description` of `None` or `Some("")` removes the `dc:description` block.
pub fn update_description(path: &Path, description: Option<&str>) -> Result<bool> {
    let content = std::fs::read_to_string(path)?;
    let modified = update_description_in_string(&content, description);
    if modified == content {
        return Ok(false);
    }
    std::fs::write(path, &modified)?;
    Ok(true)
}

/// Apply a description update to an XMP string, returning the modified string.
fn update_description_in_string(content: &str, description: Option<&str>) -> String {
    let desc_text = description.unwrap_or("");

    // Match existing dc:description block with rdf:Alt
    let desc_re = Regex::new(
        r"(?s)([ \t]*)<dc:description>\s*<rdf:Alt>\s*<rdf:li[^>]*>[^<]*</rdf:li>\s*</rdf:Alt>\s*</dc:description>"
    ).unwrap();

    if let Some(caps) = desc_re.captures(content) {
        let full_match = caps.get(0).unwrap();

        if desc_text.is_empty() {
            // Remove the entire dc:description block including the preceding newline
            let start = full_match.start();
            let end = full_match.end();
            let trim_start = if content[..start].ends_with('\n') {
                start - 1
            } else {
                start
            };
            return format!("{}{}", &content[..trim_start], &content[end..]);
        }

        // Replace inner rdf:li text
        let li_re = Regex::new(r"(<rdf:li[^>]*>)[^<]*(</rdf:li>)").unwrap();
        let replaced = li_re.replace(
            full_match.as_str(),
            format!("${{1}}{}{}", xml_escape(desc_text), "${2}"),
        );
        return format!(
            "{}{}{}",
            &content[..full_match.start()],
            replaced,
            &content[full_match.end()..]
        );
    }

    // No existing dc:description — only proceed if we have text to add
    if desc_text.is_empty() {
        return content.to_string();
    }

    // Ensure xmlns:dc namespace is declared
    let mut content = content.to_string();
    if !content.contains("xmlns:dc") {
        let ns_re = Regex::new(r#"(<rdf:Description\b)"#).unwrap();
        if ns_re.is_match(&content) {
            content = ns_re
                .replace(
                    &content,
                    r#"${1} xmlns:dc="http://purl.org/dc/elements/1.1/""#,
                )
                .into_owned();
        }
    }

    // Try to inject before </rdf:Description>
    let close_re = Regex::new(r"([ \t]*)</rdf:Description>").unwrap();
    if let Some(caps) = close_re.captures(&content) {
        let m = caps.get(0).unwrap();
        let desc_indent = caps.get(1).unwrap().as_str();
        let indent = format!("{} ", desc_indent);
        let alt_indent = format!("{}  ", desc_indent);
        let li_indent = format!("{}   ", desc_indent);

        let block = format!(
            "{}<dc:description>\n{}<rdf:Alt>\n{}<rdf:li xml:lang=\"x-default\">{}</rdf:li>\n{}</rdf:Alt>\n{}</dc:description>\n",
            indent, alt_indent, li_indent, xml_escape(desc_text), alt_indent, indent
        );

        return format!("{}{}{}", &content[..m.start()], block, &content[m.start()..]);
    }

    // Try to handle self-closing rdf:Description: convert /> to > and append
    let self_close_re = Regex::new(r"(?s)([ \t]*)<rdf:Description\b([^>]*?)/>").unwrap();
    if let Some(caps) = self_close_re.captures(&content) {
        let m = caps.get(0).unwrap();
        let desc_indent = caps.get(1).unwrap().as_str();
        let attrs = caps.get(2).unwrap().as_str();
        let indent = format!("{} ", desc_indent);
        let alt_indent = format!("{}  ", desc_indent);
        let li_indent = format!("{}   ", desc_indent);

        let block = format!(
            "{}<rdf:Description{}>\n{}<dc:description>\n{}<rdf:Alt>\n{}<rdf:li xml:lang=\"x-default\">{}</rdf:li>\n{}</rdf:Alt>\n{}</dc:description>\n{}</rdf:Description>",
            desc_indent, attrs, indent, alt_indent, li_indent, xml_escape(desc_text), alt_indent, indent, desc_indent
        );

        return format!("{}{}{}", &content[..m.start()], block, &content[m.end()..]);
    }

    content
}

/// Update the `xmp:Label` value in an XMP file on disk.
///
/// Uses string-based find/replace to preserve all other XMP content byte-for-byte.
/// Returns `Ok(true)` if the file was modified, `Ok(false)` if no change was needed.
/// `None` removes the label attribute/element entirely (unlike rating which uses "0").
pub fn update_label(path: &Path, label: Option<&str>) -> Result<bool> {
    let content = std::fs::read_to_string(path)?;
    let modified = update_label_in_string(&content, label);
    if modified == content {
        return Ok(false);
    }
    std::fs::write(path, &modified)?;
    Ok(true)
}

/// Apply a label update to an XMP string, returning the modified string.
fn update_label_in_string(content: &str, label: Option<&str>) -> String {
    let label_str = label.unwrap_or("");

    // Try attribute form: xmp:Label="..."
    let attr_re = Regex::new(r#"\s*xmp:Label="[^"]*""#).unwrap();
    if attr_re.is_match(content) {
        if label_str.is_empty() {
            // Remove the attribute (including leading whitespace)
            return attr_re.replace(content, "").into_owned();
        }
        // Replace — use the version without leading \s* to preserve spacing
        let replace_re = Regex::new(r#"xmp:Label="[^"]*""#).unwrap();
        return replace_re
            .replace(content, format!(r#"xmp:Label="{label_str}""#))
            .into_owned();
    }

    // Try element form: <xmp:Label>...</xmp:Label>
    let elem_re = Regex::new(r"[ \t]*<xmp:Label>[^<]*</xmp:Label>\n?").unwrap();
    if elem_re.is_match(content) {
        if label_str.is_empty() {
            // Remove the element
            return elem_re.replace(content, "").into_owned();
        }
        let replace_re = Regex::new(r"<xmp:Label>[^<]*</xmp:Label>").unwrap();
        return replace_re
            .replace(content, format!("<xmp:Label>{label_str}</xmp:Label>"))
            .into_owned();
    }

    // Neither form found — inject attribute if label is non-empty
    if label_str.is_empty() {
        return content.to_string();
    }

    // Inject xmp:Label attribute into the first rdf:Description element
    let desc_re = Regex::new(r"(<rdf:Description\b)").unwrap();
    if desc_re.is_match(content) {
        return desc_re
            .replace(
                content,
                format!(r#"${{1}} xmp:Label="{label_str}""#),
            )
            .into_owned();
    }

    // No rdf:Description found — can't inject, return unchanged
    content.to_string()
}

/// Escape special XML characters in a string.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Parse XMP metadata from an XML string.
pub(crate) fn parse_xmp(xml: &str) -> XmpData {
    let mut data = XmpData::empty();
    let mut reader = Reader::from_str(xml);

    let mut context = Context::None;
    let mut in_li = false;
    let mut capture_rating = false;
    let mut capture_label = false;
    let mut text_buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                handle_open_tag(
                    &name, e, &mut context, &mut in_li,
                    &mut capture_rating, &mut capture_label,
                    &mut text_buf, &mut data,
                );
            }
            Ok(Event::Empty(ref e)) => {
                let name = local_name(e.name().as_ref());
                handle_open_tag(
                    &name, e, &mut context, &mut in_li,
                    &mut capture_rating, &mut capture_label,
                    &mut text_buf, &mut data,
                );
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(t) = e.unescape() {
                    if in_li || capture_rating || capture_label {
                        text_buf.push_str(&t);
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_slice() {
                    b"li" => {
                        if in_li {
                            let text = text_buf.trim().to_string();
                            if !text.is_empty() {
                                match context {
                                    Context::SubjectBag => {
                                        data.keywords.push(text);
                                    }
                                    Context::HierarchicalBag => {
                                        // Keep pipe-separated form as-is — `|` is the
                                        // internal hierarchy separator.
                                        data.hierarchical_keywords.push(text);
                                    }
                                    Context::DescriptionAlt => {
                                        if data.description.is_none() {
                                            data.description = Some(text);
                                        }
                                    }
                                    Context::CreatorContainer => {
                                        data.source_metadata
                                            .entry("creator".to_string())
                                            .or_insert(text);
                                    }
                                    Context::RightsAlt => {
                                        data.source_metadata
                                            .entry("copyright".to_string())
                                            .or_insert(text);
                                    }
                                    Context::None => {}
                                }
                            }
                            in_li = false;
                            text_buf.clear();
                        }
                    }
                    b"Rating" => {
                        if capture_rating {
                            let val = text_buf.trim().to_string();
                            if !val.is_empty() && val != "0" {
                                data.source_metadata.insert("rating".to_string(), val);
                            }
                            capture_rating = false;
                            text_buf.clear();
                        }
                    }
                    b"Label" => {
                        if capture_label {
                            let val = text_buf.trim().to_string();
                            if !val.is_empty() {
                                data.source_metadata.insert("label".to_string(), val);
                            }
                            capture_label = false;
                            text_buf.clear();
                        }
                    }
                    b"subject" | b"hierarchicalSubject" | b"description" | b"creator" | b"rights" => {
                        context = Context::None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    data
}

/// Handle a Start or Empty element event.
fn handle_open_tag(
    name: &[u8],
    e: &quick_xml::events::BytesStart<'_>,
    context: &mut Context,
    in_li: &mut bool,
    capture_rating: &mut bool,
    capture_label: &mut bool,
    text_buf: &mut String,
    data: &mut XmpData,
) {
    match name {
        b"Description" => {
            for attr in e.attributes().flatten() {
                let key = local_name(attr.key.as_ref());
                let val = String::from_utf8_lossy(&attr.value).to_string();
                match key.as_slice() {
                    b"Rating" => {
                        if !val.is_empty() && val != "0" {
                            data.source_metadata.insert("rating".to_string(), val);
                        }
                    }
                    b"Label" => {
                        if !val.is_empty() {
                            data.source_metadata.insert("label".to_string(), val);
                        }
                    }
                    _ => {}
                }
            }
        }
        b"subject" => *context = Context::SubjectBag,
        b"hierarchicalSubject" => *context = Context::HierarchicalBag,
        b"description" => *context = Context::DescriptionAlt,
        b"creator" => *context = Context::CreatorContainer,
        b"rights" => *context = Context::RightsAlt,
        b"Rating" => {
            if !data.source_metadata.contains_key("rating") {
                *capture_rating = true;
                text_buf.clear();
            }
        }
        b"Label" => {
            if !data.source_metadata.contains_key("label") {
                *capture_label = true;
                text_buf.clear();
            }
        }
        b"li" => {
            if *context != Context::None {
                *in_li = true;
                text_buf.clear();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn empty_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.xmp");
        std::fs::write(&path, "").unwrap();

        let data = extract(&path);
        assert!(data.keywords.is_empty());
        assert!(data.description.is_none());
        assert!(data.source_metadata.is_empty());
    }

    #[test]
    fn nonexistent_file_returns_empty() {
        let data = extract(&PathBuf::from("/nonexistent/file.xmp"));
        assert!(data.keywords.is_empty());
        assert!(data.description.is_none());
        assert!(data.source_metadata.is_empty());
    }

    #[test]
    fn full_xmp_extracts_all_fields() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4"
    xmp:Label="Blue">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
     <rdf:li>sunset</rdf:li>
     <rdf:li>ocean</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">A beautiful sunset over the ocean</rdf:li>
    </rdf:Alt>
   </dc:description>
   <dc:creator>
    <rdf:Seq>
     <rdf:li>John Doe</rdf:li>
    </rdf:Seq>
   </dc:creator>
   <dc:rights>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Copyright 2024 John Doe</rdf:li>
    </rdf:Alt>
   </dc:rights>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("full.xmp");
        std::fs::write(&path, xmp).unwrap();

        let data = extract(&path);
        assert_eq!(data.keywords, vec!["landscape", "sunset", "ocean"]);
        assert_eq!(
            data.description.as_deref(),
            Some("A beautiful sunset over the ocean")
        );
        assert_eq!(data.source_metadata.get("rating").unwrap(), "4");
        assert_eq!(data.source_metadata.get("label").unwrap(), "Blue");
        assert_eq!(data.source_metadata.get("creator").unwrap(), "John Doe");
        assert_eq!(
            data.source_metadata.get("copyright").unwrap(),
            "Copyright 2024 John Doe"
        );
    }

    #[test]
    fn partial_xmp_returns_available_fields() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>portrait</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let data = parse_xmp(xmp);
        assert_eq!(data.keywords, vec!["portrait"]);
        assert!(data.description.is_none());
        assert_eq!(data.source_metadata.get("rating").unwrap(), "3");
        assert!(!data.source_metadata.contains_key("label"));
        assert!(!data.source_metadata.contains_key("creator"));
        assert!(!data.source_metadata.contains_key("copyright"));
    }

    #[test]
    fn attributes_on_rdf_description() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="5"
    xmp:Label="Red"/>
 </rdf:RDF>
</x:xmpmeta>"#;

        let data = parse_xmp(xmp);
        assert_eq!(data.source_metadata.get("rating").unwrap(), "5");
        assert_eq!(data.source_metadata.get("label").unwrap(), "Red");
    }

    #[test]
    fn element_form_rating_and_label() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating>2</xmp:Rating>
   <xmp:Label>Green</xmp:Label>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let data = parse_xmp(xmp);
        assert_eq!(data.source_metadata.get("rating").unwrap(), "2");
        assert_eq!(data.source_metadata.get("label").unwrap(), "Green");
    }

    // ── hierarchical subject tests ──────────────────────────

    #[test]
    fn parse_hierarchical_subject() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>animals</rdf:li>
     <rdf:li>birds</rdf:li>
     <rdf:li>eagles</rdf:li>
     <rdf:li>sunset</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>animals|birds|eagles</rdf:li>
     <rdf:li>nature|sky|sunset</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let data = parse_xmp(xmp);
        assert_eq!(data.keywords, vec!["animals", "birds", "eagles", "sunset"]);
        assert_eq!(
            data.hierarchical_keywords,
            vec!["animals|birds|eagles", "nature|sky|sunset"]
        );
    }

    #[test]
    fn parse_hierarchical_subject_single_level() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/">
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let data = parse_xmp(xmp);
        assert!(data.keywords.is_empty());
        assert_eq!(data.hierarchical_keywords, vec!["landscape"]);
    }

    #[test]
    fn parse_no_hierarchical_subject() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let data = parse_xmp(xmp);
        assert_eq!(data.keywords, vec!["landscape"]);
        assert!(data.hierarchical_keywords.is_empty());
    }

    // ── update_rating tests ──────────────────────────────────

    #[test]
    fn update_rating_attribute_form() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3"
    xmp:Label="Blue">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_rating_in_string(xmp, "5");
        assert!(result.contains(r#"xmp:Rating="5""#));
        assert!(result.contains(r#"xmp:Label="Blue""#));
        assert!(!result.contains(r#"xmp:Rating="3""#));
    }

    #[test]
    fn update_rating_element_form() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating>2</xmp:Rating>
   <xmp:Label>Green</xmp:Label>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_rating_in_string(xmp, "4");
        assert!(result.contains("<xmp:Rating>4</xmp:Rating>"));
        assert!(result.contains("<xmp:Label>Green</xmp:Label>"));
        assert!(!result.contains("<xmp:Rating>2</xmp:Rating>"));
    }

    #[test]
    fn update_rating_inject_when_missing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Label="Red">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_rating_in_string(xmp, "3");
        assert!(result.contains(r#"xmp:Rating="3""#));
        assert!(result.contains(r#"xmp:Label="Red""#));
    }

    #[test]
    fn update_rating_clear_sets_zero() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_rating_in_string(xmp, "0");
        assert!(result.contains(r#"xmp:Rating="0""#));
        assert!(!result.contains(r#"xmp:Rating="4""#));
    }

    #[test]
    fn update_rating_no_inject_when_clearing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_rating_in_string(xmp, "0");
        // Should not inject xmp:Rating="0" when there's no existing rating
        assert!(!result.contains("xmp:Rating"));
        assert_eq!(result, xmp);
    }

    #[test]
    fn update_rating_preserves_other_content() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="2"
    xmp:Label="Blue">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
     <rdf:li>sunset</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">A beautiful sunset</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_rating_in_string(xmp, "5");
        assert!(result.contains(r#"xmp:Rating="5""#));
        assert!(result.contains(r#"xmp:Label="Blue""#));
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(result.contains("<rdf:li>sunset</rdf:li>"));
        assert!(result.contains("A beautiful sunset"));
    }

    #[test]
    fn update_rating_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="1">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_rating(&path, Some(4)).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#"xmp:Rating="4""#));
    }

    #[test]
    fn update_rating_nonexistent_file() {
        let result = update_rating(Path::new("/nonexistent/file.xmp"), Some(3));
        assert!(result.is_err());
    }

    #[test]
    fn update_rating_no_change_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_rating(&path, Some(3)).unwrap();
        assert!(!modified);
    }

    // ── update_tags tests ────────────────────────────────────

    #[test]
    fn update_tags_add_to_existing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
     <rdf:li>sunset</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["ocean".to_string()],
            &[],
        );
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(result.contains("<rdf:li>sunset</rdf:li>"));
        assert!(result.contains("<rdf:li>ocean</rdf:li>"));
    }

    #[test]
    fn update_tags_remove_from_existing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
     <rdf:li>sunset</rdf:li>
     <rdf:li>ocean</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &[],
            &["sunset".to_string()],
        );
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(!result.contains("<rdf:li>sunset</rdf:li>"));
        assert!(result.contains("<rdf:li>ocean</rdf:li>"));
    }

    #[test]
    fn update_tags_add_and_remove() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
     <rdf:li>sunset</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["mountains".to_string()],
            &["sunset".to_string()],
        );
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(!result.contains("<rdf:li>sunset</rdf:li>"));
        assert!(result.contains("<rdf:li>mountains</rdf:li>"));
    }

    #[test]
    fn update_tags_remove_all_removes_block() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmp:Rating="3">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &[],
            &["landscape".to_string()],
        );
        assert!(!result.contains("dc:subject"));
        assert!(!result.contains("rdf:Bag"));
        assert!(!result.contains("landscape"));
        // Other content preserved
        assert!(result.contains("xmp:Rating"));
    }

    #[test]
    fn update_tags_inject_when_no_subject() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["landscape".to_string(), "sunset".to_string()],
            &[],
        );
        assert!(result.contains("<dc:subject>"));
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(result.contains("<rdf:li>sunset</rdf:li>"));
        assert!(result.contains("xmp:Rating"));
    }

    #[test]
    fn update_tags_inject_adds_xmlns_dc() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["portrait".to_string()],
            &[],
        );
        assert!(result.contains("xmlns:dc"));
        assert!(result.contains("<rdf:li>portrait</rdf:li>"));
    }

    #[test]
    fn update_tags_inject_self_closing_description() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3"/>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["nature".to_string()],
            &[],
        );
        assert!(result.contains("xmlns:dc"));
        assert!(result.contains("<rdf:li>nature</rdf:li>"));
        assert!(result.contains("</rdf:Description>"));
        assert!(!result.contains("/>"));
    }

    #[test]
    fn update_tags_no_change_add_existing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["landscape".to_string()],
            &[],
        );
        // Should still contain the tag, and the content should round-trip
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
    }

    #[test]
    fn update_tags_remove_nonexistent_is_noop() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &[],
            &["nonexistent".to_string()],
        );
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
    }

    #[test]
    fn update_tags_preserves_other_content() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4"
    xmp:Label="Blue">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">A beautiful sunset</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["ocean".to_string()],
            &[],
        );
        assert!(result.contains(r#"xmp:Rating="4""#));
        assert!(result.contains(r#"xmp:Label="Blue""#));
        assert!(result.contains("A beautiful sunset"));
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(result.contains("<rdf:li>ocean</rdf:li>"));
    }

    #[test]
    fn update_tags_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_tags(&path, &["ocean".to_string()], &["landscape".to_string()]).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<rdf:li>ocean</rdf:li>"));
        assert!(!content.contains("<rdf:li>landscape</rdf:li>"));
    }

    #[test]
    fn update_tags_xml_escapes_special_chars() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>existing</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_tags_in_string(
            xmp,
            &["black & white".to_string()],
            &[],
        );
        assert!(result.contains("<rdf:li>black &amp; white</rdf:li>"));
    }

    // ── update_description tests ──────────────────────────────

    #[test]
    fn update_description_existing_block() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Old description</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some("New description"));
        assert!(result.contains("New description"));
        assert!(!result.contains("Old description"));
        assert!(result.contains(r#"xmp:Rating="3""#));
    }

    #[test]
    fn update_description_clear_removes_block() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmp:Rating="4">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Remove me</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, None);
        assert!(!result.contains("dc:description"));
        assert!(!result.contains("Remove me"));
        assert!(result.contains("xmp:Rating"));
    }

    #[test]
    fn update_description_clear_with_empty_string() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Remove me</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some(""));
        assert!(!result.contains("dc:description"));
    }

    #[test]
    fn update_description_inject_when_missing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some("Injected description"));
        assert!(result.contains("dc:description"));
        assert!(result.contains("Injected description"));
        assert!(result.contains("rdf:Alt"));
        assert!(result.contains(r#"xml:lang="x-default""#));
        assert!(result.contains("xmp:Rating"));
    }

    #[test]
    fn update_description_inject_adds_xmlns_dc() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some("New desc"));
        assert!(result.contains("xmlns:dc"));
        assert!(result.contains("New desc"));
    }

    #[test]
    fn update_description_inject_self_closing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3"/>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some("Self-closing test"));
        assert!(result.contains("xmlns:dc"));
        assert!(result.contains("Self-closing test"));
        assert!(result.contains("</rdf:Description>"));
        assert!(!result.contains("/>"));
    }

    #[test]
    fn update_description_preserves_other_content() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4"
    xmp:Label="Blue">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
     <rdf:li>sunset</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">A beautiful sunset</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some("Updated sunset"));
        assert!(result.contains(r#"xmp:Rating="4""#));
        assert!(result.contains(r#"xmp:Label="Blue""#));
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
        assert!(result.contains("<rdf:li>sunset</rdf:li>"));
        assert!(result.contains("Updated sunset"));
        assert!(!result.contains("A beautiful sunset"));
    }

    #[test]
    fn update_description_xml_escapes() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">old</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, Some("black & white <nice>"));
        assert!(result.contains("black &amp; white &lt;nice&gt;"));
    }

    #[test]
    fn update_description_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Original</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_description(&path, Some("Updated")).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Updated"));
        assert!(!content.contains("Original"));
    }

    #[test]
    fn update_description_no_change_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Same text</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_description(&path, Some("Same text")).unwrap();
        assert!(!modified);
    }

    #[test]
    fn update_description_none_no_existing_is_noop() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_description_in_string(xmp, None);
        assert_eq!(result, xmp);
    }

    // ── update_label tests ──────────────────────────────────

    #[test]
    fn update_label_attribute_form() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3"
    xmp:Label="Blue">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, Some("Red"));
        assert!(result.contains(r#"xmp:Label="Red""#));
        assert!(!result.contains(r#"xmp:Label="Blue""#));
        assert!(result.contains(r#"xmp:Rating="3""#));
    }

    #[test]
    fn update_label_element_form() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating>2</xmp:Rating>
   <xmp:Label>Green</xmp:Label>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, Some("Yellow"));
        assert!(result.contains("<xmp:Label>Yellow</xmp:Label>"));
        assert!(!result.contains("<xmp:Label>Green</xmp:Label>"));
        assert!(result.contains("<xmp:Rating>2</xmp:Rating>"));
    }

    #[test]
    fn update_label_clear_removes_attribute() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4"
    xmp:Label="Blue">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, None);
        assert!(!result.contains("xmp:Label"));
        assert!(result.contains(r#"xmp:Rating="4""#));
    }

    #[test]
    fn update_label_clear_removes_element() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/">
   <xmp:Rating>2</xmp:Rating>
   <xmp:Label>Green</xmp:Label>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, None);
        assert!(!result.contains("xmp:Label"));
        assert!(result.contains("<xmp:Rating>2</xmp:Rating>"));
    }

    #[test]
    fn update_label_inject_when_missing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, Some("Red"));
        assert!(result.contains(r#"xmp:Label="Red""#));
        assert!(result.contains(r#"xmp:Rating="3""#));
    }

    #[test]
    fn update_label_no_inject_when_clearing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, None);
        assert!(!result.contains("xmp:Label"));
        assert_eq!(result, xmp);
    }

    #[test]
    fn update_label_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Label="Blue">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_label(&path, Some("Green")).unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#"xmp:Label="Green""#));
    }

    #[test]
    fn update_label_no_change_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Label="Red">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        let modified = update_label(&path, Some("Red")).unwrap();
        assert!(!modified);
    }

    #[test]
    fn update_label_preserves_other_content() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4"
    xmp:Label="Blue">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_label_in_string(xmp, Some("Purple"));
        assert!(result.contains(r#"xmp:Label="Purple""#));
        assert!(result.contains(r#"xmp:Rating="4""#));
        assert!(result.contains("<rdf:li>landscape</rdf:li>"));
    }

    // ── update_hierarchical_subjects tests ──────────────────

    #[test]
    fn update_hierarchical_add_to_existing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/">
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>animals|birds</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_hierarchical_in_string(
            xmp,
            &["nature|sky|sunset".to_string()],
            &[],
        );
        assert!(result.contains("<rdf:li>animals|birds</rdf:li>"));
        assert!(result.contains("<rdf:li>nature|sky|sunset</rdf:li>"));
    }

    #[test]
    fn update_hierarchical_remove_from_existing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/">
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>animals|birds|eagles</rdf:li>
     <rdf:li>nature|sunset</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_hierarchical_in_string(
            xmp,
            &[],
            &["animals|birds|eagles".to_string()],
        );
        assert!(!result.contains("animals|birds|eagles"));
        assert!(result.contains("<rdf:li>nature|sunset</rdf:li>"));
    }

    #[test]
    fn update_hierarchical_remove_all_removes_block() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/"
    xmp:Rating="3">
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>animals|birds</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_hierarchical_in_string(
            xmp,
            &[],
            &["animals|birds".to_string()],
        );
        assert!(!result.contains("lr:hierarchicalSubject"));
        assert!(!result.contains("animals|birds"));
        assert!(result.contains("xmp:Rating"));
    }

    #[test]
    fn update_hierarchical_inject_when_missing() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmp:Rating="3">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        let result = update_hierarchical_in_string(
            xmp,
            &["animals|birds|eagles".to_string()],
            &[],
        );
        assert!(result.contains("lr:hierarchicalSubject"));
        assert!(result.contains("xmlns:lr"));
        assert!(result.contains("<rdf:li>animals|birds|eagles</rdf:li>"));
        assert!(result.contains("xmp:Rating"));
    }

    #[test]
    fn update_hierarchical_subjects_filters_flat_tags() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xmp");
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/">
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
        std::fs::write(&path, xmp).unwrap();

        // Flat tags should be ignored
        let modified = update_hierarchical_subjects(
            &path,
            &["landscape".to_string()],
            &[],
        )
        .unwrap();
        assert!(!modified, "flat tags should be ignored by update_hierarchical_subjects");

        // Hierarchical tags (containing `|`) should be written
        let modified = update_hierarchical_subjects(
            &path,
            &["animals|birds|eagles".to_string()],
            &[],
        )
        .unwrap();
        assert!(modified);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("animals|birds|eagles"));
    }

    #[test]
    fn update_hierarchical_round_trip() {
        let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/">
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>animals|birds|eagles</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

        // Parse it
        let data = parse_xmp(xmp);
        assert_eq!(data.hierarchical_keywords, vec!["animals|birds|eagles"]);

        // Add a new hierarchical tag
        let result = update_hierarchical_in_string(
            xmp,
            &["nature|sky|sunset".to_string()],
            &[],
        );
        assert!(result.contains("<rdf:li>animals|birds|eagles</rdf:li>"));
        assert!(result.contains("<rdf:li>nature|sky|sunset</rdf:li>"));

        // Parse the result — should have both
        let data2 = parse_xmp(&result);
        assert_eq!(
            data2.hierarchical_keywords,
            vec!["animals|birds|eagles", "nature|sky|sunset"]
        );
    }
}
