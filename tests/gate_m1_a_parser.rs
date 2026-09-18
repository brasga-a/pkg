//! Gate M1-A (Parser) Release Gate Verification Suite.
//!
//! Verifies:
//! - malformed/truncated `.deb` corpus is rejected safely;
//! - extraction rejects absolute paths, `..` traversal, unsafe symlink/hardlink escapes, and configured expansion limits;
//! - maintainer scripts are inventoried but never executed;
//! - architecture mismatch is detected before promotion;
//! - normalized metadata is produced without invoking `dpkg`.

#[path = "common/mod.rs"]
mod common;

use common::DebPackageBuilder;
use pkg_core::domain::package::{Architecture, PackageFormat};
use pkg_core::error::Error;
use pkg_core::format::deb::DebAdapter;
use pkg_core::format::{ArtifactAdapter, ExtractionLimits};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_malformed_truncated_deb_rejected() {
    let temp = tempdir().unwrap();
    let bad_deb = temp.path().join("corrupt.deb");
    fs::write(&bad_deb, b"!<arch>\ntruncated-and-invalid").unwrap();

    let adapter = DebAdapter::new();
    let result = adapter.parse_metadata(&bad_deb);
    assert!(result.is_err(), "Corrupted archive must be rejected");

    let dest = temp.path().join("extract");
    let extract_res = adapter.extract_payload(&bad_deb, &dest, &ExtractionLimits::default());
    assert!(
        extract_res.is_err(),
        "Corrupted archive extraction must fail"
    );
}

