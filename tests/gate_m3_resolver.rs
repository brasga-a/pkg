//! Gate M3 (Resolver) Release Gate Verification Suite.
//!
//! Verifies:
//! - Debian, RPM, and ALPM source version semantics are preserved without SemVer coercion (INV-008);
//! - Normalized constraint IR represents required dependency/capability/conflict fixtures (ADR-009);
//! - RPM artifact probing, metadata normalization, extraction, and scriptlet inventory under default-deny (ADR-011, ADR-017, INV-003, INV-004);
//! - ALPM artifact probing, `.PKGINFO` normalization, extraction, and scriptlet inventory under default-deny (ADR-011, ADR-017, INV-003, INV-004);
//! - Solver produces human-readable explanation chains on unsatisfied constraints (INV-020);
//! - Compatibility tests reject package-name-only false equivalence across ecosystems (ADR-016, INV-007);
//! - ELF/SONAME capability evidence can invalidate a nominal package-name match (INV-009);
//! - RPM and ALPM adapters enter the same normalized planning model without leaking source-specific types (ADR-005, ADR-017).

mod common;

use common::{AlpmPackageBuilder, RpmPackageBuilder};
use pkg_core::domain::capability::Capability;
use pkg_core::domain::constraint::{
    CapabilityConstraint, Constraint, VersionConstraint, VersionOp,
};
use pkg_core::domain::package::{
    Architecture, ArtifactDigest, NormalizedPackage, PackageFormat, PackageName, PackageVersion,
};
use pkg_core::domain::version::{
    VersionEcosystem, compare_alpm_versions, compare_debian_versions, compare_rpm_versions,
};
use pkg_core::format::{ExtractionLimits, detect_format, get_adapter};
use pkg_core::planner::Planner;
use pkg_core::resolver::Resolver;
use pkg_core::resolver::evidence::HostEvidence;
use pkg_core::resolver::explanation::RejectionReason;
use pkg_core::state::StateDatabase;
use pkg_core::store::StoreLayout;
use std::cmp::Ordering;
use tempfile::tempdir;

#[test]
fn test_gate_m3_source_version_semantics_preserved() {
    // 1. Debian source version ordering (epochs, tildes, revisions)
    assert_eq!(
        compare_debian_versions("1.0~rc1", "1.0"),
        Ordering::Less,
        "Debian tilde sorts earlier than base release"
    );
    assert_eq!(
        compare_debian_versions("1.0", "1.0-1"),
        Ordering::Less,
        "Debian release with revision sorts newer than unrevised"
    );
    assert_eq!(
        compare_debian_versions("1.0-1", "1.0-2"),
        Ordering::Less,
        "Debian revision ordering"
    );
    assert_eq!(
        compare_debian_versions("1.0-2", "1.1-1"),
        Ordering::Less,
        "Debian minor version bump"
    );
    assert_eq!(
        compare_debian_versions("1:1.0", "2.0"),
        Ordering::Greater,
        "Debian epoch 1 overrides higher upstream version"
    );

    // 2. RPM source version ordering (rpmvercmp: epochs, tildes, carets, dist tags)
    assert_eq!(
        compare_rpm_versions("1.0~rc1", "1.0"),
        Ordering::Less,
        "RPM tilde represents pre-release"
    );
    assert_eq!(
        compare_rpm_versions("1.0", "1.0^20240101"),
        Ordering::Less,
        "RPM caret represents snapshot post-release (newer than base)"
    );
    assert_eq!(
        compare_rpm_versions("1.0^20240101", "1.0.1"),
        Ordering::Less,
        "RPM caret sorts earlier than next numeric release"
    );
    assert_eq!(
        compare_rpm_versions("1.0-1.fc40", "1.0-2.fc40"),
        Ordering::Less,
        "RPM release comparison"
    );
    assert_eq!(
        compare_rpm_versions("2:1.0-1", "1:2.0-1"),
        Ordering::Greater,
        "RPM epoch overrides upstream version"
    );

    // 3. ALPM source version ordering (vercmp: epochs, revisions)
    assert_eq!(
        compare_alpm_versions("1.0-1", "1.0-2"),
        Ordering::Less,
        "ALPM pkgrel ordering"
    );
    assert_eq!(
        compare_alpm_versions("1.0-2", "1.1-1"),
        Ordering::Less,
        "ALPM pkgver ordering"
    );
    assert_eq!(
        compare_alpm_versions("1:1.0-1", "2.0-1"),
        Ordering::Greater,
        "ALPM epoch ordering"
    );

    // 4. Test VersionConstraint matching under native ecosystem rules (no SemVer coercion)
    let rpm_constraint =
        VersionConstraint::Relational(VersionOp::Greater, PackageVersion::new("1.0^20240101"));
    assert!(
        rpm_constraint.matches("1.0.1", VersionEcosystem::Rpm),
        "RPM caret constraint matching"
    );
    assert!(
        !rpm_constraint.matches("1.0", VersionEcosystem::Rpm),
        "RPM base release fails greater than caret snapshot"
    );

    let deb_constraint =
        VersionConstraint::Relational(VersionOp::GreaterEqual, PackageVersion::new("1:1.0"));
    assert!(
        deb_constraint.matches("1:1.1", VersionEcosystem::Debian),
        "Debian epoch constraint matching"
    );
    assert!(
        !deb_constraint.matches("2.0", VersionEcosystem::Debian),
        "Debian version without epoch fails constraint requiring epoch 1"
    );
}

