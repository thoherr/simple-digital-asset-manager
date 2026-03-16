use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

/// Return a Command for the `maki` binary.
fn maki() -> Command {
    cargo_bin_cmd!(assert_cmd::pkg_name!()).into()
}

/// Initialize a catalog and register a volume pointing at `dir`.
/// Returns the canonical path (needed on macOS where /var -> /private/var)
/// so that volume lookup matches canonicalized import paths.
fn init_catalog(dir: &Path) -> PathBuf {
    let canonical = dir.canonicalize().expect("canonicalize tempdir");
    maki().current_dir(&canonical).arg("init").assert().success();
    maki()
        .current_dir(&canonical)
        .args(["volume", "add", "test-vol", canonical.to_str().unwrap()])
        .assert()
        .success();
    canonical
}

/// Write a small file and return its path.
fn create_test_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

/// Count files in the previews directory (two levels: shard dirs containing preview files).
fn count_preview_files(previews_dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(shards) = std::fs::read_dir(previews_dir) {
        for shard in shards.flatten() {
            if shard.path().is_dir() {
                if let Ok(files) = std::fs::read_dir(shard.path()) {
                    for file in files.flatten() {
                        if file.path().is_file() {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    count
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn init_creates_catalog() {
    let dir = tempdir().unwrap();
    maki()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));
}

#[test]
fn init_fails_if_already_exists() {
    let dir = tempdir().unwrap();
    maki().current_dir(dir.path()).arg("init").assert().success();
    maki()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn commands_fail_without_init() {
    let dir = tempdir().unwrap();
    maki()
        .current_dir(dir.path())
        .args(["search", "foo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No maki catalog found"));
}

#[test]
fn volume_add_and_list() {
    let dir = tempdir().unwrap();
    maki().current_dir(dir.path()).arg("init").assert().success();
    maki()
        .current_dir(dir.path())
        .args(["volume", "add", "my-vol", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-vol"));

    maki()
        .current_dir(dir.path())
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-vol"));
}

#[test]
fn import_single_file() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"fake jpeg data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));
}

#[test]
fn import_directory() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let sub = root.join("batch");
    std::fs::create_dir_all(&sub).unwrap();
    create_test_file(&sub, "a.jpg", b"aaa");
    create_test_file(&sub, "b.png", b"bbb");

    maki()
        .current_dir(&root)
        .args(["import", sub.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 imported"));
}

#[test]
fn search_finds_imported_asset() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "sunset.jpg", b"sunset data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["search", "sunset"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sunset"));
}

#[test]
fn show_displays_asset_details() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "rose.jpg", b"rose data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Extract asset ID from search output
    let output = maki()
        .current_dir(&root)
        .args(["search", "rose"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Asset:")
                .and(predicate::str::contains("rose.jpg"))
                .and(predicate::str::contains("Variants:")),
        );
}

#[test]
fn tag_add_and_remove() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "bird.jpg", b"bird data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "bird"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().unwrap();

    // Add tag
    maki()
        .current_dir(&root)
        .args(["tag", short_id, "nature", "wildlife"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Added tags:")
                .and(predicate::str::contains("nature")),
        );

    // Remove tag
    maki()
        .current_dir(&root)
        .args(["tag", short_id, "--remove", "wildlife"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed tags:").and(predicate::str::contains("wildlife")));

    // Verify remaining tags via show
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("nature")
                .and(predicate::str::contains("wildlife").not()),
        );
}

#[test]
fn duplicates_shows_multi_location_files() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let content = b"identical content";
    let file1 = create_test_file(&root, "copy1.jpg", content);
    let file2 = create_test_file(&root, "subdir/copy2.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .arg("duplicates")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("copy1.jpg")
                .or(predicate::str::contains("copy2.jpg")),
        );
}

#[test]
fn duplicates_empty_when_no_duplicates() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "unique.jpg", b"unique bytes");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .arg("duplicates")
        .assert()
        .success()
        .stdout(predicate::str::contains("No duplicates found"));
}

#[test]
fn import_xmp_applies_metadata() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_100.nef", b"raw image bytes");

    let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3"
    xmp:Label="Yellow">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>wildlife</rdf:li>
     <rdf:li>birds</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    create_test_file(&photos, "DSC_100.xmp", xmp.as_bytes());

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Get asset ID via search
    let output = maki()
        .current_dir(&root)
        .args(["search", "DSC_100"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Verify tags and metadata appear in show output
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("wildlife")
                .and(predicate::str::contains("birds"))
                .and(predicate::str::contains("rating"))
                .and(predicate::str::contains("3"))
                .and(predicate::str::contains("label"))
                .and(predicate::str::contains("Yellow")),
        );
}

#[test]
fn import_skips_captureone_by_default() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_001.nef", b"raw data for cos test");
    create_test_file(&photos, "DSC_001.cos", b"captureone sidecar");

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Show should NOT mention CaptureOne recipe
    let output = maki()
        .current_dir(&root)
        .args(["search", "DSC_001"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Recipes:").not());
}

#[test]
fn import_includes_captureone_with_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_002.nef", b"raw data for cos include test");
    create_test_file(&photos, "DSC_002.cos", b"captureone sidecar data");

    maki()
        .current_dir(&root)
        .args([
            "import",
            "--include",
            "captureone",
            photos.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Show should mention CaptureOne recipe
    let output = maki()
        .current_dir(&root)
        .args(["search", "DSC_002"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Recipes:")
                .and(predicate::str::contains("CaptureOne")),
        );
}

#[test]
fn import_skip_audio_excludes_audio_files() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photo.jpg", b"jpeg data");
    create_test_file(&root, "song.mp3", b"audio data");

    // Import with --skip audio
    maki()
        .current_dir(&root)
        .args([
            "import",
            "--skip",
            "audio",
            root.join("photo.jpg").to_str().unwrap(),
            root.join("song.mp3").to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Only photo should be searchable
    maki()
        .current_dir(&root)
        .args(["search", "song"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results"));
}

#[test]
fn import_unknown_group_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"data");

    maki()
        .current_dir(&root)
        .args([
            "import",
            "--include",
            "bogus",
            root.join("photo.jpg").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown file type group"));
}

#[test]
fn generate_previews_command_runs() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "preview_test.jpg", b"preview data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .arg("generate-previews")
        .assert()
        .success()
        .stdout(predicate::str::contains("preview(s)"));
}

#[test]
fn show_displays_preview_status() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "preview_show.jpg", b"show preview data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "preview_show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Preview:"));
}

/// Initialize a catalog with two volumes: vol1 and vol2.
/// Returns (canonical root, vol1 path, vol2 path).
fn init_two_volumes(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let canonical = dir.canonicalize().expect("canonicalize tempdir");
    maki().current_dir(&canonical).arg("init").assert().success();

    let vol1 = canonical.join("vol1");
    let vol2 = canonical.join("vol2");
    std::fs::create_dir_all(&vol1).unwrap();
    std::fs::create_dir_all(&vol2).unwrap();

    maki()
        .current_dir(&canonical)
        .args(["volume", "add", "vol1", vol1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&canonical)
        .args(["volume", "add", "vol2", vol2.to_str().unwrap()])
        .assert()
        .success();

    (canonical, vol1, vol2)
}

#[test]
fn relocate_copies_files_between_volumes() {
    let dir = tempdir().unwrap();
    let (root, vol1, vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "photo.jpg", b"relocate test data");

    // Import on vol1
    maki()
        .current_dir(&root)
        .args(["import", vol1.join("photo.jpg").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Get asset ID
    let output = maki()
        .current_dir(&root)
        .args(["search", "photo"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Relocate to vol2
    maki()
        .current_dir(&root)
        .args(["relocate", short_id, "vol2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Relocate complete"));

    // Verify file exists on vol2
    assert!(vol2.join("photo.jpg").exists());
    // File still on vol1
    assert!(vol1.join("photo.jpg").exists());

    // Show should list both volumes
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("vol1")
                .and(predicate::str::contains("vol2")),
        );
}

#[test]
fn relocate_with_remove_source_flag() {
    let dir = tempdir().unwrap();
    let (root, vol1, vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "move_me.jpg", b"move test data");

    maki()
        .current_dir(&root)
        .args(["import", vol1.join("move_me.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "move_me"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Relocate with --remove-source
    maki()
        .current_dir(&root)
        .args(["relocate", short_id, "vol2", "--remove-source"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Relocate complete"));

    // File should be on vol2 but not on vol1
    assert!(vol2.join("move_me.jpg").exists());
    assert!(!vol1.join("move_me.jpg").exists());

    // Show should only list vol2
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("vol2")
                .and(predicate::str::contains("vol1").not()),
        );
}

#[test]
fn relocate_dry_run_no_changes() {
    let dir = tempdir().unwrap();
    let (root, vol1, vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "dry.jpg", b"dry run test data");

    maki()
        .current_dir(&root)
        .args(["import", vol1.join("dry.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "dry"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Dry run
    maki()
        .current_dir(&root)
        .args(["relocate", short_id, "vol2", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // File should NOT exist on vol2
    assert!(!vol2.join("dry.jpg").exists());
    // File still on vol1
    assert!(vol1.join("dry.jpg").exists());
}

#[test]
fn relocate_batch_with_query() {
    let dir = tempdir().unwrap();
    let (root, vol1, vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "a.jpg", b"batch relocate A");
    create_test_file(&vol1, "b.jpg", b"batch relocate B");

    // Import both files with a tag for querying
    maki()
        .current_dir(&root)
        .args(["import", "--add-tag", "batch-test", vol1.to_str().unwrap()])
        .assert()
        .success();

    // Batch relocate all tagged assets to vol2
    maki()
        .current_dir(&root)
        .args(["relocate", "--query", "tag:batch-test", "--target", "vol2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 assets"))
        .stdout(predicate::str::contains("copied"));

    // Both files should now exist on vol2
    assert!(vol2.join("a.jpg").exists());
    assert!(vol2.join("b.jpg").exists());
    // Still on vol1 (no --remove-source)
    assert!(vol1.join("a.jpg").exists());
    assert!(vol1.join("b.jpg").exists());
}

#[test]
fn relocate_batch_dry_run() {
    let dir = tempdir().unwrap();
    let (root, vol1, vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "dry_batch.jpg", b"dry batch data");

    maki()
        .current_dir(&root)
        .args(["import", "--add-tag", "drybatch", vol1.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["relocate", "--query", "tag:drybatch", "--target", "vol2", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // File should NOT exist on vol2
    assert!(!vol2.join("dry_batch.jpg").exists());
}

#[test]
fn import_conflicting_include_skip_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"data");

    maki()
        .current_dir(&root)
        .args([
            "import",
            "--include",
            "audio",
            "--skip",
            "audio",
            root.join("photo.jpg").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be both included and skipped"));
}

#[test]
fn rebuild_catalog_restores_data() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "keeper.jpg", b"keeper data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Verify it's searchable
    maki()
        .current_dir(&root)
        .args(["search", "keeper"])
        .assert()
        .success()
        .stdout(predicate::str::contains("keeper"));

    // Rebuild
    maki()
        .current_dir(&root)
        .arg("rebuild-catalog")
        .assert()
        .success()
        .stdout(predicate::str::contains("Rebuild complete"));

    // Still searchable after rebuild
    maki()
        .current_dir(&root)
        .args(["search", "keeper"])
        .assert()
        .success()
        .stdout(predicate::str::contains("keeper"));
}

#[test]
fn verify_all_passes() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "intact.jpg", b"intact file data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("verified"));
}

#[test]
fn verify_detects_corruption() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "corrupt.jpg", b"original data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Corrupt the file
    std::fs::write(&file, b"corrupted data!!!").unwrap();

    maki()
        .current_dir(&root)
        .arg("verify")
        .assert()
        .failure()
        .stdout(predicate::str::contains("FAILED"));
}

#[test]
fn verify_with_volume_flag() {
    let dir = tempdir().unwrap();
    let (root, vol1, _vol2) = init_two_volumes(dir.path());
    let file = create_test_file(&vol1, "vol_verify.jpg", b"vol verify data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["verify", "--volume", "vol1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("verified"));
}

#[test]
fn verify_specific_path() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file1 = create_test_file(&root, "file_a.jpg", b"data a");
    let file2 = create_test_file(&root, "file_b.jpg", b"data b");

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap(), file2.to_str().unwrap()])
        .assert()
        .success();

    // Verify only file_a
    maki()
        .current_dir(&root)
        .args(["verify", file1.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 verified"));
}

#[test]
fn verify_path_recognizes_recipe_sidecars() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_500.nef", b"raw image for verify");
    create_test_file(&photos, "DSC_500.xmp", b"xmp sidecar for verify");

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Verify the whole directory — both the NEF and XMP should be verified, not untracked
    maki()
        .current_dir(&root)
        .args(["verify", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 verified"));
}

#[test]
fn reimport_updated_recipe_updates_in_place() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_200.nef", b"raw image for update test");

    let xmp_v1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>original_tag</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Original description</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    create_test_file(&photos, "DSC_200.xmp", xmp_v1.as_bytes());

    // First import
    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Modify the XMP
    let xmp_v2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="5">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>original_tag</rdf:li>
     <rdf:li>new_tag</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Updated description</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    std::fs::write(photos.join("DSC_200.xmp"), xmp_v2).unwrap();

    // Re-import same directory
    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 recipe(s) updated")
                .and(predicate::str::contains("1 skipped")),
        );

    // Verify metadata was updated
    let output = maki()
        .current_dir(&root)
        .args(["search", "DSC_200"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("new_tag")
                .and(predicate::str::contains("Updated description"))
                .and(predicate::str::contains("5")),
        );
}

#[test]
fn standalone_recipe_attaches_to_parent() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_300.nef", b"raw image for standalone test");

    // Import NEF only
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_300.nef").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Now create XMP and import it separately
    let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>standalone_tag</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    create_test_file(&photos, "DSC_300.xmp", xmp.as_bytes());

    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_300.xmp").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 recipe"));

    // Verify only one asset exists (no standalone Other asset created)
    let output = maki()
        .current_dir(&root)
        .args(["search", "DSC_300"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 result"),
        "Expected 1 result, got: {stdout}"
    );

    // Verify the recipe is attached and metadata applied
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Recipes:")
                .and(predicate::str::contains("standalone_tag"))
                .and(predicate::str::contains("rating")),
        );
}

#[test]
fn verify_recipe_modification_not_failure() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_400.nef", b"raw image for verify modify test");
    create_test_file(&photos, "DSC_400.xmp", b"xmp original content");

    // Import
    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Modify XMP on disk
    std::fs::write(photos.join("DSC_400.xmp"), b"xmp modified content").unwrap();

    // Verify — should report "modified", NOT "FAILED", and exit 0
    maki()
        .current_dir(&root)
        .arg("verify")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("modified")
                .and(predicate::str::contains("FAILED").not()),
        );
}

// ─── Formatting tests ──────────────────────────────────────────────

#[test]
fn search_format_ids_outputs_uuids() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"ids-format-test");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "--format=ids", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 1);
    // Should be a UUID (36 chars with hyphens)
    assert_eq!(lines[0].len(), 36, "Expected full UUID, got: {}", lines[0]);
    // Should NOT have "result(s)" count
    assert!(!stdout.contains("result(s)"));
}

#[test]
fn search_quiet_shorthand() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"quiet-test");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].len(), 36);
}

#[test]
fn search_format_json_outputs_valid_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"json-format-test");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "--format=json", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed.as_array().expect("should be array");
    assert_eq!(arr.len(), 1);
    assert!(arr[0]["asset_id"].is_string());
    assert!(arr[0]["content_hash"].is_string());
    assert!(arr[0]["tags"].is_array());
}

#[test]
fn search_format_template_renders_placeholders() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "sunset.jpg", b"template-test");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "--format={short_id}\t{filename}\t{format}", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 1);
    let parts: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].len(), 8, "short_id should be 8 chars");
    assert_eq!(parts[1], "sunset.jpg");
    assert_eq!(parts[2], "jpg");
}

#[test]
fn search_json_global_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"global-json-test");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "search", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed.is_array());
}

#[test]
fn show_json_outputs_asset_details() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"show-json-test");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID via search -q
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let asset_id = String::from_utf8(output).unwrap().trim().to_string();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["id"].is_string());
    assert!(parsed["asset_type"].is_string());
    assert!(parsed["variants"].is_array());
}

