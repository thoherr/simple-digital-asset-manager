use std::path::Path;

use crate::xmp_reader::{XmpData, parse_xmp};

/// XMP namespace identifier in JPEG APP1 markers.
const XMP_NAMESPACE: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Extract embedded XMP metadata from a JPEG or TIFF file.
///
/// Returns `XmpData::empty()` for unsupported formats or on any error.
pub fn extract_embedded_xmp(path: &Path) -> XmpData {
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => return XmpData::empty(),
    };

    match ext.as_str() {
        "jpg" | "jpeg" => extract_from_jpeg(path),
        "tif" | "tiff" => extract_from_tiff(path),
        _ => XmpData::empty(),
    }
}

/// Extract XMP XML from a JPEG file's APP1 marker.
fn extract_from_jpeg(path: &Path) -> XmpData {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return XmpData::empty(),
    };

    match extract_xmp_xml_from_jpeg(&data) {
        Some(xml) => parse_xmp(&xml),
        None => XmpData::empty(),
    }
}

/// Parse JPEG binary data to find XMP XML in an APP1 marker.
fn extract_xmp_xml_from_jpeg(data: &[u8]) -> Option<String> {
    // Verify SOI marker
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }

    let mut pos = 2;
    while pos + 4 <= data.len() {
        // Each marker starts with 0xFF
        if data[pos] != 0xFF {
            return None;
        }

        let marker = data[pos + 1];

        // SOS (Start of Scan) — no more metadata markers after this
        if marker == 0xDA {
            return None;
        }

        // Skip padding bytes (0xFF)
        if marker == 0xFF {
            pos += 1;
            continue;
        }

        // Markers without length (SOI, EOI, RST0-RST7)
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }

        // Read 2-byte segment length (includes the length bytes themselves)
        if pos + 4 > data.len() {
            return None;
        }
        let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if length < 2 {
            return None;
        }

        let segment_start = pos + 4; // payload starts after marker (2) + length (2)
        let segment_end = pos + 2 + length; // marker (2) + length bytes

        if segment_end > data.len() {
            return None;
        }

        // APP1 marker = 0xE1
        if marker == 0xE1 {
            let payload = &data[segment_start..segment_end];
            if payload.len() > XMP_NAMESPACE.len()
                && payload[..XMP_NAMESPACE.len()] == *XMP_NAMESPACE
            {
                let xml_bytes = &payload[XMP_NAMESPACE.len()..];
                if let Ok(xml) = std::str::from_utf8(xml_bytes) {
                    return Some(xml.to_string());
                }
            }
        }

        pos = segment_end;
    }

    None
}

/// Extract XMP XML from a TIFF file's IFD tag 700.
fn extract_from_tiff(path: &Path) -> XmpData {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return XmpData::empty(),
    };

    match extract_xmp_xml_from_tiff(&data) {
        Some(xml) => parse_xmp(&xml),
        None => XmpData::empty(),
    }
}

