#[path = "common/mod.rs"]
mod common;

use common::DebPackageBuilder;
use pkg_core::error::Error;
use pkg_core::format::deb::DebAdapter;
use pkg_core::format::{ArtifactAdapter, ExtractionLimits};
use pkg_core::lock::ProcessLock;
use pkg_core::{Engine, StoreLayout};
use std::fs;
use std::io::{Cursor, Read};
use tempfile::tempdir;

#[test]
fn version_traversal_is_rejected_without_touching_user_files() {
    let temp = tempdir().unwrap();
    let victim = temp.path().join("victim");
    fs::create_dir(&victim).unwrap();
    fs::write(victim.join("keep"), b"user data").unwrap();
    let artifact = temp.path().join("bad.deb");
    DebPackageBuilder::new("test")
        .version("1/../../../victim")
        .file("usr/bin/test", b"payload", 0o755)
        .write_to(&artifact)
        .unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    assert!(matches!(
        engine.install(&artifact, "default", false),
        Err(Error::MalformedArchive(_))
    ));
    assert_eq!(fs::read(victim.join("keep")).unwrap(), b"user data");
    assert!(!victim.join("usr").exists());
    assert_eq!(
        fs::read_dir(engine.layout().store_dir()).unwrap().count(),
        0
    );
}

#[test]
fn store_promotion_rejects_escape_even_with_direct_api_use() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    layout.ensure_dirs().unwrap();
    let staging = layout.staging_dir("test");
    fs::create_dir(&staging).unwrap();
    let outside = temp.path().join("keep");
    fs::create_dir(&outside).unwrap();
    assert!(layout.promote_staging(&staging, &outside).is_err());
    assert!(outside.exists());
    let link = layout.store_object_dir("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(layout.promote_staging(&staging, &link).is_err());
    assert!(outside.exists());
}

#[test]
fn chained_symlinks_cannot_escape_staging() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("bad.deb");
    DebPackageBuilder::new("test")
        .symlink("a", ".")
        .symlink("b", "a/..")
        .write_to(&artifact)
        .unwrap();
    let staging = temp.path().join("staging");
    let result =
        DebAdapter::new().extract_payload(&artifact, &staging, &ExtractionLimits::default());
    assert!(matches!(result, Err(Error::SecurityViolation(_))));
}

#[test]
fn payload_does_not_write_through_existing_parent_symlink() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("bad.deb");
    DebPackageBuilder::new("test")
        .file("b/escaped", b"bad", 0o644)
        .write_to(&artifact)
        .unwrap();
    let staging = temp.path().join("staging");
    fs::create_dir(&staging).unwrap();
    std::os::unix::fs::symlink(temp.path(), staging.join("b")).unwrap();
    assert!(
        DebAdapter::new()
            .extract_payload(&artifact, &staging, &ExtractionLimits::default())
            .is_err()
    );
    assert!(!temp.path().join("escaped").exists());
}

#[test]
fn valid_internal_links_work_and_duplicate_entries_fail() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("good.deb");
    DebPackageBuilder::new("test")
        .file("usr/lib/data", b"ok", 0o644)
        .symlink("usr/lib/alias", "data")
        .symlink("usr/lib/alias2", "alias")
        .write_to(&artifact)
        .unwrap();
    let staging = temp.path().join("staging");
    DebAdapter::new()
        .extract_payload(&artifact, &staging, &ExtractionLimits::default())
        .unwrap();
    assert_eq!(fs::read(staging.join("usr/lib/alias2")).unwrap(), b"ok");
    let duplicate = temp.path().join("duplicate.deb");
    DebPackageBuilder::new("test")
        .file("usr/lib/data", b"first", 0o644)
        .file("usr/lib/data", b"second", 0o644)
        .write_to(&duplicate)
        .unwrap();
    assert!(matches!(
        DebAdapter::new().extract_payload(
            &duplicate,
            &temp.path().join("duplicate"),
            &ExtractionLimits::default()
        ),
        Err(Error::MalformedArchive(_))
    ));
}

