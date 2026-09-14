//! Smoke tests for the `pkg` CLI entry point.

use assert_cmd::Command;
use predicates::prelude::*;

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
        .stdout(predicate::str::contains("pkg 0.1.0"));
}

#[test]
fn test_short_version_flag() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains("pkg 0.1.0"));
}