#[test]
fn volume_list_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let output = maki()
        .current_dir(&root)
        .args(["--json", "volume", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed.as_array().expect("should be array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["label"].as_str(), Some("test-vol"));
    assert!(arr[0]["is_online"].is_boolean());
}

#[test]
fn volume_add_with_purpose() {
    let dir = tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    maki().current_dir(&canonical).arg("init").assert().success();

    maki()
        .current_dir(&canonical)
        .args([
            "volume", "add", "backup-drive",
            canonical.to_str().unwrap(),
            "--purpose", "backup",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("backup-drive"))
        .stdout(predicate::str::contains("Purpose: backup"));

    // Verify it shows in list
    maki()
        .current_dir(&canonical)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[backup]"));
}

#[test]
fn volume_add_purpose_invalid() {
    let dir = tempdir().unwrap();
    maki().current_dir(dir.path()).arg("init").assert().success();

    maki()
        .current_dir(dir.path())
        .args([
            "volume", "add", "test",
            dir.path().to_str().unwrap(),
            "--purpose", "invalid",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid purpose"));
}

#[test]
fn volume_set_purpose() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Volume starts with no purpose
    maki()
        .current_dir(&root)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[online]"))
        .stdout(predicate::str::is_match(r"\[backup\]|\[archive\]|\[working\]|\[cloud\]").unwrap().not());

    // Set purpose
    maki()
        .current_dir(&root)
        .args(["volume", "set-purpose", "test-vol", "archive"])
        .assert()
        .success()
        .stdout(predicate::str::contains("purpose set to: archive"));

    // Verify in list
    maki()
        .current_dir(&root)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[archive]"));

    // Clear purpose
    maki()
        .current_dir(&root)
        .args(["volume", "set-purpose", "test-vol", "none"])
        .assert()
        .success()
        .stdout(predicate::str::contains("purpose cleared"));
}

#[test]
fn volume_set_purpose_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let output = maki()
        .current_dir(&root)
        .args(["--json", "volume", "set-purpose", "test-vol", "backup"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["label"].as_str(), Some("test-vol"));
    assert_eq!(parsed["purpose"].as_str(), Some("backup"));
}

#[test]
fn volume_purpose_in_list_json() {
    let dir = tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    maki().current_dir(&canonical).arg("init").assert().success();

    maki()
        .current_dir(&canonical)
        .args([
            "volume", "add", "my-archive",
            canonical.to_str().unwrap(),
            "--purpose", "archive",
        ])
        .assert()
        .success();

    let output = maki()
        .current_dir(&canonical)
        .args(["--json", "volume", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed.as_array().expect("should be array");
    assert_eq!(arr[0]["purpose"].as_str(), Some("archive"));
}

#[test]
fn volume_remove_report_only() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"volume-remove-report");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Report-only (no --apply)
    maki()
        .current_dir(&root)
        .args(["volume", "remove", "test-vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("would remove"))
        .stdout(predicate::str::contains("1 locations"));

    // Volume should still exist
    maki()
        .current_dir(&root)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("test-vol"));

    // Catalog should still have the asset
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    1"));
}

#[test]
fn volume_remove_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"volume-remove-apply");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Apply removal
    maki()
        .current_dir(&root)
        .args(["volume", "remove", "test-vol", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed"));

    // Volume should be gone
    maki()
        .current_dir(&root)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No volumes"));

    // Asset should be gone (orphaned)
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    0"));
}

#[test]
fn volume_remove_empty() {
    let dir = tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    maki().current_dir(&canonical).arg("init").assert().success();
    maki()
        .current_dir(&canonical)
        .args(["volume", "add", "empty-vol", canonical.to_str().unwrap()])
        .assert()
        .success();

    // Remove empty volume
    maki()
        .current_dir(&canonical)
        .args(["volume", "remove", "empty-vol", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 locations removed"));

    // Volume should be gone
    maki()
        .current_dir(&canonical)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No volumes"));
}

#[test]
fn volume_remove_unknown() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["volume", "remove", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No volume found"));
}

#[test]
fn volume_remove_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"volume-remove-json");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "volume", "remove", "test-vol"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["volume_label"].as_str(), Some("test-vol"));
    assert_eq!(parsed["locations"].as_u64(), Some(1));
    assert_eq!(parsed["apply"].as_bool(), Some(false));
    assert!(parsed["orphaned_assets"].as_u64().unwrap() >= 1);
}

#[test]
fn import_json_outputs_result() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"import-json-test");

    let output = maki()
        .current_dir(&root)
        .args(["--json", "import", root.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["imported"].is_number());
    assert_eq!(parsed["imported"].as_u64(), Some(1));
}

#[test]
fn import_dry_run_reports_without_changes() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let sub = root.join("batch");
    std::fs::create_dir_all(&sub).unwrap();
    create_test_file(&sub, "a.jpg", b"dry-run-a");
    create_test_file(&sub, "b.png", b"dry-run-b");

    // Dry run should report what would happen
    maki()
        .current_dir(&root)
        .args(["import", "--dry-run", sub.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("2 imported"));

    // Search should find nothing — no actual imports happened
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));

    // No sidecar YAML files should have been created
    let sidecar_count = std::fs::read_dir(root.join("assets"))
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    assert_eq!(sidecar_count, 0, "dry run should not create sidecar files");
}

#[test]
fn import_dry_run_json_includes_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"dry-run-json-test");

    let output = maki()
        .current_dir(&root)
        .args(["--json", "import", "--dry-run", root.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["dry_run"].as_bool(), Some(true));
    assert_eq!(parsed["imported"].as_u64(), Some(1));
}

#[test]
fn duplicates_format_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let output = maki()
        .current_dir(&root)
        .args(["duplicates", "--format=json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed.is_array());
}

// ─── Stats tests ────────────────────────────────────────────────

#[test]
fn stats_shows_overview() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"stats overview data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .arg("stats")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Catalog Overview")
                .and(predicate::str::contains("Assets:"))
                .and(predicate::str::contains("Variants:"))
                .and(predicate::str::contains("Volumes:")),
        );
}

#[test]
fn stats_empty_catalog() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .arg("stats")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Assets:    0")
                .and(predicate::str::contains("Variants:  0")),
        );
}

#[test]
fn stats_all_shows_all_sections() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"stats all data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["stats", "--all"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Catalog Overview")
                .and(predicate::str::contains("Asset Types"))
                .and(predicate::str::contains("Volumes"))
                .and(predicate::str::contains("Tags"))
                .and(predicate::str::contains("Verification")),
        );
}

#[test]
fn stats_types_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"stats types data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["stats", "--types"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Asset Types")
                .and(predicate::str::contains("image"))
                .and(predicate::str::contains("Variant Formats"))
                .and(predicate::str::contains("jpg")),
        );
}

#[test]
fn stats_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"stats json data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "stats", "--all"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["overview"]["assets"].is_number());
    assert_eq!(parsed["overview"]["assets"].as_u64(), Some(1));
    assert!(parsed["types"]["asset_types"].is_array());
    assert!(parsed["volumes"].is_array());
    assert!(parsed["tags"]["unique_tags"].is_number());
    assert!(parsed["verified"]["total_locations"].is_number());
}

#[test]
fn stats_tags_shows_frequencies() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"stats tags data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID and add tags
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    maki()
        .current_dir(&root)
        .args(["tag", &asset_id, "landscape", "sunset"])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["stats", "--tags"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Tags")
                .and(predicate::str::contains("Tagged assets:   1"))
                .and(predicate::str::contains("landscape"))
                .and(predicate::str::contains("sunset")),
        );
}

#[test]
fn debug_flag_accepted() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // --debug before subcommand
    maki()
        .current_dir(&root)
        .args(["--debug", "stats"])
        .assert()
        .success();

    // -d shorthand
    maki()
        .current_dir(&root)
        .args(["-d", "stats"])
        .assert()
        .success();
}

#[test]
fn import_with_volume_flag() {
    let dir = tempdir().unwrap();
    let (root, vol1, _vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "explicit_vol.jpg", b"explicit volume test data");

    // Import with explicit --volume instead of auto-detect
    maki()
        .current_dir(&root)
        .args([
            "import",
            "--volume",
            "vol1",
            vol1.join("explicit_vol.jpg").to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Verify it landed on vol1
    let output = maki()
        .current_dir(&root)
        .args(["search", "explicit_vol"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("vol1"));
}

#[test]
fn generate_previews_with_volume_filter() {
    let dir = tempdir().unwrap();
    let (root, vol1, vol2) = init_two_volumes(dir.path());

    create_test_file(&vol1, "vol1_photo.jpg", b"vol1 preview data");
    create_test_file(&vol2, "vol2_photo.jpg", b"vol2 preview data");

    // Import both
    maki()
        .current_dir(&root)
        .args(["import", vol1.join("vol1_photo.jpg").to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", vol2.join("vol2_photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Generate previews only for vol1
    maki()
        .current_dir(&root)
        .args(["generate-previews", "--volume", "vol1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("preview(s)"));
}

#[test]
fn generate_previews_with_paths() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let sub = root.join("photos");
    std::fs::create_dir_all(&sub).unwrap();
    create_test_file(&sub, "path_test.jpg", b"path preview data");

    // Import first
    maki()
        .current_dir(&root)
        .args(["import", sub.to_str().unwrap()])
        .assert()
        .success();

    // Generate previews using PATHS mode
    maki()
        .current_dir(&root)
        .args(["generate-previews", sub.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("preview(s)"));
}

#[test]
fn edit_sets_name_and_description() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "editable.jpg", b"edit test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set name and description
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--name", "My Photo", "--description", "A lovely sunset"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Name: My Photo")
                .and(predicate::str::contains("Description: A lovely sunset")),
        );

    // Verify via show --json
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["name"].as_str(), Some("My Photo"));
    assert_eq!(parsed["description"].as_str(), Some("A lovely sunset"));
}

#[test]
fn edit_sets_and_clears_rating() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "rated.jpg", b"rating test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set rating
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--rating", "4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rating: \u{2605}\u{2605}\u{2605}\u{2605}\u{2606} (4/5)"));

    // Verify via show --json
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["rating"].as_u64(), Some(4));

    // Clear rating
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rating: (none)"));

    // Verify cleared
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["rating"].is_null());
}

#[test]
fn edit_clears_name_and_description() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "clearable.jpg", b"clear test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set name and description
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--name", "Test Name", "--description", "Test Desc"])
        .assert()
        .success();

    // Clear them
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-name", "--clear-description"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Name: (none)")
                .and(predicate::str::contains("Description: (none)")),
        );

    // Verify via show --json
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["name"].is_null());
    assert!(parsed["description"].is_null());
}

#[test]
fn edit_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "json_edit.jpg", b"json edit test");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "edit", &asset_id, "--name", "JSON Name", "--rating", "3"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["asset_id"].as_str().unwrap().len(), 36);
    assert_eq!(parsed["name"].as_str(), Some("JSON Name"));
    assert_eq!(parsed["rating"].as_u64(), Some(3));
}

#[test]
fn edit_no_flags_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "noflags.jpg", b"no flags test");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    maki()
        .current_dir(&root)
        .args(["edit", &asset_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No edit flags provided"));
}

#[test]
fn generate_previews_log_shows_per_file_progress() {
    use image::{ImageBuffer, Rgb};

    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create a real 1x1 PNG so preview generation succeeds
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgb([255, 0, 0]));
    let img_path = root.join("log_test.png");
    img.save(&img_path).unwrap();

    maki()
        .current_dir(&root)
        .args(["import", img_path.to_str().unwrap()])
        .assert()
        .success();

    // With --log, per-file progress appears on stderr
    maki()
        .current_dir(&root)
        .args(["--log", "generate-previews"])
        .assert()
        .success()
        .stderr(predicate::str::contains("log_test.png"));

    // Without --log, no per-file output on stderr
    maki()
        .current_dir(&root)
        .args(["generate-previews"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

// ── Sync tests ─────────────────────────────────────────────────────

#[test]
fn sync_detects_unchanged() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"photo data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("unchanged"));
}

#[test]
fn sync_detects_moved_file() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let sub = root.join("originals");
    std::fs::create_dir_all(&sub).unwrap();
    let file = create_test_file(&sub, "photo.jpg", b"moved photo data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Move the file to a new directory
    let new_sub = root.join("renamed");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("photo.jpg")).unwrap();

    // Dry run — should detect moved
    maki()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("moved"));

    // Apply — should update location
    maki()
        .current_dir(&root)
        .args(["sync", "--apply", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("moved"));

    // Verify the location was updated by running show (should show new path)
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim();

    maki()
        .current_dir(&root)
        .args(["--json", "show", asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("renamed/photo.jpg"));
}

#[test]
fn sync_detects_new_file() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file1 = create_test_file(&root, "existing.jpg", b"existing data");

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();

    // Create a new file that wasn't imported
    create_test_file(&root, "brand_new.jpg", b"brand new data");

    maki()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("new"))
        .stdout(predicate::str::contains("Tip: run 'maki import'"));
}

#[test]
fn sync_detects_missing_file() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "gone.jpg", b"will be deleted");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Delete the file
    std::fs::remove_file(&file).unwrap();

    maki()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("missing"));
}

#[test]
fn sync_remove_stale() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "stale.jpg", b"stale data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id before deleting
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    std::fs::remove_file(&file).unwrap();

    // --remove-stale requires --apply
    maki()
        .current_dir(&root)
        .args(["sync", "--remove-stale", root.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--remove-stale requires --apply"));

    // Apply with --remove-stale
    maki()
        .current_dir(&root)
        .args(["sync", "--apply", "--remove-stale", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("stale removed"));

    // Show should still work but location should be gone
    let show_output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success();
    let show_stdout = String::from_utf8_lossy(&show_output.get_output().stdout);
    let show_json: serde_json::Value = serde_json::from_str(&show_stdout).expect("valid JSON");
    let locations = &show_json["variants"][0]["locations"];
    assert_eq!(locations.as_array().unwrap().len(), 0, "location should be removed");
}

#[test]
fn sync_detects_modified_recipe() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create a NEF + XMP pair
    create_test_file(&root, "DSC_001.nef", b"raw image data");
    let xmp = create_test_file(&root, "DSC_001.xmp", b"<xmp>original</xmp>");

    maki()
        .current_dir(&root)
        .args(["import", "--include", "captureone", root.to_str().unwrap()])
        .assert()
        .success();

    // Modify the XMP
    std::fs::write(&xmp, b"<xmp>modified content</xmp>").unwrap();

    maki()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("modified"));
}

#[test]
fn sync_default_is_dry_run() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let sub = root.join("before");
    std::fs::create_dir_all(&sub).unwrap();
    let file = create_test_file(&sub, "moveme.jpg", b"dry run data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Move the file
    let new_sub = root.join("after");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("moveme.jpg")).unwrap();

    // Sync without --apply (dry run)
    maki()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("moved"));

    // Show should still have the old path (catalog unchanged)
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim();

    maki()
        .current_dir(&root)
        .args(["--json", "show", asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("before/moveme.jpg"));
}

#[test]
fn sync_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "json_test.jpg", b"json test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "sync", root.to_str().unwrap()])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(json.get("unchanged").is_some());
    assert!(json.get("moved").is_some());
    assert!(json.get("new_files").is_some());
    assert!(json.get("modified").is_some());
    assert!(json.get("missing").is_some());
    assert!(json.get("stale_removed").is_some());
    assert!(json.get("errors").is_some());
}

#[test]
fn sync_no_paths_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No paths specified"));
}

// ── Cleanup ─────────────────────────────────────────────────────────

#[test]
fn cleanup_no_stale() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"photo data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 stale"));
}

#[test]
fn cleanup_detects_stale() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "gone.jpg", b"gone data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    maki()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stale"));
}

#[test]
fn cleanup_apply_removes_stale() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "remove_me.jpg", b"remove data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    maki()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 removed"))
        .stdout(predicate::str::contains("1 orphaned assets removed"));

    // Asset should be fully removed (no results from search)
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "*"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(search_output).unwrap();
    assert!(stdout.trim().is_empty(), "orphaned asset should be removed");
}

#[test]
fn cleanup_default_is_report_only() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "keep_it.jpg", b"keep data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    std::fs::remove_file(&file).unwrap();

    // Without --apply: reports stale but doesn't remove
    maki()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stale"))
        .stdout(predicate::str::contains("--apply"));

    // Location should still be in the catalog
    let show_output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success();
    let show_stdout = String::from_utf8_lossy(&show_output.get_output().stdout);
    let show_json: serde_json::Value = serde_json::from_str(&show_stdout).expect("valid JSON");
    let locations = &show_json["variants"][0]["locations"];
    assert_eq!(locations.as_array().unwrap().len(), 1, "location should still exist");
}

#[test]
fn cleanup_volume_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Add a second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Import a file on the main volume
    let file1 = create_test_file(&root, "on_vol1.jpg", b"vol1 data");
    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();

    // Import a file on volume 2
    let file2 = create_test_file(&vol2_path, "on_vol2.jpg", b"vol2 data");
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file2.to_str().unwrap()])
        .assert()
        .success();

    // Delete both files
    std::fs::remove_file(&file1).unwrap();
    std::fs::remove_file(&file2).unwrap();

    // Cleanup only vol2 — should only see 1 stale
    maki()
        .current_dir(&root)
        .args(["cleanup", "--volume", "vol2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stale"));
}

#[test]
fn cleanup_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "json_cleanup.jpg", b"json cleanup data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "cleanup"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(json.get("checked").is_some());
    assert!(json.get("stale").is_some());
    assert!(json.get("removed").is_some());
    assert!(json.get("skipped_offline").is_some());
    assert!(json.get("errors").is_some());
}

#[test]
fn cleanup_stale_recipe() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create a NEF + XMP pair
    create_test_file(&root, "DSC_001.nef", b"raw image data for cleanup");
    let xmp = create_test_file(&root, "DSC_001.xmp", b"<xmp>recipe data</xmp>");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Confirm recipe exists
    let show_output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success();
    let show_stdout = String::from_utf8_lossy(&show_output.get_output().stdout);
    let show_json: serde_json::Value = serde_json::from_str(&show_stdout).expect("valid JSON");
    assert!(!show_json["recipes"].as_array().unwrap().is_empty(), "recipe should exist");

    // Delete the XMP file
    std::fs::remove_file(&xmp).unwrap();

    // Cleanup --apply should remove the stale recipe
    maki()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stale"));

    // Recipe should be gone
    let show_output2 = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success();
    let show_stdout2 = String::from_utf8_lossy(&show_output2.get_output().stdout);
    let show_json2: serde_json::Value = serde_json::from_str(&show_stdout2).expect("valid JSON");
    assert!(show_json2["recipes"].as_array().unwrap().is_empty(), "recipe should be removed");
}

#[test]
fn cleanup_list_shows_only_stale() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "present.jpg", b"present data");
    let gone = create_test_file(&root, "gone.jpg", b"gone data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&gone).unwrap();

    let output = maki()
        .current_dir(&root)
        .args(["cleanup", "--list"])
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    // Should list the stale file
    assert!(stderr.contains("gone.jpg"), "should list stale file on stderr");
    // Should NOT list the present file (--list filters to stale only)
    assert!(!stderr.contains("present.jpg"), "should not list ok files on stderr");
}

// ── Search location health filters ─────────────────────────────────

#[test]
fn search_orphan_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "orphan_test.jpg", b"orphan data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();
    assert!(!asset_id.is_empty(), "should find imported asset");

    // Delete the file, then cleanup (report only) to remove just locations
    std::fs::remove_file(&file).unwrap();

    // Use report-only cleanup — doesn't remove anything
    maki()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stale"));

    // Manually remove the stale location via cleanup --apply but the orphan
    // asset is also removed now. So test orphan search by using --json cleanup
    // without --apply first, then verifying the orphan count.
    // For the orphan:true search test, we need an asset with no locations.
    // Use SQLite directly to delete the file_location.
    let db_path = root.join("catalog.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("DELETE FROM file_locations", []).unwrap();
    drop(conn);

    // Now search orphan:true should find the asset
    maki()
        .current_dir(&root)
        .args(["search", "orphan:true"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&asset_id[..8]));
}

#[test]
fn search_orphan_filter_excludes_located() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "located.jpg", b"located data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // File still exists on disk, locations intact — orphan:true should return nothing
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "orphan:true"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.trim().is_empty(), "orphan:true should find nothing when locations exist");
}