#[test]
fn test_corrupt_debian_binary_rejected() {
    let temp = tempdir().unwrap();
    let bad_deb = temp.path().join("bad_version.deb");

    DebPackageBuilder::new("testpkg")
        .corrupt_debian_binary()
        .write_to(&bad_deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let result = adapter.parse_metadata(&bad_deb);
    assert!(
        matches!(result, Err(Error::MalformedArchive(_))),
        "Invalid debian-binary version must yield MalformedArchive"
    );
}

#[test]
fn test_absolute_path_in_payload_rejected() {
    let temp = tempdir().unwrap();
    let evil_deb = temp.path().join("absolute_path.deb");

    DebPackageBuilder::new("evilpkg")
        .file("/etc/evil.conf", b"malicious", 0o644)
        .write_to(&evil_deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let dest = temp.path().join("extract");
    let result = adapter.extract_payload(&evil_deb, &dest, &ExtractionLimits::default());

    assert!(
        matches!(result, Err(Error::SecurityViolation(_))),
        "Absolute payload path must be rejected with SecurityViolation, got: {result:?}"
    );
    assert!(!dest.join("etc/evil.conf").exists());
}

#[test]
fn test_path_traversal_in_payload_rejected() {
    let temp = tempdir().unwrap();
    let evil_deb = temp.path().join("traversal.deb");

    DebPackageBuilder::new("evilpkg")
        .file("./usr/bin/../../etc/passwd", b"malicious", 0o644)
        .write_to(&evil_deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let dest = temp.path().join("extract");
    let result = adapter.extract_payload(&evil_deb, &dest, &ExtractionLimits::default());

    assert!(
        matches!(result, Err(Error::SecurityViolation(_))),
        "Traversal '..' path must be rejected with SecurityViolation, got: {result:?}"
    );
}

#[test]
fn test_escaping_symlink_rejected() {
    let temp = tempdir().unwrap();
    let evil_deb = temp.path().join("escaping_symlink.deb");

    DebPackageBuilder::new("evilpkg")
        .symlink("./usr/bin/evil_link", "../../../../etc/shadow")
        .write_to(&evil_deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let dest = temp.path().join("extract");
    let result = adapter.extract_payload(&evil_deb, &dest, &ExtractionLimits::default());

    assert!(
        matches!(result, Err(Error::SecurityViolation(_))),
        "Escaping symlink must be rejected with SecurityViolation, got: {result:?}"
    );
}

#[test]
fn test_extraction_limits_enforced() {
    let temp = tempdir().unwrap();
    let deb = temp.path().join("large_payload.deb");

    let payload = vec![b'A'; 1024 * 1024]; // 1 MB
    DebPackageBuilder::new("bigpkg")
        .file("./usr/share/big.dat", payload, 0o644)
        .write_to(&deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let dest = temp.path().join("extract");
    // Limit to 500 KB
    let limits = ExtractionLimits {
        max_entries: 100,
        max_total_bytes: 500 * 1024,
        max_single_file_bytes: 500 * 1024,
    };

    let result = adapter.extract_payload(&deb, &dest, &limits);
    assert!(
        matches!(result, Err(Error::LimitsExceeded(_))),
        "Extraction exceeding byte limit must be rejected with LimitsExceeded"
    );
}

#[test]
fn test_maintainer_scripts_inventoried_never_executed() {
    let temp = tempdir().unwrap();
    let deb = temp.path().join("scripted.deb");
    let canary_file = temp.path().join("canary_executed");

    let evil_script = format!("touch {}", canary_file.display());

    DebPackageBuilder::new("scriptpkg")
        .script("preinst", &evil_script)
        .script("postinst", &evil_script)
        .file("./usr/bin/app", b"#!/bin/sh\necho ok", 0o755)
        .write_to(&deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let meta = adapter.parse_metadata(&deb).unwrap();

    assert_eq!(meta.scripts.len(), 2);
    let names: Vec<&str> = meta.scripts.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"preinst"));
    assert!(names.contains(&"postinst"));

    // Extract payload
    let dest = temp.path().join("extract");
    let _ = adapter
        .extract_payload(&deb, &dest, &ExtractionLimits::default())
        .unwrap();

    // Verify script was NEVER executed (INV-003, ADR-011)
    assert!(
        !canary_file.exists(),
        "Maintainer scripts must NEVER be executed during parse or extract"
    );
}

#[test]
fn test_architecture_mismatch_detected() {
    let temp = tempdir().unwrap();
    let deb = temp.path().join("arm64_pkg.deb");

    DebPackageBuilder::new("armpkg")
        .architecture("arm64")
        .write_to(&deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let meta = adapter.parse_metadata(&deb).unwrap();
    assert_eq!(meta.architecture, Architecture::Other("arm64".to_string()));

    // When planning on host x86_64, mismatch must be detected
    let layout = pkg_core::StoreLayout::new(temp.path().join("pkg_data"));
    layout.ensure_dirs().unwrap();
    let db = pkg_core::state::StateDatabase::open(&layout.db_path()).unwrap();

    let plan_res = pkg_core::planner::Planner::plan_install(&deb, &layout, &db, "default", true);
    assert!(
        matches!(plan_res, Err(Error::ArchitectureMismatch { .. })),
        "Architecture mismatch must be detected before promotion"
    );
}

#[test]
fn test_normalized_metadata_produced_without_dpkg() {
    let temp = tempdir().unwrap();
    let deb = temp.path().join("good_pkg.deb");

    DebPackageBuilder::new("ripgrep")
        .version("14.1.0-1")
        .architecture("amd64")
        .depends("libc6 (>= 2.34)")
        .file("./usr/bin/rg", b"\x7fELFfakebinary", 0o755)
        .write_to(&deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let meta = adapter.parse_metadata(&deb).unwrap();

    assert_eq!(meta.name.as_str(), "ripgrep");
    assert_eq!(meta.version.as_str(), "14.1.0-1");
    assert_eq!(meta.architecture, Architecture::X86_64);
    assert_eq!(meta.format, PackageFormat::Deb);
    assert!(meta.digest.hex().len() >= 32);
    assert_eq!(meta.dependencies.len(), 1);
    assert_eq!(meta.dependencies[0].name, "libc6");
    assert_eq!(
        meta.dependencies[0].version_constraint.as_deref(),
        Some(">= 2.34")
    );
    assert_eq!(
        meta.provides,
        vec![pkg_core::domain::Capability::Executable("rg".to_string())]
    );
}

#[test]
fn test_dangling_or_cross_package_symlinks_within_staging_accepted() {
    let temp = tempdir().unwrap();
    let deb = temp.path().join("dangling_link.deb");

    // E.g. virt-manager having usr/share/doc/virt-manager/NEWS.md.gz -> ../virtinst/NEWS.md.gz
    DebPackageBuilder::new("virt-manager")
        .file("usr/share/doc/virt-manager/README", b"readme", 0o644)
        .symlink(
            "usr/share/doc/virt-manager/NEWS.md.gz",
            "../virtinst/NEWS.md.gz",
        )
        .write_to(&deb)
        .unwrap();

    let adapter = DebAdapter::new();
    let dest = temp.path().join("extract");
    let extract_res = adapter.extract_payload(&deb, &dest, &ExtractionLimits::default());
    assert!(
        extract_res.is_ok(),
        "Non-escaping symlinks pointing to cross-package/dangling targets must be accepted: {:?}",
        extract_res.err()
    );
}
