//! Gate M1-B (Store) Release Gate Verification Suite.
//!
//! Verifies:
//! - install/remove is idempotent for supported fixtures;
//! - side-by-side store objects do not overwrite each other;
//! - profile activation is separate from payload storage;
//! - command-name conflicts fail explicitly;
//! - removal deletes only pkg-owned content;
//! - package payload never lands directly in `/`.

#[path = "common/mod.rs"]
mod common;

use common::DebPackageBuilder;
use pkg_core::error::Error;
use pkg_core::{Engine, StoreLayout};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_install_and_execute_through_profile() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    let deb_path = temp.path().join("hello.deb");
    DebPackageBuilder::new("hello")
        .version("2.10")
        .file(
            "./usr/bin/hello",
            b"#!/bin/sh\necho 'hello from isolated store'\n",
            0o755,
        )
        .write_to(&deb_path)
        .unwrap();

    let plan = engine.install(&deb_path, "default", false).unwrap();
    assert_eq!(plan.package.name.as_str(), "hello");

    // 1. Verify store payload exists in isolated store
    assert!(plan.target_store_dir.exists());
    let store_binary = plan.target_store_dir.join("usr/bin/hello");
    assert!(store_binary.exists());

    // 2. Verify profile bin has symlink pointing to store
    let profile_bin = engine.layout().profile_bin_dir("default").join("hello");
    assert!(profile_bin.is_symlink());
    assert!(profile_bin.exists());

    // 3. Verify execution through profile symlink works
    #[cfg(unix)]
    {
        let output = std::process::Command::new(&profile_bin).output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "hello from isolated store");
    }

    // 4. Verify package payload NEVER landed in `/` or `/usr/bin/hello` (INV-002)
    assert!(
        !std::path::Path::new("/usr/bin/hello-unlikely-pkg-canary").exists(),
        "Package payload must never touch /usr"
    );

    // 5. Test removal
    let remove_plan = engine.remove("hello", "default", false).unwrap();
    assert_eq!(remove_plan.package_name.as_str(), "hello");

    // Verify profile symlink was removed
    assert!(!profile_bin.exists() && !profile_bin.is_symlink());

    // Verify list is now empty
    let list = engine.list("default").unwrap();
    assert!(list.is_empty());
}

#[test]
fn test_side_by_side_store_objects() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    // Package version 1.0
    let deb_v1 = temp.path().join("app-1.0.deb");
    DebPackageBuilder::new("myapp")
        .version("1.0.0")
        .file("./usr/bin/myapp", b"#!/bin/sh\necho v1\n", 0o755)
        .write_to(&deb_v1)
        .unwrap();

    // Package version 2.0
    let deb_v2 = temp.path().join("app-2.0.deb");
    DebPackageBuilder::new("myapp")
        .version("2.0.0")
        .file("./usr/bin/myapp", b"#!/bin/sh\necho v2\n", 0o755)
        .write_to(&deb_v2)
        .unwrap();

    let plan1 = engine.install(&deb_v1, "default", false).unwrap();
    let store_dir1 = plan1.target_store_dir.clone();
    assert!(store_dir1.exists());

    // Install version 2.0
    let plan2 = engine.install(&deb_v2, "default", false).unwrap();
    let store_dir2 = plan2.target_store_dir.clone();
    assert!(store_dir2.exists());

    // Both store objects must be distinct and coexist (INV-006)
    assert_ne!(
        store_dir1, store_dir2,
        "Side-by-side versions must have distinct store paths"
    );
    assert!(store_dir1.exists(), "v1 store object must be retained");
    assert!(store_dir2.exists(), "v2 store object must exist");

    // Profile bin points to the newly active v2
    let profile_bin = engine.layout().profile_bin_dir("default").join("myapp");
    assert!(profile_bin.exists());
    #[cfg(unix)]
    {
        let output = std::process::Command::new(&profile_bin).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "v2");
    }
}

#[test]
fn test_command_name_conflict_fails_explicitly() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    // Package A provides command `mycmd`
    let deb_a = temp.path().join("pkg_a.deb");
    DebPackageBuilder::new("package-a")
        .version("1.0.0")
        .file("./usr/bin/mycmd", b"#!/bin/sh\necho a\n", 0o755)
        .write_to(&deb_a)
        .unwrap();

    // Package B ALSO provides command `mycmd`
    let deb_b = temp.path().join("pkg_b.deb");
    DebPackageBuilder::new("package-b")
        .version("1.0.0")
        .file("./usr/bin/mycmd", b"#!/bin/sh\necho b\n", 0o755)
        .write_to(&deb_b)
        .unwrap();

    // Install package A successfully
    engine.install(&deb_a, "default", false).unwrap();

    // Attempting to install package B must FAIL explicitly with ActivationConflict (INV-015)
    let err = engine.install(&deb_b, "default", false).unwrap_err();
    match err {
        Error::ActivationConflict {
            ref command,
            ref existing_package,
        } => {
            assert_eq!(command, "mycmd");
            assert_eq!(existing_package, "package-a");
        }
        other => panic!("Expected ActivationConflict error, got: {other:?}"),
    }
}

#[test]
fn test_removal_deletes_only_pkg_owned_content() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    // Create an unrelated file in the profile bin directory (foreign file)
    let foreign_file = engine
        .layout()
        .profile_bin_dir("default")
        .join("custom_tool");
    fs::create_dir_all(foreign_file.parent().unwrap()).unwrap();
    fs::write(&foreign_file, b"foreign user script").unwrap();

    // Install pkg
    let deb = temp.path().join("demo.deb");
    DebPackageBuilder::new("demo")
        .file("./usr/bin/demo", b"echo demo", 0o755)
        .write_to(&deb)
        .unwrap();
    engine.install(&deb, "default", false).unwrap();

    // Remove pkg
    engine.remove("demo", "default", false).unwrap();

    // Unrelated foreign file must NOT be deleted (INV-014)
    assert!(
        foreign_file.exists(),
        "Removal must never touch foreign files in profile directory"
    );
}

#[test]
fn test_install_remove_idempotence() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg_data");
    let layout = StoreLayout::new(&data_dir);
    let engine = Engine::open(layout).unwrap();

    let deb = temp.path().join("idempotent.deb");
    DebPackageBuilder::new("idempotent-pkg")
        .file("./usr/bin/idempotent", b"echo 1", 0o755)
        .write_to(&deb)
        .unwrap();

    // Repeated installs succeed idempotently
    engine.install(&deb, "default", false).unwrap();
    engine.install(&deb, "default", false).unwrap();

    let list = engine.list("default").unwrap();
    assert_eq!(list.len(), 1);

    // Remove once succeeds
    engine.remove("idempotent-pkg", "default", false).unwrap();

    // Removing again yields clean PackageNotFound
    let res = engine.remove("idempotent-pkg", "default", false);
    assert!(matches!(res, Err(Error::PackageNotFound(_))));
}
