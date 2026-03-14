use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Command;

use ab_glyph::FontRef;
use anyhow::{Context, Result};
use image::{Rgb, RgbImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut, text_size};
use imageproc::rect::Rect;

use crate::config::{PreviewConfig, PreviewFormat};

const BG_COLOR: Rgb<u8> = Rgb([35, 35, 40]);
const TEXT_COLOR: Rgb<u8> = Rgb([220, 220, 225]);
const DIM_COLOR: Rgb<u8> = Rgb([140, 140, 150]);
const SEPARATOR_COLOR: Rgb<u8> = Rgb([60, 60, 70]);

// Format badge colors
const BADGE_AUDIO: Rgb<u8> = Rgb([60, 100, 180]);
const BADGE_DOCUMENT: Rgb<u8> = Rgb([180, 140, 40]);
const BADGE_IMAGE: Rgb<u8> = Rgb([50, 150, 80]);
const BADGE_VIDEO: Rgb<u8> = Rgb([180, 50, 50]);
const BADGE_OTHER: Rgb<u8> = Rgb([100, 100, 110]);
const BADGE_TEXT: Rgb<u8> = Rgb([255, 255, 255]);

static FONT_DATA: &[u8] = include_bytes!("fonts/DejaVuSans.ttf");

const AUDIO_FORMATS: &[&str] = &[
    "mp3", "flac", "aac", "ogg", "wav", "wma", "m4a", "aiff", "aif", "opus", "alac", "ape",
    "wv",
];

const DOCUMENT_FORMATS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt",
    "csv", "epub",
];

/// Creates and caches thumbnails for browsing.
pub struct PreviewGenerator {
    preview_dir: PathBuf,
    smart_dir: PathBuf,
    verbosity: crate::Verbosity,
    max_edge: u32,
    format: PreviewFormat,
    quality: u8,
    smart_max_edge: u32,
    smart_quality: u8,
}

impl PreviewGenerator {
    pub fn new(catalog_root: &Path, verbosity: crate::Verbosity, config: &PreviewConfig) -> Self {
        Self {
            preview_dir: catalog_root.join("previews"),
            smart_dir: catalog_root.join("smart-previews"),
            verbosity,
            max_edge: config.max_edge,
            format: config.format.clone(),
            quality: config.quality,
            smart_max_edge: config.smart_max_edge,
            smart_quality: config.smart_quality,
        }
    }

    /// Return the path where a preview for this content hash would be stored.
    pub fn preview_path(&self, content_hash: &str) -> PathBuf {
        let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
        let prefix = &hex[..2.min(hex.len())];
        let ext = self.format.extension();
        self.preview_dir.join(prefix).join(format!("{hex}.{ext}"))
    }

    /// Check if a preview already exists on disk.
    pub fn has_preview(&self, content_hash: &str) -> bool {
        self.preview_path(content_hash).exists()
    }

    /// Generate a preview for a file. Returns the preview path on success,
    /// `Ok(None)` if the format is unsupported or an external tool is missing.
    /// Preview failure never returns `Err` for missing tools — only for I/O issues
    /// that shouldn't silently pass.
    pub fn generate(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
    ) -> Result<Option<PathBuf>> {
        let dest = self.preview_path(content_hash);
        if dest.exists() {
            return Ok(Some(dest));
        }
        self.do_generate(content_hash, source_path, format, None)
    }

    /// Like `generate`, but forces regeneration even if a preview already exists.
    pub fn regenerate(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
    ) -> Result<Option<PathBuf>> {
        let dest = self.preview_path(content_hash);
        if dest.exists() {
            std::fs::remove_file(&dest).ok();
        }
        self.do_generate(content_hash, source_path, format, None)
    }

    /// Like `regenerate`, but applies a manual rotation (0/90/180/270 degrees).
    pub fn regenerate_with_rotation(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
        rotation: Option<u16>,
    ) -> Result<Option<PathBuf>> {
        let dest = self.preview_path(content_hash);
        if dest.exists() {
            std::fs::remove_file(&dest).ok();
        }
        self.do_generate(content_hash, source_path, format, rotation)
    }

    /// Return the path where a smart preview for this content hash would be stored.
    pub fn smart_preview_path(&self, content_hash: &str) -> PathBuf {
        let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
        let prefix = &hex[..2.min(hex.len())];
        let ext = self.format.extension();
        self.smart_dir.join(prefix).join(format!("{hex}.{ext}"))
    }

    /// Check if a smart preview already exists on disk.
    pub fn has_smart_preview(&self, content_hash: &str) -> bool {
        self.smart_preview_path(content_hash).exists()
    }

