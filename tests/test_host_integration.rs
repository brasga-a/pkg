use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

#[path = "common/mod.rs"]
mod common;

fn pkg(data_dir: &std::path::Path, xdg_data_home: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("pkg").unwrap();
    command
        .env("XDG_DATA_HOME", xdg_data_home)
        .arg("--data-dir")
        .arg(data_dir);
    command
}

#[test]
fn explicit_user_integration_is_reversible_and_owned() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("pkg");
    let xdg_data_home = temp.path().join("xdg");
    let artifact = temp.path().join("gui.deb");
    common::DebPackageBuilder::new("gui")
        .file("usr/bin/gui", b"#!/bin/sh\necho gui\n", 0o755)
        .file(
            "usr/share/applications/gui.desktop",
            b"[Desktop Entry]\nType=Application\nName=Gui\nExec=gui\n",
            0o644,
        )
        .file("usr/share/icons/hicolor/48x48/apps/gui.png", b"png", 0o644)
        .file(
            "usr/share/mime/packages/gui.xml",
            b"<?xml version=\"1.0\"?><mime-info><mime-type type=\"application/x-gui\"/></mime-info>",
            0o644,
        )
        .write_to(&artifact)
        .unwrap();

    let install = pkg(&data_dir, &xdg_data_home)
        .args(["install", artifact.to_str().unwrap(), "--yes"])
        .output()
        .unwrap();
    assert!(install.status.success(), "{install:?}");

    let integrate = pkg(&data_dir, &xdg_data_home)
        .args(["--json", "integrate", "gui"])
        .output()
        .unwrap();
    assert!(integrate.status.success(), "{integrate:?}");
    let value: Value = serde_json::from_slice(&integrate.stdout).unwrap();
    assert_eq!(value["status"], "success");
    assert_eq!(value["plan"]["actions"].as_array().unwrap().len(), 3);
    assert!(
        xdg_data_home
            .join("applications/gui.desktop")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        xdg_data_home
            .join("icons/hicolor/48x48/apps/gui.png")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(xdg_data_home.join("mime/packages/gui.xml").is_symlink());

    let deintegrate = pkg(&data_dir, &xdg_data_home)
        .args(["--json", "deintegrate", "gui"])
        .output()
        .unwrap();
    assert!(deintegrate.status.success(), "{deintegrate:?}");
    assert!(!xdg_data_home.join("applications/gui.desktop").exists());
    assert!(!xdg_data_home.join("mime/packages/gui.xml").exists());

    let integrate_again = pkg(&data_dir, &xdg_data_home)
        .args(["integrate", "gui"])
        .output()
        .unwrap();
    assert!(integrate_again.status.success(), "{integrate_again:?}");
    let target = xdg_data_home.join("applications/gui.desktop");
    fs::remove_file(&target).unwrap();
    fs::write(&target, b"user replacement").unwrap();
    let conflict = pkg(&data_dir, &xdg_data_home)
        .args(["deintegrate", "gui"])
        .output()
        .unwrap();
    assert_eq!(conflict.status.code(), Some(20));
    assert_eq!(fs::read(&target).unwrap(), b"user replacement");

    // A normal package removal also reverses owned host integrations, while
    // the conflict above remains a hard stop.
    fs::remove_file(&target).unwrap();
    let integrate_final = pkg(&data_dir, &xdg_data_home)
        .args(["integrate", "gui"])
        .output()
        .unwrap();
    assert!(integrate_final.status.success(), "{integrate_final:?}");
    let remove = pkg(&data_dir, &xdg_data_home)
        .args(["remove", "gui"])
        .output()
        .unwrap();
    assert!(remove.status.success(), "{remove:?}");
    assert!(!xdg_data_home.join("applications/gui.desktop").exists());
    assert!(
        !xdg_data_home
            .join("icons/hicolor/48x48/apps/gui.png")
            .exists()
    );
    assert!(!xdg_data_home.join("mime/packages/gui.xml").exists());
}
