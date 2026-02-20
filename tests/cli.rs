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