#[test]
fn startup_respects_an_active_writer_and_recovers_after_unlock() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path());
    let engine = Engine::open(layout.clone()).unwrap();
    let held = ProcessLock::acquire(&layout.lock_path()).unwrap();
    engine
        .db()
        .record_transaction_start(
            "active",
            "install",
            "Staging",
            "test",
            Some("test-store"),
            None,
        )
        .unwrap();
    let staging = layout.staging_dir("active");
    fs::create_dir(&staging).unwrap();
    fs::write(staging.join("data"), b"in flight").unwrap();
    assert!(matches!(
        Engine::open(layout.clone()),
        Err(Error::LockError(_))
    ));
    assert_eq!(fs::read(staging.join("data")).unwrap(), b"in flight");
    assert_eq!(
        engine.db().list_incomplete_transactions().unwrap()[0].phase,
        "Staging"
    );
    drop(held);
    Engine::open(layout).unwrap();
    assert!(!staging.exists());
}

#[test]
fn recovery_rejects_transaction_ids_that_escape_staging() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let engine = Engine::open(layout.clone()).unwrap();
    let victim = temp.path().join("victim");
    fs::create_dir(&victim).unwrap();
    fs::write(victim.join("keep"), b"user data").unwrap();
    engine
        .db()
        .record_transaction_start("../../victim", "install", "Staging", "test", None, None)
        .unwrap();
    drop(engine);
    assert!(matches!(
        Engine::open(layout),
        Err(Error::SecurityViolation(_))
    ));
    assert_eq!(fs::read(victim.join("keep")).unwrap(), b"user data");
}

#[test]
fn shared_store_and_activations_survive_reinstall_and_profile_removal() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let engine = Engine::open(layout.clone()).unwrap();
    let artifact = temp.path().join("test.deb");
    DebPackageBuilder::new("test")
        .file("usr/bin/test", b"#!/bin/sh\necho ok\n", 0o755)
        .write_to(&artifact)
        .unwrap();
    let plan = engine.install(&artifact, "first", false).unwrap();
    engine.install(&artifact, "second", false).unwrap();
    engine.install(&artifact, "second", false).unwrap();
    assert_eq!(engine.list("first").unwrap()[0].binaries, vec!["test"]);
    // Recovery must use references across all profiles, not the default profile.
    engine
        .db()
        .record_transaction_start(
            "committing",
            "install",
            "Committing",
            "test",
            Some(&plan.store_id),
            None,
        )
        .unwrap();
    drop(engine);
    let engine = Engine::open(layout.clone()).unwrap();
    assert!(plan.target_store_dir.exists());
    engine.remove("test", "first", false).unwrap();
    assert!(plan.target_store_dir.exists());
    assert_eq!(engine.list("second").unwrap().len(), 1);
    let output = std::process::Command::new(layout.profile_bin_dir("second").join("test"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"ok\n");
    engine.remove("test", "second", false).unwrap();
    assert!(!plan.target_store_dir.exists());
}

#[test]
fn failed_upgrade_restores_previous_activation_and_allows_retry() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let engine = Engine::open(layout.clone()).unwrap();
    let first = temp.path().join("first.deb");
    let second = temp.path().join("second.deb");
    DebPackageBuilder::new("test")
        .version("1.0")
        .file("usr/bin/test", b"#!/bin/sh\necho old\n", 0o755)
        .write_to(&first)
        .unwrap();
    DebPackageBuilder::new("test")
        .version("2.0")
        .file("usr/bin/test", b"#!/bin/sh\necho new\n", 0o755)
        .write_to(&second)
        .unwrap();
    engine.install(&first, "default", false).unwrap();
    let trigger_db = rusqlite::Connection::open(engine.layout().db_path()).unwrap();
    trigger_db
        .execute(
            "CREATE TRIGGER injected_review_failure BEFORE INSERT ON store_objects BEGIN SELECT RAISE(FAIL, 'injected'); END",
            [],
        )
        .unwrap();
    assert!(engine.install(&second, "default", false).is_err());
    trigger_db
        .execute("DROP TRIGGER injected_review_failure", [])
        .unwrap();
    drop(trigger_db);
    drop(engine);
    let engine = Engine::open(layout.clone()).unwrap();
    let output = std::process::Command::new(layout.profile_bin_dir("default").join("test"))
        .output()
        .unwrap();
    assert_eq!(output.stdout, b"old\n");
    engine.install(&second, "default", false).unwrap();
}

