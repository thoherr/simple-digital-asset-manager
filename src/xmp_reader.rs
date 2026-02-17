use std::collections::HashMap;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

/// Extracted metadata from an XMP sidecar file.
pub struct XmpData {
    /// Keywords from `dc:subject`.
    pub keywords: Vec<String>,
    /// Description from `dc:description`.
    pub description: Option<String>,
    /// Additional metadata: rating, label, creator, copyright.
    pub source_metadata: HashMap<String, String>,
}

impl XmpData {
    fn empty() -> Self {
        Self {
            keywords: Vec::new(),
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

/// Parse XMP metadata from an XML string.
fn parse_xmp(xml: &str) -> XmpData {
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
                    b"subject" | b"description" | b"creator" | b"rights" => {
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
}
