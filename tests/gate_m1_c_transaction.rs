//! Gate M1-C (Transaction) Release Gate Verification Suite.
//!
//! Verifies:
//! - forced interruption at transaction phases is detectable/recoverable;
//! - one-writer locking prevents concurrent state mutation races;
//! - staging/promotion/state reconciliation produces a known state after restart;
//! - install planning is side-effect free;
//! - native package-manager databases remain untouched.

#[path = "common/mod.rs"]
mod common;

use common::DebPackageBuilder;
use pkg_core::domain::package::{
    Architecture, ArtifactDigest, PackageFormat, PackageName, PackageVersion,
};
use pkg_core::error::Error;
use pkg_core::lock::ProcessLock;
use pkg_core::state::NewStoreObject;
use pkg_core::{Engine, StoreLayout};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_dry_run_zero_host_mutation() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    let deb = temp.path().join("dry_run_pkg.deb");
    DebPackageBuilder::new("dryrun-pkg")
        .version("1.0.0")
        .file("./usr/bin/drycmd", b"#!/bin/sh\necho dry", 0o755)
        .write_to(&deb)
        .unwrap();

    // Run with dry_run = true (INV-010)
    let plan = engine.install(&deb, "default", true).unwrap();
    assert!(plan.is_dry_run);
    assert_eq!(plan.package.name.as_str(), "dryrun-pkg");
    assert_eq!(plan.binaries.len(), 1);

    // Verify ZERO mutations occurred on disk or in DB
    assert!(
        !plan.target_store_dir.exists(),
        "Dry run must not create store object directory"
    );

    let profile_bin = engine.layout().profile_bin_dir("default").join("drycmd");
    assert!(
        !profile_bin.exists() && !profile_bin.is_symlink(),
        "Dry run must not create profile symlinks"
    );

    let list = engine.list("default").unwrap();
    assert!(
        list.is_empty(),
        "Dry run must not insert records into state database"
    );
}

#[test]
fn test_process_lock_prevents_concurrent_mutations() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    let lock_path = engine.layout().lock_path();

    // Acquire lock simulating a concurrently running mutate process (INV-012)
    let _held_lock = ProcessLock::acquire(&lock_path).unwrap();

    let deb = temp.path().join("lock_test.deb");
    DebPackageBuilder::new("lock-pkg")
        .file("./usr/bin/lockcmd", b"echo lock", 0o755)
        .write_to(&deb)
        .unwrap();

    // Concurrent install must fail cleanly with LockError
    let result = engine.install(&deb, "default", false);
    assert!(
        matches!(result, Err(Error::LockError(_))),
        "Concurrent install must fail with LockError when lock is held, got: {result:?}"
    );

    // Release held lock
    drop(_held_lock);

    // Now install must succeed
    let result2 = engine.install(&deb, "default", false);
    assert!(
        result2.is_ok(),
        "Install must succeed after lock is released"
    );
}

#[test]
fn test_startup_transaction_recovery_and_reconciliation() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);

    // Setup initial state database and directories
    layout.ensure_dirs().unwrap();
    let db = pkg_core::state::StateDatabase::open(&layout.db_path()).unwrap();

    let broken_tx_id = "tx-interrupted-12345";
    let broken_store_id = "interrupted-orphan-store";
    let broken_staging = layout.staging_dir(broken_tx_id);
    let broken_store_path = layout.store_object_dir(broken_store_id);

    // Simulate an interrupted transaction halted at 'Staging'
    fs::create_dir_all(&broken_staging).unwrap();
    fs::write(broken_staging.join("partial_file"), b"in-flight data").unwrap();

    // Also simulate an uncommitted store directory left from aborted 'Promoting'
    fs::create_dir_all(&broken_store_path).unwrap();
    fs::write(broken_store_path.join("uncommitted"), b"stray data").unwrap();

    // Record incomplete transaction in DB
    db.record_transaction_start(
        broken_tx_id,
        "install",
        "Staging",
        "broken-pkg",
        Some(broken_store_id),
        Some("Simulated crash at staging"),
    )
    .unwrap();

    // Also record the store object in store_objects table without completing package registration
    let pkg_name = PackageName::new("broken-pkg").unwrap();
    let ver = PackageVersion::new("0.9.0");
    let arch = Architecture::X86_64;
    let digest = ArtifactDigest::sha256("ffffffffffffffff");

    db.record_store_object(&NewStoreObject {
        store_id: broken_store_id,
        name: &pkg_name,
        version: &ver,
        architecture: &arch,
        format: PackageFormat::Deb,
        digest: &digest,
        store_path: &broken_store_path,
        files: &[],
    })
    .unwrap();

    // Verify the incomplete transaction is detectable (INV-013)
    let incomplete = db.list_incomplete_transactions().unwrap();
    assert_eq!(incomplete.len(), 1);
    assert_eq!(incomplete[0].phase, "Staging");

    // Engine::open performs automatic recovery on startup
    let engine = Engine::open(layout.clone()).unwrap();

    // Verify recovery cleaned up the orphan staging directory
    assert!(
        !broken_staging.exists(),
        "Staging directory from interrupted transaction must be cleaned up"
    );

    // Verify recovery cleaned up uncommitted store object
    assert!(
        !broken_store_path.exists(),
        "Uncommitted store object from interrupted transaction must be removed"
    );

    // Verify transaction state was updated to 'FailedClean'
    let remaining_incomplete = engine.db().list_incomplete_transactions().unwrap();
    assert!(
        remaining_incomplete.is_empty(),
        "No incomplete transactions should remain after reconciliation"
    );

    // Verify package is not considered installed
    assert!(
        engine
            .db()
            .get_package("default", "broken-pkg")
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_native_package_manager_databases_untouched() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    let deb = temp.path().join("app.deb");
    DebPackageBuilder::new("native-test")
        .file("./usr/bin/nativecmd", b"echo native", 0o755)
        .write_to(&deb)
        .unwrap();

    // Record mtime of native dpkg status file if it exists
    let dpkg_status = std::path::Path::new("/var/lib/dpkg/status");
    let initial_mtime = dpkg_status.metadata().and_then(|m| m.modified()).ok();

    engine.install(&deb, "default", false).unwrap();

    // Check that /var/lib/dpkg/status was NOT touched (INV-001)
    if let Some(initial) = initial_mtime {
        let current_mtime = dpkg_status.metadata().and_then(|m| m.modified()).ok();
        assert_eq!(
            Some(initial),
            current_mtime,
            "Native dpkg database must never be modified by pkg"
        );
    }
}
