use pkg_core::{Engine, InstallOptions, StoreLayout};
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/alpm-python-runtime/alpm-python-runtime-fixture-1.0.0-1-x86_64.pkg.tar.zst")
}

#[test]
fn test_python_runtime_launcher_and_scoped_pythonpath() {
    let temp = tempdir().unwrap();
    let pkg_path = fixture_path();
    assert!(pkg_path.is_file(), "missing ALPM Python runtime fixture: {}", pkg_path.display());

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

    assert_eq!(plan.package.name.as_str(), "alpm-python-runtime-fixture");
    assert_eq!(plan.binaries.len(), 1);
    assert_eq!(plan.binaries[0].command, "alpm-python-runtime-fixture");
    assert_eq!(
        plan.binaries[0].relative_store_path,
        PathBuf::from(".pkg-launcher/alpm-python-runtime-fixture")
    );

    let profile_bin = engine
        .layout()
        .profile_bin_dir("default")
        .join("alpm-python-runtime-fixture");
    assert!(profile_bin.is_symlink());

    let output = Command::new(&profile_bin)
        .arg("--arg1")
        .arg("val2")
        .output()
        .expect("failed to execute ALPM Python fixture via runtime launcher");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "command failed with status: {:?}\nstderr: {}\nstdout: {}",
        output.status,
        stderr,
        stdout
    );
    assert!(stdout.contains("MODULE_VAL=RUNTIME_LAUNCHER_SUCCESS"));
    assert!(stdout.contains("ARG=--arg1"));
    assert!(stdout.contains("ARG=val2"));
}