#[test]
fn test_gate_m3_normalized_constraint_ir_representation() {
    let complex_constraint = Constraint::AllOf(vec![
        Constraint::Capability(CapabilityConstraint {
            identifier: "bin:sh".to_string(),
            version: VersionConstraint::Any,
            original_expression: "/bin/sh".to_string(),
        }),
        Constraint::AnyOf(vec![
            Constraint::Capability(CapabilityConstraint {
                identifier: "lib:libssl.so.3".to_string(),
                version: VersionConstraint::Any,
                original_expression: "libssl.so.3()(64bit)".to_string(),
            }),
            Constraint::Capability(CapabilityConstraint {
                identifier: "lib:libssl.so.1.1".to_string(),
                version: VersionConstraint::Any,
                original_expression: "libssl.so.1.1()(64bit)".to_string(),
            }),
        ]),
        Constraint::Package {
            name: PackageName::new("coreutils").unwrap(),
            version: VersionConstraint::Relational(
                VersionOp::GreaterEqual,
                PackageVersion::new("8.32"),
            ),
            ecosystem: "debian".to_string(),
            original_expression: "coreutils (>= 8.32)".to_string(),
        },
        Constraint::Conflict {
            target: "incompatible-tool".to_string(),
            version: VersionConstraint::Any,
            original_expression: "Conflicts: incompatible-tool".to_string(),
        },
    ]);

    let display_str = complex_constraint.to_string();
    assert!(display_str.contains("bin:sh"));
    assert!(display_str.contains("lib:libssl.so.3"));
    assert!(display_str.contains("lib:libssl.so.1.1"));
    assert!(display_str.contains("pkg:coreutils (>= 8.32)"));
    assert!(display_str.contains("conflict:incompatible-tool"));

    // Verify roundtrip serialization
    let serialized = serde_json::to_string(&complex_constraint).unwrap();
    let deserialized: Constraint = serde_json::from_str(&serialized).unwrap();
    assert_eq!(complex_constraint, deserialized);
}

