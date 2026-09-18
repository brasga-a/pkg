#[path = "common/mod.rs"]
mod common;

use pkg_core::{Engine, InstallOptions, StoreLayout};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/alpm-python-runtime/alpm-python-runtime-fixture-1.0.0-1-x86_64.pkg.tar.zst")
}

#[test]
fn test_python_runtime_launcher_and_scoped_pythonpath() {
    let temp = tempdir().unwrap();
    let pkg_path = fixture_path();
    assert!(
        pkg_path.is_file(),
        "missing ALPM Python runtime fixture: {}",
        pkg_path.display()
    );

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

    let runtime_manifest = engine
        .layout()
        .runtime_dir(&plan.runtimes[0].runtime_id)
        .join("manifest.json");
    assert_eq!(
        plan.runtimes[0].runtime_id,
        pkg_core::runtime::runtime_identity(&plan.runtimes[0])
    );
    let mut tampered: serde_json::Value =
        serde_json::from_slice(&fs::read(&runtime_manifest).unwrap()).unwrap();
    tampered["runner_version"] = serde_json::Value::String("tampered".into());
    fs::write(
        &runtime_manifest,
        serde_json::to_vec_pretty(&tampered).unwrap(),
    )
    .unwrap();
    let error = engine
        .run_command("default", "alpm-python-runtime-fixture", &[])
        .unwrap_err();
    assert!(error.to_string().contains("digest mismatch"));
}

#[test]
fn generic_interpreter_adapters_preserve_arguments() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("script-runtimes.deb");
    common::DebPackageBuilder::new("script-runtimes")
        .file(
            "usr/bin/perl-runtime",
            b"#!/usr/bin/perl\nprint \"perl=$ARGV[0]\n\";\n".to_vec(),
            0o755,
        )
        .file(
            "usr/bin/node-runtime",
            b"#!/usr/bin/env node\nconsole.log('node=' + process.argv[2]);\n".to_vec(),
            0o755,
        )
        .file(
            "usr/bin/status-runtime",
            b"#!/bin/sh\nexit 7\n".to_vec(),
            0o755,
        )
        .file(
            "usr/bin/signal-runtime",
            b"#!/bin/sh\nkill -TERM $$\n".to_vec(),
            0o755,
        )
        .write_to(&artifact)
        .unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    engine.install(&artifact, "default", false).unwrap();

    let perl = Command::new(
        engine
            .layout()
            .profile_bin_dir("default")
            .join("perl-runtime"),
    )
    .arg("argument")
    .output()
    .unwrap();
    assert!(
        perl.status.success(),
        "{}",
        String::from_utf8_lossy(&perl.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&perl.stdout).trim(),
        "perl=argument"
    );

    let node = Command::new(
        engine
            .layout()
            .profile_bin_dir("default")
            .join("node-runtime"),
    )
    .arg("argument")
    .output()
    .unwrap();
    assert!(
        node.status.success(),
        "{}",
        String::from_utf8_lossy(&node.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&node.stdout).trim(),
        "node=argument"
    );

    let status = engine
        .run_command("default", "status-runtime", &[])
        .unwrap();
    assert_eq!(status.code(), Some(7));
    let signal = engine
        .run_command("default", "signal-runtime", &[])
        .unwrap();
    #[cfg(unix)]
    assert_eq!(signal.signal(), Some(15));
}

