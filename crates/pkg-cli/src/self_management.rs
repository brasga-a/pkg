use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tar::Archive;

const DEFAULT_BASE_URL: &str = "https://pkg.atlantic.sh/releases";
const GITHUB_REPO: &str = "brasga-a/pkg";

fn is_safe_to_delete(path: &Path) -> bool {
    let p = path.to_string_lossy();
    if p.is_empty() || p == "/" || p == "/usr" || p == "/usr/local" || p == "/var" || p == "/home" {
        return false;
    }
    if let Ok(home) = std::env::var("HOME") {
        if p == home {
            return false;
        }
    }
    true
}

fn prompt_confirm(prompt: &str, default_yes: bool) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(default_yes);
    }
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        return Ok(default_yes);
    }
    Ok(trimmed == "y" || trimmed == "yes")
}

pub(crate) async fn run_self_update(check_only: bool, assume_yes: bool) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let current_tag = if current_version.starts_with('v') {
        current_version.to_string()
    } else {
        format!("v{current_version}")
    };

    println!("Current version: {current_tag}");
    println!("Checking for updates...");

    let client = reqwest::Client::builder()
        .user_agent("pkg-self-update")
        .build()?;

    // 1. Resolve latest version
    let mut latest_tag = None;
    let latest_url = format!("{DEFAULT_BASE_URL}/latest.txt");
    if let Ok(res) = client.get(&latest_url).send().await {
        if res.status().is_success() {
            if let Ok(text) = res.text().await {
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    latest_tag = Some(trimmed);
                }
            }
        }
    }

    // Fallback: GitHub Releases API
    if latest_tag.is_none() {
        let gh_api = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
        if let Ok(res) = client.get(&gh_api).send().await {
            if res.status().is_success() {
                if let Ok(text) = res.text().await {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(tag) = json.get("tag_name").and_then(|v| v.as_str()) {
                            latest_tag = Some(tag.trim().to_string());
                        }
                    }
                }
            }
        }
    }

    let latest_tag = latest_tag.ok_or_else(|| {
        anyhow::anyhow!("Unable to fetch latest release version from release servers")
    })?;

    let latest_clean = latest_tag.trim_start_matches('v');
    let current_clean = current_version.trim_start_matches('v');

    if latest_clean == current_clean {
        println!("pkg is already up to date ({latest_tag}).");
        return Ok(());
    }

    println!("A newer version of pkg is available: {latest_tag} (current: {current_tag})");
    if check_only {
        return Ok(());
    }

    if !assume_yes && !prompt_confirm("Do you want to upgrade pkg now? [Y/n]: ", true)? {
        println!("Update cancelled by user.");
        return Ok(());
    }

    // 2. Resolve architecture and platform triple
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "riscv64" => "riscv64",
        other => anyhow::bail!("Unsupported architecture for self-update: {other}"),
    };

    let target_triple = match arch {
        "x86_64" => "x86_64-unknown-linux-gnu",
        "aarch64" => "aarch64-unknown-linux-gnu",
        "riscv64" => "riscv64gc-unknown-linux-gnu",
        _ => unreachable!(),
    };

    let candidate_assets = [
        format!("pkg-{latest_tag}-{target_triple}.tar.gz"),
        format!("pkg-{latest_clean}-{target_triple}.tar.gz"),
        format!("pkg-{target_triple}.tar.gz"),
        format!("pkg-linux-{arch}.tar.gz"),
    ];

    let mut download_url = None;
    let mut asset_data = None;

    for asset in &candidate_assets {
        let url_base = format!("{DEFAULT_BASE_URL}/{latest_tag}/{asset}");
        let url_gh =
            format!("https://github.com/{GITHUB_REPO}/releases/download/{latest_tag}/{asset}");

        for url in [&url_base, &url_gh] {
            if let Ok(res) = client.get(url).send().await {
                if res.status().is_success() {
                    if let Ok(bytes) = res.bytes().await {
                        download_url = Some(url.clone());
                        asset_data = Some(bytes);
                        break;
                    }
                }
            }
        }
        if asset_data.is_some() {
            break;
        }
    }

    let download_url = download_url.ok_or_else(|| {
        anyhow::anyhow!(
            "Could not find suitable binary download for {target_triple} in release {latest_tag}"
        )
    })?;
    let asset_bytes = asset_data.unwrap();

    println!("Downloaded update from {download_url}");

    // 3. Verify Checksum if available
    let checksum_url = format!("{download_url}.sha256");
    if let Ok(res) = client.get(&checksum_url).send().await {
        if res.status().is_success() {
            if let Ok(text) = res.text().await {
                if let Some(expected_hash) = text.split_whitespace().next() {
                    let mut hasher = Sha256::new();
                    hasher.update(&asset_bytes);
                    let computed_hash = format!("{:x}", hasher.finalize());
                    if !computed_hash.eq_ignore_ascii_case(expected_hash) {
                        anyhow::bail!(
                            "Cryptographic checksum mismatch! Expected: {expected_hash}, Computed: {computed_hash}"
                        );
                    }
                    println!("Cryptographic SHA-256 checksum verified.");
                }
            }
        }
    }

    // 4. Extract executable from tar.gz
    let gz = GzDecoder::new(&asset_bytes[..]);
    let mut archive = Archive::new(gz);
    let mut extracted_binary = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.file_name().and_then(|n| n.to_str()) == Some("pkg") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            extracted_binary = Some(buf);
            break;
        }
    }

    let new_binary = extracted_binary
        .ok_or_else(|| anyhow::anyhow!("Archive did not contain the 'pkg' binary executable"))?;

    // 5. Replace current executable atomically
    let current_exe =
        std::env::current_exe().context("Failed to determine path of running executable")?;
    let exe_dir = current_exe.parent().ok_or_else(|| {
        anyhow::anyhow!("Cannot determine parent directory of current executable")
    })?;

    let tmp_path = exe_dir.join(format!(".pkg_update_tmp_{}", std::process::id()));
    fs::write(&tmp_path, &new_binary)
        .context("Failed to write temporary binary in target directory")?;

    let mut perms = fs::metadata(&tmp_path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&tmp_path, perms)?;

    // Atomic rename
    if let Err(e) = fs::rename(&tmp_path, &current_exe) {
        let _ = fs::remove_file(&tmp_path);
        return Err(anyhow::anyhow!(
            "Failed to replace binary at '{}': {e}. You may need elevated permissions.",
            current_exe.display()
        ));
    }

    println!("\nSuccessfully updated pkg to {latest_tag}!");
    Ok(())
}