#[test]
fn search_missing_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "missing_test.jpg", b"missing data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();
    assert!(!asset_id.is_empty(), "should find imported asset");

    // Delete the file but DON'T run cleanup (location record still exists)
    std::fs::remove_file(&file).unwrap();

    // missing:true should find it (file missing from disk but location in catalog)
    maki()
        .current_dir(&root)
        .args(["search", "missing:true"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&asset_id[..8]));
}

#[test]
fn search_missing_filter_excludes_present() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "present.jpg", b"present data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // File still exists — missing:true should find nothing
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "missing:true"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.trim().is_empty(), "missing:true should find nothing when files exist");
}

#[test]
fn search_stale_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "stale_test.jpg", b"stale data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();
    assert!(!asset_id.is_empty(), "should find imported asset");

    // Never explicitly verified, so verified_at is NULL — stale:0 should match
    maki()
        .current_dir(&root)
        .args(["search", "stale:0"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&asset_id[..8]));
}

#[test]
fn search_volume_none_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "volnone_test.jpg", b"volnone data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();
    assert!(!asset_id.is_empty(), "should find imported asset");

    // Remove locations directly via SQLite (not cleanup --apply, which also removes orphaned assets)
    let db_path = root.join("catalog.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("DELETE FROM file_locations", []).unwrap();
    drop(conn);

    // volume:none should find the asset (no locations on any online volume)
    maki()
        .current_dir(&root)
        .args(["search", "volume:none"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&asset_id[..8]));
}

#[test]
fn search_volume_label_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "vol_label_test.jpg", b"vol label data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Search by volume label should find the asset
    maki()
        .current_dir(&root)
        .args(["search", "volume:test-vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result"));

    // Case-insensitive match
    maki()
        .current_dir(&root)
        .args(["search", "volume:Test-Vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result"));

    // Unknown volume should error
    maki()
        .current_dir(&root)
        .args(["search", "volume:nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown volume"));

    // Negated volume should exclude (use -- to prevent clap flag parsing)
    maki()
        .current_dir(&root)
        .args(["search", "--", "-volume:test-vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results"));
}

// ── Cleanup orphaned assets and previews ────────────────────────────

#[test]
fn cleanup_removes_orphaned_assets() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "orphan_cleanup.jpg", b"orphan cleanup data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    maki()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 orphaned assets removed"));

    // search orphan:true should return nothing — the orphan was removed
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "orphan:true"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(search_output).unwrap();
    assert!(stdout.trim().is_empty(), "orphaned asset should be removed by cleanup --apply");
}

#[test]
fn cleanup_reports_orphaned_without_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "report_orphan.jpg", b"report orphan data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    std::fs::remove_file(&file).unwrap();

    // Report-only mode: should count orphaned assets but not remove them
    maki()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 orphaned assets"));

    // Asset should still exist
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success();
}

#[test]
fn cleanup_removes_orphaned_previews() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    // Use .mp3 extension — generates an info card preview (always succeeds, audio is on by default)
    let file = create_test_file(&root, "preview_orphan.mp3", b"preview orphan data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Check that a preview was generated
    let previews_dir = root.join("previews");
    let preview_count_before = count_preview_files(&previews_dir);
    assert!(preview_count_before > 0, "preview should exist after import");

    std::fs::remove_file(&file).unwrap();

    maki()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("orphaned assets removed"));

    // Preview should be gone (removed as part of orphaned asset cleanup)
    let preview_count_after = count_preview_files(&previews_dir);
    assert_eq!(preview_count_after, 0, "orphaned previews should be removed");
}

#[test]
fn cleanup_preserves_non_orphaned_assets() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "keep_me.jpg", b"keep me data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // File still on disk — cleanup --apply should not remove anything
    maki()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 stale"));

    // Asset should still exist
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success();
}

#[test]
fn cleanup_json_includes_orphan_fields() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "json_orphan.jpg", b"json orphan data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "cleanup", "--apply"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(json["orphaned_assets"], 1);
    assert_eq!(json["removed_assets"], 1);
    assert!(json.get("orphaned_previews").is_some());
    assert!(json.get("removed_previews").is_some());
}

// ── update-location tests ─────────────────────────────────────────

#[test]
fn update_location_moves_variant_path() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let sub = root.join("originals");
    std::fs::create_dir_all(&sub).unwrap();
    let file = create_test_file(&sub, "photo.jpg", b"update location test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Move the file on disk
    let new_sub = root.join("moved");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("photo.jpg")).unwrap();

    let old_path = format!("originals/photo.jpg");
    let new_path = new_sub.join("photo.jpg");

    // Run update-location
    maki()
        .current_dir(&root)
        .args([
            "update-location", &asset_id,
            "--from", &old_path,
            "--to", new_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated variant location"));

    // Verify show has the new path
    maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("moved/photo.jpg")
                .and(predicate::str::contains("originals/photo.jpg").not()),
        );
}

#[test]
fn update_location_moves_recipe_path() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "DSC_001.nef", b"raw image for recipe move");
    let xmp = create_test_file(&root, "DSC_001.xmp", b"<xmp>recipe</xmp>");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID
    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Move the XMP file on disk
    let sub = root.join("recipes");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::rename(&xmp, sub.join("DSC_001.xmp")).unwrap();

    let new_path = sub.join("DSC_001.xmp");

    // Run update-location
    maki()
        .current_dir(&root)
        .args([
            "update-location", &asset_id,
            "--from", "DSC_001.xmp",
            "--to", new_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated recipe location"));

    // Verify show has the new recipe path
    maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("recipes/DSC_001.xmp"));
}

