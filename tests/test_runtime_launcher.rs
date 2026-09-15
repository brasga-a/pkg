mod common;

use common::AlpmPackageBuilder;
use pkg_core::{Engine, InstallOptions, StoreLayout};
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_python_runtime_launcher_and_scoped_pythonpath() {
    let temp = tempdir().unwrap();
    let pkg_path = temp.path().join("pytool-1.0.0-1-x86_64.pkg.tar.zst");

    let script_content = r#"#!/usr/bin/python
import sys
import my_test_module

print(f"MODULE_VAL={my_test_module.GREETING}")
for arg in sys.argv[1:]:
    print(f"ARG={arg}")
"#;

    let module_content = r#"GREETING = "RUNTIME_LAUNCHER_SUCCESS"
"#;

    AlpmPackageBuilder::new("pytool")
        .version("1.0.0-1")
        .file("usr/bin/pytool", script_content.as_bytes(), 0o755)
        .file(
            "usr/lib/python3.14/site-packages/my_test_module.py",
            module_content.as_bytes(),
            0o644,
        )
        .write_to(&pkg_path)
        .unwrap();

    let store_dir = temp.path().join("store");
    let engine = Engine::open(StoreLayout::new(store_dir)).unwrap();

    let plan = engine
        .install_with_options(
            &pkg_path,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: false,
            },
        )
        .unwrap();

    assert_eq!(plan.package.name.as_str(), "pytool");
    assert_eq!(plan.binaries.len(), 1);
    assert_eq!(plan.binaries[0].command, "pytool");
    assert_eq!(
        plan.binaries[0].relative_store_path,
        std::path::PathBuf::from(".pkg-launcher/pytool")
    );

    // Profile symlink must point to the launcher
    let profile_bin = engine.layout().profile_bin_dir("default").join("pytool");
    assert!(profile_bin.is_symlink());

    // Execute through profile symlink with arguments
    let output = Command::new(&profile_bin)
        .arg("--arg1")
        .arg("val2")
        .output()
        .expect("Failed to execute pytool via launcher");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Command failed with status: {:?}\nstderr: {}\nstdout: {}",
        output.status,
        stderr,
        stdout
    );
    assert!(stdout.contains("MODULE_VAL=RUNTIME_LAUNCHER_SUCCESS"));
    assert!(stdout.contains("ARG=--arg1"));
    assert!(stdout.contains("ARG=val2"));
}