#[test]
fn test_gate_m3_rpm_artifact_probing_metadata_and_scriptlet_default_deny() {
    let temp = tempdir().unwrap();
    let rpm_path = temp.path().join("fixture-tool-1.2.0-1.fc40.x86_64.rpm");

    // Build synthetic RPM fixture
    RpmPackageBuilder::new("fixture-tool")
        .version("1.2.0")
        .release("1.fc40")
        .architecture("x86_64")
        .requires("/bin/sh")
        .requires("libc.so.6")
        .provides("libfixture.so.1")
        .script("prein", "echo 'UNTRUSTED PREINSTALL SCRIPT'")
        .script("postin", "echo 'UNTRUSTED POSTINSTALL SCRIPT'")
        .file(
            "usr/bin/fixture-tool",
            b"#!/bin/sh\necho 'hello from rpm fixture'\n",
            0o755,
        )
        .file("usr/lib64/libfixture.so.1", b"FAKE_ELF_SO_BYTES", 0o644)
        .write_to(&rpm_path)
        .unwrap();

    // 1. Probing format
    let format = detect_format(&rpm_path).unwrap();
    assert_eq!(format, PackageFormat::Rpm);

    // 2. Metadata normalization
    let adapter = get_adapter(format);
    let pkg = adapter.parse_metadata(&rpm_path).unwrap();
    assert_eq!(pkg.name.as_str(), "fixture-tool");
    assert_eq!(pkg.version.as_str(), "1.2.0-1.fc40");
    assert_eq!(pkg.architecture, Architecture::X86_64);
    assert_eq!(pkg.format, PackageFormat::Rpm);

    // Verify provides normalized
    assert!(
        pkg.provides
            .iter()
            .any(|c| matches!(c, Capability::Executable(cmd) if cmd == "fixture-tool"))
    );
    assert!(
        pkg.provides
            .iter()
            .any(|c| matches!(c, Capability::SharedLibrary(lib) if lib == "libfixture.so.1"))
    );

    // Verify scriptlets inventoried but NOT executed (INV-003 default-deny)
    assert_eq!(pkg.scripts.len(), 2);
    let script_names: Vec<_> = pkg.scripts.iter().map(|s| s.name.as_str()).collect();
    assert!(script_names.contains(&"prein"));
    assert!(script_names.contains(&"postin"));

    // 3. Payload extraction under limits
    let dest = temp.path().join("store-object");
    let limits = ExtractionLimits::default();
    let report = adapter.extract_payload(&rpm_path, &dest, &limits).unwrap();
    assert!(
        report
            .extracted_files
            .contains(&std::path::PathBuf::from("usr/bin/fixture-tool"))
    );
    assert!(dest.join("usr/bin/fixture-tool").is_file());
}

#[test]
fn test_gate_m3_alpm_artifact_probing_metadata_and_scriptlet_default_deny() {
    let temp = tempdir().unwrap();
    let alpm_path = temp.path().join("arch-tool-2.1.0-1-x86_64.pkg.tar.zst");

    // Build synthetic ALPM fixture
    AlpmPackageBuilder::new("arch-tool")
        .version("2.1.0-1")
        .architecture("x86_64")
        .depend("glibc>=2.34")
        .provides("libarch.so.1")
        .conflict("legacy-arch-tool")
        .install_script("post_install() { echo 'UNTRUSTED ARCH SCRIPT'; }")
        .file(
            "usr/bin/arch-tool",
            b"#!/bin/sh\necho 'hello from arch'\n",
            0o755,
        )
        .write_to(&alpm_path)
        .unwrap();

    // 1. Probing format
    let format = detect_format(&alpm_path).unwrap();
    assert_eq!(format, PackageFormat::Alpm);

    // 2. Metadata normalization
    let adapter = get_adapter(format);
    let pkg = adapter.parse_metadata(&alpm_path).unwrap();
    assert_eq!(pkg.name.as_str(), "arch-tool");
    assert_eq!(pkg.version.as_str(), "2.1.0-1");
    assert_eq!(pkg.architecture, Architecture::X86_64);
    assert_eq!(pkg.format, PackageFormat::Alpm);

    // Verify constraints and provides
    assert!(
        pkg.provides
            .iter()
            .any(|c| matches!(c, Capability::Executable(cmd) if cmd == "arch-tool"))
    );
    assert!(
        pkg.provides
            .iter()
            .any(|c| matches!(c, Capability::SharedLibrary(lib) if lib == "libarch.so.1"))
    );
    assert!(
        pkg.constraints.iter().any(
            |c| matches!(c, Constraint::Conflict { target, .. } if target == "legacy-arch-tool")
        )
    );

    // Verify scriptlet inventoried but default-deny enforced (INV-003)
    assert_eq!(pkg.scripts.len(), 1);
    assert_eq!(pkg.scripts[0].name, "install");
    assert!(pkg.scripts[0].content.contains("UNTRUSTED ARCH SCRIPT"));

    // 3. Payload extraction
    let dest = temp.path().join("store-object-alpm");
    let limits = ExtractionLimits::default();
    let report = adapter.extract_payload(&alpm_path, &dest, &limits).unwrap();
    assert!(
        report
            .extracted_files
            .contains(&std::path::PathBuf::from("usr/bin/arch-tool"))
    );
    assert!(dest.join("usr/bin/arch-tool").is_file());
}