#[test]
fn failed_upgrade_removes_new_activation_links_without_old_db_rows() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("data"));
    let engine = Engine::open(layout.clone()).unwrap();
    let first = temp.path().join("first.deb");
    let second = temp.path().join("second.deb");
    DebPackageBuilder::new("test")
        .version("1.0")
        .file("usr/bin/current", b"#!/bin/sh\necho old\n", 0o755)
        .write_to(&first)
        .unwrap();
    DebPackageBuilder::new("test")
        .version("2.0")
        .file("usr/bin/current", b"#!/bin/sh\necho new\n", 0o755)
        .file("usr/bin/new-command", b"#!/bin/sh\necho new\n", 0o755)
        .write_to(&second)
        .unwrap();
    engine.install(&first, "default", false).unwrap();
    let trigger_db = rusqlite::Connection::open(engine.layout().db_path()).unwrap();
    trigger_db
        .execute(
            "CREATE TRIGGER injected_review_failure BEFORE INSERT ON store_objects BEGIN SELECT RAISE(FAIL, 'injected'); END",
            [],
        )
        .unwrap();
    assert!(engine.install(&second, "default", false).is_err());
    trigger_db
        .execute("DROP TRIGGER injected_review_failure", [])
        .unwrap();
    drop(trigger_db);
    drop(engine);
    let engine = Engine::open(layout.clone()).unwrap();
    assert!(
        !layout
            .profile_bin_dir("default")
            .join("new-command")
            .exists()
    );
    assert!(layout.profile_bin_dir("default").join("current").exists());
    assert!(engine.install(&second, "default", false).is_ok());
}

#[test]
fn upgrade_removes_binaries_that_no_longer_exist() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let first = temp.path().join("first.deb");
    let second = temp.path().join("second.deb");
    DebPackageBuilder::new("test")
        .version("1.0")
        .file("usr/bin/current", b"#!/bin/sh\necho old\n", 0o755)
        .file("usr/bin/retired", b"#!/bin/sh\necho retired\n", 0o755)
        .write_to(&first)
        .unwrap();
    DebPackageBuilder::new("test")
        .version("2.0")
        .file("usr/bin/current", b"#!/bin/sh\necho new\n", 0o755)
        .write_to(&second)
        .unwrap();
    engine.install(&first, "default", false).unwrap();
    engine.install(&second, "default", false).unwrap();
    assert!(
        !engine
            .layout()
            .profile_bin_dir("default")
            .join("retired")
            .exists()
    );
    assert!(engine.remove("test", "default", false).is_ok());
}

#[test]
fn duplicate_data_members_are_rejected_before_install() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.deb");
    DebPackageBuilder::new("test")
        .file("usr/bin/test", b"payload", 0o755)
        .write_to(&source)
        .unwrap();
    let mut archive = ar::Archive::new(Cursor::new(fs::read(&source).unwrap()));
    let mut members = Vec::new();
    let mut duplicate = None;
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.unwrap();
        let name = entry.header().identifier().to_vec();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if String::from_utf8_lossy(&name)
            .trim()
            .starts_with("data.tar")
        {
            duplicate = Some((name.clone(), bytes.clone()));
        }
        members.push((name, bytes));
    }
    let duplicate = duplicate.unwrap();
    let mut output = Cursor::new(Vec::new());
    {
        let mut builder = ar::Builder::new(&mut output);
        for (name, bytes) in members.into_iter().chain(std::iter::once(duplicate)) {
            let mut header = ar::Header::new(name, bytes.len() as u64);
            header.set_mode(0o644);
            builder.append(&header, &bytes[..]).unwrap();
        }
    }
    let artifact = temp.path().join("duplicate.deb");
    fs::write(&artifact, output.into_inner()).unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    assert!(matches!(
        engine.install(&artifact, "default", false),
        Err(Error::MalformedArchive(_))
    ));
}