    /// Generate a smart preview (high-resolution) for a file.
    pub fn generate_smart(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
    ) -> Result<Option<PathBuf>> {
        let dest = self.smart_preview_path(content_hash);
        if dest.exists() {
            return Ok(Some(dest));
        }
        self.do_generate_to(&dest, source_path, format, self.smart_max_edge, self.smart_quality, None)
    }

    /// Like `generate_smart`, but forces regeneration.
    pub fn regenerate_smart(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
    ) -> Result<Option<PathBuf>> {
        let dest = self.smart_preview_path(content_hash);
        if dest.exists() {
            std::fs::remove_file(&dest).ok();
        }
        self.do_generate_to(&dest, source_path, format, self.smart_max_edge, self.smart_quality, None)
    }

    /// Like `regenerate_smart`, but applies a manual rotation (0/90/180/270 degrees).
    pub fn regenerate_smart_with_rotation(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
        rotation: Option<u16>,
    ) -> Result<Option<PathBuf>> {
        let dest = self.smart_preview_path(content_hash);
        if dest.exists() {
            std::fs::remove_file(&dest).ok();
        }
        self.do_generate_to(&dest, source_path, format, self.smart_max_edge, self.smart_quality, rotation)
    }

    fn do_generate(
        &self,
        _content_hash: &str,
        source_path: &Path,
        format: &str,
        manual_rotation: Option<u16>,
    ) -> Result<Option<PathBuf>> {
        let dest = self.preview_path(_content_hash);
        self.do_generate_to(&dest, source_path, format, self.max_edge, self.quality, manual_rotation)
    }

