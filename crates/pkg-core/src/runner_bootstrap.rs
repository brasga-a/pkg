//! Minimal static bootstrap used by command-local native runtime launchers.
//!
//! This file is compiled directly by `build.rs` with `crt-static`.  It has no
//! dependency on pkg or on package-provided libraries, so loader variables
//! cannot influence the code that sanitizes them.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("pkg native runner: {}", message.as_ref());
    std::process::exit(126);
}

fn main() {
    let launcher = env::current_exe().unwrap_or_else(|error| fail(format!("cannot resolve itself: {error}")));
    let config_path = launcher.with_extension("conf");
    let config = fs::read_to_string(&config_path)
        .unwrap_or_else(|error| fail(format!("cannot read {}: {error}", config_path.display())));
    let mut target = None;
    let mut library_path = None;
    for line in config.lines() {
        let Some((key, value)) = line.split_once('=') else {
            fail(format!("malformed configuration line in {}", config_path.display()));
        };
        if value.contains('\n') || value.contains('\r') {
            fail("newline in runner configuration");
        }
        match key {
            "target" if target.is_none() => target = Some(PathBuf::from(value)),
            "library_path" if library_path.is_none() => library_path = Some(PathBuf::from(value)),
            _ => fail(format!("unknown or duplicate configuration key: {key}")),
        }
    }
    let target = target.unwrap_or_else(|| fail("runner target is missing"));
    let target = if target.is_absolute() {
        target
    } else {
        launcher
            .parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| fail("runner has no store root"))
            .join(target)
    };
    if !target.is_file() {
        fail(format!("target is unavailable: {}", target.display()));
    }

    for key in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "LD_DEBUG",
        "LD_DEBUG_OUTPUT",
        "LD_ORIGIN_PATH",
        "LD_PROFILE",
        "LD_USE_LOAD_BIAS",
    ] {
        // The runner is single-threaded before `exec`, so changing the
        // process environment cannot race another Rust thread.
        unsafe { env::remove_var(key) };
    }
    if let Some(path) = library_path {
        if !path.is_dir() {
            fail(format!("runtime library view is unavailable: {}", path.display()));
        }
        // See the single-threaded safety argument above.
        unsafe { env::set_var("LD_LIBRARY_PATH", path) };
    }

    let mut args = env::args_os();
    let argv0: OsString = args.next().unwrap_or_else(|| target.clone().into_os_string());
    let mut command = Command::new(&target);
    command.args(args);
    command.arg0(argv0);
    let error = command.exec();
    fail(format!("cannot execute {}: {error}", target.display()));
}