/// Parse TIFF binary data to find XMP XML in tag 700 (0x02BC).
fn extract_xmp_xml_from_tiff(data: &[u8]) -> Option<String> {
    if data.len() < 8 {
        return None;
    }

    // Determine byte order
    let big_endian = match &data[0..2] {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };

    let read_u16 = |offset: usize| -> Option<u16> {
        if offset + 2 > data.len() {
            return None;
        }
        Some(if big_endian {
            u16::from_be_bytes([data[offset], data[offset + 1]])
        } else {
            u16::from_le_bytes([data[offset], data[offset + 1]])
        })
    };

    let read_u32 = |offset: usize| -> Option<u32> {
        if offset + 4 > data.len() {
            return None;
        }
        Some(if big_endian {
            u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
        } else {
            u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
        })
    };

    // Verify magic number 42
    if read_u16(2)? != 42 {
        return None;
    }

    // Get offset to first IFD
    let mut ifd_offset = read_u32(4)? as usize;

    // Walk IFDs (limit iterations to prevent infinite loops)
    for _ in 0..100 {
        if ifd_offset == 0 || ifd_offset + 2 > data.len() {
            return None;
        }

        let entry_count = read_u16(ifd_offset)? as usize;
        let entries_start = ifd_offset + 2;

        for i in 0..entry_count {
            let entry_offset = entries_start + i * 12;
            if entry_offset + 12 > data.len() {
                return None;
            }

            let tag = read_u16(entry_offset)?;

            // Tag 700 = XMP (0x02BC)
            if tag == 0x02BC {
                let count = read_u32(entry_offset + 4)? as usize;
                let value_offset = if count <= 4 {
                    // Value is inline in the entry
                    entry_offset + 8
                } else {
                    read_u32(entry_offset + 8)? as usize
                };

                if value_offset + count > data.len() {
                    return None;
                }

                let xml_bytes = &data[value_offset..value_offset + count];
                if let Ok(xml) = std::str::from_utf8(xml_bytes) {
                    return Some(xml.to_string());
                }
            }
        }

        // Move to next IFD
        let next_ifd_ptr = entries_start + entry_count * 12;
        if next_ifd_ptr + 4 > data.len() {
            return None;
        }
        ifd_offset = read_u32(next_ifd_ptr)? as usize;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal JPEG with an XMP APP1 marker.
    fn build_jpeg_with_xmp(xmp_xml: &str) -> Vec<u8> {
        let mut data = Vec::new();

        // SOI
        data.extend_from_slice(&[0xFF, 0xD8]);

        // APP1 marker with XMP namespace
        data.extend_from_slice(&[0xFF, 0xE1]);
        let payload_len = XMP_NAMESPACE.len() + xmp_xml.len();
        let segment_len = (payload_len + 2) as u16; // +2 for length bytes
        data.extend_from_slice(&segment_len.to_be_bytes());
        data.extend_from_slice(XMP_NAMESPACE);
        data.extend_from_slice(xmp_xml.as_bytes());

        // EOI
        data.extend_from_slice(&[0xFF, 0xD9]);

        data
    }

    /// Build a minimal JPEG with an EXIF APP1 marker (no XMP).
    fn build_jpeg_with_exif() -> Vec<u8> {
        let mut data = Vec::new();

        // SOI
        data.extend_from_slice(&[0xFF, 0xD8]);

        // APP1 marker with EXIF identifier
        data.extend_from_slice(&[0xFF, 0xE1]);
        let exif_header = b"Exif\0\0";
        let segment_len = (exif_header.len() + 2) as u16;
        data.extend_from_slice(&segment_len.to_be_bytes());
        data.extend_from_slice(exif_header);

        // EOI
        data.extend_from_slice(&[0xFF, 0xD9]);

        data
    }

    /// Build a minimal little-endian TIFF with an XMP tag (700).
    fn build_tiff_le_with_xmp(xmp_xml: &str) -> Vec<u8> {
        let mut data = Vec::new();
        let xmp_bytes = xmp_xml.as_bytes();

        // TIFF header: little-endian, magic 42, IFD offset = 8
        data.extend_from_slice(b"II");
        data.extend_from_slice(&42u16.to_le_bytes());
        data.extend_from_slice(&8u32.to_le_bytes());

        // IFD at offset 8
        let entry_count: u16 = 1;
        data.extend_from_slice(&entry_count.to_le_bytes()); // 2 bytes

        // IFD entry: tag=700, type=BYTE(1), count, value_offset
        data.extend_from_slice(&700u16.to_le_bytes()); // tag
        data.extend_from_slice(&1u16.to_le_bytes()); // type = BYTE
        data.extend_from_slice(&(xmp_bytes.len() as u32).to_le_bytes()); // count
        // XMP data offset: header(8) + entry_count(2) + entry(12) + next_ifd(4) = 26
        let xmp_offset: u32 = 26;
        data.extend_from_slice(&xmp_offset.to_le_bytes()); // value offset

        // Next IFD offset = 0 (no more IFDs)
        data.extend_from_slice(&0u32.to_le_bytes());

        // XMP data at offset 26
        data.extend_from_slice(xmp_bytes);

        data
    }

    /// Build a minimal big-endian TIFF with an XMP tag (700).
    fn build_tiff_be_with_xmp(xmp_xml: &str) -> Vec<u8> {
        let mut data = Vec::new();
        let xmp_bytes = xmp_xml.as_bytes();

        // TIFF header: big-endian, magic 42, IFD offset = 8
        data.extend_from_slice(b"MM");
        data.extend_from_slice(&42u16.to_be_bytes());
        data.extend_from_slice(&8u32.to_be_bytes());

        // IFD at offset 8
        let entry_count: u16 = 1;
        data.extend_from_slice(&entry_count.to_be_bytes());

        // IFD entry: tag=700, type=BYTE(1), count, value_offset
        data.extend_from_slice(&700u16.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&(xmp_bytes.len() as u32).to_be_bytes());
        let xmp_offset: u32 = 26;
        data.extend_from_slice(&xmp_offset.to_be_bytes());

        // Next IFD offset = 0
        data.extend_from_slice(&0u32.to_be_bytes());

        // XMP data at offset 26
        data.extend_from_slice(xmp_bytes);

        data
    }

    fn sample_xmp_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
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
</x:xmpmeta>"#
    }

    #[test]
    fn extract_xmp_from_jpeg_basic() {
        let xmp_xml = sample_xmp_xml();
        let jpeg_data = build_jpeg_with_xmp(xmp_xml);
        let result = extract_xmp_xml_from_jpeg(&jpeg_data);
        assert!(result.is_some());
        let xml = result.unwrap();
        assert!(xml.contains("landscape"));
        assert!(xml.contains("xmp:Rating"));
    }

    #[test]
    fn extract_xmp_from_jpeg_no_xmp() {
        let jpeg_data = build_jpeg_with_exif();
        let result = extract_xmp_xml_from_jpeg(&jpeg_data);
        assert!(result.is_none());
    }

    #[test]
    fn extract_xmp_from_jpeg_multiple_app1() {
        // EXIF APP1 first, then XMP APP1
        let xmp_xml = sample_xmp_xml();
        let mut data = Vec::new();

        // SOI
        data.extend_from_slice(&[0xFF, 0xD8]);

        // EXIF APP1
        data.extend_from_slice(&[0xFF, 0xE1]);
        let exif_header = b"Exif\0\0";
        let exif_len = (exif_header.len() + 2) as u16;
        data.extend_from_slice(&exif_len.to_be_bytes());
        data.extend_from_slice(exif_header);

        // XMP APP1
        data.extend_from_slice(&[0xFF, 0xE1]);
        let payload_len = XMP_NAMESPACE.len() + xmp_xml.len();
        let segment_len = (payload_len + 2) as u16;
        data.extend_from_slice(&segment_len.to_be_bytes());
        data.extend_from_slice(XMP_NAMESPACE);
        data.extend_from_slice(xmp_xml.as_bytes());

        // EOI
        data.extend_from_slice(&[0xFF, 0xD9]);

        let result = extract_xmp_xml_from_jpeg(&data);
        assert!(result.is_some());
        assert!(result.unwrap().contains("landscape"));
    }

    #[test]
    fn extract_xmp_from_jpeg_not_a_jpeg() {
        let data = b"This is not a JPEG file at all";
        let result = extract_xmp_xml_from_jpeg(data);
        assert!(result.is_none());
    }

    #[test]
    fn extract_xmp_from_tiff_basic() {
        let xmp_xml = sample_xmp_xml();
        let tiff_data = build_tiff_le_with_xmp(xmp_xml);
        let result = extract_xmp_xml_from_tiff(&tiff_data);
        assert!(result.is_some());
        let xml = result.unwrap();
        assert!(xml.contains("landscape"));
        assert!(xml.contains("xmp:Rating"));
    }

    #[test]
    fn extract_xmp_from_tiff_big_endian() {
        let xmp_xml = sample_xmp_xml();
        let tiff_data = build_tiff_be_with_xmp(xmp_xml);
        let result = extract_xmp_xml_from_tiff(&tiff_data);
        assert!(result.is_some());
        let xml = result.unwrap();
        assert!(xml.contains("sunset"));
        assert!(xml.contains("xmp:Label"));
    }

    #[test]
    fn extract_embedded_xmp_roundtrip() {
        let xmp_xml = sample_xmp_xml();
        let jpeg_data = build_jpeg_with_xmp(xmp_xml);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jpg");
        std::fs::write(&path, &jpeg_data).unwrap();

        let xmp = extract_embedded_xmp(&path);
        assert_eq!(xmp.keywords, vec!["landscape", "sunset"]);
        assert_eq!(xmp.description.as_deref(), Some("A beautiful sunset"));
        assert_eq!(xmp.source_metadata.get("rating").unwrap(), "4");
        assert_eq!(xmp.source_metadata.get("label").unwrap(), "Blue");
    }

    #[test]
    fn extract_embedded_xmp_unsupported_ext() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        std::fs::write(&path, b"PNG data").unwrap();

        let xmp = extract_embedded_xmp(&path);
        assert!(xmp.keywords.is_empty());
        assert!(xmp.description.is_none());
        assert!(xmp.source_metadata.is_empty());
    }

    #[test]
    fn extract_embedded_xmp_tiff_roundtrip() {
        let xmp_xml = sample_xmp_xml();
        let tiff_data = build_tiff_le_with_xmp(xmp_xml);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.tiff");
        std::fs::write(&path, &tiff_data).unwrap();

        let xmp = extract_embedded_xmp(&path);
        assert_eq!(xmp.keywords, vec!["landscape", "sunset"]);
        assert_eq!(xmp.description.as_deref(), Some("A beautiful sunset"));
        assert_eq!(xmp.source_metadata.get("rating").unwrap(), "4");
    }

    #[test]
    fn extract_embedded_xmp_nonexistent_file() {
        let xmp = extract_embedded_xmp(Path::new("/nonexistent/file.jpg"));
        assert!(xmp.keywords.is_empty());
    }

    #[test]
    fn extract_xmp_from_tiff_no_xmp_tag() {
        // Minimal TIFF with a different tag (not 700)
        let mut data = Vec::new();
        data.extend_from_slice(b"II");
        data.extend_from_slice(&42u16.to_le_bytes());
        data.extend_from_slice(&8u32.to_le_bytes());

        // 1 entry: tag=256 (ImageWidth), type=SHORT(3), count=1, value=100
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&256u16.to_le_bytes());
        data.extend_from_slice(&3u16.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&100u32.to_le_bytes());

        // Next IFD = 0
        data.extend_from_slice(&0u32.to_le_bytes());

        let result = extract_xmp_xml_from_tiff(&data);
        assert!(result.is_none());
    }
}
