#[path = "common/mod.rs"]
mod common;

use common::DebPackageBuilder;
use pkg_core::{Engine, StoreLayout};
use tempfile::tempdir;

#[test]
fn install_failpoints_recover_to_a_clean_previous_state() {
    for phase in [
        "Staging",
        "Prepared",
        "Promoting",
        "Activating",
        "Committing",
    ] {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("data"));
        let artifact = temp.path().join("fault.deb");
        DebPackageBuilder::new("fault")
            .file("usr/bin/fault", b"#!/bin/sh\necho fault\n", 0o755)
            .write_to(&artifact)
            .unwrap();
        let engine =
            Engine::open_with_failpoint(layout.clone(), Some(phase)).expect("engine opens");
        assert!(
            engine.install(&artifact, "default", false).is_err(),
            "phase {phase} should fail"
        );
        drop(engine);

        let recovered = Engine::open(layout.clone()).expect("recovery should succeed");
        assert!(
            recovered.list("default").unwrap().is_empty(),
            "phase {phase}"
        );
        assert!(
            recovered
                .db()
                .list_incomplete_transactions()
                .unwrap()
                .is_empty(),
            "phase {phase}"
        );
    }
}

#[test]
fn gc_failpoint_leaves_unreachable_objects_for_a_later_run() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let artifact = temp.path().join("gc-fault.deb");
    DebPackageBuilder::new("gc-fault")
        .file("usr/bin/gc-fault", b"#!/bin/sh\necho gc\n", 0o755)
        .write_to(&artifact)
        .unwrap();

    let engine = Engine::open(layout.clone()).unwrap();
    engine.create_profile("ephemeral").unwrap();
    engine.install(&artifact, "ephemeral", false).unwrap();
    engine.drop_profile("ephemeral").unwrap();

    let failing = Engine::open_with_failpoint(layout.clone(), Some("GcBeforeCollect")).unwrap();
    assert!(failing.gc(false).is_err());
    drop(failing);

    let recovered = Engine::open(layout.clone()).unwrap();
    let report = recovered.gc(false).unwrap();
    assert_eq!(report.candidates.len(), 1);
    assert!(recovered.db().list_store_objects().unwrap().is_empty());
}

#[test]
fn rollback_failpoints_retain_the_last_committed_generation() {
    for phase in ["Activating", "Committing"] {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("data"));
        let first = temp.path().join("rollback-v1.deb");
        let second = temp.path().join("rollback-v2.deb");
        DebPackageBuilder::new("rollback-fault")
            .version("1.0.0")
            .file("usr/bin/rollback-fault", b"#!/bin/sh\necho one\n", 0o755)
            .write_to(&first)
            .unwrap();
        DebPackageBuilder::new("rollback-fault")
            .version("2.0.0")
            .file("usr/bin/rollback-fault", b"#!/bin/sh\necho two\n", 0o755)
            .write_to(&second)
            .unwrap();

        let engine = Engine::open(layout.clone()).unwrap();
        engine.install(&first, "default", false).unwrap();
        engine.install(&second, "default", false).unwrap();
        assert_eq!(engine.list("default").unwrap()[0].version.as_str(), "2.0.0");

        let failing = Engine::open_with_failpoint(layout.clone(), Some(phase)).unwrap();
        assert!(failing.rollback("default", None).is_err(), "phase {phase}");
        drop(failing);

        let recovered = Engine::open(layout.clone()).unwrap();
        assert_eq!(
            recovered.list("default").unwrap()[0].version.as_str(),
            "2.0.0",
            "phase {phase} must preserve the committed generation"
        );
        assert!(
            recovered
                .db()
                .list_incomplete_transactions()
                .unwrap()
                .is_empty(),
            "phase {phase}"
        );
        recovered.rollback("default", None).unwrap();
        assert_eq!(
            recovered.list("default").unwrap()[0].version.as_str(),
            "1.0.0"
        );
    }
}