    fn do_generate_to(
        &self,
        dest: &Path,
        source_path: &Path,
        format: &str,
        max_edge: u32,
        quality: u8,
        manual_rotation: Option<u16>,
    ) -> Result<Option<PathBuf>> {
        let fmt = format.to_lowercase();

        if self.verbosity.verbose {
            let method = match fmt.as_str() {
                "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "ico" => "image",
                "raw" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf"
                | "orf" | "rw2" | "dng" | "raf" | "pef" | "srw" | "mrw"
                | "3fr" | "fff" | "iiq" | "erf" | "kdc" | "dcr"
                | "mef" | "mos" | "rwl" | "bay" | "x3f" => "RAW (dcraw)",
                "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
                | "3gp" | "mts" | "m2ts" => "video (ffmpeg)",
                _ => "info card",
            };
            let name = source_path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            eprintln!("  Preview: {name} via {method} ({max_edge}px)");
        }

        let result = match fmt.as_str() {
            // Standard image formats the `image` crate can decode
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "ico" => {
                self.generate_image(dest, source_path, max_edge, quality, manual_rotation)
            }
            // RAW camera formats
            "raw" | "cr2" | "cr3" | "crw" | "nef" | "nrw" | "arw" | "sr2" | "srf"
            | "orf" | "rw2" | "dng" | "raf" | "pef" | "srw" | "mrw"
            | "3fr" | "fff" | "iiq" | "erf" | "kdc" | "dcr"
            | "mef" | "mos" | "rwl" | "bay" | "x3f" => self.generate_raw(dest, source_path, max_edge, quality, manual_rotation),
            // Video formats
            "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
            | "3gp" | "mts" | "m2ts" => self.generate_video(dest, source_path, max_edge, quality, manual_rotation),
            // Audio and everything else → info card
            _ => return self.generate_info_card(dest, source_path, &fmt, max_edge, quality),
        };

        match result {
            Ok(()) => Ok(Some(dest.to_path_buf())),
            Err(e) => {
                // If the dest was partially written, clean up
                std::fs::remove_file(dest).ok();
                // Check if it's a missing-tool error — fall back to info card
                let msg = e.to_string();
                if msg.contains("not found")
                    || msg.contains("No such file")
                    || msg.contains("does not contain any stream")
                {
                    self.generate_info_card(dest, source_path, &fmt, max_edge, quality)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Save a preview image using the configured format and given quality.
    fn save_preview(&self, img: &image::DynamicImage, dest: &Path, quality: u8) -> Result<()> {
        let file = std::fs::File::create(dest)
            .with_context(|| format!("Failed to create preview file {}", dest.display()))?;
        let writer = BufWriter::new(file);
        match self.format {
            PreviewFormat::Jpeg => {
                let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality);
                img.write_with_encoder(encoder)
                    .with_context(|| format!("Failed to save preview to {}", dest.display()))?;
            }
            PreviewFormat::Webp => {
                let encoder = image::codecs::webp::WebPEncoder::new_lossless(writer);
                img.write_with_encoder(encoder)
                    .with_context(|| format!("Failed to save preview to {}", dest.display()))?;
            }
        }
        Ok(())
    }

    /// Generate preview from a standard image format using the `image` crate.
    fn generate_image(&self, dest: &Path, source: &Path, max_edge: u32, quality: u8, manual_rotation: Option<u16>) -> Result<()> {
        let img = image::open(source)
            .with_context(|| format!("Failed to open image {}", source.display()))?;

        // Apply EXIF orientation from the source file
        let exif_data = crate::exif_reader::extract(source);
        let img = if let Some(orient) = exif_data.orientation {
            crate::exif_reader::apply_exif_orientation(img, orient)
        } else {
            img
        };

        // Apply manual rotation override if set
        let img = if let Some(degrees) = manual_rotation {
            crate::exif_reader::apply_rotation(img, degrees)
        } else {
            img
        };

        let resized = resize_image(&img, max_edge);
        ensure_parent(dest)?;
        self.save_preview(&resized, dest, quality)?;
        Ok(())
    }

    /// Generate preview from a RAW camera file using dcraw or dcraw_emu.
    fn generate_raw(&self, dest: &Path, source: &Path, max_edge: u32, quality: u8, manual_rotation: Option<u16>) -> Result<()> {
        ensure_parent(dest)?;

        // Strategy 1: dcraw -e -c extracts the embedded JPEG preview to stdout
        if tool_available("dcraw") {
            let output = Command::new("dcraw")
                .args(["-e", "-c"])
                .arg(source)
                .output()
                .context("Failed to run dcraw")?;
            if self.verbosity.debug {
                eprintln!("[debug] dcraw -e -c {}", source.display());
                if !output.stderr.is_empty() {
                    eprintln!("[debug] dcraw stderr: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            if output.status.success() && !output.stdout.is_empty() {
                let img = image::load_from_memory(&output.stdout)
                    .context("Failed to decode dcraw output")?;
                // Apply EXIF orientation from the embedded JPEG.
                // Some cameras (e.g. Nikon Z9) store the embedded preview in sensor
                // orientation with an EXIF tag indicating the correct rotation.
                // Cameras that pixel-rotate set orientation to 1, making this a no-op.
                let img = if let Some(orient) = crate::exif_reader::orientation_from_bytes(&output.stdout) {
                    crate::exif_reader::apply_exif_orientation(img, orient)
                } else {
                    img
                };
                // Apply manual rotation override if set
                let img = if let Some(degrees) = manual_rotation {
                    crate::exif_reader::apply_rotation(img, degrees)
                } else {
                    img
                };
                let resized = resize_image(&img, max_edge);
                self.save_preview(&resized, dest, quality)?;
                return Ok(());
            }
        }

        // Strategy 2: dcraw_emu — process the RAW to a temp TIFF (half-size for speed)
        if tool_available("dcraw_emu") {
            let temp_tiff = dest.with_extension("tmp.tiff");
            if self.verbosity.debug {
                eprintln!("[debug] dcraw_emu -h -T -Z {} {}", temp_tiff.display(), source.display());
            }
            let output = Command::new("dcraw_emu")
                .args(["-h", "-T", "-Z"])
                .arg(&temp_tiff)
                .arg(source)
                .output()
                .context("Failed to run dcraw_emu")?;
            if self.verbosity.debug && !output.stderr.is_empty() {
                eprintln!("[debug] dcraw_emu stderr: {}", String::from_utf8_lossy(&output.stderr));
            }
            if output.status.success() && temp_tiff.exists() {
                // Read EXIF orientation from the output TIFF (not the source RAW).
                // dcraw_emu may already pixel-rotate the output, in which case
                // the TIFF has no orientation tag and this is a no-op.
                let tiff_exif = crate::exif_reader::extract(&temp_tiff);
                let img = image::open(&temp_tiff).with_context(|| {
                    format!("Failed to open dcraw_emu output {}", temp_tiff.display())
                })?;
                std::fs::remove_file(&temp_tiff).ok();
                let img = if let Some(orient) = tiff_exif.orientation {
                    crate::exif_reader::apply_exif_orientation(img, orient)
                } else {
                    img
                };
                // Apply manual rotation
                let img = if let Some(degrees) = manual_rotation {
                    crate::exif_reader::apply_rotation(img, degrees)
                } else {
                    img
                };
                let resized = resize_image(&img, max_edge);
                self.save_preview(&resized, dest, quality)?;
                return Ok(());
            }
            if !output.status.success() {
                let stderr_text = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("dcraw_emu failed: {}", stderr_text.trim());
            }
            std::fs::remove_file(&temp_tiff).ok();
        }

        anyhow::bail!("Neither dcraw nor dcraw_emu not found in PATH");
    }

    /// Generate preview from a video file using ffmpeg.
    fn generate_video(&self, dest: &Path, source: &Path, max_edge: u32, quality: u8, manual_rotation: Option<u16>) -> Result<()> {
        if !tool_available("ffmpeg") {
            anyhow::bail!("ffmpeg not found in PATH");
        }

        ensure_parent(dest)?;

        // Extract first frame to a temp file, then resize
        let temp_frame = dest.with_extension("tmp.jpg");
        if self.verbosity.debug {
            eprintln!("[debug] ffmpeg -i {} -vframes 1 -f image2 -update 1 -y {}", source.display(), temp_frame.display());
        }
        let output = Command::new("ffmpeg")
            .args(["-i"])
            .arg(source)
            .args(["-vframes", "1", "-f", "image2", "-update", "1", "-y"])
            .arg(&temp_frame)
            .output()
            .context("Failed to run ffmpeg")?;

        if self.verbosity.debug && !output.stderr.is_empty() {
            eprintln!("[debug] ffmpeg stderr: {}", String::from_utf8_lossy(&output.stderr));
        }

        if !output.status.success() {
            std::fs::remove_file(&temp_frame).ok();
            let stderr_text = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("ffmpeg failed: {}", stderr_text.trim());
        }

        // Load the extracted frame, resize, and save as the final preview
        let img = image::open(&temp_frame)
            .with_context(|| format!("Failed to open ffmpeg frame {}", temp_frame.display()))?;
        std::fs::remove_file(&temp_frame).ok();

        // ffmpeg auto-rotates, so only apply manual rotation
        let img = if let Some(degrees) = manual_rotation {
            crate::exif_reader::apply_rotation(img, degrees)
        } else {
            img
        };

        let resized = resize_image(&img, max_edge);
        self.save_preview(&resized, dest, quality)?;
        Ok(())
    }

    /// Generate an info card preview showing textual metadata.
    fn generate_info_card(
        &self,
        dest: &Path,
        source_path: &Path,
        format: &str,
        max_edge: u32,
        quality: u8,
    ) -> Result<Option<PathBuf>> {
        let info = InfoCardData::from_file(source_path, format);
        let card_width = max_edge;
        let card_height = (max_edge as f64 * 0.75) as u32;
        let img = render_info_card(&info, card_width, card_height);
        ensure_parent(dest)?;
        self.save_preview(&image::DynamicImage::ImageRgb8(img), dest, quality)?;
        Ok(Some(dest.to_path_buf()))
    }
}

// ── Info card rendering ──────────────────────────────────────────────────────

struct InfoCardData {
    display_name: String,
    format: String,
    file_size: String,
    duration: Option<String>,
    bitrate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<String>,
}

impl InfoCardData {
    fn from_file(source_path: &Path, format: &str) -> Self {
        let display_name = source_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown".into());

        let file_size = std::fs::metadata(source_path)
            .ok()
            .map(|m| format_file_size(m.len()))
            .unwrap_or_else(|| "Unknown size".into());

        let (duration, bitrate, sample_rate, channels) = extract_audio_metadata(source_path);

        InfoCardData {
            display_name,
            format: format.to_uppercase(),
            file_size,
            duration,
            bitrate,
            sample_rate,
            channels,
        }
    }

    fn format_category(&self) -> FormatCategory {
        let fmt = self.format.to_lowercase();
        if AUDIO_FORMATS.contains(&fmt.as_str()) {
            FormatCategory::Audio
        } else if DOCUMENT_FORMATS.contains(&fmt.as_str()) {
            FormatCategory::Document
        } else if matches!(
            fmt.as_str(),
            "jpg" | "jpeg"
                | "png"
                | "gif"
                | "bmp"
                | "tiff"
                | "tif"
                | "webp"
                | "ico"
                | "raw"
                | "cr2"
                | "cr3"
                | "crw"
                | "nef"
                | "nrw"
                | "arw"
                | "sr2"
                | "srf"
                | "orf"
                | "rw2"
                | "dng"
                | "raf"
                | "pef"
                | "srw"
                | "mrw"
                | "3fr"
                | "fff"
                | "iiq"
                | "erf"
                | "kdc"
                | "dcr"
                | "mef"
                | "mos"
                | "rwl"
                | "bay"
                | "x3f"
        ) {
            FormatCategory::Image
        } else if matches!(
            fmt.as_str(),
            "mp4" | "mov"
                | "avi"
                | "mkv"
                | "wmv"
                | "flv"
                | "webm"
                | "m4v"
                | "mpg"
                | "mpeg"
                | "3gp"
                | "mts"
                | "m2ts"
        ) {
            FormatCategory::Video
        } else {
            FormatCategory::Other
        }
    }
}

#[derive(Clone, Copy)]
enum FormatCategory {
    Audio,
    Document,
    Image,
    Video,
    Other,
}

impl FormatCategory {
    fn badge_color(self) -> Rgb<u8> {
        match self {
            FormatCategory::Audio => BADGE_AUDIO,
            FormatCategory::Document => BADGE_DOCUMENT,
            FormatCategory::Image => BADGE_IMAGE,
            FormatCategory::Video => BADGE_VIDEO,
            FormatCategory::Other => BADGE_OTHER,
        }
    }

    fn label(self) -> &'static str {
        match self {
            FormatCategory::Audio => "AUDIO",
            FormatCategory::Document => "DOCUMENT",
            FormatCategory::Image => "IMAGE",
            FormatCategory::Video => "VIDEO",
            FormatCategory::Other => "FILE",
        }
    }
}

fn extract_audio_metadata(
    path: &Path,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let tagged_file = match lofty::read_from_path(path) {
        Ok(f) => f,
        Err(_) => return (None, None, None, None),
    };

    use lofty::file::AudioFile;
    let props = tagged_file.properties();

    let duration = {
        let dur = props.duration();
        let total_secs = dur.as_secs();
        if total_secs == 0 && dur.subsec_millis() == 0 {
            None
        } else {
            let hours = total_secs / 3600;
            let mins = (total_secs % 3600) / 60;
            let secs = total_secs % 60;
            Some(if hours > 0 {
                format!("{hours}:{mins:02}:{secs:02}")
            } else {
                format!("{mins}:{secs:02}")
            })
        }
    };

    let bitrate = props
        .audio_bitrate()
        .or_else(|| props.overall_bitrate())
        .map(|b| format!("{b} kbps"));

    let sample_rate = props.sample_rate().map(|sr| {
        if sr % 1000 == 0 {
            format!("{} kHz", sr / 1000)
        } else {
            format!("{:.1} kHz", sr as f64 / 1000.0)
        }
    });

    let channels = props.channels().map(|ch| match ch {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        n => format!("{n} channels"),
    });

    (duration, bitrate, sample_rate, channels)
}

fn render_info_card(info: &InfoCardData, width: u32, height: u32) -> RgbImage {
    let font = FontRef::try_from_slice(FONT_DATA).expect("embedded font is valid");

    let mut img = RgbImage::from_pixel(width, height, BG_COLOR);

    let category = info.format_category();
    let badge_color = category.badge_color();

    // ── Format badge ─────────────────────────────────────────────────────
    let badge_scale = 18.0_f32;
    let badge_text = format!("{} · {}", category.label(), info.format);
    let (badge_tw, badge_th) = text_size(badge_scale, &font, &badge_text);
    let badge_pad_x: u32 = 16;
    let badge_pad_y: u32 = 8;
    let badge_x: i32 = 40;
    let badge_y: i32 = 180;
    let badge_w = badge_tw + badge_pad_x * 2;
    let badge_h = badge_th + badge_pad_y * 2;
    draw_filled_rect_mut(
        &mut img,
        Rect::at(badge_x, badge_y).of_size(badge_w, badge_h),
        badge_color,
    );
    draw_text_mut(
        &mut img,
        BADGE_TEXT,
        badge_x + badge_pad_x as i32,
        badge_y + badge_pad_y as i32,
        badge_scale,
        &font,
        &badge_text,
    );

    // ── Display name ─────────────────────────────────────────────────────
    let name_scale = 28.0_f32;
    let max_name_width = width.saturating_sub(80);
    let display_name = truncate_to_width(&info.display_name, name_scale, &font, max_name_width);
    let name_y = badge_y + badge_h as i32 + 30;
    draw_text_mut(
        &mut img,
        TEXT_COLOR,
        40,
        name_y,
        name_scale,
        &font,
        &display_name,
    );

    // ── Separator line ───────────────────────────────────────────────────
    let (_, name_th) = text_size(name_scale, &font, &display_name);
    let sep_y = name_y + name_th as i32 + 16;
    draw_filled_rect_mut(
        &mut img,
        Rect::at(40, sep_y).of_size(width.saturating_sub(80), 1),
        SEPARATOR_COLOR,
    );

    // ── Metadata lines ───────────────────────────────────────────────────
    let meta_scale = 20.0_f32;
    let line_height: i32 = 34;
    let mut y = sep_y + 20;

    let mut draw_meta_line = |label: &str, value: &str| {
        draw_text_mut(&mut img, DIM_COLOR, 40, y, meta_scale, &font, label);
        let (label_w, _) = text_size(meta_scale, &font, label);
        draw_text_mut(
            &mut img,
            TEXT_COLOR,
            40 + label_w as i32 + 8,
            y,
            meta_scale,
            &font,
            value,
        );
        y += line_height;
    };

    draw_meta_line("Size:", &info.file_size);

    if let Some(ref dur) = info.duration {
        draw_meta_line("Duration:", dur);
    }
    if let Some(ref br) = info.bitrate {
        draw_meta_line("Bitrate:", br);
    }
    if let Some(ref sr) = info.sample_rate {
        draw_meta_line("Sample rate:", sr);
    }
    if let Some(ref ch) = info.channels {
        draw_meta_line("Channels:", ch);
    }

    img
}

fn truncate_to_width(text: &str, scale: f32, font: &FontRef, max_width: u32) -> String {
    let (w, _) = text_size(scale, font, text);
    if w <= max_width {
        return text.to_string();
    }

    let ellipsis = "...";
    let (ew, _) = text_size(scale, font, ellipsis);
    let target = max_width.saturating_sub(ew);

    // Binary search for the longest prefix that fits
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let (pw, _) = text_size(scale, font, &prefix);
        if pw <= target {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    let prefix: String = chars[..lo].iter().collect();
    format!("{prefix}{ellipsis}")
}

fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Resize an image so the longest edge is at most `max_edge` pixels.
/// If already smaller, returns as-is.
fn resize_image(img: &image::DynamicImage, max_edge: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let max_dim = w.max(h);
    if max_dim <= max_edge {
        return img.clone();
    }
    let nwidth = (w as f64 * max_edge as f64 / max_dim as f64).round() as u32;
    let nheight = (h as f64 * max_edge as f64 / max_dim as f64).round() as u32;
    image::DynamicImage::ImageRgba8(image::imageops::resize(
        img,
        nwidth,
        nheight,
        image::imageops::FilterType::Lanczos3,
    ))
}

/// Ensure the parent directory of a path exists.
fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

/// Check if a command-line tool is available on PATH.
fn tool_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Display a preview image inline in the terminal using viuer.
/// Returns Ok(true) if displayed, Ok(false) if no preview exists.
pub fn display_in_terminal(path: &Path, max_width: Option<u32>, max_height: Option<u32>) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let conf = viuer::Config {
        width: max_width,
        height: max_height,
        absolute_offset: false,
        ..Default::default()
    };
    viuer::print_from_file(path, &conf)
        .context("Failed to display image in terminal")?;
    Ok(true)
}

/// Open a preview image in the OS default viewer.
pub fn open_in_viewer(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn().context("Failed to open image viewer")?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(path).spawn().context("Failed to open image viewer")?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd").args(["/c", "start", ""]).arg(path).spawn().context("Failed to open image viewer")?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_path_shards_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());
        let path = gen.preview_path("sha256:abcdef1234567890");
        assert_eq!(
            path,
            dir.path()
                .join("previews/ab/abcdef1234567890.jpg")
        );
    }

    #[test]
    fn has_preview_false_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());
        assert!(!gen.has_preview("sha256:0000000000"));
    }

    #[test]
    fn generate_creates_info_card_for_audio() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        // Create a dummy file so file_size can be read
        let source = dir.path().join("song.mp3");
        std::fs::write(&source, b"fake audio data for testing").unwrap();

        let result = gen
            .generate("sha256:audiocard1", &source, "mp3")
            .unwrap();
        assert!(result.is_some(), "audio format should produce an info card");

        let preview_path = result.unwrap();
        assert!(preview_path.exists());

        let preview = image::open(&preview_path).unwrap();
        assert!(preview.width() <= 800);
        assert!(preview.height() <= 800);
    }

