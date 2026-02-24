use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

/// Return a Command for the `dam` binary.
fn dam() -> Command {
    cargo_bin_cmd!(assert_cmd::pkg_name!()).into()
}

/// Initialize a catalog and register a volume pointing at `dir`.
/// Returns the canonical path (needed on macOS where /var -> /private/var)
/// so that volume lookup matches canonicalized import paths.
fn init_catalog(dir: &Path) -> PathBuf {
    let canonical = dir.canonicalize().expect("canonicalize tempdir");
    dam().current_dir(&canonical).arg("init").assert().success();
    dam()
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
    dam()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));
}

#[test]
fn init_fails_if_already_exists() {
    let dir = tempdir().unwrap();
    dam().current_dir(dir.path()).arg("init").assert().success();
    dam()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn commands_fail_without_init() {
    let dir = tempdir().unwrap();
    dam()
        .current_dir(dir.path())
        .args(["search", "foo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No dam catalog found"));
}

#[test]
fn volume_add_and_list() {
    let dir = tempdir().unwrap();
    dam().current_dir(dir.path()).arg("init").assert().success();
    dam()
        .current_dir(dir.path())
        .args(["volume", "add", "my-vol", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-vol"));

    dam()
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

    dam()
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

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Extract asset ID from search output
    let output = dam()
        .current_dir(&root)
        .args(["search", "rose"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "bird"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().unwrap();

    // Add tag
    dam()
        .current_dir(&root)
        .args(["tag", short_id, "nature", "wildlife"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Added tags:")
                .and(predicate::str::contains("nature")),
        );

    // Remove tag
    dam()
        .current_dir(&root)
        .args(["tag", short_id, "--remove", "wildlife"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed tags:").and(predicate::str::contains("wildlife")));

    // Verify remaining tags via show
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();

    dam()
        .current_dir(&root)
        .args(["import", file2.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Get asset ID via search
    let output = dam()
        .current_dir(&root)
        .args(["search", "DSC_100"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Verify tags and metadata appear in show output
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Show should NOT mention CaptureOne recipe
    let output = dam()
        .current_dir(&root)
        .args(["search", "DSC_001"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    dam()
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

    dam()
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
    let output = dam()
        .current_dir(&root)
        .args(["search", "DSC_002"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    dam()
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
    dam()
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
    dam()
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

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "preview_show"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    dam()
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
    dam().current_dir(&canonical).arg("init").assert().success();

    let vol1 = canonical.join("vol1");
    let vol2 = canonical.join("vol2");
    std::fs::create_dir_all(&vol1).unwrap();
    std::fs::create_dir_all(&vol2).unwrap();

    dam()
        .current_dir(&canonical)
        .args(["volume", "add", "vol1", vol1.to_str().unwrap()])
        .assert()
        .success();
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", vol1.join("photo.jpg").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Get asset ID
    let output = dam()
        .current_dir(&root)
        .args(["search", "photo"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Relocate to vol2
    dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", vol1.join("move_me.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "move_me"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Relocate with --remove-source
    dam()
        .current_dir(&root)
        .args(["relocate", short_id, "vol2", "--remove-source"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Relocate complete"));

    // File should be on vol2 but not on vol1
    assert!(vol2.join("move_me.jpg").exists());
    assert!(!vol1.join("move_me.jpg").exists());

    // Show should only list vol2
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", vol1.join("dry.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "dry"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Dry run
    dam()
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
fn import_conflicting_include_skip_errors() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"data");

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Verify it's searchable
    dam()
        .current_dir(&root)
        .args(["search", "keeper"])
        .assert()
        .success()
        .stdout(predicate::str::contains("keeper"));

    // Rebuild
    dam()
        .current_dir(&root)
        .arg("rebuild-catalog")
        .assert()
        .success()
        .stdout(predicate::str::contains("Rebuild complete"));

    // Still searchable after rebuild
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Corrupt the file
    std::fs::write(&file, b"corrupted data!!!").unwrap();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap(), file2.to_str().unwrap()])
        .assert()
        .success();

    // Verify only file_a
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 imported")
                .and(predicate::str::contains("1 recipe")),
        );

    // Verify the whole directory — both the NEF and XMP should be verified, not untracked
    dam()
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
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("1 recipe(s) updated")
                .and(predicate::str::contains("1 skipped")),
        );

    // Verify metadata was updated
    let output = dam()
        .current_dir(&root)
        .args(["search", "DSC_200"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.join("DSC_300.xmp").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 recipe"));

    // Verify only one asset exists (no standalone Other asset created)
    let output = dam()
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
    dam()
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
    dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID via search -q
    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let asset_id = String::from_utf8(output).unwrap().trim().to_string();

    let output = dam()
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

    let output = dam()
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
fn import_json_outputs_result() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    create_test_file(&root, "photo.jpg", b"import-json-test");

    let output = dam()
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
    dam()
        .current_dir(&root)
        .args(["import", "--dry-run", sub.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"))
        .stdout(predicate::str::contains("2 imported"));

    // Search should find nothing — no actual imports happened
    dam()
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

    let output = dam()
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

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID and add tags
    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    dam()
        .current_dir(&root)
        .args(["tag", &asset_id, "landscape", "sunset"])
        .assert()
        .success();

    dam()
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
    dam()
        .current_dir(&root)
        .args(["--debug", "stats"])
        .assert()
        .success();

    // -d shorthand
    dam()
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
    dam()
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
    let output = dam()
        .current_dir(&root)
        .args(["search", "explicit_vol"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", vol1.join("vol1_photo.jpg").to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", vol2.join("vol2_photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Generate previews only for vol1
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", sub.to_str().unwrap()])
        .assert()
        .success();

    // Generate previews using PATHS mode
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set name and description
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--name", "My Photo", "--description", "A lovely sunset"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Name: My Photo")
                .and(predicate::str::contains("Description: A lovely sunset")),
        );

    // Verify via show --json
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set rating
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--rating", "4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rating: \u{2605}\u{2605}\u{2605}\u{2605}\u{2606} (4/5)"));

    // Verify via show --json
    let output = dam()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["rating"].as_u64(), Some(4));

    // Clear rating
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rating: (none)"));

    // Verify cleared
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set name and description
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--name", "Test Name", "--description", "Test Desc"])
        .assert()
        .success();

    // Clear them
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-name", "--clear-description"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Name: (none)")
                .and(predicate::str::contains("Description: (none)")),
        );

    // Verify via show --json
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", img_path.to_str().unwrap()])
        .assert()
        .success();

    // With --log, per-file progress appears on stderr
    dam()
        .current_dir(&root)
        .args(["--log", "generate-previews"])
        .assert()
        .success()
        .stderr(predicate::str::contains("log_test.png"));

    // Without --log, no per-file output on stderr
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Move the file to a new directory
    let new_sub = root.join("renamed");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("photo.jpg")).unwrap();

    // Dry run — should detect moved
    dam()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("moved"));

    // Apply — should update location
    dam()
        .current_dir(&root)
        .args(["sync", "--apply", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("moved"));

    // Verify the location was updated by running show (should show new path)
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();

    // Create a new file that wasn't imported
    create_test_file(&root, "brand_new.jpg", b"brand new data");

    dam()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("new"))
        .stdout(predicate::str::contains("Tip: run 'dam import'"));
}

#[test]
fn sync_detects_missing_file() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "gone.jpg", b"will be deleted");

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Delete the file
    std::fs::remove_file(&file).unwrap();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id before deleting
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    std::fs::remove_file(&file).unwrap();

    // --remove-stale requires --apply
    dam()
        .current_dir(&root)
        .args(["sync", "--remove-stale", root.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--remove-stale requires --apply"));

    // Apply with --remove-stale
    dam()
        .current_dir(&root)
        .args(["sync", "--apply", "--remove-stale", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("stale removed"));

    // Show should still work but location should be gone
    let show_output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", "--include", "captureone", root.to_str().unwrap()])
        .assert()
        .success();

    // Modify the XMP
    std::fs::write(&xmp, b"<xmp>modified content</xmp>").unwrap();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Move the file
    let new_sub = root.join("after");
    std::fs::create_dir_all(&new_sub).unwrap();
    std::fs::rename(&file, new_sub.join("moveme.jpg")).unwrap();

    // Sync without --apply (dry run)
    dam()
        .current_dir(&root)
        .args(["sync", root.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("moved"));

    // Show should still have the old path (catalog unchanged)
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    dam()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 removed"))
        .stdout(predicate::str::contains("1 orphaned assets removed"));

    // Asset should be fully removed (no results from search)
    let search_output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    std::fs::remove_file(&file).unwrap();

    // Without --apply: reports stale but doesn't remove
    dam()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stale"))
        .stdout(predicate::str::contains("--apply"));

    // Location should still be in the catalog
    let show_output = dam()
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
    dam()
        .current_dir(&root)
        .args(["volume", "add", "vol2", vol2_path.to_str().unwrap()])
        .assert()
        .success();

    // Import a file on the main volume
    let file1 = create_test_file(&root, "on_vol1.jpg", b"vol1 data");
    dam()
        .current_dir(&root)
        .args(["import", file1.to_str().unwrap()])
        .assert()
        .success();

    // Import a file on volume 2
    let file2 = create_test_file(&vol2_path, "on_vol2.jpg", b"vol2 data");
    dam()
        .current_dir(&root)
        .args(["import", "--volume", "vol2", file2.to_str().unwrap()])
        .assert()
        .success();

    // Delete both files
    std::fs::remove_file(&file1).unwrap();
    std::fs::remove_file(&file2).unwrap();

    // Cleanup only vol2 — should only see 1 stale
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Confirm recipe exists
    let show_output = dam()
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
    dam()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stale"));

    // Recipe should be gone
    let show_output2 = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&gone).unwrap();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
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
    dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // File still exists on disk, locations intact — orphan:true should return nothing
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // File still exists — missing:true should find nothing
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();
    assert!(!asset_id.is_empty(), "should find imported asset");

    // Never explicitly verified, so verified_at is NULL — stale:0 should match
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
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
    dam()
        .current_dir(&root)
        .args(["search", "volume:none"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&asset_id[..8]));
}

// ── Cleanup orphaned assets and previews ────────────────────────────

#[test]
fn cleanup_removes_orphaned_assets() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "orphan_cleanup.jpg", b"orphan cleanup data");

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    dam()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 orphaned assets removed"));

    // search orphan:true should return nothing — the orphan was removed
    let search_output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    std::fs::remove_file(&file).unwrap();

    // Report-only mode: should count orphaned assets but not remove them
    dam()
        .current_dir(&root)
        .args(["cleanup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 orphaned assets"));

    // Asset should still exist
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Check that a preview was generated
    let previews_dir = root.join("previews");
    let preview_count_before = count_preview_files(&previews_dir);
    assert!(preview_count_before > 0, "preview should exist after import");

    std::fs::remove_file(&file).unwrap();

    dam()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("orphaned previews removed"));

    // Preview should be gone
    let preview_count_after = count_preview_files(&previews_dir);
    assert_eq!(preview_count_after, 0, "orphaned previews should be removed");
}

#[test]
fn cleanup_preserves_non_orphaned_assets() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());
    let file = create_test_file(&root, "keep_me.jpg", b"keep me data");

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset id
    let search_output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // File still on disk — cleanup --apply should not remove anything
    dam()
        .current_dir(&root)
        .args(["cleanup", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0 stale"));

    // Asset should still exist
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    std::fs::remove_file(&file).unwrap();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID
    let search_output = dam()
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
    dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID
    let search_output = dam()
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
    dam()
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
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // Create a DIFFERENT file at the new path
    let new_file = create_test_file(&root, "moved/photo.jpg", b"different content entirely");

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "*"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&search_output.get_output().stdout);
    let asset_id = stdout.trim().to_string();

    // --from path doesn't exist in catalog, --to is a valid file
    let new_file = create_test_file(&root, "elsewhere/photo.jpg", b"some data");

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = dam()
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

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let search_output = dam()
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

    dam()
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
    dam()
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
    dam()
        .current_dir(&root)
        .args(["saved-search", "save", "Landscapes", "type:image tag:landscape rating:4+"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Saved search 'Landscapes'"));

    // List shows it
    dam()
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

    dam()
        .current_dir(&root)
        .args(["ss", "save", "Test", "type:video"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Saved search 'Test'"));

    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", root.join("photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Save and run a search that matches
    dam()
        .current_dir(&root)
        .args(["ss", "save", "All Images", "type:image"])
        .assert()
        .success();

    dam()
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

    dam()
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

    dam()
        .current_dir(&root)
        .args(["ss", "save", "ToDelete", "type:video"])
        .assert()
        .success();

    dam()
        .current_dir(&root)
        .args(["ss", "delete", "ToDelete"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted saved search 'ToDelete'"));

    // List is now empty
    dam()
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

    dam()
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

    dam()
        .current_dir(&root)
        .args(["--json", "ss", "save", "Test", "type:image"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\""))
        .stdout(predicate::str::contains("\"saved\""));

    dam()
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

    dam()
        .current_dir(&root)
        .args(["ss", "save", "My Search", "type:image"])
        .assert()
        .success();

    // Save again with same name — should replace
    dam()
        .current_dir(&root)
        .args(["ss", "save", "My Search", "type:video", "--sort", "name_asc"])
        .assert()
        .success();

    // List should show updated query
    dam()
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

    dam()
        .current_dir(&root)
        .args(["collection", "create", "Portfolio", "--description", "Best shots"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created collection 'Portfolio'"));

    dam()
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

    dam()
        .current_dir(&root)
        .args(["col", "create", "Test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created collection 'Test'"));

    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", root.join("col_photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Get the asset ID
    let output = dam()
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
    dam()
        .current_dir(&root)
        .args(["col", "create", "MyPicks"])
        .assert()
        .success();

    dam()
        .current_dir(&root)
        .args(["col", "add", "MyPicks", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added 1 asset"));

    // Show collection contents
    dam()
        .current_dir(&root)
        .args(["col", "show", "MyPicks"])
        .assert()
        .success()
        .stdout(predicate::str::contains("col_photo"));

    // List shows count
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", root.join("rm_photo.jpg").to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "--format", "ids", "rm_photo"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string();

    dam()
        .current_dir(&root)
        .args(["col", "create", "Temp"])
        .assert()
        .success();

    dam()
        .current_dir(&root)
        .args(["col", "add", "Temp", &asset_id])
        .assert()
        .success();

    // Remove asset from collection
    dam()
        .current_dir(&root)
        .args(["col", "remove", "Temp", &asset_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 1 asset"));

    // Delete collection
    dam()
        .current_dir(&root)
        .args(["col", "delete", "Temp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted collection 'Temp'"));

    // List shows empty
    dam()
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

    dam()
        .current_dir(&root)
        .args(["--json", "col", "create", "JTest"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\""))
        .stdout(predicate::str::contains("JTest"));

    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", root.join("alpha_col.jpg").to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", root.join("beta_col.jpg").to_str().unwrap()])
        .assert()
        .success();

    // Get alpha's asset ID
    let output = dam()
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
    dam()
        .current_dir(&root)
        .args(["col", "create", "Filtered"])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["col", "add", "Filtered", &asset_id])
        .assert()
        .success();

    // Search with collection filter should find only the one in the collection
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file_a.to_str().unwrap(), file_b.to_str().unwrap(), file_c.to_str().unwrap()])
        .assert()
        .success();

    // path: filter should match only files under Capture/2026-02-22
    dam()
        .current_dir(&root)
        .args(["search", "path:Capture/2026-02-22"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // path: filter for Archive should match only the sunset file
    dam()
        .current_dir(&root)
        .args(["search", "path:Archive/"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sunset"))
        .stdout(predicate::str::contains("1 result"));

    // path: with no match
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file_a.to_str().unwrap(), file_b.to_str().unwrap(), file_c.to_str().unwrap()])
        .assert()
        .success();

    // Search with absolute path should find the same results as relative
    let abs_path = format!("path:{}/photos", root.display());
    dam()
        .current_dir(&root)
        .args(["search", &abs_path])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // Verify relative path works identically
    dam()
        .current_dir(&root)
        .args(["search", "path:photos"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // Bogus absolute path should return nothing
    dam()
        .current_dir(&root)
        .args(["search", "path:/nonexistent/volume/photos"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No results found"));

    // ./ relative to cwd should resolve and normalize
    dam()
        .current_dir(root.join("photos"))
        .args(["search", "path:./"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DSC_001"))
        .stdout(predicate::str::contains("DSC_002"))
        .stdout(predicate::str::contains("2 result"));

    // ../ relative to cwd should resolve and normalize
    dam()
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
    dam()
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
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", capture.to_str().unwrap()])
        .assert()
        .success();

    // Second import: export with --auto-group
    let output = root.join("session2/Output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("DSC_200.JPG"), b"jpeg-incr-auto-group").unwrap();
    dam()
        .current_dir(&root)
        .args(["import", "--auto-group", output.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Auto-group"));

    // Should be 1 asset (existing RAW picked up the export)
    dam()
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

    dam()
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
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", session_a.to_str().unwrap()])
        .assert()
        .success();

    // Import session B with --auto-group — should NOT merge with session A
    // because they are under different session roots
    dam()
        .current_dir(&root)
        .args(["import", "--auto-group", session_b.to_str().unwrap()])
        .assert()
        .success();

    // Should still be 2 separate assets
    dam()
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

    let out = dam()
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
    dam()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Verify we have 2 assets (search returns one row per variant)
    dam()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"));

    // Dry run should report the match
    dam()
        .current_dir(&root)
        .args(["auto-group"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stem group"))
        .stdout(predicate::str::contains("would merge"));

    // Assets should still be separate (dry run)
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Apply auto-group
    dam()
        .current_dir(&root)
        .args(["auto-group", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stem group"))
        .stdout(predicate::str::contains("merged"));

    // Should now be 1 unique asset (search -q outputs one ID per variant row)
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Apply auto-group — fuzzy prefix should match
    dam()
        .current_dir(&root)
        .args(["auto-group", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 stem group"))
        .stdout(predicate::str::contains("merged"));

    // Should be 1 unique asset
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", root.join("sub").to_str().unwrap()])
        .assert()
        .success();

    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    // Should be 2 separate assets
    dam()
        .current_dir(&root)
        .args(["search", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 result(s)"));

    // Get variant hashes from show --json
    let output = dam()
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
        let output = dam()
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
    dam()
        .current_dir(&root)
        .args(["group", &hashes[0], &hashes[1]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Grouped 2 variant(s)"));

    // Should now be 1 asset
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", sub1.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", sub2.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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
        let output = dam()
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

    let output = dam()
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
fn fix_roles_dry_run_reports() {
    let dir = tempdir().unwrap();
    let root = init_catalog(dir.path());

    create_test_file(&root, "photos/DSC_100.ARW", b"raw-fixroles-1");
    create_test_file(&root, "photos/DSC_100.JPG", b"jpg-fixroles-1");
    dam()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Since auto-grouping now sets roles correctly, fix-roles should report already correct
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", raw_dir.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", jpg_dir.to_str().unwrap()])
        .assert()
        .success();

    // Auto-group to merge them into one asset
    dam()
        .current_dir(&root)
        .args(["auto-group", "--apply"])
        .assert()
        .success();

    // After auto-group the JPG should already be Export — fix-roles reports 0 fixed
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
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
    dam()
        .current_dir(&root)
        .args(["import", root.join("photos").to_str().unwrap()])
        .assert()
        .success();

    // Refresh without changes — should report unchanged
    dam()
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
    dam()
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
    dam()
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
    dam()
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
    dam()
        .current_dir(&root)
        .args(["refresh", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"))
        .stderr(predicate::str::contains("Dry run"));

    // Run again without dry-run — should still see the change (wasn't applied)
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Set label
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--label", "Red"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Label: Red"));

    // Verify via show --json
    let output = dam()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(parsed["color_label"].as_str(), Some("Red"));

    // Change to another label (case-insensitive)
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--label", "blue"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Label: Blue"));

    // Clear label
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-label"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Label: (none)"));

    // Verify cleared
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", file.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Invalid label should fail
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--label", "Magenta"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown color label"));
}

// ── Export-based preview tests ──────────────────────────────────

/// Helper: compute the SHA-256 hex of some content (matches dam's content_hash minus "sha256:" prefix).
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    // Get asset ID — may return multiple rows (one per variant), take the first
    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "DSC_900"])
        .output()
        .unwrap();
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let asset_id = stdout_str.lines().next().unwrap().trim().to_string();
    assert_eq!(asset_id.len(), 36, "Should get a UUID");

    // Verify via show --json that the asset has an export variant
    let show_json = dam()
        .current_dir(&root)
        .args(["--json", "show", &asset_id])
        .output()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&show_json.stdout).unwrap();
    let variants = parsed["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2, "Should have 2 variants (RAW + JPG)");

    // The JPG should have role "export"
    let has_export = variants.iter().any(|v| v["role"].as_str() == Some("export"));
    assert!(has_export, "JPG variant should have export role");

    // dam show should show the JPG's hash in the Preview line (export preferred)
    let jpg_hash = sha256_hex(&jpg_content);
    let show_output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", img_path.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "solo"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let hash = sha256_hex(&content);
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", dir_a.to_str().unwrap()])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["import", dir_b.to_str().unwrap()])
        .assert()
        .success();

    // Get variant hashes via show --json
    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "IMG_001.nef"])
        .output()
        .unwrap();
    let raw_asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "IMG_001_export"])
        .output()
        .unwrap();
    let jpg_asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Get content hashes from show --json
    let raw_show = dam()
        .current_dir(&root)
        .args(["--json", "show", &raw_asset_id])
        .output()
        .unwrap();
    let raw_json: serde_json::Value = serde_json::from_slice(&raw_show.stdout).unwrap();
    let raw_variant_hash = raw_json["variants"][0]["content_hash"].as_str().unwrap().to_string();

    let jpg_show = dam()
        .current_dir(&root)
        .args(["--json", "show", &jpg_asset_id])
        .output()
        .unwrap();
    let jpg_json: serde_json::Value = serde_json::from_slice(&jpg_show.stdout).unwrap();
    let jpg_variant_hash = jpg_json["variants"][0]["content_hash"].as_str().unwrap().to_string();

    // Group them
    dam()
        .current_dir(&root)
        .args(["group", &raw_variant_hash, &jpg_variant_hash])
        .assert()
        .success();

    // After grouping, the merged asset should prefer JPG (export) preview.
    // The target of `group` is the oldest asset (the RAW one, imported first).
    let jpg_hash = sha256_hex(&jpg_content);
    let show_output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    // --upgrade should run without error and report results
    dam()
        .current_dir(&root)
        .args(["generate-previews", "--upgrade"])
        .assert()
        .success()
        .stdout(predicate::str::contains("preview(s)"));

    // --upgrade --json should include upgraded field
    let output = dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Get asset ID via search
    let output = dam()
        .current_dir(&root)
        .args(["search", "skyline"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let short_id = stdout.split_whitespace().next().expect("search returned an ID");

    // Verify embedded XMP metadata appears in show output
    dam()
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
    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 imported"));

    // Get asset ID
    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Verify initial metadata
    dam()
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
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["tag", &asset_id, "--remove", "nature", "forest"])
        .assert()
        .success();

    // Verify tags are cleared
    dam()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Tags:").not()
                .and(predicate::str::contains("Rating:").not()),
        );

    // Run refresh --media to re-extract embedded XMP
    dam()
        .current_dir(&root)
        .args(["refresh", "--media"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"));

    // Verify metadata is restored
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Clear rating
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success();

    // Dry run — should report but not apply
    dam()
        .current_dir(&root)
        .args(["refresh", "--media", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1 refreshed"))
        .stderr(predicate::str::contains("Dry run"));

    // Verify rating is still cleared (not restored)
    dam()
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

    dam()
        .current_dir(&root)
        .args(["import", photos.to_str().unwrap()])
        .assert()
        .success();

    let output = dam()
        .current_dir(&root)
        .args(["search", "-q", "type:image"])
        .output()
        .unwrap();
    let asset_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Clear metadata
    dam()
        .current_dir(&root)
        .args(["edit", &asset_id, "--clear-rating"])
        .assert()
        .success();
    dam()
        .current_dir(&root)
        .args(["tag", &asset_id, "--remove", "mountain"])
        .assert()
        .success();

    // Regular refresh (no --media) — no recipes, nothing to check
    dam()
        .current_dir(&root)
        .args(["refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to check"));

    // Verify metadata is NOT restored (no Tags: or Rating: lines)
    dam()
        .current_dir(&root)
        .args(["show", &asset_id])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Tags:").not()
                .and(predicate::str::contains("Rating:").not()),
        );
}
