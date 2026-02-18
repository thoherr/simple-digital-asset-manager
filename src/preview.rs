use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

const PREVIEW_MAX_EDGE: u32 = 800;

/// Creates and caches thumbnails for browsing.
pub struct PreviewGenerator {
    preview_dir: PathBuf,
    debug: bool,
}

impl PreviewGenerator {
    pub fn new(catalog_root: &Path, debug: bool) -> Self {
        Self {
            preview_dir: catalog_root.join("previews"),
            debug,
        }
    }

    /// Return the path where a preview for this content hash would be stored.
    pub fn preview_path(&self, content_hash: &str) -> PathBuf {
        let hex = content_hash.strip_prefix("sha256:").unwrap_or(content_hash);
        let prefix = &hex[..2.min(hex.len())];
        self.preview_dir.join(prefix).join(format!("{hex}.jpg"))
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
        self.do_generate(content_hash, source_path, format)
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
        self.do_generate(content_hash, source_path, format)
    }

    fn do_generate(
        &self,
        content_hash: &str,
        source_path: &Path,
        format: &str,
    ) -> Result<Option<PathBuf>> {
        let dest = self.preview_path(content_hash);

        let result = match format.to_lowercase().as_str() {
            // Standard image formats the `image` crate can decode
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "ico" => {
                self.generate_image(&dest, source_path)
            }
            // RAW camera formats
            "raw" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "dng" | "raf" | "pef"
            | "srw" => self.generate_raw(&dest, source_path),
            // Video formats
            "mp4" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg"
            | "3gp" | "mts" | "m2ts" => self.generate_video(&dest, source_path),
            // Unsupported for preview (audio, documents, etc.)
            _ => return Ok(None),
        };

        match result {
            Ok(()) => Ok(Some(dest)),
            Err(e) => {
                // If the dest was partially written, clean up
                std::fs::remove_file(&dest).ok();
                // Check if it's a missing-tool error — return None instead of propagating
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("No such file") {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Generate preview from a standard image format using the `image` crate.
    fn generate_image(&self, dest: &Path, source: &Path) -> Result<()> {
        let img = image::open(source)
            .with_context(|| format!("Failed to open image {}", source.display()))?;

        let resized = resize_image(&img);
        ensure_parent(dest)?;
        resized
            .save(dest)
            .with_context(|| format!("Failed to save preview to {}", dest.display()))?;
        Ok(())
    }

    /// Generate preview from a RAW camera file using dcraw or dcraw_emu.
    fn generate_raw(&self, dest: &Path, source: &Path) -> Result<()> {
        ensure_parent(dest)?;

        // Strategy 1: dcraw -e -c extracts the embedded JPEG preview to stdout
        if tool_available("dcraw") {
            let output = Command::new("dcraw")
                .args(["-e", "-c"])
                .arg(source)
                .output()
                .context("Failed to run dcraw")?;
            if self.debug {
                eprintln!("[debug] dcraw -e -c {}", source.display());
                if !output.stderr.is_empty() {
                    eprintln!("[debug] dcraw stderr: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            if output.status.success() && !output.stdout.is_empty() {
                let img = image::load_from_memory(&output.stdout)
                    .context("Failed to decode dcraw output")?;
                let resized = resize_image(&img);
                resized.save(dest).with_context(|| {
                    format!("Failed to save preview to {}", dest.display())
                })?;
                return Ok(());
            }
        }

        // Strategy 2: dcraw_emu — process the RAW to a temp TIFF (half-size for speed)
        if tool_available("dcraw_emu") {
            let temp_tiff = dest.with_extension("tmp.tiff");
            if self.debug {
                eprintln!("[debug] dcraw_emu -h -T -Z {} {}", temp_tiff.display(), source.display());
            }
            let output = Command::new("dcraw_emu")
                .args(["-h", "-T", "-Z"])
                .arg(&temp_tiff)
                .arg(source)
                .output()
                .context("Failed to run dcraw_emu")?;
            if self.debug && !output.stderr.is_empty() {
                eprintln!("[debug] dcraw_emu stderr: {}", String::from_utf8_lossy(&output.stderr));
            }
            if output.status.success() && temp_tiff.exists() {
                let img = image::open(&temp_tiff).with_context(|| {
                    format!("Failed to open dcraw_emu output {}", temp_tiff.display())
                })?;
                std::fs::remove_file(&temp_tiff).ok();
                let resized = resize_image(&img);
                resized.save(dest).with_context(|| {
                    format!("Failed to save preview to {}", dest.display())
                })?;
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
    fn generate_video(&self, dest: &Path, source: &Path) -> Result<()> {
        if !tool_available("ffmpeg") {
            anyhow::bail!("ffmpeg not found in PATH");
        }

        ensure_parent(dest)?;

        // Extract first frame to a temp file, then resize
        let temp_frame = dest.with_extension("tmp.jpg");
        if self.debug {
            eprintln!("[debug] ffmpeg -i {} -vframes 1 -f image2 -update 1 -y {}", source.display(), temp_frame.display());
        }
        let output = Command::new("ffmpeg")
            .args(["-i"])
            .arg(source)
            .args(["-vframes", "1", "-f", "image2", "-update", "1", "-y"])
            .arg(&temp_frame)
            .output()
            .context("Failed to run ffmpeg")?;

        if self.debug && !output.stderr.is_empty() {
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

        let resized = resize_image(&img);
        resized
            .save(dest)
            .with_context(|| format!("Failed to save preview to {}", dest.display()))?;
        Ok(())
    }
}

/// Resize an image so the longest edge is at most `PREVIEW_MAX_EDGE` pixels.
/// If already smaller, returns as-is.
fn resize_image(img: &image::DynamicImage) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let max_dim = w.max(h);
    if max_dim <= PREVIEW_MAX_EDGE {
        return img.clone();
    }
    let nwidth = (w as f64 * PREVIEW_MAX_EDGE as f64 / max_dim as f64).round() as u32;
    let nheight = (h as f64 * PREVIEW_MAX_EDGE as f64 / max_dim as f64).round() as u32;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_path_shards_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), false);
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
        let gen = PreviewGenerator::new(dir.path(), false);
        assert!(!gen.has_preview("sha256:0000000000"));
    }

    #[test]
    fn generate_returns_none_for_audio() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), false);
        let result = gen
            .generate("sha256:abc123", Path::new("/fake/file.mp3"), "mp3")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn generate_image_creates_preview() {
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), false);

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
        let gen = PreviewGenerator::new(dir.path(), false);

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
        let gen = PreviewGenerator::new(dir.path(), false);

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
        let resized = resize_image(&img);
        assert_eq!(resized.width(), 800);
        assert_eq!(resized.height(), 400);
    }

    #[test]
    fn resize_noop_for_small_image() {
        let img = image::DynamicImage::new_rgb8(400, 300);
        let resized = resize_image(&img);
        assert_eq!(resized.width(), 400);
        assert_eq!(resized.height(), 300);
    }

    #[test]
    fn generate_video_includes_stderr_on_failure() {
        if !tool_available("ffmpeg") {
            return; // skip if ffmpeg not installed
        }
        let dir = tempfile::tempdir().unwrap();
        let gen = PreviewGenerator::new(dir.path(), false);

        // Create a file that is not a valid video
        let bad_source = dir.path().join("bad.mov");
        std::fs::write(&bad_source, b"not a video").unwrap();

        let dest = gen.preview_path("sha256:badvideo");
        ensure_parent(&dest).unwrap();
        // Call generate_video directly to bypass do_generate's error filter
        let err = gen.generate_video(&dest, &bad_source).unwrap_err();
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
}