#[test]
fn test_gate_m3_solver_explanation_chain_on_unsatisfied_constraints() {
    let host = HostEvidence::builder()
        .architecture(Architecture::X86_64)
        .add_library("libc.so.6", Some("2.34"), &["GLIBC_2.34"])
        .build();

    let resolver = Resolver::new(host);

    // Target requires libc.so.6 >= 2.38, but host only provides 2.34
    let target = NormalizedPackage {
        name: PackageName::new("modern-tool").unwrap(),
        version: PackageVersion::new("1.0.0"),
        architecture: Architecture::X86_64,
        format: PackageFormat::Rpm,
        digest: ArtifactDigest::sha256("11".repeat(32)),
        size_bytes: 2048,
        description: None,
        dependencies: Vec::new(),
        constraints: vec![Constraint::Capability(CapabilityConstraint {
            identifier: "lib:libc.so.6".to_string(),
            version: VersionConstraint::Relational(
                VersionOp::GreaterEqual,
                PackageVersion::new("2.38"),
            ),
            original_expression: "libc.so.6(GLIBC_2.38)(64bit)".to_string(),
        })],
        provides: Vec::new(),
        versioned_provides: Vec::new(),
        scripts: Vec::new(),
        entries: Vec::new(),
        installed_size: None,
    };

    let err = resolver.resolve(&target).unwrap_err();

    // Verify structured explanation chain (INV-020)
    assert_eq!(err.chain.root_target, "modern-tool-1.0.0");
    assert!(
        err.chain
            .has_rejection(|r| matches!(r, RejectionReason::IncompatibleVersion { .. }))
    );

    let explanation_text = err.to_string();
    assert!(
        explanation_text.contains("Resolution failed for 'modern-tool-1.0.0'"),
        "Explanation text should identify target"
    );
    assert!(
        explanation_text.contains("does not satisfy '>= 2.38'"),
        "Explanation text should describe version deficiency"
    );
}

