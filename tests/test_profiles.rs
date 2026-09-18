use std::fs;

#[path = "common/mod.rs"]
mod common;

use pkg_core::{Engine, Error, StoreLayout};
use tempfile::tempdir;

#[test]
fn profile_lifecycle_is_isolated_and_gc_friendly() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("pkg"));
    let engine = Engine::open(layout.clone()).unwrap();

    engine.create_profile("task-1").unwrap();
    assert_eq!(engine.list_profiles().unwrap(), vec!["task-1"]);
    assert!(layout.profile_dir("task-1").is_dir());

    engine.drop_profile("task-1").unwrap();
    assert!(engine.list_profiles().unwrap().is_empty());
    assert!(!layout.profile_dir("task-1").exists());
}

#[test]
fn profile_drop_refuses_unknown_regular_content() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("pkg"));
    let engine = Engine::open(layout.clone()).unwrap();
    engine.create_profile("task-2").unwrap();
    fs::write(layout.profile_dir("task-2").join("user-data"), b"keep me").unwrap();

    let error = engine.drop_profile("task-2").unwrap_err();
    assert!(matches!(error, Error::ActivationConflict { .. }));
    assert!(layout.profile_dir("task-2").exists());
}

#[test]
fn populated_profile_can_be_dropped_without_deleting_shared_store_objects() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("pkg"));
    let artifact = temp.path().join("tool.deb");
    common::DebPackageBuilder::new("tool")
        .file("usr/bin/tool", b"#!/bin/sh\necho tool\n", 0o755)
        .write_to(&artifact)
        .unwrap();
    let engine = Engine::open(layout.clone()).unwrap();
    engine.create_profile("task-3").unwrap();
    engine.install(&artifact, "task-3", false).unwrap();
    let store_objects = fs::read_dir(layout.store_dir()).unwrap().count();
    let runtime_objects = fs::read_dir(layout.runtimes_dir()).unwrap().count();
    assert!(store_objects > 0);
    assert!(runtime_objects > 0);
    engine.drop_profile("task-3").unwrap();
    assert!(!layout.profile_dir("task-3").exists());
    assert_eq!(
        fs::read_dir(layout.store_dir()).unwrap().count(),
        store_objects
    );
    assert_eq!(fs::read_dir(layout.runtimes_dir()).unwrap().count(), 0);
}
