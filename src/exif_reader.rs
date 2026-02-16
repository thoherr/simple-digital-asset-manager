use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};

/// Extracted EXIF metadata from an image file.
pub struct ExifData {
    /// Key/value pairs for the variant's `source_metadata`.
    pub source_metadata: HashMap<String, String>,
    /// Parsed DateTimeOriginal, if available.
    pub date_taken: Option<DateTime<Utc>>,
}

impl ExifData {
    fn empty() -> Self {
        Self {
            source_metadata: HashMap::new(),
            date_taken: None,
        }
    }
}

/// Extract a clean string from an EXIF field.
///
/// Some cameras (notably Fujifilm) store ASCII tags like LensModel as
/// multi-component values where only the first component is meaningful
/// and the rest are empty. `display_value()` renders all of them as
/// comma-separated quoted strings. This function returns just the first
/// non-empty ASCII component, falling back to `display_value()` for
/// non-ASCII types.
fn clean_field_value(field: &exif::Field) -> String {
    if let exif::Value::Ascii(ref components) = field.value {
        for component in components {
            let s = String::from_utf8_lossy(component);
            let s = s.trim().trim_matches('\0');
            if !s.is_empty() {
                return s.to_string();
            }
        }
        return String::new();
    }
    field.display_value().to_string()
}

/// Extract EXIF metadata from a file. Infallible — returns empty data on any error.
pub fn extract(path: &Path) -> ExifData {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return ExifData::empty(),
    };
    let exif = match exif::Reader::new().read_from_container(&mut BufReader::new(file)) {
        Ok(e) => e,
        Err(_) => return ExifData::empty(),
    };

    let mut meta = HashMap::new();

    // Simple tag mappings
    let tag_map: &[(exif::Tag, &str)] = &[
        (exif::Tag::Make, "camera_make"),
        (exif::Tag::Model, "camera_model"),
        (exif::Tag::LensModel, "lens_model"),
        (exif::Tag::PhotographicSensitivity, "iso"),
        (exif::Tag::ExposureTime, "exposure_time"),
        (exif::Tag::FNumber, "f_number"),
        (exif::Tag::FocalLength, "focal_length"),
        (exif::Tag::PixelXDimension, "image_width"),
        (exif::Tag::PixelYDimension, "image_height"),
    ];

    for (tag, key) in tag_map {
        if let Some(field) = exif.get_field(*tag, exif::In::PRIMARY) {
            let val = clean_field_value(field);
            if !val.is_empty() {
                meta.insert(key.to_string(), val);
            }
        }
    }

    // GPS latitude
    if let Some(lat) = exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY) {
        let ref_val = exif
            .get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string())
            .unwrap_or_default();
        let coord = lat.display_value().to_string();
        if !coord.is_empty() {
            meta.insert("gps_latitude".to_string(), format!("{coord} {ref_val}").trim().to_string());
        }
    }

    // GPS longitude
    if let Some(lon) = exif.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY) {
        let ref_val = exif
            .get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string())
            .unwrap_or_default();
        let coord = lon.display_value().to_string();
        if !coord.is_empty() {
            meta.insert("gps_longitude".to_string(), format!("{coord} {ref_val}").trim().to_string());
        }
    }

    // GPS altitude
    if let Some(alt) = exif.get_field(exif::Tag::GPSAltitude, exif::In::PRIMARY) {
        let val = alt.display_value().to_string();
        if !val.is_empty() {
            meta.insert("gps_altitude".to_string(), val);
        }
    }

    // Parse DateTimeOriginal
    let date_taken = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .and_then(|f| {
            let s = f.display_value().to_string();
            // Format: "YYYY:MM:DD HH:MM:SS" (sometimes quoted by display_value)
            let s = s.trim_matches('"');
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y:%m:%d %H:%M:%S"))
                .ok()
        })
        .map(|ndt| ndt.and_utc());

    ExifData {
        source_metadata: meta,
        date_taken,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn non_image_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();

        let data = extract(&path);
        assert!(data.source_metadata.is_empty());
        assert!(data.date_taken.is_none());
    }

    #[test]
    fn nonexistent_file_returns_empty() {
        let data = extract(&PathBuf::from("/nonexistent/file.jpg"));
        assert!(data.source_metadata.is_empty());
        assert!(data.date_taken.is_none());
    }

    #[test]
    fn fuji_lens_model_is_clean_string() {
        // Fuji cameras store LensModel as multi-component ASCII where only
        // the first component is the actual lens name.
        let path = PathBuf::from("/private/tmp/dam-test/fuji1.jpg");
        if !path.exists() {
            eprintln!("Skipping fuji test — sample file not found");
            return;
        }
        let data = extract(&path);
        let lens = data.source_metadata.get("lens_model").expect("lens_model should be present");
        assert!(
            !lens.contains(','),
            "lens_model should be a single value, got: {lens}"
        );
        assert_eq!(lens, "XF56mmF1.2 R");
    }
}