#[test]
fn update_location_rejects_wrong_hash() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"original content");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Create a DIFFERENT file at the new path
    let new_file = create_test_file(&root, "moved/photo.jpg", b"different content entirely");

    maki()
        .current_dir(&root)
        .args([
            "update-location", &asset_id,
            "--from", "photo.jpg",
            "--to", new_file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Hash mismatch"));
}

#[test]
fn update_location_rejects_missing_from() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "photo.jpg", b"some data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // --from path doesn't exist in catalog, --to is a valid file
    let new_file = create_test_file(&root, "elsewhere/photo.jpg", b"some data");

    maki()
        .current_dir(&root)
        .args([
            "update-location", &asset_id,
            "--from", "nonexistent/photo.jpg",
            "--to", new_file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No variant or recipe found"));
}

#[test]
fn update_location_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let sub = root.join("originals");
    std::fs::create_dir_all(&sub).unwrap();
    let file = create_test_file(&sub, "photo.jpg", b"json output test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Move on disk
    let new_sub = root.join("moved");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("photo.jpg")).unwrap();

    let new_path = new_sub.join("photo.jpg");

    let output = maki()
        .current_dir(&root)
        .args([
            "--json", "update-location", &asset_id,
            "--from", "originals/photo.jpg",
            "--to", new_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let out = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(json["file_type"], "variant");
    assert_eq!(json["old_path"], "originals/photo.jpg");
    assert_eq!(json["new_path"], "moved/photo.jpg");
    assert_eq!(json["volume_label"], "test-vol");
    assert!(json["asset_id"].as_str().is_some());
    assert!(json["content_hash"].as_str().is_some());
}

#[test]
fn update_location_auto_detects_volume() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "auto.jpg", b"auto detect volume test");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Move on disk (same volume, no --volume flag)
    let new_sub = root.join("newdir");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("auto.jpg")).unwrap();

    let new_path = new_sub.join("auto.jpg");

    maki()
        .current_dir(&root)
        .args([
            "update-location", &asset_id,
            "--from", "auto.jpg",
            "--to", new_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated variant location"));

    // Verify new path in show output
    maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("newdir/auto.jpg"));
}

// ── Saved Search tests ──────────────────────────────────────────

#[test]
fn saved_search_save_and_list() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Save a search
    maki()
        .current_dir(&root)
        .args(["saved-search", "save", "Landscapes", "type:image tag:landscape rating:4+"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Saved search 'Landscapes'"));

    // List shows it
    maki()
        .current_dir(&root)
        .args(["saved-search", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Landscapes"))
        .stdout(predicate::str::contains("type:image tag:landscape rating:4+"));
}

#[test]
fn saved_search_alias_works() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["ss", "save", "Test", "type:video"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Saved search 'Test'"));

    maki()
        .current_dir(&root)
        .args(["ss", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test"));
}

#[test]
fn saved_search_run() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import a file
    create_test_file(&root, "photo.jpg", b"saved-search-test-photo");
    maki()
        .current_dir(&root)
        .args(["import", root.join("photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Save and run a search that matches
    maki()
        .current_dir(&root)
        .args(["ss", "save", "All Images", "type:image"])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["ss", "run", "All Images"])
        .assert()
        .success()
        .stdout(predicate::str::contains("photo"));
}

#[test]
fn saved_search_run_not_found() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["ss", "run", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No saved search named"));
}

#[test]
fn saved_search_delete() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["ss", "save", "ToDelete", "type:video"])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["ss", "delete", "ToDelete"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted saved search 'ToDelete'"));

    // List is now empty
    maki()
        .current_dir(&root)
        .args(["ss", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No saved searches"));
}

#[test]
fn saved_search_delete_not_found() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["ss", "delete", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No saved search named"));
}

#[test]
fn saved_search_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["--json", "ss", "save", "Test", "type:image"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\""))
        .stdout(predicate::str::contains("\"saved\""));

    maki()
        .current_dir(&root)
        .args(["--json", "ss", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\""))
        .stdout(predicate::str::contains("Test"));
}

#[test]
fn saved_search_replace_existing() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["ss", "save", "My Search", "type:image"])
        .assert()
        .success();

    // Save again with same name — should replace
    maki()
        .current_dir(&root)
        .args(["ss", "save", "My Search", "type:video", "--sort", "name_asc"])
        .assert()
        .success();

    // List should show updated query
    maki()
        .current_dir(&root)
        .args(["ss", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("type:video"))
        .stdout(predicate::str::contains("name_asc"));
}

// ── Collection tests ────────────────────────────────────────────

#[test]
fn collection_create_and_list() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["collection", "create", "Portfolio", "--description", "Best shots"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created collection 'Portfolio'"));

    maki()
        .current_dir(&root)
        .args(["collection", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Portfolio"))
        .stdout(predicate::str::contains("0 assets"));
}

#[test]
fn collection_alias_works() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["col", "create", "Test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created collection 'Test'"));

    maki()
        .current_dir(&root)
        .args(["col", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test"));
}

#[test]
fn collection_add_and_show() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import a file
    create_test_file(&root, "col_photo.jpg", b"collection-test-photo");
    maki()
        .current_dir(&root)
        .args(["import", root.join("col_photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Get the asset ID
    let output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "col_photo"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(!asset_id.is_empty());

    // Create collection and add asset
    maki()
        .current_dir(&root)
        .args(["col", "create", "MyPicks"])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["col", "add", "MyPicks", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 asset"));

    // Show collection contents
    maki()
        .current_dir(&root)
        .args(["col", "show", "MyPicks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("col_photo"));

    // List shows count
    maki()
        .current_dir(&root)
        .args(["col", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 assets"));
}

#[test]
fn collection_remove_and_delete() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import a file
    create_test_file(&root, "rm_photo.jpg", b"collection-remove-test");
    maki()
        .current_dir(&root)
        .args(["import", root.join("rm_photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "rm_photo"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string();

    maki()
        .current_dir(&root)
        .args(["col", "create", "Temp"])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["col", "add", "Temp", &asset_id])
        .assert()
        .success();

    // Remove asset from collection
    maki()
        .current_dir(&root)
        .args(["col", "remove", "Temp", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 asset"));

    // Delete collection
    maki()
        .current_dir(&root)
        .args(["col", "delete", "Temp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted collection 'Temp'"));

    // List shows empty
    maki()
        .current_dir(&root)
        .args(["col", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No collections"));
}

#[test]
fn collection_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["--json", "col", "create", "JTest"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\""))
        .stdout(predicate::str::contains("JTest"));

    maki()
        .current_dir(&root)
        .args(["--json", "col", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"asset_count\""));
}

#[test]
fn collection_search_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import two files with unique names
    create_test_file(&root, "alpha_col.jpg", b"alpha-collection-unique");
    create_test_file(&root, "beta_col.jpg", b"beta-collection-unique");
    maki()
        .current_dir(&root)
        .args(["import", root.join("alpha_col.jpg").to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", root.join("beta_col.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Get alpha's asset ID
    let output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "alpha_col"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(!asset_id.is_empty(), "alpha_col asset should exist");

    // Create collection with only alpha
    maki()
        .current_dir(&root)
        .args(["col", "create", "Filtered"])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["col", "add", "Filtered", &asset_id])
        .assert()
        .success();

    // Search with collection filter should find only the one in the collection
    maki()
        .current_dir(&root)
        .args(["search", "collection:Filtered"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha_col"))
        .stdout(predicate::str::contains("1 result"));
}

#[test]
fn search_path_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create files in different subdirectories
    let file_a = create_test_file(&root, "Capture/2026-02-22/DSC_001.jpg", b"photo a");
    let file_b = create_test_file(&root, "Capture/2026-02-22/DSC_002.jpg", b"photo b");
    let file_c = create_test_file(&root, "Archive/old/sunset.jpg", b"photo c");

    maki()
        .current_dir(&root)
        .args(["import", file_a.to_str().unwrap(), file_b.to_str().unwrap(), file_c.to_str().unwrap()])
        .assert()
        .success();

    // path: filter should match only files under Capture/2026-02-22
    maki()
        .current_dir(&root)
        .args(["search", "path:Capture/2026-02-22"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // path: filter for Archive should match only the sunset file
    maki()
        .current_dir(&root)
        .args(["search", "path:Archive/"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sunset"))
        .stdout(predicate::str::contains("1 result"));

    // path: with no match
    maki()
        .current_dir(&root)
        .args(["search", "path:Nonexistent/"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));
}

#[test]
fn search_path_absolute_normalizes_to_relative() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create files in a subdirectory
    let file_a = create_test_file(&root, "photos/DSC_001.jpg", b"abs path photo a");
    let file_b = create_test_file(&root, "photos/DSC_002.jpg", b"abs path photo b");
    let file_c = create_test_file(&root, "other/sunset.jpg", b"abs path photo c");

    maki()
        .current_dir(&root)
        .args(["import", file_a.to_str().unwrap(), file_b.to_str().unwrap(), file_c.to_str().unwrap()])
        .assert()
        .success();

    // Search with absolute path should find the same results as relative
    let abs_path = format!("path:{}/photos", root.display());
    maki()
        .current_dir(&root)
        .args(["search", &abs_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // Verify relative path works identically
    maki()
        .current_dir(&root)
        .args(["search", "path:photos"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // Bogus absolute path should return nothing
    maki()
        .current_dir(&root)
        .args(["search", "path:/nonexistent/volume/photos"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));

    // ./ relative to cwd should resolve and normalize
    maki()
        .current_dir(root.join("photos"))
        .args(["search", "path:./"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // ../ relative to cwd should resolve and normalize
    maki()
        .current_dir(root.join("photos"))
        .args(["search", "path:../other"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sunset"))
        .stdout(predicate::str::contains("1 result"));
}

// ── import --auto-group tests ────────────────────────────────

#[test]
fn import_auto_group_sibling_dirs() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // CaptureOne-style: RAW in Capture/, export in Output/ under a session root
    let capture = root.join("session/Capture");
    let output = root.join("session/Output");
    std::fs::create_dir_all(&capture).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(capture.join("DSC_100.ARW"), b"raw-auto-group-sibling").unwrap();
    std::fs::write(output.join("DSC_100.JPG"), b"jpeg-auto-group-sibling").unwrap();

    // Import both directories at once with --auto-group
    maki()
        .current_dir(&root)
        .args([
            "import",
            "--auto-group",
            capture.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 imported"))
        .stdout(predicate::str::contains("Auto-group"))
        .stdout(predicate::str::contains("merged"));

    // Should be 1 asset with 2 variants
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result(s)"));
}

#[test]
fn import_auto_group_incremental() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // First import: RAW file only, no --auto-group
    let capture = root.join("session2/Capture");
    std::fs::create_dir_all(&capture).unwrap();
    std::fs::write(capture.join("DSC_200.ARW"), b"raw-incr-auto-group").unwrap();
    maki()
        .current_dir(&root)
        .args(["import", capture.to_str().unwrap()])
        .assert()
        .success();

    // Second import: export with --auto-group
    let output = root.join("session2/Output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("DSC_200.JPG"), b"jpeg-incr-auto-group").unwrap();
    maki()
        .current_dir(&root)
        .args(["import", "--auto-group", output.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Auto-group"));

    // Should be 1 asset (existing RAW picked up the export)
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result(s)"));
}

#[test]
fn import_auto_group_fuzzy_prefix() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // CaptureOne-style export with suffix appended
    let capture = root.join("session3/Capture");
    let output = root.join("session3/Output");
    std::fs::create_dir_all(&capture).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(capture.join("Z91_8561.ARW"), b"raw-fuzzy-prefix").unwrap();
    std::fs::write(
        output.join("Z91_8561-1-HighRes.tif"),
        b"tif-fuzzy-prefix",
    )
    .unwrap();

    maki()
        .current_dir(&root)
        .args([
            "import",
            "--auto-group",
            capture.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Auto-group"));

    // Should be 1 asset
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result(s)"));
}

#[test]
fn import_auto_group_no_false_positives() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Files in unrelated session directories (different session roots)
    let session_a = root.join("2024-01-01/Capture");
    let session_b = root.join("2024-06-15/Capture");
    std::fs::create_dir_all(&session_a).unwrap();
    std::fs::create_dir_all(&session_b).unwrap();
    std::fs::write(session_a.join("DSC_001.ARW"), b"raw-session-a-001").unwrap();
    std::fs::write(session_b.join("DSC_001.ARW"), b"raw-session-b-001").unwrap();

    // Import session A first (no --auto-group)
    maki()
        .current_dir(&root)
        .args(["import", session_a.to_str().unwrap()])
        .assert()
        .success();

    // Import session B with --auto-group — should NOT merge with session A
    // because they are under different session roots
    maki()
        .current_dir(&root)
        .args(["import", "--auto-group", session_b.to_str().unwrap()])
        .assert()
        .success();

    // Should still be 2 separate assets
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"));
}

#[test]
fn import_auto_group_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let capture = root.join("session4/Capture");
    let output = root.join("session4/Output");
    std::fs::create_dir_all(&capture).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(capture.join("IMG_001.ARW"), b"raw-json-auto-group").unwrap();
    std::fs::write(output.join("IMG_001.JPG"), b"jpeg-json-auto-group").unwrap();

    let out = maki()
        .current_dir(&root)
        .args([
            "--json",
            "import",
            "--auto-group",
            capture.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&out.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["imported"], 2);
    assert!(json["auto_group"].is_object(), "auto_group key should be present");
    assert!(json["auto_group"]["groups"].is_array());
    assert_eq!(json["auto_group"]["groups"].as_array().unwrap().len(), 1);
}

// ── auto-group tests ─────────────────────────────────────────

#[test]
fn auto_group_dry_run_reports() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create two files with the same stem in different directories
    let sub1 = root.join("raw");
    let sub2 = root.join("export");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub1.join("DSC_001.ARW"), b"raw-content-for-autogroup").unwrap();
    std::fs::write(sub2.join("DSC_001.JPG"), b"jpeg-content-for-autogroup").unwrap();

    // Import each directory separately so they become separate assets
    maki()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Verify we have 2 assets (search returns one row per variant)
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"));

    // Dry run should report the match
    maki()
        .current_dir(&root)
        .args(["auto-group"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stem group"))
        .stdout(predicate::str::contains("would merge"));

    // Assets should still be separate (dry run)
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"));
}

#[test]
fn auto_group_apply_merges() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let sub1 = root.join("raw");
    let sub2 = root.join("export");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub1.join("DSC_002.ARW"), b"raw-content-ag-apply").unwrap();
    std::fs::write(sub2.join("DSC_002.JPG"), b"jpeg-content-ag-apply").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Apply auto-group
    maki()
        .current_dir(&root)
        .args(["auto-group", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stem group"))
        .stdout(predicate::str::contains("merged"));

    // Should now be 1 unique asset (search -q outputs one ID per variant row)
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unique_ids: std::collections::HashSet<&str> = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(unique_ids.len(), 1, "Expected 1 unique asset after auto-group, got {}", unique_ids.len());
}

#[test]
fn auto_group_no_matches() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "IMG_001.JPG", b"content-ag-no-match-1");
    create_test_file(&root, "IMG_002.JPG", b"content-ag-no-match-2");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["auto-group"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No groupable assets"));
}

#[test]
fn auto_group_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let sub1 = root.join("raw2");
    let sub2 = root.join("export2");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub1.join("DSC_003.ARW"), b"raw-content-ag-json").unwrap();
    std::fs::write(sub2.join("DSC_003.JPG"), b"jpeg-content-ag-json").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "auto-group"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["groups"].as_array().unwrap().len(), 1);
}

#[test]
fn auto_group_fuzzy_prefix_merges_exports() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // RAW file with short name, export with appended suffix
    let sub1 = root.join("raw");
    let sub2 = root.join("export");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub1.join("Z91_8561.ARW"), b"raw-content-fuzzy").unwrap();
    std::fs::write(
        sub2.join("Z91_8561-1-HighRes-(c)_2025_Thomas.JPG"),
        b"export-content-fuzzy",
    )
    .unwrap();

    maki()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Apply auto-group — fuzzy prefix should match
    maki()
        .current_dir(&root)
        .args(["auto-group", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stem group"))
        .stdout(predicate::str::contains("merged"));

    // Should be 1 unique asset
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let unique_ids: std::collections::HashSet<&str> = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(unique_ids.len(), 1);
}

#[test]
fn auto_group_fuzzy_rejects_numeric_continuation() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // DSC_001 and DSC_0010 are different shots — should NOT match
    create_test_file(&root, "sub/DSC_001.ARW", b"raw-content-no-fuzzy-1");
    create_test_file(&root, "sub/DSC_0010.JPG", b"jpg-content-no-fuzzy-2");

    maki()
        .current_dir(&root)
        .args(["import", root.join("sub").to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["auto-group"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No groupable assets"));
}

#[test]
fn group_merges_two_assets() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create two separate assets (different stems in different dirs)
    let sub1 = root.join("raw");
    let sub2 = root.join("export");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub1.join("IMG_100.ARW"), b"raw-group-test").unwrap();
    std::fs::write(sub2.join("IMG_100_edit.JPG"), b"jpg-group-test").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Should be 2 separate assets
    maki()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"));

    // Get variant hashes from show --json
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let ids: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(ids.len(), 2);

    let mut hashes = Vec::new();
    for id in &ids {
        let output = maki()
            .current_dir(&root)
            .args(["--json", "show", id])
            .output()
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("valid JSON");
        let hash = parsed["variants"][0]["content_hash"]
            .as_str()
            .unwrap()
            .to_string();
        hashes.push(hash);
    }

    // Group them
    maki()
        .current_dir(&root)
        .args(["group", &hashes[0], &hashes[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Grouped 2 variant(s)"));

    // Should now be 1 asset
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let unique_ids: std::collections::HashSet<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(unique_ids.len(), 1);
}

#[test]
fn group_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let sub1 = root.join("a");
    let sub2 = root.join("b");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub1.join("GRP_001.ARW"), b"raw-grp-json").unwrap();
    std::fs::write(sub2.join("GRP_001_v2.JPG"), b"jpg-grp-json").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let ids: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();

    let mut hashes = Vec::new();
    for id in &ids {
        let output = maki()
            .current_dir(&root)
            .args(["--json", "show", id])
            .output()
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("valid JSON");
        hashes.push(
            parsed["variants"][0]["content_hash"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }

    let output = maki()
        .current_dir(&root)
        .args(["--json", "group", &hashes[0], &hashes[1]])
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(parsed["target_id"].is_string());
    assert!(parsed["variants_moved"].is_number());
    assert!(parsed["donors_removed"].is_number());
}

#[test]
fn split_extracts_variant_into_new_asset() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create two files with the same stem so they auto-group
    std::fs::write(root.join("IMG_001.ARW"), b"raw-split-test").unwrap();
    std::fs::write(root.join("IMG_001.JPG"), b"jpg-split-test").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Should be 1 asset with 2 variants
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let ids: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(ids.len(), 1);

    // Get variant hashes
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", ids[0]])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let variants = parsed["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2);

    // Find the non-original (alternate) variant and split it out
    let alt_hash = variants
        .iter()
        .find(|v| v["role"].as_str().unwrap() != "original")
        .unwrap()["content_hash"]
        .as_str()
        .unwrap();

    maki()
        .current_dir(&root)
        .args(["split", ids[0], alt_hash])
        .assert()
        .success()
        .stdout(predicate::str::contains("Split 1 variant(s)"));

    // Should now be 2 assets
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let new_ids: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(new_ids.len(), 2);
}

#[test]
fn split_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    std::fs::write(root.join("IMG_002.ARW"), b"raw-split-json").unwrap();
    std::fs::write(root.join("IMG_002.JPG"), b"jpg-split-json").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let ids: Vec<&str> = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(ids.len(), 1);

    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", ids[0]])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let variants = parsed["variants"].as_array().unwrap();
    let alt_hash = variants
        .iter()
        .find(|v| v["role"].as_str().unwrap() != "original")
        .unwrap()["content_hash"]
        .as_str()
        .unwrap();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "split", ids[0], alt_hash])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(parsed["source_id"].is_string());
    assert_eq!(parsed["new_assets"].as_array().unwrap().len(), 1);
    assert!(parsed["new_assets"][0]["asset_id"].is_string());
    assert!(parsed["new_assets"][0]["variant_hash"].is_string());
    assert!(parsed["new_assets"][0]["original_filename"].is_string());
}

#[test]
fn split_refuses_all_variants() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    std::fs::write(root.join("single.jpg"), b"single-file").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let id = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .trim();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", id])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let hash = parsed["variants"][0]["content_hash"].as_str().unwrap();

    // Splitting the only variant should fail
    maki()
        .current_dir(&root)
        .args(["split", id, hash])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cannot extract all variants"));
}

#[test]
fn split_inherits_metadata() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    std::fs::write(root.join("META_001.ARW"), b"raw-meta-split").unwrap();
    std::fs::write(root.join("META_001.JPG"), b"jpg-meta-split").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", ""])
        .output()
        .unwrap();
    let id = std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .trim();

    // Add tags and rating
    maki()
        .current_dir(&root)
        .args(["tag", id, "landscape", "nature"])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["edit", id, "--rating", "4"])
        .assert()
        .success();

    // Get alternate variant hash
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", id])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let alt_hash = parsed["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["role"].as_str().unwrap() != "original")
        .unwrap()["content_hash"]
        .as_str()
        .unwrap();

    // Split
    let output = maki()
        .current_dir(&root)
        .args(["--json", "split", id, alt_hash])
        .output()
        .unwrap();
    let split_result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let new_id = split_result["new_assets"][0]["asset_id"].as_str().unwrap();

    // Check new asset has inherited metadata
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", new_id])
        .output()
        .unwrap();
    let new_asset: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(new_asset["rating"].as_u64().unwrap(), 4);
    let tags: Vec<&str> = new_asset["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(tags.contains(&"landscape"));
    assert!(tags.contains(&"nature"));
    // New variant should be role "original"
    assert_eq!(new_asset["variants"][0]["role"].as_str().unwrap(), "original");
}

#[test]
fn fix_roles_dry_run_reports() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photos/DSC_100.ARW", b"raw-fixroles-1");
    create_test_file(&root, "photos/DSC_100.JPG", b"jpg-fixroles-1");
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Since auto-grouping now sets roles correctly, fix-roles should report already correct
    maki()
        .current_dir(&root)
        .args(["fix-roles"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already correct"));
}

#[test]
fn fix_roles_apply_corrects_roles() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import RAW and JPG from separate directories → 2 separate assets, both Original
    let raw_dir = root.join("raw");
    let jpg_dir = root.join("jpg");
    std::fs::create_dir_all(&raw_dir).unwrap();
    std::fs::create_dir_all(&jpg_dir).unwrap();
    std::fs::write(raw_dir.join("DSC_200.ARW"), b"raw-fixroles-apply").unwrap();
    std::fs::write(jpg_dir.join("DSC_200.JPG"), b"jpg-fixroles-apply").unwrap();

    maki()
        .current_dir(&root)
        .args(["import", raw_dir.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", jpg_dir.to_str().unwrap()])
        .assert()
        .success();

    // Auto-group to merge them into one asset
    maki()
        .current_dir(&root)
        .args(["auto-group", "--apply"])
        .assert()
        .success();

    // After auto-group the JPG should already be Export — fix-roles reports 0 fixed
    maki()
        .current_dir(&root)
        .args(["fix-roles"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 fixed"));
}

#[test]
fn fix_roles_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photos/DSC_300.ARW", b"raw-fixroles-json");
    create_test_file(&root, "photos/DSC_300.JPG", b"jpg-fixroles-json");
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "fix-roles"])
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(parsed["checked"].is_number());
    assert!(parsed["fixed"].is_number());
    assert!(parsed["variants_fixed"].is_number());
    assert!(parsed["already_correct"].is_number());
    assert_eq!(parsed["dry_run"].as_bool(), Some(true));
}

#[test]
fn refresh_detects_unchanged_recipes() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photos/DSC_400.ARW", b"raw-refresh-test");
    create_test_file(
        &root,
        "photos/DSC_400.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"3\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Refresh without changes — should report unchanged
    maki()
        .current_dir(&root)
        .args(["refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 unchanged"));
}

#[test]
fn refresh_detects_modified_recipe() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let xmp_path = root.join("photos/DSC_500.xmp");
    create_test_file(&root, "photos/DSC_500.ARW", b"raw-refresh-modify");
    create_test_file(
        &root,
        "photos/DSC_500.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"2\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Modify the XMP file externally
    std::fs::write(
        &xmp_path,
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"5\"/></rdf:RDF></x:xmpmeta>",
    )
    .unwrap();

    // Refresh should detect the change
    maki()
        .current_dir(&root)
        .args(["refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"));
}

#[test]
fn refresh_dry_run_does_not_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let xmp_path = root.join("photos/DSC_600.xmp");
    create_test_file(&root, "photos/DSC_600.ARW", b"raw-refresh-dry");
    create_test_file(
        &root,
        "photos/DSC_600.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"1\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Modify the XMP file
    std::fs::write(
        &xmp_path,
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"4\"/></rdf:RDF></x:xmpmeta>",
    )
    .unwrap();

    // Dry run — should report but not apply
    maki()
        .current_dir(&root)
        .args(["refresh", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"))
        .stderr(predicate::str::contains("Dry run"));

    // Run again without dry-run — should still see the change (wasn't applied)
    maki()
        .current_dir(&root)
        .args(["refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"));
}

#[test]
fn edit_sets_and_clears_label() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "labeled.jpg", b"label test data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set label
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--label", "Red"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Label: Red"));

    // Verify via show --json
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(parsed["color_label"].as_str(), Some("Red"));

    // Change to another label (case-insensitive)
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--label", "blue"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Label: Blue"));

    // Clear label
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-label"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Label: (none)"));

    // Verify cleared
    let output = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(parsed["color_label"].is_null());
}

#[test]
fn edit_label_validates_color() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "badlabel.jpg", b"bad label test");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Invalid label should fail
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--label", "Magenta"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown color label"));
}

// ── Export-based preview tests ──────────────────────────────────

/// Helper: compute the SHA-256 hex of some content (matches maki's content_hash minus "sha256:" prefix).
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let digest = Sha256::digest(data);
    format!("{:x}", digest)
}

#[test]
fn show_preview_prefers_export_variant() {
    use image::{ImageBuffer, Rgb};

    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create a RAW + real JPG pair with the same stem — auto-groups with JPG as export
    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    let raw_content = b"raw image data for preview test nef";
    create_test_file(&photos, "DSC_900.nef", raw_content);

    // Create a real 1x1 JPEG so preview generation succeeds for the JPG
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgb([0, 255, 0]));
    let jpg_path = photos.join("DSC_900.jpg");
    img.save(&jpg_path).unwrap();
    let jpg_content = std::fs::read(&jpg_path).unwrap();

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID — may return multiple rows (one per variant), take the first
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "DSC_900"])
        .output()
        .unwrap();
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let asset_id = stdout_str.lines().next().unwrap().trim().to_string();
    assert_eq!(asset_id.len(), 36, "Should get a UUID");

    // Verify via show --json that the asset has an export variant
    let show_json = maki()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&show_json.stdout).unwrap();
    let variants = parsed["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2, "Should have 2 variants (RAW + JPG)");

    // The JPG should have role "alternate" (non-RAW alongside RAW)
    let has_alternate = variants.iter().any(|v| v["role"].as_str() == Some("alternate"));
    assert!(has_alternate, "JPG variant should have alternate role");

    // maki show should show the JPG's hash in the Preview line (export preferred)
    let jpg_hash = sha256_hex(&jpg_content);
    let show_output = maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&show_output.stdout);

    // The preview line should reference the JPG export hash
    let preview_line = stdout.lines().find(|l| l.starts_with("Preview:"));
    assert!(
        preview_line.is_some(),
        "Should have a Preview: line"
    );
    assert!(
        preview_line.unwrap().contains(&jpg_hash),
        "Preview should use JPG hash ({jpg_hash}), got: {}",
        preview_line.unwrap()
    );
}

#[test]
fn show_preview_falls_back_to_original() {
    use image::{ImageBuffer, Rgb};

    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Create a real 1x1 PNG so preview generation succeeds
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgb([128, 0, 0]));
    let img_path = root.join("solo.png");
    img.save(&img_path).unwrap();
    let content = std::fs::read(&img_path).unwrap();

    maki()
        .current_dir(&root)
        .args(["import", img_path.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "solo"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let hash = sha256_hex(&content);
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains(&hash));
}

#[test]
fn group_shows_export_preview() {
    use image::{ImageBuffer, Rgb};

    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import RAW and JPG as separate assets (different directories, different stems)
    let dir_a = root.join("dir_a");
    let dir_b = root.join("dir_b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();

    create_test_file(&dir_a, "IMG_001.nef", b"raw data for group preview test");

    // Real image for the JPG so preview works
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(1, 1, Rgb([0, 0, 255]));
    let jpg_path = dir_b.join("IMG_001_export.jpg");
    img.save(&jpg_path).unwrap();
    let jpg_content = std::fs::read(&jpg_path).unwrap();

    // Import separately so they become different assets
    maki()
        .current_dir(&root)
        .args(["import", dir_a.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", dir_b.to_str().unwrap()])
        .assert()
        .success();

    // Get variant hashes via show --json
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "IMG_001.nef"])
        .output()
        .unwrap();
    let raw_asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "IMG_001_export"])
        .output()
        .unwrap();
    let jpg_asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get content hashes from show --json
    let raw_show = maki()
        .current_dir(&root)
        .args(["--json", "show", &raw_asset_id])
        .output()
        .unwrap();
    let raw_json: serde_json::Value = serde_json::from_slice(&raw_show.stdout).unwrap();
    let raw_variant_hash = raw_json["variants"][0]["content_hash"].as_str().unwrap().to_string();

    let jpg_show = maki()
        .current_dir(&root)
        .args(["--json", "show", &jpg_asset_id])
        .output()
        .unwrap();
    let jpg_json: serde_json::Value = serde_json::from_slice(&jpg_show.stdout).unwrap();
    let jpg_variant_hash = jpg_json["variants"][0]["content_hash"].as_str().unwrap().to_string();

    // Group them
    maki()
        .current_dir(&root)
        .args(["group", &raw_variant_hash, &jpg_variant_hash])
        .assert()
        .success();

    // After grouping, the merged asset should prefer JPG (export) preview.
    // The target of `group` is the oldest asset (the RAW one, imported first).
    let jpg_hash = sha256_hex(&jpg_content);
    let show_output = maki()
        .current_dir(&root)
        .args(["show", &raw_asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&show_output.stdout);

    let preview_line = stdout.lines().find(|l| l.starts_with("Preview:"));
    assert!(
        preview_line.is_some_and(|l| l.contains(&jpg_hash)),
        "After group, preview should use JPG hash ({jpg_hash}), got:\n{stdout}"
    );
}

#[test]
fn generate_previews_upgrade_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import RAW + JPG pair
    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_800.nef", b"raw for upgrade test");
    create_test_file(&photos, "DSC_800.jpg", b"jpg for upgrade test");

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    // --upgrade should run without error and report results
    maki()
        .current_dir(&root)
        .args(["generate-previews", "--upgrade"])
        .assert()
        .success()
        .stdout(predicate::str::contains("preview(s)"));

    // --upgrade --json should include upgraded field
    let output = maki()
        .current_dir(&root)
        .args(["--json", "generate-previews", "--upgrade"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(parsed["upgraded"].is_number(), "JSON should include upgraded field");
}

#[test]
fn import_jpeg_with_embedded_xmp_extracts_metadata() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    // Build a minimal JPEG with embedded XMP in APP1
    let xmp_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="5"
    xmp:Label="Green">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>architecture</rdf:li>
     <rdf:li>urban</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <dc:description>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">City skyline at dusk</rdf:li>
    </rdf:Alt>
   </dc:description>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    let xmp_namespace = b"http://ns.adobe.com/xap/1.0/\0";
    let mut jpeg_data: Vec<u8> = Vec::new();
    // SOI
    jpeg_data.extend_from_slice(&[0xFF, 0xD8]);
    // APP1 with XMP
    jpeg_data.extend_from_slice(&[0xFF, 0xE1]);
    let payload_len = xmp_namespace.len() + xmp_xml.len();
    let segment_len = (payload_len + 2) as u16;
    jpeg_data.extend_from_slice(&segment_len.to_be_bytes());
    jpeg_data.extend_from_slice(xmp_namespace);
    jpeg_data.extend_from_slice(xmp_xml.as_bytes());
    // EOI
    jpeg_data.extend_from_slice(&[0xFF, 0xD9]);

    let jpeg_path = photos.join("skyline.jpg");
    std::fs::write(&jpeg_path, &jpeg_data).unwrap();

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Get asset ID via search
    let output = maki()
        .current_dir(&root)
        .args(["search", "skyline"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Verify embedded XMP metadata appears in show output
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("architecture")
                .and(predicate::str::contains("urban"))
                .and(predicate::str::contains("5"))
                .and(predicate::str::contains("Green"))
                .and(predicate::str::contains("City skyline at dusk")),
        );
}

/// Helper: build a minimal JPEG with embedded XMP metadata.
fn build_test_jpeg_with_xmp(xmp_xml: &str) -> Vec<u8> {
    let xmp_namespace = b"http://ns.adobe.com/xap/1.0/\0";
    let mut jpeg_data: Vec<u8> = Vec::new();
    // SOI
    jpeg_data.extend_from_slice(&[0xFF, 0xD8]);
    // APP1 with XMP
    jpeg_data.extend_from_slice(&[0xFF, 0xE1]);
    let payload_len = xmp_namespace.len() + xmp_xml.len();
    let segment_len = (payload_len + 2) as u16;
    jpeg_data.extend_from_slice(&segment_len.to_be_bytes());
    jpeg_data.extend_from_slice(xmp_namespace);
    jpeg_data.extend_from_slice(xmp_xml.as_bytes());
    // EOI
    jpeg_data.extend_from_slice(&[0xFF, 0xD9]);
    jpeg_data
}

#[test]
fn refresh_media_extracts_embedded_xmp() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    let xmp_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>nature</rdf:li>
     <rdf:li>forest</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    let jpeg_data = build_test_jpeg_with_xmp(xmp_xml);
    std::fs::write(photos.join("forest.jpg"), &jpeg_data).unwrap();

    // Import — metadata should be extracted
    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Get asset ID
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Verify initial metadata
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("nature")
                .and(predicate::str::contains("forest"))
                .and(predicate::str::contains("4")),
        );

    // Clear the metadata
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["tag", &asset_id, "--remove", "nature", "forest"])
        .assert()
        .success();

    // Verify tags are cleared
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Tags:").not()
                .and(predicate::str::contains("Rating:").not()),
        );

    // Run refresh --media to re-extract embedded XMP
    maki()
        .current_dir(&root)
        .args(["refresh", "--media"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"));

    // Verify metadata is restored
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Tags:")
                .and(predicate::str::contains("Rating:")),
        );
}

#[test]
fn refresh_media_dry_run_does_not_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    let xmp_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="3"/>
 </rdf:RDF>
</x:xmpmeta>"#;

    let jpeg_data = build_test_jpeg_with_xmp(xmp_xml);
    std::fs::write(photos.join("drytest.jpg"), &jpeg_data).unwrap();

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Clear rating
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success();

    // Dry run — should report but not apply
    maki()
        .current_dir(&root)
        .args(["refresh", "--media", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"))
        .stderr(predicate::str::contains("Dry run"));

    // Verify rating is still cleared (not restored)
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rating: 3").not());
}

#[test]
fn refresh_without_media_ignores_jpeg() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    let xmp_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="5">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>mountain</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    let jpeg_data = build_test_jpeg_with_xmp(xmp_xml);
    std::fs::write(photos.join("mountain.jpg"), &jpeg_data).unwrap();

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Clear metadata
    maki()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["tag", &asset_id, "--remove", "mountain"])
        .assert()
        .success();

    // Regular refresh (no --media) — no recipes, nothing to check
    maki()
        .current_dir(&root)
        .args(["refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to check"));

    // Verify metadata is NOT restored (no Tags: or Rating: lines)
    maki()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Tags:").not()
                .and(predicate::str::contains("Rating:").not()),
        );
}

// ── fix-recipes tests ─────────────────────────────────────────────

#[test]
fn fix_recipes_reattaches_xmp() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    // Create an XMP file and import it first (becomes standalone since no media yet)
    let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>landscape</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    create_test_file(&photos, "DSC_001.xmp", xmp.as_bytes());
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_001.xmp").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Now create and import the NRW media file
    create_test_file(&photos, "DSC_001.NRW", b"raw-nrw-fix-recipes-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_001.NRW").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Verify we have 2 assets (standalone XMP + NRW)
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    2"));

    // Run fix-recipes --apply
    maki()
        .current_dir(&root)
        .args(["fix-recipes", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 reattached"));

    // Should be down to 1 asset now
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    1"));

    // Get the NRW asset's ID via search --format ids
    let output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "type:image"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let asset_id = stdout.trim();
    assert!(!asset_id.is_empty(), "should have the NRW image asset");

    // The remaining asset should have the recipe attached and XMP metadata applied
    maki()
        .current_dir(&root)
        .args(["show", asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Recipe")
                .and(predicate::str::contains("Rating:"))
                .and(predicate::str::contains("landscape")),
        );
}

#[test]
fn fix_recipes_dry_run() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    create_test_file(&photos, "DSC_002.xmp", b"xmp-dry-run-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_002.xmp").to_str().unwrap()])
        .assert()
        .success();

    create_test_file(&photos, "DSC_002.NRW", b"nrw-dry-run-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_002.NRW").to_str().unwrap()])
        .assert()
        .success();

    // Dry run (no --apply) — reports what would happen
    maki()
        .current_dir(&root)
        .args(["fix-recipes"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 reattached")
                .and(predicate::str::contains("--apply")),
        );

    // Still 2 assets — nothing changed
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    2"));
}

#[test]
fn fix_recipes_compound_extension() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    // Create DSC_003.NRW.xmp (compound extension) and import as standalone
    create_test_file(&photos, "DSC_003.NRW.xmp", b"xmp-compound-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_003.NRW.xmp").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Import the NRW media file
    create_test_file(&photos, "DSC_003.NRW", b"nrw-compound-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_003.NRW").to_str().unwrap()])
        .assert()
        .success();

    // fix-recipes should match via compound stem stripping
    maki()
        .current_dir(&root)
        .args(["fix-recipes", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 reattached"));

    // Only 1 asset remains
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    1"));
}

#[test]
fn fix_recipes_no_parent() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    // Import an XMP with no matching media file
    create_test_file(&photos, "ORPHAN.xmp", b"xmp-orphan-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("ORPHAN.xmp").to_str().unwrap()])
        .assert()
        .success();

    // fix-recipes reports no parent found
    maki()
        .current_dir(&root)
        .args(["fix-recipes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 no parent found"));
}

#[test]
fn fix_recipes_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();

    create_test_file(&photos, "DSC_004.xmp", b"xmp-json-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_004.xmp").to_str().unwrap()])
        .assert()
        .success();

    create_test_file(&photos, "DSC_004.NRW", b"nrw-json-test");
    maki()
        .current_dir(&root)
        .args(["import", photos.join("DSC_004.NRW").to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "fix-recipes"])
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(parsed["checked"].as_u64(), Some(1));
    assert_eq!(parsed["reattached"].as_u64(), Some(1));
    assert_eq!(parsed["no_parent"].as_u64(), Some(0));
    assert_eq!(parsed["skipped"].as_u64(), Some(0));
    assert_eq!(parsed["dry_run"].as_bool(), Some(true));
}