#[test]
fn test_native_runner_uses_promoted_package_library_view() {
    let temp = tempdir().unwrap();
    let build = temp.path().join("build");
    fs::create_dir_all(&build).unwrap();
    fs::write(
        build.join("provider.c"),
        "int answer(void) { return 42; }\n",
    )
    .unwrap();
    fs::write(
        build.join("app.c"),
        "#include <stdio.h>\nint answer(void);\nint main(void) { printf(\"answer=%d\\n\", answer()); return 0; }\n",
    )
    .unwrap();
    let status = Command::new("gcc")
        .args([
            "-fPIC",
            "-shared",
            "-Wl,-soname,libpkg-runtime-provider.so",
            "-o",
            build.join("libpkg-runtime-provider.so").to_str().unwrap(),
            build.join("provider.c").to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("gcc")
        .args([
            "-o",
            build.join("app").to_str().unwrap(),
            build.join("app.c").to_str().unwrap(),
            "-L",
            build.to_str().unwrap(),
            "-lpkg-runtime-provider",
            "-Wl,-rpath,$ORIGIN/../lib",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let artifact = temp.path().join("native-runtime.deb");
    let app = fs::read(build.join("app")).unwrap();
    let provider = fs::read(build.join("libpkg-runtime-provider.so")).unwrap();
    common::DebPackageBuilder::new("native-runtime")
        .file("usr/bin/native-runtime", app, 0o755)
        .file(
            "usr/lib/libpkg-runtime-provider.so",
            provider.clone(),
            0o755,
        )
        .write_to(&artifact)
        .unwrap();

    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let plan = engine
        .install_with_options(
            &artifact,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: false,
            },
        )
        .unwrap();
    assert_eq!(
        plan.binaries[0].relative_store_path,
        PathBuf::from(".pkg-launcher/native-runtime-native")
    );
    assert!(
        plan.runtimes[0]
            .library_view
            .contains_key("libpkg-runtime-provider.so")
    );
    assert!(
        plan.runtimes[0]
            .execution
            .providers
            .iter()
            .find(|provider| provider.requirement == "libpkg-runtime-provider.so")
            .and_then(|provider| provider.digest.as_ref())
            .is_some()
    );

    let status = engine
        .run_command("default", "native-runtime", &[])
        .unwrap();
    assert!(status.success());

    let profile_status = Command::new(
        engine
            .layout()
            .profile_bin_dir("default")
            .join("native-runtime"),
    )
    .output()
    .unwrap();
    assert!(profile_status.status.success());
    assert_eq!(profile_status.stdout, b"answer=42\n");

    let ambient_loader = Command::new(
        engine
            .layout()
            .profile_bin_dir("default")
            .join("native-runtime"),
    )
    .env("LD_PRELOAD", "/tmp/pkg-nonexistent-preload.so")
    .env("LD_LIBRARY_PATH", "/tmp/pkg-ambient-loader")
    .arg("argument with spaces;$(no-shell-expansion)")
    .output()
    .unwrap();
    assert!(
        ambient_loader.status.success(),
        "ambient loader controls must not alter the frozen runtime: {}",
        String::from_utf8_lossy(&ambient_loader.stderr)
    );
    assert_eq!(ambient_loader.stdout, b"answer=42\n");

    // A runtime manifest does not turn a mutable host/store file into silent
    // compatibility: changing the selected provider must force replanning.
    fs::write(
        plan.target_store_dir
            .join("usr/lib/libpkg-runtime-provider.so"),
        b"tampered",
    )
    .unwrap();
    let error = engine
        .run_command("default", "native-runtime", &[])
        .unwrap_err();
    assert!(error.to_string().contains("ELF") || error.to_string().contains("ABI"));

    fs::write(
        plan.target_store_dir
            .join("usr/lib/libpkg-runtime-provider.so"),
        provider,
    )
    .unwrap();
    fs::write(
        plan.target_store_dir
            .join(".pkg-launcher/native-runtime-native"),
        b"tampered",
    )
    .unwrap();
    let error = engine
        .run_command("default", "native-runtime", &[])
        .unwrap_err();
    assert!(error.to_string().contains("bootstrap"));
}

#[test]
fn native_runner_resolves_a_real_transitive_elf_closure() {
    let temp = tempdir().unwrap();
    let build = temp.path().join("transitive-build");
    fs::create_dir_all(&build).unwrap();
    fs::write(build.join("helper.c"), "int helper(void) { return 7; }\n").unwrap();
    fs::write(
        build.join("provider.c"),
        "int helper(void); int answer(void) { return helper() + 35; }\n",
    )
    .unwrap();
    fs::write(
        build.join("consumer.c"),
        "#include <stdio.h>\nint answer(void); int main(void) { printf(\"transitive=%d\\n\", answer()); return 0; }\n",
    )
    .unwrap();

    let status = Command::new("gcc")
        .args([
            "-fPIC",
            "-shared",
            "-Wl,-soname,libpkg-transitive-helper.so.1",
            "-o",
            build
                .join("libpkg-transitive-helper.so.1")
                .to_str()
                .unwrap(),
            build.join("helper.c").to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("gcc")
        .args([
            "-fPIC",
            "-shared",
            "-Wl,-soname,libpkg-transitive-provider.so.1",
            "-o",
            build
                .join("libpkg-transitive-provider.so.1")
                .to_str()
                .unwrap(),
            build.join("provider.c").to_str().unwrap(),
            "-L",
            build.to_str().unwrap(),
            "-l:libpkg-transitive-helper.so.1",
            "-Wl,-rpath,$ORIGIN",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("gcc")
        .args([
            "-o",
            build.join("consumer").to_str().unwrap(),
            build.join("consumer.c").to_str().unwrap(),
            "-L",
            build.to_str().unwrap(),
            "-l:libpkg-transitive-provider.so.1",
            "-Wl,-rpath,$ORIGIN/../lib",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let artifact = temp.path().join("transitive-runtime.deb");
    common::DebPackageBuilder::new("transitive-runtime")
        .file(
            "usr/bin/transitive-runtime",
            fs::read(build.join("consumer")).unwrap(),
            0o755,
        )
        .file(
            "usr/lib/libpkg-transitive-provider.so.1",
            fs::read(build.join("libpkg-transitive-provider.so.1")).unwrap(),
            0o755,
        )
        .file(
            "usr/lib/libpkg-transitive-helper.so.1",
            fs::read(build.join("libpkg-transitive-helper.so.1")).unwrap(),
            0o755,
        )
        .write_to(&artifact)
        .unwrap();

    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let plan = engine.install(&artifact, "default", false).unwrap();
    let runtime = &plan.runtimes[0];
    assert!(
        runtime
            .library_view
            .contains_key("libpkg-transitive-provider.so.1")
    );
    assert!(
        runtime
            .library_view
            .contains_key("libpkg-transitive-helper.so.1")
    );
    assert!(
        runtime
            .execution
            .closure
            .iter()
            .any(|path| path.ends_with("libpkg-transitive-helper.so.1"))
    );
    let output = Command::new(
        engine
            .layout()
            .profile_bin_dir("default")
            .join("transitive-runtime"),
    )
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "transitive=42"
    );
}

#[test]
fn invalid_native_extension_is_rejected_before_activation() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("invalid-extension.deb");
    common::DebPackageBuilder::new("invalid-extension")
        .file(
            "usr/bin/invalid-extension",
            b"#!/usr/bin/python3\nprint('should not run')\n".to_vec(),
            0o755,
        )
        .file(
            "usr/lib/python3/dist-packages/broken.so",
            b"this is not an ELF extension\n".to_vec(),
            0o644,
        )
        .write_to(&artifact)
        .unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let error = engine.install(&artifact, "default", false).unwrap_err();
    assert!(
        matches!(error, pkg_core::Error::MalformedArchive(_)),
        "{error:?}"
    );
    assert!(engine.list("default").unwrap().is_empty());
    assert!(
        !engine
            .layout()
            .profile_bin_dir("default")
            .join("invalid-extension")
            .exists()
    );
}

#[test]
fn unsupported_interpreter_family_is_rejected_before_activation() {
    let temp = tempdir().unwrap();
    let artifact = temp.path().join("unsupported-interpreter.deb");
    common::DebPackageBuilder::new("unsupported-interpreter")
        .file(
            "usr/bin/unsupported-interpreter",
            b"#!/usr/bin/future-lang\nexit 0\n".to_vec(),
            0o755,
        )
        .write_to(&artifact)
        .unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let error = engine.install(&artifact, "default", false).unwrap_err();
    assert!(
        matches!(error, pkg_core::Error::IncompatibleHost(_)),
        "{error:?}"
    );
    assert!(engine.list("default").unwrap().is_empty());
}

#[test]
fn package_rpath_cannot_escape_staging_root() {
    let temp = tempdir().unwrap();
    let build = temp.path().join("build");
    fs::create_dir_all(&build).unwrap();
    fs::write(
        build.join("provider.c"),
        "int answer(void) { return 42; }\n",
    )
    .unwrap();
    fs::write(
        build.join("app.c"),
        "int answer(void); int main(void) { return answer() == 42 ? 0 : 1; }\n",
    )
    .unwrap();
    let status = Command::new("gcc")
        .args([
            "-fPIC",
            "-shared",
            "-Wl,-soname,libpkg-rpath-escape.so",
            "-o",
            build.join("libpkg-rpath-escape.so").to_str().unwrap(),
            build.join("provider.c").to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("gcc")
        .args([
            "-o",
            build.join("app").to_str().unwrap(),
            build.join("app.c").to_str().unwrap(),
            "-L",
            build.to_str().unwrap(),
            "-l:libpkg-rpath-escape.so",
            "-Wl,-rpath,$ORIGIN/../../../../escape",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let layout = StoreLayout::new(temp.path().join("data"));
    // This file is intentionally outside the transaction staging tree.  The
    // loader metadata points at it through `..`; inspection must reject that
    // escape before any package object is promoted.
    fs::create_dir_all(layout.base_dir()).unwrap();
    fs::create_dir_all(layout.base_dir().join("escape")).unwrap();
    fs::copy(
        build.join("libpkg-rpath-escape.so"),
        layout
            .base_dir()
            .join("escape")
            .join("libpkg-rpath-escape.so"),
    )
    .unwrap();

    let artifact = temp.path().join("rpath-escape.deb");
    common::DebPackageBuilder::new("rpath-escape")
        .file(
            "usr/bin/rpath-escape",
            fs::read(build.join("app")).unwrap(),
            0o755,
        )
        .write_to(&artifact)
        .unwrap();
    let engine = Engine::open(layout).unwrap();
    let error = engine
        .install_with_options(
            &artifact,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: false,
            },
        )
        .unwrap_err();
    assert!(
        matches!(error, pkg_core::Error::SecurityViolation(_)),
        "{error:?}"
    );
    assert!(engine.list("default").unwrap().is_empty());
}

#[test]
fn installing_an_unrelated_package_preserves_existing_runtime_commands() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let first_artifact = temp.path().join("first.deb");
    let second_artifact = temp.path().join("second.deb");
    let host_binary = fs::read("/bin/true").expect("host true fixture is available");

    common::DebPackageBuilder::new("first-runtime")
        .file("usr/bin/first-runtime", host_binary.clone(), 0o755)
        .write_to(&first_artifact)
        .unwrap();
    common::DebPackageBuilder::new("second-runtime")
        .file("usr/bin/second-runtime", host_binary, 0o755)
        .write_to(&second_artifact)
        .unwrap();

    engine.install(&first_artifact, "default", false).unwrap();
    engine.install(&second_artifact, "default", false).unwrap();

    let status = engine
        .run_command("default", "first-runtime", &[])
        .expect("the first generation runtime remains selected");
    assert!(status.success());
}

#[test]
fn real_elf_provider_is_reused_across_deb_rpm_and_alpm_consumers() {
    let temp = tempdir().unwrap();
    let build = temp.path().join("matrix-build");
    fs::create_dir_all(&build).unwrap();
    fs::write(
        build.join("provider.c"),
        "int pkg_answer(void) { return 73; }\n",
    )
    .unwrap();
    fs::write(
        build.join("consumer.c"),
        "#include <stdio.h>\nint pkg_answer(void);\nint main(void) { printf(\"matrix=%d\\n\", pkg_answer()); return 0; }\n",
    )
    .unwrap();

    let status = Command::new("gcc")
        .args([
            "-fPIC",
            "-shared",
            "-Wl,-soname,libpkg-matrix.so.1",
            "-o",
            build.join("libpkg-matrix.so.1").to_str().unwrap(),
            build.join("provider.c").to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("gcc")
        .args([
            "-o",
            build.join("consumer").to_str().unwrap(),
            build.join("consumer.c").to_str().unwrap(),
            "-L",
            build.to_str().unwrap(),
            "-Wl,-rpath,$ORIGIN/../lib",
            "-Wl,-z,origin",
            "-l:libpkg-matrix.so.1",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let provider = fs::read(build.join("libpkg-matrix.so.1")).unwrap();
    let consumer = fs::read(build.join("consumer")).unwrap();
    let provider_artifact = temp.path().join("matrix-provider.deb");
    common::DebPackageBuilder::new("matrix-provider")
        .file(
            "usr/lib/x86_64-linux-gnu/libpkg-matrix.so.1",
            provider,
            0o755,
        )
        .write_to(&provider_artifact)
        .unwrap();
    let rpm_artifact = temp.path().join("matrix-consumer.rpm");
    common::RpmPackageBuilder::new("matrix-consumer-rpm")
        .requires("libpkg-matrix.so.1")
        .file("usr/bin/matrix-consumer-rpm", consumer.clone(), 0o755)
        .write_to(&rpm_artifact)
        .unwrap();
    let alpm_artifact = temp.path().join("matrix-consumer-alpm.pkg.tar.zst");
    common::AlpmPackageBuilder::new("matrix-consumer-alpm")
        .depend("libpkg-matrix.so.1")
        .file("usr/bin/matrix-consumer-alpm", consumer, 0o755)
        .write_to(&alpm_artifact)
        .unwrap();

    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let provider_plan = engine
        .install(&provider_artifact, "default", false)
        .unwrap();
    engine.install(&rpm_artifact, "default", false).unwrap();
    engine.install(&alpm_artifact, "default", false).unwrap();

    for runtime in engine
        .db()
        .active_generation("default")
        .unwrap()
        .map(|generation| {
            let path = engine
                .layout()
                .profile_generations_dir("default")
                .join(generation.generation_id)
                .join("manifest.json");
            let generation: pkg_core::domain::contracts::ActivationGeneration =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            generation.runtimes
        })
        .unwrap()
    {
        let manifest = engine.layout().runtime_dir(&runtime).join("manifest.json");
        let manifest: pkg_core::domain::contracts::RuntimeManifest =
            serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
        assert!(
            engine.layout().runtime_lib_dir(&runtime).is_dir(),
            "runtime lib missing before provider removal: {runtime}"
        );
        assert!(
            manifest.library_view.values().all(|path| path.exists()),
            "runtime provider missing before provider removal: {runtime}"
        );
    }

    for command in ["matrix-consumer-rpm", "matrix-consumer-alpm"] {
        let output = Command::new(engine.layout().profile_bin_dir("default").join(command))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "matrix=73");
    }

    // Runtime views must retain the direct provider object rather than the
    // mutable profile/lib compatibility alias. Removing the provider package
    // therefore leaves already published consumers executable and reachable
    // by the conservative collector.
    engine.remove("matrix-provider", "default", false).unwrap();
    for runtime in engine
        .db()
        .active_generation("default")
        .unwrap()
        .map(|generation| {
            let path = engine
                .layout()
                .profile_generations_dir("default")
                .join(generation.generation_id)
                .join("manifest.json");
            let generation: pkg_core::domain::contracts::ActivationGeneration =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            generation.runtimes
        })
        .unwrap()
    {
        assert!(
            engine.layout().runtime_lib_dir(&runtime).is_dir(),
            "runtime lib missing after provider removal: {runtime}"
        );
    }
    for command in ["matrix-consumer-rpm", "matrix-consumer-alpm"] {
        let output = Command::new(engine.layout().profile_bin_dir("default").join(command))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command} failed after provider removal: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "matrix=73");
    }
    let runtime_refs = engine
        .db()
        .runtime_store_references(
            &engine.layout().runtimes_dir(),
            &engine.layout().store_dir(),
        )
        .unwrap();
    assert!(
        runtime_refs
            .iter()
            .any(|path| path == &provider_plan.target_store_dir)
    );
}

#[test]
fn command_runtimes_keep_independent_same_soname_providers() {
    let temp = tempdir().unwrap();
    let build = temp.path().join("same-soname-build");
    fs::create_dir_all(&build).unwrap();
    fs::write(
        build.join("provider-one.c"),
        "int pkg_value(void) { return 11; }\n",
    )
    .unwrap();
    fs::write(
        build.join("provider-two.c"),
        "int pkg_value(void) { return 22; }\n",
    )
    .unwrap();
    fs::write(
        build.join("consumer-one.c"),
        "#include <stdio.h>\nint pkg_value(void);\nint main(void) { printf(\"value=%d\\n\", pkg_value()); return 0; }\n",
    )
    .unwrap();
    fs::write(
        build.join("consumer-two.c"),
        "#include <stdio.h>\nint pkg_value(void);\nint main(void) { printf(\"value=%d\\n\", pkg_value()); return 0; }\n",
    )
    .unwrap();

    for (source, output) in [
        ("provider-one.c", "provider-one.so"),
        ("provider-two.c", "provider-two.so"),
    ] {
        let status = Command::new("gcc")
            .args([
                "-fPIC",
                "-shared",
                "-Wl,-soname,libpkg-same.so.1",
                "-o",
                build.join(output).to_str().unwrap(),
                build.join(source).to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
    }
    for (source, output, provider) in [
        ("consumer-one.c", "consumer-one", "provider-one.so"),
        ("consumer-two.c", "consumer-two", "provider-two.so"),
    ] {
        let status = Command::new("gcc")
            .args([
                "-o",
                build.join(output).to_str().unwrap(),
                build.join(source).to_str().unwrap(),
                "-L",
                build.to_str().unwrap(),
                "-Wl,-rpath,$ORIGIN/../lib",
                "-Wl,-z,origin",
                &format!("-l:{provider}"),
            ])
            .status()
            .unwrap();
        assert!(status.success());
    }

    let provider_one = temp.path().join("provider-one.deb");
    common::DebPackageBuilder::new("same-provider-one")
        .file(
            "usr/lib/x86_64-linux-gnu/libpkg-same.so.1",
            fs::read(build.join("provider-one.so")).unwrap(),
            0o755,
        )
        .write_to(&provider_one)
        .unwrap();
    let provider_two = temp.path().join("provider-two.deb");
    common::DebPackageBuilder::new("same-provider-two")
        .file(
            "usr/lib/x86_64-linux-gnu/libpkg-same.so.1",
            fs::read(build.join("provider-two.so")).unwrap(),
            0o755,
        )
        .write_to(&provider_two)
        .unwrap();
    let consumer_one = temp.path().join("consumer-one.rpm");
    common::RpmPackageBuilder::new("same-consumer-one")
        .requires("libpkg-same.so.1")
        .file(
            "usr/bin/same-consumer-one",
            fs::read(build.join("consumer-one")).unwrap(),
            0o755,
        )
        .write_to(&consumer_one)
        .unwrap();
    let consumer_two = temp.path().join("consumer-two.pkg.tar.zst");
    common::AlpmPackageBuilder::new("same-consumer-two")
        .depend("libpkg-same.so.1")
        .file(
            "usr/bin/same-consumer-two",
            fs::read(build.join("consumer-two")).unwrap(),
            0o755,
        )
        .write_to(&consumer_two)
        .unwrap();

    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    engine.install(&provider_one, "default", false).unwrap();
    let first = engine.install(&consumer_one, "default", false).unwrap();
    engine.install(&provider_two, "default", false).unwrap();
    let second = engine.install(&consumer_two, "default", false).unwrap();

    let run = |command: &str| {
        Command::new(engine.layout().profile_bin_dir("default").join(command))
            .output()
            .unwrap()
    };
    let first_output = run("same-consumer-one");
    assert!(first_output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&first_output.stdout).trim(),
        "value=11"
    );
    let second_output = run("same-consumer-two");
    assert!(second_output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&second_output.stdout).trim(),
        "value=22"
    );

    let first_provider = first.runtimes[0]
        .execution
        .providers
        .iter()
        .find(|provider| provider.requirement == "libpkg-same.so.1")
        .and_then(|provider| provider.digest.clone())
        .unwrap();
    let second_provider = second.runtimes[0]
        .execution
        .providers
        .iter()
        .find(|provider| provider.requirement == "libpkg-same.so.1")
        .and_then(|provider| provider.digest.clone())
        .unwrap();
    assert_ne!(first_provider, second_provider);
}
