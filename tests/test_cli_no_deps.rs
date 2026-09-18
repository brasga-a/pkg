mod common;

use assert_cmd::Command;
use common::deb_builder::DebPackageBuilder;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_cli_install_no_deps_help() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.args(["install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-deps"))
        .stdout(predicate::str::contains(
            "Skip resolving and installing declared dependencies",
        ));
}

#[test]
fn test_cli_install_with_no_deps_flag() {
    let temp = tempdir().unwrap();
    let store_dir = temp.path().join("store");
    let artifact = temp.path().join("standalone-app.deb");

    DebPackageBuilder::new("standalone-app")
        .architecture("amd64")
        .depends("missing-lib-dependency-xyz (>= 2.0.0)")
        .file(
            "usr/bin/standalone-tool",
            b"#!/bin/sh\necho standalone\n",
            0o755,
        )
        .write_to(&artifact)
        .unwrap();

    // 1. Without --no-deps: fails due to declared dependency resolution failure
    let mut cmd_fail = Command::cargo_bin("pkg").unwrap();
    cmd_fail
        .args([
            "--data-dir",
            store_dir.to_str().unwrap(),
            "install",
            "--yes",
            artifact.to_str().unwrap(),
        ])
        .assert()
        .failure();

    // 2. With --no-deps: succeeds and activates binary
    let mut cmd_pass = Command::cargo_bin("pkg").unwrap();
    cmd_pass
        .args([
            "--data-dir",
            store_dir.to_str().unwrap(),
            "install",
            "--yes",
            "--no-deps",
            artifact.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed standalone-app"));

    // Verify binary execution
    let mut cmd_run = Command::cargo_bin("pkg").unwrap();
    cmd_run
        .args([
            "--data-dir",
            store_dir.to_str().unwrap(),
            "run",
            "standalone-tool",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("standalone"));
}

#[test]
fn test_cli_install_with_skip_deps_alias() {
    let temp = tempdir().unwrap();
    let store_dir = temp.path().join("store");
    let artifact = temp.path().join("standalone-app2.deb");

    DebPackageBuilder::new("standalone-app2")
        .architecture("amd64")
        .depends("missing-lib-dependency-xyz (>= 2.0.0)")
        .file(
            "usr/bin/standalone-tool2",
            b"#!/bin/sh\necho standalone2\n",
            0o755,
        )
        .write_to(&artifact)
        .unwrap();

    // With alias --skip-deps: succeeds
    let mut cmd_pass = Command::cargo_bin("pkg").unwrap();
    cmd_pass
        .args([
            "--data-dir",
            store_dir.to_str().unwrap(),
            "install",
            "--yes",
            "--skip-deps",
            artifact.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed standalone-app2"));
}
