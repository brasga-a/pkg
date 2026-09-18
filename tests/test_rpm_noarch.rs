mod common;

use common::RpmPackageBuilder;
use pkg_core::domain::package::{Architecture, PackageFormat};
use pkg_core::format::get_adapter;
use pkg_core::{Engine, InstallOptions, StoreLayout};
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_rpm_noarch_metadata_and_installation() {
    let temp = tempdir().unwrap();
    let rpm_path = temp.path().join("noarch-test-tool-1.0.0-1.fc41.noarch.rpm");

    RpmPackageBuilder::new("noarch-test-tool")
        .version("1.0.0")
        .release("1.fc41")
        .architecture("noarch")
        .file(
            "usr/bin/noarch-test-tool",
            b"#!/bin/sh\necho 'noarch payload executed successfully'\n",
            0o755,
        )
        .write_to(&rpm_path)
        .unwrap();

    // 1. Verify metadata parses as Architecture::All
    let adapter = get_adapter(PackageFormat::Rpm);
    let meta = adapter.parse_metadata(&rpm_path).unwrap();
    assert_eq!(meta.name.as_str(), "noarch-test-tool");
    assert_eq!(meta.architecture, Architecture::All);

    // 2. Verify engine installs without architecture mismatch
    let store_dir = temp.path().join("store");
    let engine = Engine::open(StoreLayout::new(store_dir)).unwrap();
    let plan = engine
        .install_with_options(
            &rpm_path,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: false,
                skip_dependencies: false,
            },
        )
        .unwrap();

    assert_eq!(plan.package.name.as_str(), "noarch-test-tool");
    assert_eq!(plan.binaries.len(), 1);
    assert_eq!(plan.binaries[0].command, "noarch-test-tool");

    // 3. Execute activated binary through profile
    let bin_path = engine
        .layout()
        .profile_bin_dir("default")
        .join("noarch-test-tool");
    assert!(bin_path.is_symlink());

    let output = Command::new(&bin_path)
        .output()
        .expect("Failed to execute installed noarch binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("noarch payload executed successfully"));
}