    #[test]
    fn generate_creates_info_card_for_document() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let source = dir.path().join("report.pdf");
        std::fs::write(&source, b"fake pdf content").unwrap();

        let result = gen
            .generate("sha256:doccard1", &source, "pdf")
            .unwrap();
        assert!(result.is_some(), "document format should produce an info card");
        assert!(result.unwrap().exists());
    }

    #[test]
    fn generate_creates_info_card_for_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let source = dir.path().join("data.xyz");
        std::fs::write(&source, b"unknown format data").unwrap();

        let result = gen
            .generate("sha256:unknowncard1", &source, "xyz")
            .unwrap();
        assert!(result.is_some(), "unknown format should produce an info card");
        assert!(result.unwrap().exists());
    }

    #[test]
    fn info_card_has_expected_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let source = dir.path().join("track.flac");
        std::fs::write(&source, b"fake flac").unwrap();

        let result = gen
            .generate("sha256:dimcard1", &source, "flac")
            .unwrap()
            .unwrap();

        let preview = image::open(&result).unwrap();
        assert_eq!(preview.width(), 800);
        assert_eq!(preview.height(), 600);
    }

    #[test]
    fn render_info_card_produces_valid_image() {
        let info = InfoCardData {
            display_name: "Test File".into(),
            format: "MP3".into(),
            file_size: "4.2 MB".into(),
            duration: Some("3:45".into()),
            bitrate: Some("320 kbps".into()),
            sample_rate: Some("44.1 kHz".into()),
            channels: Some("Stereo".into()),
        };

        let img = render_info_card(&info, 800, 600);
        assert_eq!(img.width(), 800);
        assert_eq!(img.height(), 600);

        // Verify it's not a solid background — some non-BG pixels should exist
        let non_bg = img.pixels().filter(|p| **p != BG_COLOR).count();
        assert!(non_bg > 100, "info card should have visible content, found {non_bg} non-bg pixels");
    }

    #[test]
    fn truncate_long_name() {
        let font = FontRef::try_from_slice(FONT_DATA).unwrap();
        let long_name = "This_is_a_very_long_filename_that_should_be_truncated_to_fit_within_bounds";
        let truncated = truncate_to_width(long_name, 28.0, &font, 300);

        assert!(truncated.ends_with("..."));
        assert!(truncated.len() < long_name.len());

        // Verify the truncated text actually fits
        let (w, _) = text_size(28.0, &font, &truncated);
        assert!(w <= 300, "truncated text width {w} should be <= 300");
    }

    #[test]
    fn truncate_short_name_unchanged() {
        let font = FontRef::try_from_slice(FONT_DATA).unwrap();
        let short_name = "photo";
        let result = truncate_to_width(short_name, 28.0, &font, 500);
        assert_eq!(result, short_name);
    }

    #[test]
    fn format_file_size_ranges() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(512), "512 B");
        assert_eq!(format_file_size(1024), "1 KB");
        assert_eq!(format_file_size(1536), "2 KB");
        assert_eq!(format_file_size(1_048_576), "1.0 MB");
        assert_eq!(format_file_size(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn info_card_fallback_when_raw_tool_missing() {
        // Only test if dcraw is NOT available — otherwise the visual preview would succeed
        if tool_available("dcraw") || tool_available("dcraw_emu") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let source = dir.path().join("photo.nef");
        std::fs::write(&source, b"fake raw data").unwrap();

        let result = gen
            .generate("sha256:rawfallback1", &source, "nef")
            .unwrap();
        assert!(
            result.is_some(),
            "RAW with missing tools should fall back to info card"
        );
        assert!(result.unwrap().exists());
    }

    #[test]
    fn generate_image_creates_preview() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        // Create a real 1600x1200 PNG in the temp dir
        let img = image::DynamicImage::new_rgb8(1600, 1200);
        let source = dir.path().join("test.png");
        img.save(&source).unwrap();

        let result = gen
            .generate("sha256:testimage123", &source, "png")
            .unwrap();
        assert!(result.is_some());

        let preview_path = result.unwrap();
        assert!(preview_path.exists());

        // Verify dimensions are within 800px
        let preview = image::open(&preview_path).unwrap();
        assert!(preview.width() <= 800);
        assert!(preview.height() <= 800);
        assert_eq!(preview.width(), 800); // longest edge
    }

    #[test]
    fn generate_skips_if_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let img = image::DynamicImage::new_rgb8(100, 100);
        let source = dir.path().join("small.png");
        img.save(&source).unwrap();

        let path1 = gen
            .generate("sha256:existing", &source, "png")
            .unwrap()
            .unwrap();
        let mtime1 = std::fs::metadata(&path1).unwrap().modified().unwrap();

        // Second call should return the same path without regenerating
        let path2 = gen
            .generate("sha256:existing", &source, "png")
            .unwrap()
            .unwrap();
        let mtime2 = std::fs::metadata(&path2).unwrap().modified().unwrap();

        assert_eq!(path1, path2);
        assert_eq!(mtime1, mtime2);
    }

    #[test]
    fn regenerate_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        // Create initial preview from a 200x200 image
        let img = image::DynamicImage::new_rgb8(200, 200);
        let source = dir.path().join("regen.png");
        img.save(&source).unwrap();

        let path1 = gen
            .generate("sha256:regen", &source, "png")
            .unwrap()
            .unwrap();
        assert!(path1.exists());

        // Regenerate with a different source image (400x400)
        let img2 = image::DynamicImage::new_rgb8(400, 400);
        let source2 = dir.path().join("regen2.png");
        img2.save(&source2).unwrap();

        let path2 = gen
            .regenerate("sha256:regen", &source2, "png")
            .unwrap()
            .unwrap();
        assert_eq!(path1, path2);
        assert!(path2.exists());
    }

    #[test]
    fn resize_preserves_aspect_ratio() {
        let img = image::DynamicImage::new_rgb8(2000, 1000);
        let resized = resize_image(&img, 800);
        assert_eq!(resized.width(), 800);
        assert_eq!(resized.height(), 400);
    }

    #[test]
    fn resize_noop_for_small_image() {
        let img = image::DynamicImage::new_rgb8(400, 300);
        let resized = resize_image(&img, 800);
        assert_eq!(resized.width(), 400);
        assert_eq!(resized.height(), 300);
    }

    #[test]
    fn generate_video_includes_stderr_on_failure() {
        if !tool_available("ffmpeg") {
            return; // skip if ffmpeg not installed
        }
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        // Create a file that is not a valid video
        let bad_source = dir.path().join("bad.mov");
        std::fs::write(&bad_source, b"not a video").unwrap();

        let dest = gen.preview_path("sha256:badvideo");
        ensure_parent(&dest).unwrap();
        // Call generate_video directly to bypass do_generate's error filter
        let err = gen.generate_video(&dest, &bad_source, 800, 85, None).unwrap_err();
        let msg = err.to_string();
        // Should contain ffmpeg's actual error output, not just "non-zero status"
        assert!(
            msg.contains("ffmpeg failed:"),
            "Expected 'ffmpeg failed:' in error, got: {msg}"
        );
        assert!(
            !msg.contains("non-zero status"),
            "Should not have generic 'non-zero status' message, got: {msg}"
        );
    }

    #[test]
    fn non_default_max_edge() {
        let config = PreviewConfig {
            max_edge: 400,
            ..PreviewConfig::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &config);

        let img = image::DynamicImage::new_rgb8(1600, 1200);
        let source = dir.path().join("big.png");
        img.save(&source).unwrap();

        let result = gen.generate("sha256:smalledge", &source, "png").unwrap().unwrap();
        let preview = image::open(&result).unwrap();
        assert_eq!(preview.width(), 400);
        assert!(preview.height() <= 400);
    }

    #[test]
    fn preview_path_uses_configured_extension() {
        let config = PreviewConfig {
            format: PreviewFormat::Webp,
            ..PreviewConfig::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &config);
        let path = gen.preview_path("sha256:abcdef1234567890");
        assert!(path.to_string_lossy().ends_with(".webp"));
    }

    #[test]
    fn resize_custom_max_edge() {
        let img = image::DynamicImage::new_rgb8(2000, 1000);
        let resized = resize_image(&img, 400);
        assert_eq!(resized.width(), 400);
        assert_eq!(resized.height(), 200);
    }

    #[test]
    fn smart_preview_path_shards_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());
        let path = gen.smart_preview_path("sha256:abcdef1234567890");
        assert_eq!(
            path,
            dir.path()
                .join("smart-previews/ab/abcdef1234567890.jpg")
        );
    }

    #[test]
    fn has_smart_preview_false_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());
        assert!(!gen.has_smart_preview("sha256:0000000000"));
    }

    #[test]
    fn generate_smart_creates_larger_preview() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        // Create a 4000x3000 image
        let img = image::DynamicImage::new_rgb8(4000, 3000);
        let source = dir.path().join("big.png");
        img.save(&source).unwrap();

        let result = gen
            .generate_smart("sha256:smarttest1", &source, "png")
            .unwrap();
        assert!(result.is_some());

        let preview_path = result.unwrap();
        assert!(preview_path.exists());
        assert!(preview_path.to_string_lossy().contains("smart-previews"));

        let preview = image::open(&preview_path).unwrap();
        assert_eq!(preview.width(), 2560); // smart_max_edge default
        assert!(preview.height() <= 2560);
    }

    #[test]
    fn generate_smart_skips_if_exists() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let img = image::DynamicImage::new_rgb8(100, 100);
        let source = dir.path().join("small.png");
        img.save(&source).unwrap();

        let path1 = gen
            .generate_smart("sha256:smartexist", &source, "png")
            .unwrap()
            .unwrap();
        let mtime1 = std::fs::metadata(&path1).unwrap().modified().unwrap();

        let path2 = gen
            .generate_smart("sha256:smartexist", &source, "png")
            .unwrap()
            .unwrap();
        let mtime2 = std::fs::metadata(&path2).unwrap().modified().unwrap();

        assert_eq!(path1, path2);
        assert_eq!(mtime1, mtime2);
    }

    #[test]
    fn regenerate_smart_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), crate::Verbosity::quiet(), &PreviewConfig::default());

        let img = image::DynamicImage::new_rgb8(200, 200);
        let source = dir.path().join("regen_smart.png");
        img.save(&source).unwrap();

        let path1 = gen
            .generate_smart("sha256:smartregen", &source, "png")
            .unwrap()
            .unwrap();
        assert!(path1.exists());

        let path2 = gen
            .regenerate_smart("sha256:smartregen", &source, "png")
            .unwrap()
            .unwrap();
        assert_eq!(path1, path2);
        assert!(path2.exists());
    }
}
