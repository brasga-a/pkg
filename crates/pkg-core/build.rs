use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/runner_bootstrap.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let output = out_dir.join("pkg-native-runner");
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join("src/runner_bootstrap.rs");
    let target = env::var("TARGET").expect("TARGET is set by Cargo");

    let mut command = Command::new(env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
    command
        .arg("--crate-name")
        .arg("pkg_native_runner")
        .arg("--edition=2024")
        .arg("--target")
        .arg(&target)
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg("panic=abort")
        .arg("-C")
        .arg("strip=symbols")
        .arg("-C")
        .arg("target-feature=+crt-static")
        .arg(&source)
        .arg("-o")
        .arg(&output);

    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            println!(
                "cargo:warning=static native runner unavailable for {target} (rustc exited {status}); native ELF commands will be rejected"
            );
            let _ = fs::write(&output, []);
        }
        Err(error) => {
            println!(
                "cargo:warning=static native runner unavailable for {target}: {error}; native ELF commands will be rejected"
            );
            let _ = fs::write(&output, []);
        }
    }
}
