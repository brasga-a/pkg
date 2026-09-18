//! Tests for preflight inspection and install options (e.g. allow_missing_libraries).

mod common;

use common::deb_builder::DebPackageBuilder;
use pkg_core::{Engine, Error, InstallOptions, StoreLayout};
use tempfile::tempdir;

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
fn test_preflight_and_install_options() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();

    let missing_lib = "libpkg-missing-custom.so.1";
    let artifact = temp.path().join("missing-test.deb");
    DebPackageBuilder::new("missing-test")
        .file("usr/bin/missing-cmd", elf_requiring(missing_lib), 0o755)
        .write_to(&artifact)
        .unwrap();

    // 1. Preflight check detects missing library without installing
    let preflight = engine.preflight_check(&artifact).unwrap();
    assert_eq!(preflight.package.name.as_str(), "missing-test");
    assert!(
        preflight
            .missing_libraries
            .contains(&missing_lib.to_string())
    );

    // Store and DB remain untouched after preflight
    assert!(engine.list("default").unwrap().is_empty());

    // 2. Default install rejects missing libraries
    let err = engine.install(&artifact, "default", false).unwrap_err();
    assert!(matches!(err, Error::IncompatibleHost(_)));

    // 3. install_with_options(allow_missing_libraries: true) succeeds
    let plan = engine
        .install_with_options(
            &artifact,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: true,
                skip_dependencies: false,
            },
        )
        .unwrap();

    assert!(plan.missing_libraries.contains(&missing_lib.to_string()));
    assert_eq!(plan.package.name.as_str(), "missing-test");

    let list = engine.list("default").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name.as_str(), "missing-test");

    // Profile activation exists
    let bin_path = engine
        .layout()
        .profile_bin_dir("default")
        .join("missing-cmd");
    assert!(bin_path.exists());

    let generation = engine.db().active_generation("default").unwrap().unwrap();
    let generation_manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            engine
                .layout()
                .profile_generations_dir("default")
                .join(generation.generation_id)
                .join("manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        generation_manifest["verified"],
        serde_json::Value::Bool(false)
    );
    assert!(engine.run_command("default", "missing-cmd", &[]).is_err());
}

#[test]
fn test_install_options_skip_dependencies() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("pkg-with-deps.deb");

    DebPackageBuilder::new("app-with-deps")
        .architecture("amd64")
        .depends("non-existent-dependency-xyz (>= 1.0.0)")
        .file("usr/bin/my-app", b"#!/bin/sh\necho hello\n", 0o755)
        .write_to(&artifact)
        .unwrap();

    let layout = StoreLayout::new(temp.path().join("data"));
    let engine = Engine::open(layout).unwrap();

    // Default install: fails due to unresolvable dependency
    let err = engine.install_with_options(
        &artifact,
        "default",
        false,
        InstallOptions {
            allow_missing_libraries: false,
            skip_dependencies: false,
        },
    );
    assert!(err.is_err(), "should fail resolving missing dependency");

    // With skip_dependencies: true, installation succeeds
    let plan = engine
        .install_with_options(
            &artifact,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: false,
                skip_dependencies: true,
            },
        )
        .expect("should succeed when skipping dependencies");

    assert_eq!(plan.package.name.as_str(), "app-with-deps");
    assert_eq!(plan.resolved_dependencies.len(), 0);

    let list = engine.list("default").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name.as_str(), "app-with-deps");
}
