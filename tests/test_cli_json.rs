use assert_cmd::Command;
use pkg_core::StoreLayout;
use pkg_core::domain::package::RemotePackage;
use pkg_core::state::StateDatabase;
use serde_json::Value;
use tempfile::tempdir;

#[path = "common/mod.rs"]
mod common;

fn pkg(data_dir: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("pkg").unwrap();
    command.arg("--data-dir").arg(data_dir);
    command
}

#[test]
fn profile_commands_emit_single_json_document() {
    let temp = tempdir().unwrap();
    let output = pkg(temp.path())
        .args(["--json", "profile", "create", "agent-task"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "success");
    assert!(output.stderr.is_empty());

    let output = pkg(temp.path())
        .args(["--json", "profile", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["profiles"][0], "agent-task");
}

#[test]
fn doctor_json_is_machine_readable() {
    let temp = tempdir().unwrap();
    let output = pkg(temp.path())
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["findings"][0]["code"], "HEALTHY");
}

#[test]
fn doctor_repair_is_explicit_and_machine_readable() {
    let temp = tempdir().unwrap();
    let output = pkg(temp.path())
        .args(["--json", "doctor", "--repair"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["findings"][0]["code"], "HEALTHY");
}

#[test]
fn typed_not_found_errors_have_stable_json_and_exit_code() {
    let temp = tempdir().unwrap();
    let output = pkg(temp.path())
        .args(["--json", "info", "missing-package"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(10));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "error");
    assert_eq!(value["code"], 10);
}

#[test]
fn local_install_json_contains_the_realized_plan() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("tool.deb");
    common::DebPackageBuilder::new("tool")
        .file("usr/bin/tool", b"#!/bin/sh\necho ok\n", 0o755)
        .write_to(&artifact)
        .unwrap();
    let output = pkg(temp.path())
        .args(["--json", "install", artifact.to_str().unwrap(), "--yes"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "success");
    assert_eq!(value["installed"], 1);
    assert_eq!(value["plans"][0]["package"]["name"], "tool");
    assert_eq!(value["plans"][0]["is_dry_run"], false);
    assert_eq!(
        value["plans"][0]["payload_manifest"]["inspection_complete"],
        true
    );
    assert!(
        !value["plans"][0]["payload_manifest"]["tree_digest"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn uncached_remote_dry_run_reports_missing_input_without_mutation() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("data");
    let layout = StoreLayout::new(&data_dir);
    layout.ensure_dirs().unwrap();
    let db = StateDatabase::open(&layout.db_path()).unwrap();
    db.commit_repository_snapshot(
        "fixture-repo",
        "deb",
        "https://example.invalid/debian",
        "stable",
        &[RemotePackage {
            repository_id: "fixture-repo".into(),
            name: "remote-fixture".into(),
            version: "1.0.0".into(),
            architecture: "amd64".into(),
            format: "deb".into(),
            digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            size_bytes: 12,
            url: "https://example.invalid/remote-fixture.deb".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        }],
    )
    .unwrap();
    drop(db);

    let output = pkg(&data_dir)
        .args(["--json", "install", "remote-fixture", "--dry-run"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "error");
    assert_eq!(value["failed"], 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("not cached"));
    assert!(
        std::fs::read_dir(layout.store_dir())
            .unwrap()
            .next()
            .is_none()
    );
}