pub(crate) fn run_self_uninstall(
    purge: bool,
    assume_yes: bool,
    dry_run: bool,
    no_modify_path: bool,
) -> Result<()> {
    // 1. Locate binaries
    let mut binaries = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if exe.exists() {
            binaries.push(exe);
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let local_bin = PathBuf::from(&home).join(".local/bin/pkg");
        if local_bin.exists() && !binaries.contains(&local_bin) {
            binaries.push(local_bin);
        }
    }

    let usr_local_bin = PathBuf::from("/usr/local/bin/pkg");
    if usr_local_bin.exists() && !binaries.contains(&usr_local_bin) {
        binaries.push(usr_local_bin);
    }

    // 2. Identify data directories
    let data_dir = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let h = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(h).join(".local/share")
        })
        .join("pkg");

    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let h = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(h).join(".config")
        })
        .join("pkg");

    let cache_dir = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let h = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(h).join(".cache")
        })
        .join("pkg");

    let mut do_purge = purge;

    if !dry_run && !assume_yes {
        if !prompt_confirm("Are you sure you want to uninstall pkg? [y/N]: ", false)? {
            println!("Uninstallation cancelled.");
            return Ok(());
        }

        if !do_purge && (data_dir.exists() || config_dir.exists()) {
            println!("\nNotice: Package store and configuration exist at:");
            if data_dir.exists() {
                println!("  - {}", data_dir.display());
            }
            if config_dir.exists() {
                println!("  - {}", config_dir.display());
            }
            if prompt_confirm(
                "Do you also want to purge all stored packages and configuration? [y/N]: ",
                false,
            )? {
                do_purge = true;
            } else {
                println!("Preserving package store and configuration.");
            }
        }
    }

    if dry_run {
        println!("[DRY-RUN MODE] No files will be removed.");
    }

    // 3. Remove binaries
    for bin in &binaries {
        if dry_run {
            println!("Would remove executable: {}", bin.display());
        } else {
            match fs::remove_file(bin) {
                Ok(_) => println!("Removed executable: {}", bin.display()),
                Err(e) => eprintln!("Warning: Failed to remove '{}': {e}", bin.display()),
            }
        }
    }

    // 4. Purge data if requested
    if do_purge {
        for (label, dir) in [
            ("data & store directory", &data_dir),
            ("configuration directory", &config_dir),
            ("cache directory", &cache_dir),
        ] {
            if dir.exists() && is_safe_to_delete(dir) {
                if dry_run {
                    println!("Would remove {label}: {}", dir.display());
                } else {
                    match fs::remove_dir_all(dir) {
                        Ok(_) => println!("Removed {label}: {}", dir.display()),
                        Err(e) => eprintln!("Warning: Failed to remove '{}': {e}", dir.display()),
                    }
                }
            }
        }
    } else if data_dir.exists() || config_dir.exists() {
        println!("Kept data and configuration intact (use --purge to remove).");
    }

    // 5. Clean shell rc files
    if !no_modify_path {
        if let Ok(home) = std::env::var("HOME") {
            let home_path = PathBuf::from(home);
            let rc_files = [
                home_path.join(".bashrc"),
                home_path.join(".bash_profile"),
                home_path.join(".profile"),
                home_path.join(".zshrc"),
                home_path.join(".config/fish/config.fish"),
            ];

            for rc in &rc_files {
                if rc.exists() {
                    if let Ok(content) = fs::read_to_string(rc) {
                        if content.contains("pkg-managed") {
                            if dry_run {
                                println!(
                                    "Would clean pkg PATH configuration from: {}",
                                    rc.display()
                                );
                            } else {
                                let lines: Vec<&str> = content.lines().collect();
                                let mut cleaned = Vec::new();
                                let mut skip_next = false;
                                for line in lines {
                                    if line.contains("# pkg-managed paths") {
                                        skip_next = true;
                                        continue;
                                    }
                                    if skip_next {
                                        skip_next = false;
                                        continue;
                                    }
                                    if (line.contains("profiles/default/bin")
                                        && line.contains("export PATH="))
                                        || line.contains("fish_add_path") && line.contains("pkg")
                                    {
                                        continue;
                                    }
                                    cleaned.push(line);
                                }
                                let mut new_content = cleaned.join("\n");
                                if !new_content.is_empty() {
                                    new_content.push('\n');
                                }
                                if fs::write(rc, new_content).is_ok() {
                                    println!("Cleaned PATH configuration in {}", rc.display());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if dry_run {
        println!("\nDry-run completed. No changes were made.");
    } else {
        println!("\npkg has been successfully uninstalled.");
    }

    Ok(())
}
