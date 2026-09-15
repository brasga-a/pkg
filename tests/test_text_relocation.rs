mod common;

use common::AlpmPackageBuilder;
use pkg_core::{Engine, InstallOptions, StoreLayout};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_text_relocation_in_scripts_and_desktop_files() {
    let temp = tempdir().unwrap();
    let pkg_path = temp.path().join("fhsapp-1.0.0-1-x86_64.pkg.tar.zst");

    // Shell script with hardcoded /usr/share paths, variable expansion, and host paths
    let script_content = r#"#!/bin/sh
BOOTSTRAP_SUFFIX=fhsapp/helper.sh
helper=/usr/share/$BOOTSTRAP_SUFFIX

if [ ! -x "$helper" ]; then
    echo "ERROR: Helper not found at $helper" >&2
    exit 1
fi

# Ensure host paths are preserved
if [ ! -f /etc/os-release ]; then
    echo "ERROR: Host /etc/os-release broken!" >&2
    exit 2
fi

exec "$helper" "$@"
"#;

    // Helper script in usr/share/fhsapp/helper.sh
    let helper_content = r#"#!/bin/sh
echo "HELPER_INVOKED_SUCCESSFULLY: $@"
"#;

    // Desktop file
    let desktop_content = r#"[Desktop Entry]
Name=FHS App
Exec=/usr/bin/fhsapp --start
Icon=fhsapp
Type=Application
"#;

    AlpmPackageBuilder::new("fhsapp")
        .version("1.0.0-1")
        .file("usr/bin/fhsapp", script_content.as_bytes(), 0o755)
        .file(
            "usr/share/fhsapp/helper.sh",
            helper_content.as_bytes(),
            0o755,
        )
        .file(
            "usr/share/applications/fhsapp.desktop",
            desktop_content.as_bytes(),
            0o644,
        )
        .write_to(&pkg_path)
        .unwrap();

    let store_dir = temp.path().join("store");
    let engine = Engine::open(StoreLayout::new(store_dir.clone())).unwrap();

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

    assert_eq!(plan.package.name.as_str(), "fhsapp");
    assert_eq!(plan.binaries.len(), 1);
    assert_eq!(plan.binaries[0].command, "fhsapp");

    // Profile symlink must exist
    let profile_bin = engine.layout().profile_bin_dir("default").join("fhsapp");
    assert!(profile_bin.is_symlink());

    // Execute through profile symlink: should successfully find relocated helper.sh!
    let output = Command::new(&profile_bin)
        .arg("hello-relocation")
        .output()
        .expect("Failed to execute fhsapp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Command failed with status: {:?}\nstderr: {}\nstdout: {}",
        output.status,
        stderr,
        stdout
    );
    assert!(stdout.contains("HELPER_INVOKED_SUCCESSFULLY: hello-relocation"));

    // Check that desktop file was relocated
    let installed_desktop = plan
        .target_store_dir
        .join("usr/share/applications/fhsapp.desktop");
    let desktop_text = fs::read_to_string(&installed_desktop).unwrap();
    let expected_exec = format!(
        "Exec={}/usr/bin/fhsapp --start",
        plan.target_store_dir.display()
    );
    assert!(
        desktop_text.contains(&expected_exec),
        "Desktop file was not relocated. Got:\n{desktop_text}"
    );
}
