//! Integration tests for CLI improvements (multi-package queue, --yes, --ignore-missing-libs).

mod common;

use assert_cmd::Command;
use common::deb_builder::DebPackageBuilder;
use predicates::prelude::*;
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
fn test_cli_install_flags_exist() {
    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.args(["install", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("-y, --yes"))
        .stdout(predicate::str::contains("--ignore-missing-libs"));
}

#[test]
fn test_multi_package_queue_and_summary() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("data");

    let deb_a = temp.path().join("alpha.deb");
    DebPackageBuilder::new("alpha")
        .file("usr/bin/alpha-cmd", b"#!/bin/sh\necho alpha\n", 0o755)
        .write_to(&deb_a)
        .unwrap();

    let deb_b = temp.path().join("beta.deb");
    DebPackageBuilder::new("beta")
        .file("usr/bin/beta-cmd", b"#!/bin/sh\necho beta\n", 0o755)
        .write_to(&deb_b)
        .unwrap();

    let mut cmd = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd.args([
        "--data-dir",
        data_dir.to_str().unwrap(),
        "install",
        "-y",
        deb_a.to_str().unwrap(),
        deb_b.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Installed alpha"))
    .stdout(predicate::str::contains("Installed beta"))
    .stdout(predicate::str::contains(
        "Summary: 2 installed, 0 skipped, 0 failed.",
    ));

    // Reinstall with -y succeeds
    let mut cmd2 = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd2.args([
        "--data-dir",
        data_dir.to_str().unwrap(),
        "install",
        "-y",
        deb_a.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Installed alpha"));
}

#[test]
fn test_ignore_missing_libs_flag() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("data");

    let deb = temp.path().join("cli-missing.deb");
    DebPackageBuilder::new("cli-missing")
        .file(
            "usr/bin/cli-missing-cmd",
            elf_requiring("libcli-not-found.so"),
            0o755,
        )
        .write_to(&deb)
        .unwrap();

    // Without --ignore-missing-libs and non-interactive, it rejects/skips
    let mut cmd_fail = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd_fail
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "install",
            deb.to_str().unwrap(),
        ])
        .assert()
        .failure();

    // With --ignore-missing-libs, it installs and prints warning
    let mut cmd_ok = Command::cargo_bin("pkg").expect("pkg binary should exist");
    cmd_ok
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "install",
            "-y",
            "--ignore-missing-libs",
            deb.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed cli-missing"))
        .stdout(predicate::str::contains(
            "WARNING: Executable(s) installed with missing libraries: libcli-not-found.so",
        ));
}
