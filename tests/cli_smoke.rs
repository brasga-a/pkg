//! Smoke tests for the `pkg` CLI entry point.

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn test_help_flag() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "pkg is a cross-distribution Linux package manager",
        ))
        .stdout(predicate::str::contains("--version"))
        .stdout(predicate::str::contains("--help"))
        .stdout(predicate::str::contains("--verbose"));
}

#[test]
fn test_short_help_flag() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "A universal, cross-distribution package manager for Linux",
        ))
        .stdout(predicate::str::contains("-V, --version"))
        .stdout(predicate::str::contains("-h, --help"));
}

#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "pkg {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn test_short_version_flag() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "pkg {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn install_does_not_fall_back_to_a_substring_match() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("data");
    Command::cargo_bin("pkg")
        .unwrap()
        .args(["--data-dir", data_dir.to_str().unwrap(), "list"])
        .assert()
        .success();
    let db = Connection::open(data_dir.join("state/pkg.db")).unwrap();
    db.execute(
        "INSERT INTO repositories (id, url, distribution, updated_at) VALUES ('test', 'http://example.invalid', 'test', '0')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO remote_packages (repository_id, name, version, architecture, format, digest, size_bytes, url) VALUES ('test', 'requested-extra', '1.0', 'amd64', 'deb', ?1, 1, 'http://example.invalid/package.deb')",
        ["0".repeat(64)],
    )
    .unwrap();
    Command::cargo_bin("pkg")
        .unwrap()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "install",
            "requested",
            "--dry-run",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_repository_sync_and_package_update_cli_behavior() {
    let mut help_cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    help_cmd
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("upgrade"))
        .stdout(predicate::str::contains("update"))
        .stdout(predicate::str::contains("repo"))
        .stdout(predicate::str::contains("self-update"))
        .stdout(predicate::str::contains("self-uninstall"));

    let temp = tempdir().expect("temporary data directory should be created");
    let mut update_guidance_cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    update_guidance_cmd
        .args([
            "--data-dir",
            temp.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            "update",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("explicitly separated"))
        .stdout(predicate::str::contains("pkg upgrade"))
        .stdout(predicate::str::contains("pkg repo update"));

    let mut upgrade_cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    upgrade_cmd
        .args([
            "--data-dir",
            temp.path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            "upgrade",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("No upgrades available"));

    let mut repo_help_cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    repo_help_cmd
        .args(["repo", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("update"))
        .stdout(predicate::str::contains("remote"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("remove"));

    let mut uninstall_dry_run_cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    uninstall_dry_run_cmd
        .args(["self-uninstall", "--dry-run", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[DRY-RUN MODE]"));
}
