mod common;

use assert_cmd::Command;
use pkg_core::domain::package::RemotePackage;
use pkg_core::{Engine, StoreLayout};
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_cli_install_interactive_flag_help() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.args(["install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("-i, --interactive"))
        .stdout(predicate::str::contains(
            "Interactively select from matching package candidates",
        ));
}

#[test]
fn test_cli_interactive_candidate_listing() {
    let temp = tempdir().unwrap();
    let store_dir = temp.path().join("store");
    let engine = Engine::open(StoreLayout::new(&store_dir)).unwrap();

    // Populate database with two matching packages across different repositories
    let deb_pkg = RemotePackage {
        repository_id: "ubuntu-noble".to_string(),
        name: "test-editor".to_string(),
        version: "1.0.0".to_string(),
        architecture: "amd64".to_string(),
        format: "deb".to_string(),
        digest: "a".repeat(64),
        size_bytes: 1000,
        url: "http://example.com/test-editor.deb".to_string(),
        constraints: Vec::new(),
        provides: Vec::new(),
        versioned_provides: Vec::new(),
    };
    let rpm_pkg = RemotePackage {
        repository_id: "fedora-41".to_string(),
        name: "test-editor".to_string(),
        version: "1.1.0".to_string(),
        architecture: "x86_64".to_string(),
        format: "rpm".to_string(),
        digest: "b".repeat(64),
        size_bytes: 1000,
        url: "http://example.com/test-editor.rpm".to_string(),
        constraints: Vec::new(),
        provides: Vec::new(),
        versioned_provides: Vec::new(),
    };

    engine
        .db()
        .commit_repository_snapshot(
            "ubuntu-noble",
            "deb",
            "http://example.com",
            "noble",
            &[deb_pkg],
        )
        .unwrap();
    engine
        .db()
        .commit_repository_snapshot("fedora-41", "rpm", "http://example.com", "41", &[rpm_pkg])
        .unwrap();

    // Run pkg install -i test-editor with dry-run/non-interactive stdin to observe candidate listing
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.args([
        "--data-dir",
        store_dir.to_str().unwrap(),
        "install",
        "-i",
        "test-editor",
    ])
    .write_stdin("\n")
    .assert()
    .stdout(predicate::str::contains(
        "Multiple candidates match 'test-editor':",
    ))
    .stdout(predicate::str::contains(
        "test-editor 1.0.0 [deb] (from repository 'ubuntu-noble')",
    ))
    .stdout(predicate::str::contains(
        "test-editor 1.1.0 [rpm] (from repository 'fedora-41')",
    ));
}
