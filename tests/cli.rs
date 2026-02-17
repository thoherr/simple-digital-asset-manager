use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

/// Return a Command for the `dam` binary.
fn dam() -> Command {
    Command::cargo_bin("dam").expect("binary exists")
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
