#[path = "common/mod.rs"]
mod common;

use assert_cmd::Command;
use common::DebPackageBuilder;
use pkg_core::domain::package::{ArtifactDigest, RemotePackage};
use pkg_core::{Engine, StoreLayout};
use tempfile::tempdir;

#[test]
fn cached_upgrade_applies_and_replaces_the_active_generation() {
    cached_update_command_applies("upgrade");
}

#[test]
fn test_update_guidance_explains_separation() {
    let temp = tempdir().unwrap();
    let output = Command::cargo_bin("pkg")
        .unwrap()
        .args([
            "--data-dir",
            temp.path().to_str().unwrap(),
            "--json",
            "update",
            "tool",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "notice");
    assert!(json["message"].as_str().unwrap().contains("pkg upgrade"));
    assert!(json["hint"].as_str().unwrap().contains("pkg upgrade tool"));
}

#[test]
fn cached_upgrade_expands_transitive_dependencies_from_artifact_metadata() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let tool_v1 = temp.path().join("tool-v1.deb");
    let tool_v2 = temp.path().join("tool-v2.deb");
    let middle = temp.path().join("middle.deb");
    let leaf = temp.path().join("leaf.deb");

    DebPackageBuilder::new("tool")
        .version("1.0")
        .file("usr/bin/tool", b"#!/bin/sh\necho one\n", 0o755)
        .write_to(&tool_v1)
        .unwrap();
    DebPackageBuilder::new("tool")
        .version("2.0")
        .depends("middle (= 2.0)")
        .file("usr/bin/tool", b"#!/bin/sh\necho two\n", 0o755)
        .write_to(&tool_v2)
        .unwrap();
    DebPackageBuilder::new("middle")
        .version("2.0")
        .depends("leaf (= 2.0)")
        .write_to(&middle)
        .unwrap();
    DebPackageBuilder::new("leaf")
        .version("2.0")
        .write_to(&leaf)
        .unwrap();

    let engine = Engine::open(layout.clone()).unwrap();
    engine.install(&tool_v1, "default", false).unwrap();

    let remote_packages = [
        cached_remote_deb("tool", "2.0", &tool_v2),
        cached_remote_deb("middle", "2.0", &middle),
        cached_remote_deb("leaf", "2.0", &leaf),
    ];
    for (package, source) in remote_packages.iter().zip([&tool_v2, &middle, &leaf]) {
        let cache_path = layout.artifact_cache_path(&package.digest);
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::copy(source, cache_path).unwrap();
    }
    engine
        .db()
        .commit_repository_snapshot(
            "local",
            "deb",
            "https://example.invalid",
            "stable",
            &remote_packages,
        )
        .unwrap();
    drop(engine);

    let output = Command::cargo_bin("pkg")
        .unwrap()
        .args([
            "--data-dir",
            layout.base_dir().to_str().unwrap(),
            "--json",
            "upgrade",
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);

    let reopened = Engine::open(layout).unwrap();
    for package in ["tool", "middle", "leaf"] {
        assert!(
            reopened
                .get_installed_package("default", package)
                .unwrap()
                .is_some(),
            "{package} must be installed after expanding artifact metadata"
        );
    }
}

fn cached_remote_deb(name: &str, version: &str, artifact: &std::path::Path) -> RemotePackage {
    let digest = ArtifactDigest::from_file(artifact).unwrap();
    RemotePackage {
        repository_id: "local".into(),
        name: name.into(),
        version: version.into(),
        architecture: "x86_64".into(),
        format: "deb".into(),
        digest: digest.hex().into(),
        size_bytes: std::fs::metadata(artifact).unwrap().len(),
        url: format!("https://example.invalid/{name}-{version}.deb"),
        // Intentionally omit constraints to model a stale or sparse snapshot.
        // The CLI must expand the actual authenticated archive metadata.
        constraints: vec![],
        provides: vec![],
        versioned_provides: vec![],
    }
}

fn cached_update_command_applies(command: &str) {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let first = temp.path().join("tool-v1.deb");
    let second = temp.path().join("tool-v2.deb");
    DebPackageBuilder::new("tool")
        .version("1.0")
        .file("usr/bin/tool", b"#!/bin/sh\necho one\n", 0o755)
        .write_to(&first)
        .unwrap();
    DebPackageBuilder::new("tool")
        .version("2.0")
        .file("usr/bin/tool", b"#!/bin/sh\necho two\n", 0o755)
        .write_to(&second)
        .unwrap();

    let engine = Engine::open(layout.clone()).unwrap();
    engine.install(&first, "default", false).unwrap();
    let digest = ArtifactDigest::from_file(&second).unwrap();
    let cache_path = layout.artifact_cache_path(digest.hex());
    std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    std::fs::copy(&second, &cache_path).unwrap();
    engine
        .db()
        .commit_repository_snapshot(
            "local",
            "deb",
            "https://example.invalid",
            "stable",
            &[RemotePackage {
                repository_id: "local".into(),
                name: "tool".into(),
                version: "2.0".into(),
                architecture: "x86_64".into(),
                format: "deb".into(),
                digest: digest.hex().into(),
                size_bytes: std::fs::metadata(&second).unwrap().len(),
                url: "https://example.invalid/tool-v2.deb".into(),
                constraints: vec![],
                provides: vec![],
                versioned_provides: vec![],
            }],
        )
        .unwrap();
    drop(engine);

    let output = Command::cargo_bin("pkg")
        .unwrap()
        .args([
            "--data-dir",
            layout.base_dir().to_str().unwrap(),
            "--json",
            command,
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "success");
    assert_eq!(json["applied"][0]["package"]["version"], "2.0");

    let reopened = Engine::open(layout).unwrap();
    assert_eq!(
        reopened
            .get_installed_package("default", "tool")
            .unwrap()
            .unwrap()
            .version
            .as_str(),
        "2.0"
    );

    let output = Command::cargo_bin("pkg")
        .unwrap()
        .args([
            "--data-dir",
            reopened.layout().base_dir().to_str().unwrap(),
            "--json",
            "run",
            "tool",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    let run_json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(run_json["status"], "success");
    assert!(run_json["stdout"].as_str().unwrap().contains("two"));
}