// ─── Duplicates flag tests ──────────────────────────────────────

#[test]
fn duplicates_same_volume_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Import the same content twice on the same volume (different paths)
    let content = b"same volume dup content";
    let file1 = create_test_file(&root, "copy_a.jpg", content);
    let file2 = create_test_file(&root, "subdir/copy_b.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    // --same-volume should find the duplicate
    maki()
        .current_dir(&root)
        .args(["duplicates", "--same-volume"])
        .assert()
        .success()
        .stdout(predicate::str::contains("same-volume duplicate"));

    // --cross-volume should NOT find anything (both on same volume)
    maki()
        .current_dir(&root)
        .args(["duplicates", "--cross-volume"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No cross-volume copies found"));
}

#[test]
fn duplicates_cross_volume_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Set up a second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Import the same content on two different volumes
    let content = b"cross volume copy content";
    let file1 = create_test_file(&root, "original.jpg", content);
    let file2 = create_test_file(&vol2_path, "backup.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file2.to_str().unwrap()])
        .assert()
        .success();

    // --cross-volume should find the cross-volume copy
    maki()
        .current_dir(&root)
        .args(["duplicates", "--cross-volume"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 volumes"));

    // --same-volume should NOT find it (each volume has only 1 copy)
    maki()
        .current_dir(&root)
        .args(["duplicates", "--same-volume"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No same-volume duplicates found"));
}

#[test]
fn duplicates_volume_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Set up a second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Import same content on both volumes
    let content = b"volume filter dup content";
    let file1 = create_test_file(&root, "vf_orig.jpg", content);
    let file2 = create_test_file(&vol2_path, "vf_backup.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file2.to_str().unwrap()])
        .assert()
        .success();

    // --volume test-vol should show the duplicate
    maki()
        .current_dir(&root)
        .args(["duplicates", "--volume", "test-vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("duplicate locations"));

    // --volume nonexistent should find nothing
    maki()
        .current_dir(&root)
        .args(["duplicates", "--volume", "nonexistent"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No duplicates found"));
}

#[test]
fn duplicates_mutually_exclusive_flags() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["duplicates", "--same-volume", "--cross-volume"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mutually exclusive"));
}

// ─── Copies search filter tests ─────────────────────────────────

#[test]
fn search_copies_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Set up a second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // File A: only on one volume (1 copy)
    let file_a = create_test_file(&root, "single.jpg", b"single copy data");
    maki()
        .current_dir(&root)
        .args(["import", file_a.to_str().unwrap()])
        .assert()
        .success();

    // File B: on both volumes (2 copies)
    let content_b = b"two copy data bytes";
    let file_b1 = create_test_file(&root, "multi_orig.jpg", content_b);
    let file_b2 = create_test_file(&vol2_path, "multi_backup.jpg", content_b);
    maki()
        .current_dir(&root)
        .args(["import", file_b1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file_b2.to_str().unwrap()])
        .assert()
        .success();

    // copies:1 should only return the single-copy asset
    let output = maki()
        .current_dir(&root)
        .args(["--json", "search", "copies:1"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let results: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(results.len(), 1, "copies:1 should return exactly 1 result");
    assert_eq!(results[0]["original_filename"], "single.jpg");

    // copies:2 should only return the two-copy asset
    let output = maki()
        .current_dir(&root)
        .args(["--json", "search", "copies:2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let results: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(results.len(), 1, "copies:2 should return exactly 1 result");
    assert_eq!(results[0]["original_filename"], "multi_orig.jpg");

    // copies:2+ should return the two-copy asset
    let output = maki()
        .current_dir(&root)
        .args(["--json", "search", "copies:2+"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let results: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(results.len(), 1, "copies:2+ should return exactly 1 result");
    assert_eq!(results[0]["original_filename"], "multi_orig.jpg");

    // copies:1+ should return both
    let output = maki()
        .current_dir(&root)
        .args(["--json", "search", "copies:1+"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let results: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(results.len(), 2, "copies:1+ should return 2 results");
    let filenames: Vec<&str> = results.iter()
        .map(|r| r["original_filename"].as_str().unwrap())
        .collect();
    assert!(filenames.contains(&"single.jpg"));
    assert!(filenames.contains(&"multi_orig.jpg"));
}

// ── maki dedup ──────────────────────────────────────────────────────

#[test]
fn dedup_report_mode() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let content = b"dedup report content";
    let file1 = create_test_file(&root, "original.jpg", content);
    let file2 = create_test_file(&root, "subdir/copy.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    // Report mode (no --apply)
    maki()
        .current_dir(&root)
        .args(["dedup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 duplicate groups"))
        .stdout(predicate::str::contains("1 redundant locations"))
        .stdout(predicate::str::contains("reclaimable"))
        .stdout(predicate::str::contains("Run with --apply"));

    // Both files should still exist
    assert!(file1.exists());
    assert!(file2.exists());
}

#[test]
fn dedup_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let content = b"dedup apply content";
    let file1 = create_test_file(&root, "keep.jpg", content);
    let file2 = create_test_file(&root, "subdir/remove.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["dedup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 duplicate groups"))
        .stdout(predicate::str::contains("1 locations removed"))
        .stdout(predicate::str::contains("1 files deleted"));

    // One file should remain, one should be gone
    let remaining = file1.exists() as usize + file2.exists() as usize;
    assert_eq!(remaining, 1, "exactly one copy should remain");

    // Duplicates should now be empty
    maki()
        .current_dir(&root)
        .args(["duplicates", "--same-volume"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No same-volume duplicates found"));
}

#[test]
fn dedup_prefer_flag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let content = b"dedup prefer content";
    let file1 = create_test_file(&root, "Originals/photo.jpg", content);
    let file2 = create_test_file(&root, "Selects/photo.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    // Prefer Selects path — so the Selects copy should be kept
    maki()
        .current_dir(&root)
        .args(["dedup", "--prefer", "Selects", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 files deleted"));

    // Selects copy should remain, Originals should be deleted
    assert!(file2.exists(), "preferred location should be kept");
    assert!(!file1.exists(), "non-preferred location should be removed");
}

#[test]
fn dedup_min_copies() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let content = b"dedup min copies content";
    let file1 = create_test_file(&root, "copy_a.jpg", content);
    let file2 = create_test_file(&root, "copy_b.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    // With min-copies=2, nothing should be removed (only 2 locations total)
    maki()
        .current_dir(&root)
        .args(["dedup", "--min-copies", "2", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 files deleted"));

    // Both files should still exist
    assert!(file1.exists());
    assert!(file2.exists());
}

#[test]
fn dedup_volume_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Set up a second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Create same-volume dups on vol2 only
    let content = b"dedup volume filter content";
    let file_v2a = create_test_file(&vol2_path, "a.jpg", content);
    let file_v2b = create_test_file(&vol2_path, "subdir/b.jpg", content);

    maki()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file_v2a.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file_v2b.to_str().unwrap()])
        .assert()
        .success();

    // Dedup only test-vol — should find nothing (dups are on vol2)
    maki()
        .current_dir(&root)
        .args(["dedup", "--volume", "test-vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 duplicate groups"));

    // Dedup vol2 — should find the duplicates
    maki()
        .current_dir(&root)
        .args(["dedup", "--volume", "vol2", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 duplicate groups"))
        .stdout(predicate::str::contains("1 files deleted"));
}

// ==========================================================================
// backup-status
// ==========================================================================

#[test]
fn backup_status_overview() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let file1 = create_test_file(&root, "photo1.jpg", b"backup status photo 1");
    let file2 = create_test_file(&root, "photo2.jpg", b"backup status photo 2");

    maki().current_dir(&root).args(["import", file1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", file2.to_str().unwrap()]).assert().success();

    maki()
        .current_dir(&root)
        .args(["backup-status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total assets:"))
        .stdout(predicate::str::contains("1 volume only:"))
        .stdout(predicate::str::contains("AT RISK"));
}

#[test]
fn backup_status_at_risk() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let file1 = create_test_file(&root, "risk1.jpg", b"at risk content 1");
    let file2 = create_test_file(&root, "risk2.jpg", b"at risk content 2");

    maki().current_dir(&root).args(["import", file1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", file2.to_str().unwrap()]).assert().success();

    // --at-risk -q should output 2 asset IDs (one per line)
    let output = maki()
        .current_dir(&root)
        .args(["backup-status", "--at-risk", "-q"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let lines: Vec<&str> = std::str::from_utf8(&output)
        .unwrap()
        .trim()
        .lines()
        .collect();
    assert_eq!(lines.len(), 2, "expected 2 at-risk asset IDs, got: {:?}", lines);
    // Each line should look like a UUID
    for line in &lines {
        assert!(line.len() >= 36, "expected UUID, got: {}", line);
    }
}

#[test]
fn backup_status_min_copies() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Set up a second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Import same file on both volumes (2 distinct volumes)
    let content = b"min copies content";
    let file1 = create_test_file(&root, "orig.jpg", content);
    let file2 = create_test_file(&vol2_path, "copy.jpg", content);

    maki().current_dir(&root).args(["import", file1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", "--volume", "vol2", file2.to_str().unwrap()]).assert().success();

    // With --min-copies 1, the asset is on 2 volumes so it's not at risk
    maki()
        .current_dir(&root)
        .args(["backup-status", "--min-copies", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No at-risk assets"));

    // With --min-copies 3, the asset is on only 2 volumes so it IS at risk
    maki()
        .current_dir(&root)
        .args(["backup-status", "--min-copies", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains("AT RISK").or(predicate::str::contains("at-risk")));
}

#[test]
fn backup_status_with_query() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let img = create_test_file(&root, "scene.jpg", b"backup query image");
    let aud = create_test_file(&root, "track.mp3", b"backup query audio");

    maki().current_dir(&root).args(["import", "--include", "audio", img.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", "--include", "audio", aud.to_str().unwrap()]).assert().success();

    // Scope to images only — should show 1 total asset
    maki()
        .current_dir(&root)
        .args(["backup-status", "type:image"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total assets:").and(predicate::str::contains("1")));
}

#[test]
fn backup_status_volume_gap() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Second volume
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Import file on test-vol only
    let file1 = create_test_file(&root, "gap_test.jpg", b"volume gap content");
    maki().current_dir(&root).args(["import", file1.to_str().unwrap()]).assert().success();

    // --volume vol2 --at-risk -q should list the asset (missing from vol2)
    let output = maki()
        .current_dir(&root)
        .args(["backup-status", "--volume", "vol2", "--at-risk", "-q"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let lines: Vec<&str> = std::str::from_utf8(&output)
        .unwrap()
        .trim()
        .lines()
        .collect();
    assert_eq!(lines.len(), 1, "expected 1 asset missing from vol2, got: {:?}", lines);
}

#[test]
fn backup_status_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let file1 = create_test_file(&root, "json_test.jpg", b"backup json content");
    maki().current_dir(&root).args(["import", file1.to_str().unwrap()]).assert().success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "backup-status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON output");
    assert!(json.get("total_assets").is_some());
    assert!(json.get("at_risk_count").is_some());
    assert!(json.get("location_distribution").is_some());
    assert!(json.get("purpose_coverage").is_some());
    assert!(json.get("volume_gaps").is_some());
    assert!(json.get("min_copies").is_some());
    // Verify distribution uses volume_count not location_count
    let dist = json["location_distribution"].as_array().unwrap();
    assert!(dist[0].get("volume_count").is_some());
}

#[test]
fn backup_status_purpose_coverage() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    // Re-register volume with a purpose
    // First we need to add a volume with --purpose
    let dir2 = tempdir().unwrap();
    let vol2_path = dir2.path().canonicalize().unwrap();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "working-vol", vol2_path.to_str().unwrap(), "--purpose", "working"])
        .assert()
        .success();

    let file1 = create_test_file(&vol2_path, "purpose_test.jpg", b"purpose coverage content");
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "working-vol", file1.to_str().unwrap()])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["backup-status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Coverage by volume purpose:"))
        .stdout(predicate::str::contains("Working"));
}

// ── Volume combine ──────────────────────────────────────────────────

#[test]
fn volume_combine_report_only() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    // Init catalog at root
    maki().current_dir(&root).arg("init").assert().success();

    // Create parent volume at root, child volume at root/sub
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    maki()
        .current_dir(&root)
        .args(["volume", "add", "parent-vol", root.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "child-vol", sub.to_str().unwrap()])
        .assert()
        .success();

    // Import a file on the child volume
    let file = create_test_file(&sub, "photo.jpg", b"combine-report-content");
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "child-vol", file.to_str().unwrap()])
        .assert()
        .success();

    // Report-only (no --apply)
    maki()
        .current_dir(&root)
        .args(["volume", "combine", "child-vol", "parent-vol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would combine"))
        .stdout(predicate::str::contains("1 locations"))
        .stdout(predicate::str::contains("prefix 'sub/'"));

    // Volume should still exist
    maki()
        .current_dir(&root)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("child-vol"));

    // Asset should still exist
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    1"));
}

#[test]
fn volume_combine_apply() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    maki().current_dir(&root).arg("init").assert().success();

    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    maki()
        .current_dir(&root)
        .args(["volume", "add", "parent-vol", root.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "child-vol", sub.to_str().unwrap()])
        .assert()
        .success();

    let file = create_test_file(&sub, "photo.jpg", b"combine-apply-content");
    maki()
        .current_dir(&root)
        .args(["import", "--volume", "child-vol", file.to_str().unwrap()])
        .assert()
        .success();

    // Apply combine
    maki()
        .current_dir(&root)
        .args(["volume", "combine", "child-vol", "parent-vol", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("combined into"))
        .stdout(predicate::str::contains("1 locations moved"));

    // Child volume should be gone
    maki()
        .current_dir(&root)
        .args(["volume", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("child-vol").not());

    // Asset should still exist
    maki()
        .current_dir(&root)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Assets:    1"));

    // Verify path was rewritten: search for asset, then show it
    let output = maki()
        .current_dir(&root)
        .args(["search", "photo"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    maki()
        .current_dir(&root)
        .args(["show", "--json", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("sub/photo.jpg"));
}

#[test]
fn volume_combine_with_recipes() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    maki().current_dir(&root).arg("init").assert().success();

    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    maki()
        .current_dir(&root)
        .args(["volume", "add", "parent-vol", root.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "child-vol", sub.to_str().unwrap()])
        .assert()
        .success();

    // Create a jpg and an xmp recipe
    let _jpg = create_test_file(&sub, "photo.jpg", b"combine-recipe-jpg");
    let _xmp = create_test_file(
        &sub,
        "photo.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"3\"/></rdf:RDF></x:xmpmeta>",
    );

    maki()
        .current_dir(&root)
        .args([
            "import",
            "--volume",
            "child-vol",
            "--include",
            "xmp",
            sub.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Combine with --apply
    maki()
        .current_dir(&root)
        .args(["volume", "combine", "child-vol", "parent-vol", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 recipes moved"));

    // Verify paths were rewritten
    let output = maki()
        .current_dir(&root)
        .args(["search", "photo"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    let show_output = maki()
        .current_dir(&root)
        .args(["show", "--json", short_id])
        .output()
        .unwrap();
    let show_stdout = String::from_utf8_lossy(&show_output.stdout);
    assert!(show_stdout.contains("sub/photo.jpg"), "variant path should be rewritten");
    assert!(show_stdout.contains("sub/photo.xmp"), "recipe path should be rewritten");
}

#[test]
fn volume_combine_same_volume_error() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["volume", "combine", "test-vol", "test-vol"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("same volume"));
}

#[test]
fn volume_combine_not_subdirectory_error() {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();

    maki().current_dir(&root).arg("init").assert().success();

    // Create two sibling directories as separate volumes
    let vol_a = root.join("vol_a");
    let vol_b = root.join("vol_b");
    std::fs::create_dir_all(&vol_a).unwrap();
    std::fs::create_dir_all(&vol_b).unwrap();

    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol-a", vol_a.to_str().unwrap()])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args(["volume", "add", "vol-b", vol_b.to_str().unwrap()])
        .assert()
        .success();

    // Neither is a subdirectory of the other
    maki()
        .current_dir(&root)
        .args(["volume", "combine", "vol-a", "vol-b"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a subdirectory"));
}

// ── Hierarchical tag tests ──────────────────────────────

#[test]
fn tag_hierarchy_search() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "eagle.jpg", b"eagle data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "eagle"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().unwrap();

    // Add hierarchical tag
    maki()
        .current_dir(&root)
        .args(["tag", short_id, "animals/birds/eagles"])
        .assert()
        .success();

    // Search for parent tag should match
    maki()
        .current_dir(&root)
        .args(["search", "tag:animals"])
        .assert()
        .success()
        .stdout(predicate::str::contains("eagle"));

    // Search for intermediate tag should match
    maki()
        .current_dir(&root)
        .args(["search", "tag:animals/birds"])
        .assert()
        .success()
        .stdout(predicate::str::contains("eagle"));

    // Search for exact tag should match
    maki()
        .current_dir(&root)
        .args(["search", "tag:animals/birds/eagles"])
        .assert()
        .success()
        .stdout(predicate::str::contains("eagle"));

    // Search for unrelated tag should not match
    maki()
        .current_dir(&root)
        .args(["search", "tag:cats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results"));
}

#[test]
fn tag_hierarchy_add_remove() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "hawk.jpg", b"hawk data");

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "hawk"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().unwrap();

    // Add hierarchical tag
    maki()
        .current_dir(&root)
        .args(["tag", short_id, "animals/birds/hawks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("animals/birds/hawks"));

    // Verify in show
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("animals/birds/hawks"));

    // Remove hierarchical tag
    maki()
        .current_dir(&root)
        .args(["tag", short_id, "--remove", "animals/birds/hawks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed tags:"));

    // Verify removal
    maki()
        .current_dir(&root)
        .args(["show", short_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("animals/birds/hawks").not());
}

#[test]
fn import_xmp_hierarchical_subject() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_200.nef", b"raw image for hier test");

    let xmp = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/"
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmp:Rating="4">
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
    </rdf:Bag>
   </lr:hierarchicalSubject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;
    create_test_file(&photos, "DSC_200.xmp", xmp.as_bytes());

    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "DSC_200"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Verify: flat components (animals, birds, eagles) should be deduplicated
    // into the hierarchical tag, while "sunset" remains
    let show_output = maki()
        .current_dir(&root)
        .args(["show", short_id])
        .output()
        .unwrap();
    let show_stdout = String::from_utf8_lossy(&show_output.stdout);

    assert!(
        show_stdout.contains("animals/birds/eagles"),
        "should contain hierarchical tag: {show_stdout}"
    );
    assert!(
        show_stdout.contains("sunset"),
        "should contain non-component flat tag: {show_stdout}"
    );

    // Hierarchical search should work
    maki()
        .current_dir(&root)
        .args(["search", "tag:animals"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result"));
}

// ====================== Stack tests ======================

/// Helper: import files and return their asset IDs.
fn import_and_get_ids(root: &Path, names: &[&str]) -> Vec<String> {
    for name in names {
        create_test_file(root, name, name.as_bytes());
        maki()
            .current_dir(root)
            .args(["import", root.join(name).to_str().unwrap()])
            .assert()
            .success();
    }
    let mut ids = Vec::new();
    for name in names {
        let stem = Path::new(name).file_stem().unwrap().to_str().unwrap();
        let output = maki()
            .current_dir(root)
            .args(["search", "--format", "ids", stem])
            .output()
            .unwrap();
        let id = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .to_string();
        assert!(!id.is_empty(), "asset not found for {name}");
        ids.push(id);
    }
    ids
}

#[test]
fn stack_create_and_list() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["stack_a.jpg", "stack_b.jpg", "stack_c.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1], &ids[2]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created stack"))
        .stdout(predicate::str::contains("3 assets"));

    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3 assets"))
        .stdout(predicate::str::contains("pick:"));
}

#[test]
fn stack_alias_works() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["st_a.jpg", "st_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["st", "create", &ids[0], &ids[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created stack"));
}

#[test]
fn stack_show_members() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["show_a.jpg", "show_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["stack", "show", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains(&ids[0]))
        .stdout(predicate::str::contains(&ids[1]))
        .stdout(predicate::str::contains("[pick]"));
}

#[test]
fn stack_set_pick() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["pick_a.jpg", "pick_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    // Initially ids[0] is the pick
    maki()
        .current_dir(&root)
        .args(["stack", "show", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("{} [pick]", &ids[0])));

    // Change pick to ids[1]
    maki()
        .current_dir(&root)
        .args(["stack", "pick", &ids[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains("as stack pick"));

    // Verify ids[1] is now pick
    maki()
        .current_dir(&root)
        .args(["stack", "show", &ids[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("{} [pick]", &ids[1])));
}

#[test]
fn stack_remove_members() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["rm_a.jpg", "rm_b.jpg", "rm_c.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1], &ids[2]])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["stack", "remove", &ids[2]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 asset"));

    // Stack should now have 2 members
    maki()
        .current_dir(&root)
        .args(["stack", "show", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains(&ids[0]))
        .stdout(predicate::str::contains(&ids[1]));
}

#[test]
fn stack_dissolve() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["diss_a.jpg", "diss_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["stack", "dissolve", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stack dissolved"));

    // No stacks should remain
    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No stacks"));
}

#[test]
fn stack_add_to_existing() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["add_a.jpg", "add_b.jpg", "add_c.jpg"]);

    // Create stack with first two
    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    // Add third to existing stack (reference: ids[0])
    maki()
        .current_dir(&root)
        .args(["stack", "add", &ids[0], &ids[1], &ids[2]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added"));

    // Show should include all three
    maki()
        .current_dir(&root)
        .args(["stack", "show", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains(&ids[2]));
}

#[test]
fn stack_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["json_a.jpg", "json_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", "--json", &ids[0], &ids[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains("member_count"))
        .stdout(predicate::str::contains("2"));

    maki()
        .current_dir(&root)
        .args(["stack", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("member_count"))
        .stdout(predicate::str::contains("2"));
}

#[test]
fn stack_search_stacked_filter() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["sf_a.jpg", "sf_b.jpg", "sf_solo.jpg"]);

    // Stack first two, leave third solo
    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    // stacked:true should find 2 assets
    maki()
        .current_dir(&root)
        .args(["search", "stacked:true"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result"));

    // stacked:false should find 1 asset
    maki()
        .current_dir(&root)
        .args(["search", "stacked:false"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 result"));
}

#[test]
fn stack_rebuild_catalog_preserves_stacks() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["rb_a.jpg", "rb_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    // Rebuild catalog
    maki()
        .current_dir(&root)
        .args(["rebuild-catalog"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stack"));

    // Stacks should survive
    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 assets"));
}

#[test]
fn stack_remove_dissolves_when_one_left() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["auto_a.jpg", "auto_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    // Remove one member — stack should auto-dissolve since only 1 would remain
    maki()
        .current_dir(&root)
        .args(["stack", "remove", &ids[1]])
        .assert()
        .success();

    // No stacks should remain
    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No stacks"));
}

// -- stack from-tag tests --

#[test]
fn stack_from_tag_dry_run() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["ft_a.jpg", "ft_b.jpg", "ft_c.jpg", "ft_d.jpg", "ft_e.jpg"]);

    // Tag 3 assets with "Group A"
    for id in &ids[0..3] {
        maki().current_dir(&root).args(["tag", id, "Group A"]).assert().success();
    }
    // Tag 2 assets with "Group B"
    for id in &ids[3..5] {
        maki().current_dir(&root).args(["tag", id, "Group B"]).assert().success();
    }

    // Dry run
    maki()
        .current_dir(&root)
        .args(["stack", "from-tag", "Group {}"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tags matched: 2"))
        .stdout(predicate::str::contains("Stacks created: 2"))
        .stdout(predicate::str::contains("dry run"));

    // Verify no stacks were actually created
    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No stacks"));
}

#[test]
fn stack_from_tag_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["fta_a.jpg", "fta_b.jpg", "fta_c.jpg"]);

    for id in &ids {
        maki().current_dir(&root).args(["tag", id, "MyGroup X"]).assert().success();
    }

    maki()
        .current_dir(&root)
        .args(["stack", "from-tag", "MyGroup {}", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Stacks created: 1"))
        .stdout(predicate::str::contains("Assets stacked: 3"));

    // Verify stack was created
    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3 assets"));
}

#[test]
fn stack_from_tag_remove_tags() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["ftr_a.jpg", "ftr_b.jpg"]);

    for id in &ids {
        maki().current_dir(&root).args(["tag", id, "Stack 99"]).assert().success();
        maki().current_dir(&root).args(["tag", id, "keeper"]).assert().success();
    }

    maki()
        .current_dir(&root)
        .args(["stack", "from-tag", "Stack {}", "--apply", "--remove-tags"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tags removed: 2"));

    // Verify "Stack 99" tag is gone but "keeper" remains
    maki()
        .current_dir(&root)
        .args(["show", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains("keeper"))
        .stdout(predicate::str::contains("Stack 99").not());
}

#[test]
fn stack_from_tag_skips_already_stacked() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["fts_a.jpg", "fts_b.jpg", "fts_c.jpg"]);

    // Tag all three with same tag FIRST (tagging does insert_asset which resets stack_id)
    for id in &ids {
        maki().current_dir(&root).args(["tag", id, "Overlap X"]).assert().success();
    }

    // Then stack first two
    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1]])
        .assert()
        .success();

    // Only 1 unstacked asset — too few to create a stack
    maki()
        .current_dir(&root)
        .args(["stack", "from-tag", "Overlap {}", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tags skipped: 1"))
        .stdout(predicate::str::contains("Stacks created: 0"));
}

#[test]
fn stack_from_tag_single_asset_skipped() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["fts1_a.jpg"]);

    maki().current_dir(&root).args(["tag", &ids[0], "Solo 1"]).assert().success();

    maki()
        .current_dir(&root)
        .args(["stack", "from-tag", "Solo {}", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Tags skipped: 1"))
        .stdout(predicate::str::contains("Stacks created: 0"));
}

#[test]
fn stack_from_tag_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let ids = import_and_get_ids(&root, &["ftj_a.jpg", "ftj_b.jpg"]);

    for id in &ids {
        maki().current_dir(&root).args(["tag", id, "Batch 42"]).assert().success();
    }

    let output = maki()
        .current_dir(&root)
        .args(["--json", "stack", "from-tag", "Batch {}", "--apply"])
        .output()
        .unwrap();

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON output");
    assert_eq!(json["tags_matched"], 1);
    assert_eq!(json["stacks_created"], 1);
    assert_eq!(json["assets_stacked"], 2);
    assert_eq!(json["dry_run"], false);
    assert!(json["details"][0]["stack_id"].is_string());
}

#[test]
fn stack_from_tag_no_wildcard_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["stack", "from-tag", "no wildcard"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("{}"));
}

// ── negation and OR search tests ─────────────────────────────────

#[test]
fn search_negated_tag_excludes_matching() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let f1 = create_test_file(&root, "good.jpg", b"good image data");
    let f2 = create_test_file(&root, "bad.jpg", b"bad image data");

    maki().current_dir(&root).args(["import", f1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f2.to_str().unwrap()]).assert().success();

    // Tag f2 as "rejected"
    let output = maki().current_dir(&root).args(["search", "-q", "bad"]).output().unwrap();
    let bad_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    maki().current_dir(&root).args(["tag", &bad_id, "rejected"]).assert().success();

    // Tag f1 as "keeper"
    let output = maki().current_dir(&root).args(["search", "-q", "good"]).output().unwrap();
    let good_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    maki().current_dir(&root).args(["tag", &good_id, "keeper"]).assert().success();

    // -tag:rejected should find good but not bad
    // Use `--` to separate flags from the query containing `-`
    let output = maki().current_dir(&root).args(["search", "-q", "--", "-tag:rejected"]).output().unwrap();
    let ids = String::from_utf8_lossy(&output.stdout);
    assert!(ids.contains(&good_id), "should include non-rejected asset");
    assert!(!ids.contains(&bad_id), "should exclude rejected asset");
}

#[test]
fn search_negated_format_excludes_matching() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let f1 = create_test_file(&root, "photo.jpg", b"jpeg data here");
    let f2 = create_test_file(&root, "raw.arw", b"raw data here");

    maki().current_dir(&root).args(["import", f1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f2.to_str().unwrap()]).assert().success();

    // -format:arw should exclude the raw file
    let output = maki().current_dir(&root).args(["search", "--json", "--", "-format:arw"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("photo.jpg"), "should include jpg");
    assert!(!stdout.contains("raw.arw"), "should exclude arw");
}

#[test]
fn search_comma_or_format() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let f1 = create_test_file(&root, "a.jpg", b"jpg data");
    let f2 = create_test_file(&root, "b.png", b"png data");
    let f3 = create_test_file(&root, "c.tif", b"tif data");

    maki().current_dir(&root).args(["import", f1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f2.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f3.to_str().unwrap()]).assert().success();

    // format:jpg,png should find both jpg and png but not tif
    let output = maki().current_dir(&root).args(["search", "--json", "format:jpg,png"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("a.jpg"), "should include jpg");
    assert!(stdout.contains("b.png"), "should include png");
    assert!(!stdout.contains("c.tif"), "should exclude tif");
}

#[test]
fn search_repeated_tags_and() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let f1 = create_test_file(&root, "both.jpg", b"both tags data");
    let f2 = create_test_file(&root, "onlya.jpg", b"only a tag data");

    maki().current_dir(&root).args(["import", f1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f2.to_str().unwrap()]).assert().success();

    let output = maki().current_dir(&root).args(["search", "-q", "both"]).output().unwrap();
    let both_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let output = maki().current_dir(&root).args(["search", "-q", "onlya"]).output().unwrap();
    let onlya_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Tag both assets with "landscape"
    maki().current_dir(&root).args(["tag", &both_id, "landscape"]).assert().success();
    maki().current_dir(&root).args(["tag", &onlya_id, "landscape"]).assert().success();

    // Tag only the first with "sunset"
    maki().current_dir(&root).args(["tag", &both_id, "sunset"]).assert().success();

    // tag:landscape tag:sunset should only match the asset with both tags
    let output = maki().current_dir(&root).args(["search", "-q", "tag:landscape tag:sunset"]).output().unwrap();
    let ids = String::from_utf8_lossy(&output.stdout);
    assert!(ids.contains(&both_id), "should include asset with both tags");
    assert!(!ids.contains(&onlya_id), "should exclude asset with only one tag");
}

#[test]
fn search_comma_or_tag() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let f1 = create_test_file(&root, "alice.jpg", b"alice data");
    let f2 = create_test_file(&root, "bob.jpg", b"bob data");
    let f3 = create_test_file(&root, "carol.jpg", b"carol data");

    maki().current_dir(&root).args(["import", f1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f2.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f3.to_str().unwrap()]).assert().success();

    let output = maki().current_dir(&root).args(["search", "-q", "alice"]).output().unwrap();
    let alice_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let output = maki().current_dir(&root).args(["search", "-q", "bob"]).output().unwrap();
    let bob_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let output = maki().current_dir(&root).args(["search", "-q", "carol"]).output().unwrap();
    let carol_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    maki().current_dir(&root).args(["tag", &alice_id, "tagA"]).assert().success();
    maki().current_dir(&root).args(["tag", &bob_id, "tagB"]).assert().success();
    maki().current_dir(&root).args(["tag", &carol_id, "tagC"]).assert().success();

    // tag:tagA,tagB should find alice and bob but not carol
    let output = maki().current_dir(&root).args(["search", "-q", "tag:tagA,tagB"]).output().unwrap();
    let ids = String::from_utf8_lossy(&output.stdout);
    assert!(ids.contains(&alice_id), "should include alice (tagA)");
    assert!(ids.contains(&bob_id), "should include bob (tagB)");
    assert!(!ids.contains(&carol_id), "should exclude carol (tagC)");
}

#[test]
fn search_negated_text_excludes_matching() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let f1 = create_test_file(&root, "sunset_beach.jpg", b"beach data");
    let f2 = create_test_file(&root, "sunset_mountain.jpg", b"mountain data");

    maki().current_dir(&root).args(["import", f1.to_str().unwrap()]).assert().success();
    maki().current_dir(&root).args(["import", f2.to_str().unwrap()]).assert().success();

    // "sunset -mountain" should find beach but not mountain
    let output = maki().current_dir(&root).args(["search", "--json", "--", "sunset -mountain"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sunset_beach"), "should include beach");
    assert!(!stdout.contains("sunset_mountain"), "should exclude mountain");
}

// ── Verify data-flow tests ──────────────────────────────────────────

#[test]
fn verify_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "check.jpg", b"verify json test data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "verify"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert!(json["verified"].as_u64().unwrap() > 0, "should verify at least one file");
}

#[test]
fn verify_max_age_skips_recent() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "recent.jpg", b"max-age skip test data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // First verify sets timestamps
    maki()
        .current_dir(&root)
        .arg("verify")
        .assert()
        .success();

    // Second verify with --max-age should skip recently verified
    let output = maki()
        .current_dir(&root)
        .args(["--json", "verify", "--max-age", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert!(json["skipped_recent"].as_u64().unwrap() > 0, "should skip recently verified");
    assert_eq!(json["verified"].as_u64().unwrap(), 0, "nothing should need re-verifying");
}

#[test]
fn verify_force_overrides_max_age() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "force.jpg", b"force override test data");

    maki()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // First verify sets timestamps
    maki()
        .current_dir(&root)
        .arg("verify")
        .assert()
        .success();

    // --force should override --max-age and re-verify everything
    let output = maki()
        .current_dir(&root)
        .args(["--json", "verify", "--force", "--max-age", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert!(json["verified"].as_u64().unwrap() > 0, "should re-verify with --force");
    assert_eq!(json["skipped_recent"].as_u64().unwrap(), 0, "nothing should be skipped with --force");
}

#[test]
fn verify_recipe_verified_at_round_trip() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let photos = root.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    create_test_file(&photos, "DSC_500.nef", b"raw image for recipe verify round trip");
    create_test_file(&photos, "DSC_500.xmp", b"xmp recipe for verify round trip");

    // Import NEF + XMP (recipe attached)
    maki()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // First verify — sets verified_at on both variant location and recipe location
    maki()
        .current_dir(&root)
        .arg("verify")
        .assert()
        .success();

    // Second verify with --max-age — both should be skipped as recently verified
    let output = maki()
        .current_dir(&root)
        .args(["--json", "verify", "--max-age", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert_eq!(
        json["skipped_recent"].as_u64().unwrap(),
        2,
        "both variant location and recipe location should be skipped"
    );
    assert_eq!(json["verified"].as_u64().unwrap(), 0, "nothing should need re-verifying");
}

// ── delete command ──────────────────────────────────────────────────

#[test]
fn delete_report_only_by_default() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_report.jpg"]);

    maki()
        .current_dir(&root)
        .args(["delete", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains("would be deleted"));

    // Asset should still exist
    maki()
        .current_dir(&root)
        .args(["show", &ids[0]])
        .assert()
        .success();
}

#[test]
fn delete_apply_removes_asset() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_apply.jpg"]);

    maki()
        .current_dir(&root)
        .args(["delete", "--apply", &ids[0]])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 deleted"));

    // Asset should be gone
    maki()
        .current_dir(&root)
        .args(["show", &ids[0]])
        .assert()
        .failure();
}

#[test]
fn delete_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_json.jpg"]);

    let output = maki()
        .current_dir(&root)
        .args(["--json", "delete", "--apply", &ids[0]])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert_eq!(json["deleted"].as_u64().unwrap(), 1);
    assert_eq!(json["dry_run"].as_bool().unwrap(), false);
    assert!(json["not_found"].as_array().unwrap().is_empty());
}

#[test]
fn delete_prefix_matching() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_prefix.jpg"]);

    // Use first 8 chars as prefix
    let prefix = &ids[0][..8];

    maki()
        .current_dir(&root)
        .args(["delete", "--apply", prefix])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 deleted"));

    maki()
        .current_dir(&root)
        .args(["show", &ids[0]])
        .assert()
        .failure();
}

#[test]
fn delete_not_found() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["--json", "delete", "--apply", "nonexistent-id-12345"])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["--json", "delete", "--apply", "nonexistent-id-12345"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert_eq!(json["deleted"].as_u64().unwrap(), 0);
    assert_eq!(json["not_found"].as_array().unwrap().len(), 1);
}

#[test]
fn delete_multiple_assets() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_multi_a.jpg", "del_multi_b.jpg"]);

    maki()
        .current_dir(&root)
        .args(["delete", "--apply", &ids[0], &ids[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 deleted"));

    // Both should be gone
    for id in &ids {
        maki()
            .current_dir(&root)
            .args(["show", id])
            .assert()
            .failure();
    }
}

#[test]
fn delete_stdin_piping() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_stdin.jpg"]);

    maki()
        .current_dir(&root)
        .args(["delete", "--apply"])
        .write_stdin(format!("{}\n", ids[0]))
        .assert()
        .success()
        .stdout(predicate::str::contains("1 deleted"));

    maki()
        .current_dir(&root)
        .args(["show", &ids[0]])
        .assert()
        .failure();
}

#[test]
fn delete_remove_files() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "del_rm.jpg", b"delete-me-file");
    let file_path = file.clone();

    maki()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["search", "--format", "ids", "del_rm"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8(output.stdout).unwrap().trim().to_string();

    assert!(file_path.exists(), "file should exist before delete");

    maki()
        .current_dir(&root)
        .args(["delete", "--apply", "--remove-files", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 deleted"))
        .stdout(predicate::str::contains("files removed"));

    assert!(!file_path.exists(), "file should be removed from disk");
}

#[test]
fn delete_remove_files_requires_apply() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    maki()
        .current_dir(&root)
        .args(["delete", "--remove-files", "some-id"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--remove-files requires --apply"));
}

#[test]
fn delete_removes_from_collection() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_col.jpg"]);

    // Add to a collection
    maki()
        .current_dir(&root)
        .args(["col", "create", "TestCol"])
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .args(["col", "add", "TestCol", &ids[0]])
        .assert()
        .success();

    // Delete the asset
    maki()
        .current_dir(&root)
        .args(["delete", "--apply", &ids[0]])
        .assert()
        .success();

    // Collection should now be empty
    maki()
        .current_dir(&root)
        .args(["col", "show", "TestCol"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
}

#[test]
fn delete_removes_from_stack() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let ids = import_and_get_ids(&root, &["del_stack_a.jpg", "del_stack_b.jpg", "del_stack_c.jpg"]);

    // Create a stack with all three
    maki()
        .current_dir(&root)
        .args(["stack", "create", &ids[0], &ids[1], &ids[2]])
        .assert()
        .success();

    // Delete one member
    maki()
        .current_dir(&root)
        .args(["delete", "--apply", &ids[1]])
        .assert()
        .success();

    // Stack should still exist with 2 members
    maki()
        .current_dir(&root)
        .args(["stack", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 assets"));
}

// ====================== Export tests ======================

#[test]
fn export_flat_copies_best_variant() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "export_flat.jpg", b"flat export content");
    maki()
        .current_dir(&root)
        .args(["import", root.join("export_flat.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("exported");
    maki()
        .current_dir(&root)
        .args(["export", "export_flat", export_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 files"));

    assert!(export_dir.join("export_flat.jpg").exists());
    assert_eq!(
        std::fs::read(export_dir.join("export_flat.jpg")).unwrap(),
        b"flat export content"
    );
}

#[test]
fn export_mirror_preserves_paths() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "subdir/mirror_test.jpg", b"mirror content");
    maki()
        .current_dir(&root)
        .args(["import", root.join("subdir/mirror_test.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("mirror_out");
    maki()
        .current_dir(&root)
        .args([
            "export",
            "mirror_test",
            export_dir.to_str().unwrap(),
            "--layout",
            "mirror",
        ])
        .assert()
        .success();

    assert!(export_dir.join("subdir/mirror_test.jpg").exists());
}

#[test]
fn export_dry_run_no_files() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "dry_run_export.jpg", b"dry run content");
    maki()
        .current_dir(&root)
        .args(["import", root.join("dry_run_export.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("dry_out");
    maki()
        .current_dir(&root)
        .args([
            "export",
            "dry_run_export",
            export_dir.to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry run"));

    // Directory should not be created in dry-run mode
    assert!(!export_dir.exists());
}

#[test]
fn export_skip_existing_matching_hash() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "skip_test.jpg", b"skip content");
    maki()
        .current_dir(&root)
        .args(["import", root.join("skip_test.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("skip_out");
    // First export
    maki()
        .current_dir(&root)
        .args(["export", "skip_test", export_dir.to_str().unwrap()])
        .assert()
        .success();
    assert!(export_dir.join("skip_test.jpg").exists());

    // Second export — should skip
    maki()
        .current_dir(&root)
        .args(["export", "skip_test", export_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 skipped"));
}

#[test]
fn export_overwrite() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "overwrite_test.jpg", b"overwrite content");
    maki()
        .current_dir(&root)
        .args(["import", root.join("overwrite_test.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("overwrite_out");
    // First export
    maki()
        .current_dir(&root)
        .args(["export", "overwrite_test", export_dir.to_str().unwrap()])
        .assert()
        .success();

    // Second export with --overwrite
    maki()
        .current_dir(&root)
        .args([
            "export",
            "overwrite_test",
            export_dir.to_str().unwrap(),
            "--overwrite",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 files"));
}

#[test]
fn export_include_sidecars() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "sidecar_test.jpg", b"photo with sidecar");
    // Create an XMP sidecar
    create_test_file(
        &root,
        "sidecar_test.xmp",
        b"<?xml version='1.0'?><x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args([
            "import",
            root.join("sidecar_test.jpg").to_str().unwrap(),
            root.join("sidecar_test.xmp").to_str().unwrap(),
        ])
        .assert()
        .success();

    let export_dir = dir.path().join("sidecar_out");
    maki()
        .current_dir(&root)
        .args([
            "export",
            "sidecar_test",
            export_dir.to_str().unwrap(),
            "--include-sidecars",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 sidecars"));

    assert!(export_dir.join("sidecar_test.jpg").exists());
    assert!(export_dir.join("sidecar_test.xmp").exists());
}

#[cfg(unix)]
#[test]
fn export_symlink() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "link_test.jpg", b"symlink content");
    maki()
        .current_dir(&root)
        .args(["import", root.join("link_test.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("link_out");
    maki()
        .current_dir(&root)
        .args([
            "export",
            "link_test",
            export_dir.to_str().unwrap(),
            "--symlink",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 files linked"));

    let target = export_dir.join("link_test.jpg");
    assert!(target.exists());
    assert!(target.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn export_all_variants() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    // Create two files with the same stem (auto-grouped into one asset)
    create_test_file(&root, "multi.jpg", b"jpeg variant");
    create_test_file(&root, "multi.tif", b"tiff variant");
    maki()
        .current_dir(&root)
        .args([
            "import",
            root.join("multi.jpg").to_str().unwrap(),
            root.join("multi.tif").to_str().unwrap(),
        ])
        .assert()
        .success();

    let export_dir = dir.path().join("all_variants_out");
    maki()
        .current_dir(&root)
        .args([
            "export",
            "multi",
            export_dir.to_str().unwrap(),
            "--all-variants",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 files"));

    assert!(export_dir.join("multi.jpg").exists());
    assert!(export_dir.join("multi.tif").exists());
}

#[test]
fn export_best_variant_only() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "best.jpg", b"jpeg best variant");
    create_test_file(&root, "best.tif", b"tiff extra variant data");
    maki()
        .current_dir(&root)
        .args([
            "import",
            root.join("best.jpg").to_str().unwrap(),
            root.join("best.tif").to_str().unwrap(),
        ])
        .assert()
        .success();

    let export_dir = dir.path().join("best_only_out");
    maki()
        .current_dir(&root)
        .args(["export", "best", export_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 files"));

    // Only one file should be exported (best variant)
    let files: Vec<_> = std::fs::read_dir(&export_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    assert_eq!(files.len(), 1);
}

#[test]
fn export_flat_filename_collision() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    // Two files with the same name in different directories
    create_test_file(&root, "subA/collision.jpg", b"content A");
    create_test_file(&root, "subB/collision.jpg", b"content B");
    maki()
        .current_dir(&root)
        .args([
            "import",
            root.join("subA/collision.jpg").to_str().unwrap(),
        ])
        .assert()
        .success();
    maki()
        .current_dir(&root)
        .args([
            "import",
            root.join("subB/collision.jpg").to_str().unwrap(),
        ])
        .assert()
        .success();

    let export_dir = dir.path().join("collision_out");
    maki()
        .current_dir(&root)
        .args(["export", "collision", export_dir.to_str().unwrap()])
        .assert()
        .success();

    // Should have 2 files (one with hash suffix)
    let files: Vec<_> = std::fs::read_dir(&export_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    assert_eq!(files.len(), 2, "should have 2 files, one with hash suffix");
}

#[test]
fn export_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "json_export.jpg", b"json test");
    maki()
        .current_dir(&root)
        .args(["import", root.join("json_export.jpg").to_str().unwrap()])
        .assert()
        .success();

    let export_dir = dir.path().join("json_out");
    let output = maki()
        .current_dir(&root)
        .args([
            "--json",
            "export",
            "json_export",
            export_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output).unwrap()).expect("valid JSON");
    assert_eq!(json["assets_matched"].as_u64().unwrap(), 1);
    assert_eq!(json["files_exported"].as_u64().unwrap(), 1);
    assert_eq!(json["dry_run"].as_bool().unwrap(), false);
}

#[test]
fn export_no_results() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let export_dir = dir.path().join("empty_out");
    maki()
        .current_dir(&root)
        .args([
            "export",
            "nonexistent_query_xyz",
            export_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("No assets matched"));
}

// ===========================================================================
// migrate
// ===========================================================================

#[test]
fn migrate_runs_successfully() {
    let tmp = tempdir().unwrap();
    let root = init_catalog(tmp.path());

    maki()
        .current_dir(&root)
        .arg("migrate")
        .assert()
        .success()
        .stdout(predicate::str::contains("Schema migrations applied successfully"));
}

#[test]
fn migrate_json_output() {
    let tmp = tempdir().unwrap();
    let root = init_catalog(tmp.path());

    let output = maki()
        .current_dir(&root)
        .args(["migrate", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["status"], "ok");
}

#[test]
fn migrate_idempotent() {
    let tmp = tempdir().unwrap();
    let root = init_catalog(tmp.path());

    // Run twice — should succeed both times
    maki()
        .current_dir(&root)
        .arg("migrate")
        .assert()
        .success();

    maki()
        .current_dir(&root)
        .arg("migrate")
        .assert()
        .success();
}

// ===========================================================================
// faces export / embed --export (AI) — only compiled with --features ai
// ===========================================================================

#[cfg(feature = "ai")]
mod export_ai_data {
    use super::*;

    #[test]
    fn faces_export_empty_catalog() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        maki()
            .current_dir(&root)
            .args(["faces", "export"])
            .assert()
            .success();

        // YAML files should be created even if empty
        assert!(root.join("faces.yaml").exists());
        assert!(root.join("people.yaml").exists());
    }

    #[test]
    fn faces_export_json_output() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let output = maki()
            .current_dir(&root)
            .args(["faces", "export", "--json"])
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&output.get_output().stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert!(parsed.get("faces").is_some());
        assert!(parsed.get("people").is_some());
        assert!(parsed.get("arcface_binaries").is_some());
    }

    #[test]
    fn embed_export_empty_catalog() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        maki()
            .current_dir(&root)
            .args(["embed", "--export"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Exported 0 embedding binaries"));
    }

    #[test]
    fn embed_export_json_output() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let output = maki()
            .current_dir(&root)
            .args(["embed", "--export", "--json"])
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&output.get_output().stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(parsed["exported"], 0);
        assert!(parsed["models"].is_array());
    }
}

// ===========================================================================
// auto-tag (AI) — only compiled with --features ai
// ===========================================================================

#[cfg(feature = "ai")]
mod auto_tag {
    use super::*;

    /// Check if the SigLIP model is downloaded.
    fn model_available() -> bool {
        let model_dir = dirs_model_dir();
        model_dir.join("onnx").join("vision_model_quantized.onnx").exists()
            && model_dir.join("onnx").join("text_model_quantized.onnx").exists()
            && model_dir.join("tokenizer.json").exists()
    }

    fn dirs_model_dir() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        PathBuf::from(home)
            .join(".maki")
            .join("models")
            .join("siglip-vit-b16-256")
    }

    #[test]
    fn auto_tag_list_models_no_model() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Use a custom empty model dir so we don't depend on real model
        maki()
            .current_dir(&root)
            .args(["auto-tag", "--list-models"])
            .assert()
            .success();
    }

    #[test]
    fn auto_tag_list_models_json() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let output = maki()
            .current_dir(&root)
            .args(["auto-tag", "--list-models", "--json"])
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&output.get_output().stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert!(parsed.get("model_dir").is_some());
        assert!(parsed.get("active_model").is_some());
        let models = parsed.get("models").unwrap().as_array().unwrap();
        assert!(models.len() >= 2, "Expected at least 2 models");
        // Check that each model has expected fields
        for m in models {
            assert!(m.get("id").is_some());
            assert!(m.get("name").is_some());
            assert!(m.get("downloaded").is_some());
            assert!(m.get("embedding_dim").is_some());
        }
    }

    #[test]
    fn auto_tag_model_flag() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Use --model with an unknown model
        maki()
            .current_dir(&root)
            .args(["auto-tag", "--model", "nonexistent-model", "--list-models"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Unknown model"));
    }

    #[test]
    fn auto_tag_no_scope_errors() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Running auto-tag without --query/--asset/--volume should fail
        maki()
            .current_dir(&root)
            .args(["auto-tag"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("No scope specified"));
    }

    #[test]
    fn auto_tag_no_model_errors() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Override model dir to an empty location
        let config_path = root.join("maki.toml");
        let model_tmp = tempdir().unwrap();
        std::fs::write(
            &config_path,
            format!(
                "[ai]\nmodel_dir = \"{}\"\n",
                model_tmp.path().display()
            ),
        )
        .unwrap();

        maki()
            .current_dir(&root)
            .args(["auto-tag", "--query", "*"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("Model not downloaded"));
    }

    #[test]
    fn auto_tag_remove_model_nonexistent() {
        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Override model dir to a temp location
        let config_path = root.join("maki.toml");
        let model_tmp = tempdir().unwrap();
        std::fs::write(
            &config_path,
            format!(
                "[ai]\nmodel_dir = \"{}\"\n",
                model_tmp.path().display()
            ),
        )
        .unwrap();

        maki()
            .current_dir(&root)
            .args(["auto-tag", "--remove-model"])
            .assert()
            .success();
    }

    // The following tests require the model to be downloaded.
    // They are skipped gracefully if the model is not available.

    #[test]
    fn auto_tag_dry_run() {
        if !model_available() {
            eprintln!("Skipping auto_tag_dry_run: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Create a real JPEG (small 2x2 image)
        let img = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path = root.join("test.jpg");
        img.save(&img_path).unwrap();

        // Import
        maki()
            .current_dir(&root)
            .args(["import", img_path.to_str().unwrap()])
            .assert()
            .success();

        // Auto-tag dry run
        maki()
            .current_dir(&root)
            .args(["auto-tag", "--query", "type:image"])
            .assert()
            .success()
            .stdout(predicate::str::contains("dry run"));
    }

    #[test]
    fn auto_tag_apply() {
        if !model_available() {
            eprintln!("Skipping auto_tag_apply: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let img = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path = root.join("test.jpg");
        img.save(&img_path).unwrap();

        maki()
            .current_dir(&root)
            .args(["import", img_path.to_str().unwrap()])
            .assert()
            .success();

        maki()
            .current_dir(&root)
            .args([
                "auto-tag",
                "--query",
                "type:image",
                "--apply",
                "--threshold",
                "0.01",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("processed"));
    }

    #[test]
    fn auto_tag_json_output() {
        if !model_available() {
            eprintln!("Skipping auto_tag_json_output: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let img = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path = root.join("test.jpg");
        img.save(&img_path).unwrap();

        maki()
            .current_dir(&root)
            .args(["import", img_path.to_str().unwrap()])
            .assert()
            .success();

        let output = maki()
            .current_dir(&root)
            .args(["auto-tag", "--query", "type:image", "--json"])
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&output.get_output().stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert!(parsed.get("assets_processed").is_some());
        assert!(parsed.get("dry_run").is_some());
        assert!(parsed.get("suggestions").is_some());
    }

    #[test]
    fn auto_tag_custom_labels() {
        if !model_available() {
            eprintln!("Skipping auto_tag_custom_labels: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let img = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path = root.join("test.jpg");
        img.save(&img_path).unwrap();

        maki()
            .current_dir(&root)
            .args(["import", img_path.to_str().unwrap()])
            .assert()
            .success();

        // Create custom labels file
        let labels_path = root.join("my_labels.txt");
        std::fs::write(&labels_path, "purple\nblue\nred\n").unwrap();

        maki()
            .current_dir(&root)
            .args([
                "auto-tag",
                "--query",
                "type:image",
                "--labels",
                labels_path.to_str().unwrap(),
            ])
            .assert()
            .success();
    }

    #[test]
    fn auto_tag_threshold_high() {
        if !model_available() {
            eprintln!("Skipping auto_tag_threshold_high: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let img = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path = root.join("test.jpg");
        img.save(&img_path).unwrap();

        maki()
            .current_dir(&root)
            .args(["import", img_path.to_str().unwrap()])
            .assert()
            .success();

        // High threshold = fewer/no suggestions
        let output = maki()
            .current_dir(&root)
            .args([
                "auto-tag",
                "--query",
                "type:image",
                "--threshold",
                "0.99",
                "--json",
            ])
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&output.get_output().stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(parsed["tags_suggested"].as_u64().unwrap(), 0);
    }

    #[test]
    fn auto_tag_specific_asset() {
        if !model_available() {
            eprintln!("Skipping auto_tag_specific_asset: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        let img = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path = root.join("test.jpg");
        img.save(&img_path).unwrap();

        // Import
        maki()
            .current_dir(&root)
            .args(["import", img_path.to_str().unwrap()])
            .assert()
            .success();

        // Get asset ID via search
        let search_output = maki()
            .current_dir(&root)
            .args(["search", "-q", "type:image"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let asset_id = String::from_utf8(search_output).unwrap();
        let asset_id = asset_id.trim();
        let short_id = &asset_id[..8];

        maki()
            .current_dir(&root)
            .args(["auto-tag", "--asset", short_id])
            .assert()
            .success()
            .stdout(predicate::str::contains("processed"));
    }

    #[test]
    fn auto_tag_similar() {
        if !model_available() {
            eprintln!("Skipping auto_tag_similar: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Create two images
        let img1 = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([128, 64, 200]));
        let img_path1 = root.join("test1.jpg");
        img1.save(&img_path1).unwrap();

        let img2 = image::RgbImage::from_fn(2, 2, |_, _| image::Rgb([200, 100, 50]));
        let img_path2 = root.join("test2.jpg");
        img2.save(&img_path2).unwrap();

        maki()
            .current_dir(&root)
            .args(["import", img_path1.to_str().unwrap(), img_path2.to_str().unwrap()])
            .assert()
            .success();

        // Auto-tag both to generate embeddings
        maki()
            .current_dir(&root)
            .args(["auto-tag", "--query", "type:image"])
            .assert()
            .success();

        // Get first asset ID
        let search_output = maki()
            .current_dir(&root)
            .args(["search", "type:image", "--format", "ids"])
            .assert()
            .success();

        let search_stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
        let first_id = search_stdout.lines().next().unwrap().trim();
        let short_id = &first_id[..8];

        // Find similar
        maki()
            .current_dir(&root)
            .args(["auto-tag", "--similar", short_id])
            .assert()
            .success()
            .stdout(predicate::str::contains("similarity"));
    }

    #[test]
    fn auto_tag_no_results() {
        if !model_available() {
            eprintln!("Skipping auto_tag_no_results: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // No assets to tag
        maki()
            .current_dir(&root)
            .args(["auto-tag", "--query", "type:image", "--json"])
            .assert()
            .success();
    }

    #[test]
    fn auto_tag_skip_non_image() {
        if !model_available() {
            eprintln!("Skipping auto_tag_skip_non_image: model not downloaded");
            return;
        }

        let tmp = tempdir().unwrap();
        let root = init_catalog(tmp.path());

        // Create a non-image file
        create_test_file(&root, "document.txt", b"hello world");

        maki()
            .current_dir(&root)
            .args(["import", root.join("document.txt").to_str().unwrap()])
            .assert()
            .success();

        let output = maki()
            .current_dir(&root)
            .args(["auto-tag", "--query", "*", "--json"])
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&output.get_output().stdout);
        let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        // Should skip non-image assets
        assert!(parsed["assets_skipped"].as_u64().unwrap() > 0 || parsed["assets_processed"].as_u64().unwrap() == 0);
    }
}

// ── sync-metadata ──────────────────────────────────────────────────

#[test]
fn sync_metadata_unchanged() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photos/SM_001.ARW", b"raw-sm-unchanged");
    create_test_file(
        &root,
        "photos/SM_001.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"3\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // No changes — should report unchanged
    maki()
        .current_dir(&root)
        .args(["sync-metadata"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 unchanged"));
}

#[test]
fn sync_metadata_inbound_reads_external_changes() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let xmp_path = root.join("photos/SM_002.xmp");
    create_test_file(&root, "photos/SM_002.ARW", b"raw-sm-inbound");
    create_test_file(
        &root,
        "photos/SM_002.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"2\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Modify XMP externally (simulates CaptureOne edit)
    std::fs::write(
        &xmp_path,
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"5\"/></rdf:RDF></x:xmpmeta>",
    )
    .unwrap();

    // sync-metadata should read external change
    maki()
        .current_dir(&root)
        .args(["sync-metadata"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 read from disk"));

    // Verify rating was updated
    maki()
        .current_dir(&root)
        .args(["search", "--json", "rating:5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SM_002"));
}

#[test]
fn sync_metadata_outbound_writes_pending() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let xmp_path = root.join("photos/SM_003.xmp");
    create_test_file(&root, "photos/SM_003.ARW", b"raw-sm-outbound");
    create_test_file(
        &root,
        "photos/SM_003.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"1\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Get the asset ID
    let output = maki()
        .current_dir(&root)
        .args(["search", "-q", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let asset_id = stdout.trim();

    // Edit rating in DAM — this writes back immediately
    maki()
        .current_dir(&root)
        .args(["edit", asset_id, "--rating", "4"])
        .assert()
        .success();

    // sync-metadata should report unchanged (writeback already happened inline)
    maki()
        .current_dir(&root)
        .args(["sync-metadata"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 unchanged"));

    // Verify the XMP was updated by the edit command's write-back
    let xmp_content = std::fs::read_to_string(&xmp_path).unwrap();
    assert!(xmp_content.contains("Rating=\"4\"") || xmp_content.contains("Rating='4'"));
}

#[test]
fn sync_metadata_dry_run() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let xmp_path = root.join("photos/SM_004.xmp");
    create_test_file(&root, "photos/SM_004.ARW", b"raw-sm-dryrun");
    create_test_file(
        &root,
        "photos/SM_004.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"2\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Modify XMP externally
    std::fs::write(
        &xmp_path,
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"5\"/></rdf:RDF></x:xmpmeta>",
    )
    .unwrap();

    // Dry run should report but not apply
    maki()
        .current_dir(&root)
        .args(["sync-metadata", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 read from disk"));

    // Rating should still be 2 (unchanged)
    maki()
        .current_dir(&root)
        .args(["search", "--json", "rating:2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SM_004"));
}

#[test]
fn sync_metadata_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photos/SM_005.ARW", b"raw-sm-json");
    create_test_file(
        &root,
        "photos/SM_005.xmp",
        b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:Rating=\"3\"/></rdf:RDF></x:xmpmeta>",
    );
    maki()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    let output = maki()
        .current_dir(&root)
        .args(["sync-metadata", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["unchanged"], 1);
    assert_eq!(parsed["inbound"], 0);
    assert_eq!(parsed["outbound"], 0);
    assert_eq!(parsed["conflicts"], 0);
    assert_eq!(parsed["dry_run"], false);
}

// ── Contact sheet tests ─────────────────────────────────────────────────────

#[test]
fn contact_sheet_generates_pdf() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "cs_test.jpg", b"contact sheet image");
    maki()
        .current_dir(&root)
        .args(["import", root.join("cs_test.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dir.path().join("output.pdf");
    maki()
        .current_dir(&root)
        .args([
            "contact-sheet",
            "cs_test",
            output.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 assets"));

    assert!(output.exists());
    let data = std::fs::read(&output).unwrap();
    assert!(data.starts_with(b"%PDF"), "Output should be a valid PDF");
}

#[test]
fn contact_sheet_dry_run() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "cs_dry.jpg", b"dry run image");
    maki()
        .current_dir(&root)
        .args(["import", root.join("cs_dry.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dir.path().join("dry.pdf");
    maki()
        .current_dir(&root)
        .args([
            "contact-sheet",
            "cs_dry",
            output.to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success();

    assert!(!output.exists(), "Dry run should not create the PDF");
}

#[test]
fn contact_sheet_json_output() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "cs_json.jpg", b"json image");
    maki()
        .current_dir(&root)
        .args(["import", root.join("cs_json.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dir.path().join("json.pdf");
    let cmd = maki()
        .current_dir(&root)
        .args([
            "contact-sheet",
            "cs_json",
            output.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&cmd.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["assets"], 1);
    assert_eq!(parsed["pages"], 1);
    assert_eq!(parsed["layout"], "standard");
    assert_eq!(parsed["paper"], "a4");
    assert_eq!(parsed["dry_run"], false);
}

#[test]
fn contact_sheet_zero_results_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    let output = dir.path().join("empty.pdf");
    maki()
        .current_dir(&root)
        .args([
            "contact-sheet",
            "nonexistent:true",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure();

    assert!(!output.exists());
}

#[test]
fn contact_sheet_layout_options() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "cs_opts.jpg", b"layout test");
    maki()
        .current_dir(&root)
        .args(["import", root.join("cs_opts.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dir.path().join("opts.pdf");
    maki()
        .current_dir(&root)
        .args([
            "contact-sheet",
            "cs_opts",
            output.to_str().unwrap(),
            "--layout", "dense",
            "--landscape",
            "--paper", "a3",
            "--title", "Test Sheet",
            "--fields", "filename,rating",
            "--label-style", "dot",
        ])
        .assert()
        .success();

    assert!(output.exists());
}

#[test]
fn contact_sheet_dry_run_json() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "cs_drj.jpg", b"dry run json");
    maki()
        .current_dir(&root)
        .args(["import", root.join("cs_drj.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dir.path().join("drj.pdf");
    let cmd = maki()
        .current_dir(&root)
        .args([
            "contact-sheet",
            "cs_drj",
            output.to_str().unwrap(),
            "--dry-run",
            "--json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&cmd.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["dry_run"], true);
    assert_eq!(parsed["assets"], 1);
    assert!(!output.exists());
}

// ── Shell batch variable tests ────────────────────────────

#[test]
fn shell_batch_tag_via_variable() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file1 = create_test_file(&root, "shell_tag1.jpg", b"shell tag data 1");
    let file2 = create_test_file(&root, "shell_tag2.jpg", b"shell tag data 2");

    maki().current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert().success();
    maki().current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert().success();

    // Write a script that searches then tags all results via _
    let script = root.join("batch-tag.dam");
    std::fs::write(&script, "search type:image\ntag _ batch-test\n").unwrap();

    maki().current_dir(&root)
        .args(["shell", script.to_str().unwrap()])
        .assert()
        .success();

    // Both assets should now have the "batch-test" tag
    maki().current_dir(&root)
        .args(["search", "tag:batch-test", "--format", "ids"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\n").count(2)); // two IDs, two lines
}
