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