#[test]
fn test_gate_m3_rejection_of_false_package_name_equivalence() {
    let host = HostEvidence::builder()
        .architecture(Architecture::X86_64)
        .build();

    // In repository: Fedora package named "glibc" (RPM)
    let fedora_glibc = NormalizedPackage {
        name: PackageName::new("glibc").unwrap(),
        version: PackageVersion::new("2.38-1.fc40"),
        architecture: Architecture::X86_64,
        format: PackageFormat::Rpm,
        digest: ArtifactDigest::sha256("22".repeat(32)),
        size_bytes: 4096,
        description: None,
        dependencies: Vec::new(),
        constraints: Vec::new(),
        provides: vec![Capability::SharedLibrary("libc.so.6".to_string())],
        versioned_provides: Vec::new(),
        scripts: Vec::new(),
        entries: Vec::new(),
        installed_size: None,
    };

    let resolver = Resolver::new(host).with_repository_packages(vec![fedora_glibc]);

    // Target Debian package requires direct Debian package dependency `glibc`
    let debian_target = NormalizedPackage {
        name: PackageName::new("deb-app").unwrap(),
        version: PackageVersion::new("1.0.0"),
        architecture: Architecture::X86_64,
        format: PackageFormat::Deb,
        digest: ArtifactDigest::sha256("33".repeat(32)),
        size_bytes: 1024,
        description: None,
        dependencies: Vec::new(),
        constraints: vec![Constraint::Package {
            name: PackageName::new("glibc").unwrap(),
            version: VersionConstraint::Any,
            ecosystem: "debian".to_string(),
            original_expression: "glibc".to_string(),
        }],
        provides: Vec::new(),
        versioned_provides: Vec::new(),
        scripts: Vec::new(),
        entries: Vec::new(),
        installed_size: None,
    };

    let err = resolver.resolve(&debian_target).unwrap_err();

    // ADR-016 / INV-007: Package-name equality across distros never proves satisfaction
    assert!(
        err.chain
            .has_rejection(|r| matches!(r, RejectionReason::FalseEquivalence { .. }))
    );
    let text = err.to_string();
    assert!(
        text.contains("INV-007"),
        "Explanation chain must cite INV-007 invariant violation"
    );
    assert!(text.contains("nominal match across distributions rejected"));
}