#[test]
fn unmanaged_and_modified_activation_paths_are_preserved() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let artifact = temp.path().join("test.deb");
    DebPackageBuilder::new("test")
        .file("usr/bin/test", b"#!/bin/sh\necho ok\n", 0o755)
        .write_to(&artifact)
        .unwrap();
    let link = engine.layout().profile_bin_dir("default").join("test");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    fs::write(&link, b"user file").unwrap();
    assert!(matches!(
        engine.install(&artifact, "default", false),
        Err(Error::ActivationConflict { .. })
    ));
    assert_eq!(fs::read(&link).unwrap(), b"user file");
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(temp.path().join("missing"), &link).unwrap();
    assert!(matches!(
        engine.install(&artifact, "default", false),
        Err(Error::ActivationConflict { .. })
    ));
    fs::remove_file(&link).unwrap();
    engine.install(&artifact, "default", false).unwrap();
    fs::remove_file(&link).unwrap();
    fs::write(&link, b"replacement").unwrap();
    engine.remove("test", "default", false).unwrap();
    assert_eq!(fs::read(&link).unwrap(), b"replacement");
}

#[test]
fn reusing_a_shared_store_does_not_accept_modified_payloads() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let artifact = temp.path().join("test.deb");
    DebPackageBuilder::new("test")
        .file("usr/bin/test", b"original", 0o755)
        .write_to(&artifact)
        .unwrap();
    let plan = engine.install(&artifact, "first", false).unwrap();
    fs::write(plan.target_store_dir.join("usr/bin/test"), b"modified").unwrap();
    assert!(matches!(
        engine.install(&artifact, "second", false),
        Err(Error::SecurityViolation(_))
    ));
    assert!(engine.list("second").unwrap().is_empty());
    assert_eq!(engine.list("first").unwrap().len(), 1);
}

// ELF64 with PT_LOAD, PT_DYNAMIC and one DT_NEEDED string, independent of host binaries.
fn elf_requiring(library: &str) -> Vec<u8> {
    let mut bytes = vec![0u8; 512];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    fn u16_at(b: &mut [u8], n: usize, v: u16) {
        b[n..n + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn u32_at(b: &mut [u8], n: usize, v: u32) {
        b[n..n + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn u64_at(b: &mut [u8], n: usize, v: u64) {
        b[n..n + 8].copy_from_slice(&v.to_le_bytes());
    }
    u16_at(&mut bytes, 16, 3);
    u16_at(&mut bytes, 18, 62);
    u32_at(&mut bytes, 20, 1);
    u64_at(&mut bytes, 32, 64);
    u16_at(&mut bytes, 52, 64);
    u16_at(&mut bytes, 54, 56);
    u16_at(&mut bytes, 56, 2);
    u32_at(&mut bytes, 64, 1);
    u32_at(&mut bytes, 68, 4);
    u64_at(&mut bytes, 96, 512);
    u64_at(&mut bytes, 104, 512);
    u32_at(&mut bytes, 120, 2);
    u32_at(&mut bytes, 124, 4);
    u64_at(&mut bytes, 128, 176);
    u64_at(&mut bytes, 136, 176);
    u64_at(&mut bytes, 152, 64);
    u64_at(&mut bytes, 160, 64);
    for (i, (tag, value)) in [(5, 256), (10, library.len() as u64 + 2), (1, 1), (0, 0)]
        .iter()
        .enumerate()
    {
        u64_at(&mut bytes, 176 + i * 16, *tag);
        u64_at(&mut bytes, 184 + i * 16, *value);
    }
    bytes[257..257 + library.len()].copy_from_slice(library.as_bytes());
    bytes
}

#[test]
fn invalid_elf_and_missing_libraries_fail_before_promotion() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    for (name, payload) in [
        ("invalid", b"\x7fELFbroken".to_vec()),
        ("missing", elf_requiring("libpkg-review-does-not-exist.so")),
    ] {
        let artifact = temp.path().join(format!("{name}.deb"));
        DebPackageBuilder::new(name)
            .file(format!("usr/bin/{name}"), payload, 0o755)
            .write_to(&artifact)
            .unwrap();
        let result = engine.install(&artifact, "default", false);
        if name == "invalid" {
            assert!(
                matches!(result, Err(Error::MalformedArchive(_))),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(Error::IncompatibleHost(_))),
                "{result:?}"
            );
        }
        assert!(engine.list("default").unwrap().is_empty());
        assert_eq!(
            fs::read_dir(engine.layout().store_dir()).unwrap().count(),
            0
        );
        assert!(
            !engine
                .layout()
                .profile_bin_dir("default")
                .join(name)
                .exists()
        );
    }
}
