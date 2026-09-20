use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_uninstall_help() {
    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../uninstall.sh");
    let output = Command::new("sh")
        .arg(script_path)
        .arg("--help")
        .output()
        .expect("failed to execute uninstall.sh --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pkg uninstaller"));
    assert!(stdout.contains("--dir"));
    assert!(stdout.contains("--purge"));
    assert!(stdout.contains("--yes"));
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("--no-modify-path"));
}

#[test]
fn test_uninstall_dry_run_preserves_files() {
    let temp = tempdir().expect("failed to create tempdir");
    let temp_path = temp.path();

    let bin_dir = temp_path.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pkg_bin = bin_dir.join("pkg");
    fs::write(&pkg_bin, "#!/bin/sh\necho mock pkg").unwrap();

    let data_dir = temp_path.join("share/pkg");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("store.db"), "data").unwrap();

    let config_dir = temp_path.join("config/pkg");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "config").unwrap();

    let bashrc = temp_path.join(".bashrc");
    let bashrc_content = "# User config\n# pkg-managed paths (CLI binary and active profile packages)\nexport PATH=\"/fake/bin:/fake/profiles:$PATH\"\n# Other stuff\n";
    fs::write(&bashrc, bashrc_content).unwrap();

    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../uninstall.sh");
    let output = Command::new("sh")
        .arg(script_path)
        .arg("--dry-run")
        .arg("--purge")
        .arg("--yes")
        .arg("--dir")
        .arg(&bin_dir)
        .env("HOME", temp_path)
        .env("XDG_DATA_HOME", temp_path.join("share"))
        .env("XDG_CONFIG_HOME", temp_path.join("config"))
        .env("XDG_CACHE_HOME", temp_path.join("cache"))
        .output()
        .expect("failed to execute uninstall.sh");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[DRY-RUN MODE]"));
    assert!(stdout.contains("Would remove executable"));
    assert!(stdout.contains("Would remove data & store directory"));
    assert!(stdout.contains("Would clean pkg PATH configuration from"));

    // Verify nothing was actually deleted
    assert!(pkg_bin.exists());
    assert!(data_dir.exists());
    assert!(config_dir.exists());
    assert_eq!(fs::read_to_string(&bashrc).unwrap(), bashrc_content);
}

#[test]
fn test_uninstall_without_purge_preserves_data() {
    let temp = tempdir().expect("failed to create tempdir");
    let temp_path = temp.path();

    let bin_dir = temp_path.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pkg_bin = bin_dir.join("pkg");
    fs::write(&pkg_bin, "#!/bin/sh\necho mock pkg").unwrap();

    let data_dir = temp_path.join("share/pkg");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("store.db"), "data").unwrap();

    let config_dir = temp_path.join("config/pkg");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "config").unwrap();

    let bashrc = temp_path.join(".bashrc");
    let bashrc_content = "# User config\n# pkg-managed paths (CLI binary and active profile packages)\nexport PATH=\"/fake/bin:/fake/profiles:$PATH\"\n# Other stuff\n";
    fs::write(&bashrc, bashrc_content).unwrap();

    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../uninstall.sh");
    let output = Command::new("sh")
        .arg(script_path)
        .arg("--yes")
        .arg("--dir")
        .arg(&bin_dir)
        .env("HOME", temp_path)
        .env("XDG_DATA_HOME", temp_path.join("share"))
        .env("XDG_CONFIG_HOME", temp_path.join("config"))
        .env("XDG_CACHE_HOME", temp_path.join("cache"))
        .output()
        .expect("failed to execute uninstall.sh");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pkg has been successfully uninstalled"));

    // Binary deleted
    assert!(!pkg_bin.exists());

    // Data and config preserved!
    assert!(data_dir.exists());
    assert!(config_dir.exists());

    // Shell configuration cleaned
    let updated_bashrc = fs::read_to_string(&bashrc).unwrap();
    assert!(!updated_bashrc.contains("pkg-managed paths"));
    assert!(!updated_bashrc.contains("/fake/profiles"));
    assert!(updated_bashrc.contains("# User config"));
    assert!(updated_bashrc.contains("# Other stuff"));
}

#[test]
fn test_uninstall_with_purge_deletes_all() {
    let temp = tempdir().expect("failed to create tempdir");
    let temp_path = temp.path();

    let bin_dir = temp_path.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pkg_bin = bin_dir.join("pkg");
    fs::write(&pkg_bin, "#!/bin/sh\necho mock pkg").unwrap();

    let data_dir = temp_path.join("share/pkg");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("store.db"), "data").unwrap();

    let config_dir = temp_path.join("config/pkg");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.toml"), "config").unwrap();

    let cache_dir = temp_path.join("cache/pkg");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("repo.json"), "cache").unwrap();

    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../uninstall.sh");
    let output = Command::new("sh")
        .arg(script_path)
        .arg("--yes")
        .arg("--purge")
        .arg("--dir")
        .arg(&bin_dir)
        .env("HOME", temp_path)
        .env("XDG_DATA_HOME", temp_path.join("share"))
        .env("XDG_CONFIG_HOME", temp_path.join("config"))
        .env("XDG_CACHE_HOME", temp_path.join("cache"))
        .output()
        .expect("failed to execute uninstall.sh");

    assert!(output.status.success());

    // Binary deleted
    assert!(!pkg_bin.exists());

    // Everything purged
    assert!(!data_dir.exists());
    assert!(!config_dir.exists());
    assert!(!cache_dir.exists());
}

#[test]
fn test_uninstall_cleans_fish_config() {
    let temp = tempdir().expect("failed to create tempdir");
    let temp_path = temp.path();

    let bin_dir = temp_path.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pkg_bin = bin_dir.join("pkg");
    fs::write(&pkg_bin, "#!/bin/sh\necho mock pkg").unwrap();

    let fish_conf = temp_path.join(".config/fish/config.fish");
    fs::create_dir_all(fish_conf.parent().unwrap()).unwrap();
    fs::write(
        &fish_conf,
        "# Fish configuration\n# pkg-managed paths\nfish_add_path /fake/bin /fake/profiles/bin\n# other settings\n",
    )
    .unwrap();

    let script_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../uninstall.sh");
    let output = Command::new("sh")
        .arg(script_path)
        .arg("--yes")
        .arg("--dir")
        .arg(&bin_dir)
        .env("HOME", temp_path)
        .env("XDG_DATA_HOME", temp_path.join("share"))
        .env("XDG_CONFIG_HOME", temp_path.join("config"))
        .output()
        .expect("failed to execute uninstall.sh");

    assert!(output.status.success());

    let updated_fish = fs::read_to_string(&fish_conf).unwrap();
    assert!(!updated_fish.contains("pkg-managed"));
    assert!(!updated_fish.contains("fish_add_path"));
    assert!(updated_fish.contains("# Fish configuration"));
    assert!(updated_fish.contains("# other settings"));
}