#[test]
fn test_gate_m3_elf_abi_evidence_invalidates_nominal_match() {
    let host = HostEvidence::builder()
        .architecture(Architecture::X86_64)
        .build();

    let resolver = Resolver::new(host);

    let temp = tempdir().unwrap();
    let so_path = temp.path().join("libsample.so");
    let bin_path = temp.path().join("sample_bin");

    let has_gcc = std::process::Command::new("gcc")
        .arg("-v")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_gcc {
        // 1. Compile real shared library
        let src_so = temp.path().join("so.c");
        std::fs::write(&src_so, "int sample_fn() { return 100; }\n").unwrap();
        let status1 = std::process::Command::new("gcc")
            .args([
                "-shared",
                "-fPIC",
                "-o",
                so_path.to_str().unwrap(),
                src_so.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status1.success());

        // 2. Compile binary with DT_NEEDED pointing to libsample.so
        let src_bin = temp.path().join("main.c");
        std::fs::write(
            &src_bin,
            "int sample_fn(); int main() { return sample_fn(); }\n",
        )
        .unwrap();
        let status2 = std::process::Command::new("gcc")
            .args([
                "-o",
                bin_path.to_str().unwrap(),
                src_bin.to_str().unwrap(),
                "-L",
                temp.path().to_str().unwrap(),
                "-lsample",
            ])
            .status()
            .unwrap();
        assert!(status2.success());

        // 3. Remove libsample.so so it is absent from standard host paths
        let _ = std::fs::remove_file(&so_path);

        // Scenario A: Candidate package named "libsample" is present in closure,
        // but does NOT provide Capability::SharedLibrary("libsample.so")
        let nominal_pkg = NormalizedPackage {
            name: PackageName::new("libsample").unwrap(),
            version: PackageVersion::new("1.0.0"),
            architecture: Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: ArtifactDigest::sha256("55".repeat(32)),
            size_bytes: 1024,
            description: None,
            dependencies: Vec::new(),
            constraints: Vec::new(),
            provides: vec![Capability::Feature("sample-token".to_string())],
            versioned_provides: Vec::new(),
            scripts: Vec::new(),
            entries: Vec::new(),
            installed_size: None,
        };

        let err = resolver
            .validate_elf_evidence(&bin_path, &[nominal_pkg], None)
            .unwrap_err();

        // Nominal package match is invalidated by missing SONAME evidence (INV-009)
        assert!(
            err.chain
                .has_rejection(|r| matches!(r, RejectionReason::ElfAbiMismatch { .. }))
        );
        let err_text = err.to_string();
        assert!(err_text.contains("libsample.so"));
        assert!(err_text.contains("INV-009"));

        // Scenario B: Providing actual verified Capability::SharedLibrary("libsample.so") in closure satisfies it
        let valid_pkg = NormalizedPackage {
            name: PackageName::new("libsample").unwrap(),
            version: PackageVersion::new("1.0.0"),
            architecture: Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: ArtifactDigest::sha256("55".repeat(32)),
            size_bytes: 1024,
            description: None,
            dependencies: Vec::new(),
            constraints: Vec::new(),
            provides: vec![Capability::SharedLibrary("libsample.so".to_string())],
            versioned_provides: Vec::new(),
            scripts: Vec::new(),
            entries: Vec::new(),
            installed_size: None,
        };

        assert!(
            resolver
                .validate_elf_evidence(&bin_path, &[valid_pkg], None)
                .is_ok()
        );
    } else {
        let fake_elf = temp.path().join("fake_elf");
        std::fs::write(&fake_elf, b"\x7fELF_CORRUPT").unwrap();
        let err = resolver
            .validate_elf_evidence(&fake_elf, &[], None)
            .unwrap_err();
        assert!(
            err.chain
                .has_rejection(|r| matches!(r, RejectionReason::ElfAbiMismatch { .. }))
        );
    }
}

#[test]
fn test_gate_m3_rpm_and_alpm_adapters_enter_same_normalized_planning_model() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path());
    let db = StateDatabase::open(&layout.db_path()).unwrap();
    let profile = "default";

    // 1. Build and plan RPM
    let rpm_path = temp.path().join("plan-test.rpm");
    RpmPackageBuilder::new("plan-rpm-pkg")
        .version("1.0.0")
        .release("1.fc40")
        .architecture("x86_64")
        .script("postin", "echo 'ignored postinst'")
        .file(
            "usr/bin/rpm-planned-cmd",
            b"#!/bin/sh\necho rpm planned\n",
            0o755,
        )
        .write_to(&rpm_path)
        .unwrap();

    let rpm_plan = Planner::plan_install(&rpm_path, &layout, &db, profile, false).unwrap();
    assert_eq!(rpm_plan.package.name.as_str(), "plan-rpm-pkg");
    assert_eq!(rpm_plan.binaries.len(), 1);
    assert_eq!(rpm_plan.binaries[0].command, "rpm-planned-cmd");
    assert_eq!(rpm_plan.ignored_scripts.len(), 1);
    assert!(
        rpm_plan
            .target_store_dir
            .to_str()
            .unwrap()
            .contains("plan-rpm-pkg")
    );

    // 2. Build and plan ALPM
    let alpm_path = temp.path().join("plan-test.pkg.tar.zst");
    AlpmPackageBuilder::new("plan-alpm-pkg")
        .version("2.0.0-1")
        .architecture("x86_64")
        .install_script("post_install() { echo 'ignored alpm'; }")
        .file(
            "usr/bin/alpm-planned-cmd",
            b"#!/bin/sh\necho alpm planned\n",
            0o755,
        )
        .write_to(&alpm_path)
        .unwrap();

    let alpm_plan = Planner::plan_install(&alpm_path, &layout, &db, profile, false).unwrap();
    assert_eq!(alpm_plan.package.name.as_str(), "plan-alpm-pkg");
    assert_eq!(alpm_plan.binaries.len(), 1);
    assert_eq!(alpm_plan.binaries[0].command, "alpm-planned-cmd");
    assert_eq!(alpm_plan.ignored_scripts.len(), 1);
    assert!(
        alpm_plan
            .target_store_dir
            .to_str()
            .unwrap()
            .contains("plan-alpm-pkg")
    );

    // Both plans conform to the identical InstallPlan domain contract (ADR-005, ADR-017)
    assert_eq!(rpm_plan.package.architecture, Architecture::X86_64);
    assert_eq!(alpm_plan.package.architecture, Architecture::X86_64);
}
