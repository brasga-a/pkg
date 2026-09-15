//! ALPM Sync Database repository metadata adapter.
//!
//! Synchronizes Arch Linux repositories (core, extra, multilib, and third-party binary repos
//! such as chaotic-aur) by fetching `<repo>.db`, verifying detached OpenPGP signatures,
//! and parsing `desc` metadata entries from the database tarball.

use crate::domain::package::RemotePackage;
use crate::error::{Error, Result};
use crate::repository::crypto::{bounded_response, verify_detached_signature};
use flate2::read::GzDecoder;
use reqwest::Client;
use std::io::{Cursor, Read};
use std::path::Path;
use tar::Archive;

const MAX_DB_BYTES: u64 = 64 * 1024 * 1024; // 64MB database tarball limit

/// Maps current host architecture to Arch Linux architecture string.
fn get_arch_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "arm" => "armv7h",
        "x86" | "i686" => "i686",
        other => other,
    }
}

/// Parses raw text of an ALPM `desc` file into key-value map and then into a `RemotePackage`.
pub fn parse_alpm_desc(desc_content: &str, base_package_url: &str) -> Option<RemotePackage> {
    let mut current_key: Option<&str> = None;
    let mut name = String::new();
    let mut version = String::new();
    let mut arch = String::new();
    let mut digest = String::new();
    let mut size_bytes = 0u64;
    let mut filename = String::new();

    for line in desc_content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('%') && trimmed.ends_with('%') && trimmed.len() > 2 {
            current_key = Some(&trimmed[1..trimmed.len() - 1]);
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match current_key {
            Some("NAME") if name.is_empty() => name = trimmed.to_string(),
            Some("VERSION") if version.is_empty() => version = trimmed.to_string(),
            Some("ARCH") if arch.is_empty() => arch = trimmed.to_string(),
            Some("SHA256SUM") if digest.is_empty() => digest = trimmed.to_string(),
            Some("CSIZE") if size_bytes == 0 => size_bytes = trimmed.parse::<u64>().unwrap_or(0),
            Some("FILENAME") if filename.is_empty() => filename = trimmed.to_string(),
            _ => {}
        }
    }

    if name.is_empty() || version.is_empty() || filename.is_empty() {
        return None;
    }

    let url = if filename.starts_with("http://") || filename.starts_with("https://") {
        filename
    } else {
        format!("{}/{}", base_package_url.trim_end_matches('/'), filename)
    };

    Some(RemotePackage {
        repository_id: String::new(),
        name,
        version,
        architecture: if arch.is_empty() {
            "any".to_string()
        } else {
            arch
        },
        format: "alpm".to_string(),
        digest,
        size_bytes,
        url,
    })
}

/// Parses an ALPM database tarball from raw bytes into a list of normalized `RemotePackage` entities.
pub fn parse_alpm_db_archive(
    db_bytes: &[u8],
    base_package_url: &str,
) -> Result<Vec<RemotePackage>> {
    let cursor = Cursor::new(db_bytes);
    let decoder: Box<dyn Read> = if db_bytes.starts_with(b"\x1f\x8b") {
        Box::new(GzDecoder::new(cursor))
    } else if db_bytes.starts_with(b"\x28\xb5\x2f\xfd") {
        Box::new(
            zstd::stream::Decoder::new(cursor)
                .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?,
        )
    } else {
        Box::new(cursor)
    };

    let mut archive = Archive::new(decoder);
    let entries = archive.entries().map_err(|e| {
        Error::Parse(format!(
            "Failed to read ALPM repository archive entries: {e}"
        ))
    })?;

    let mut packages = Vec::new();

    for entry_res in entries {
        let mut entry = entry_res
            .map_err(|e| Error::Parse(format!("Failed to read ALPM archive entry header: {e}")))?;

        let path = entry
            .path()
            .map_err(|e| Error::Parse(format!("Invalid ALPM archive entry path encoding: {e}")))?;

        if path.ends_with("desc") {
            let mut desc_text = String::new();
            entry.read_to_string(&mut desc_text).map_err(|e| {
                Error::Parse(format!("Failed to read ALPM desc entry content: {e}"))
            })?;

            if let Some(pkg) = parse_alpm_desc(&desc_text, base_package_url) {
                packages.push(pkg);
            }
        }
    }

    Ok(packages)
}

/// Resolves the database download URL and the base package download URL.
fn resolve_alpm_urls(url: &str, distribution: &str, host_arch: &str) -> (String, String) {
    let clean_url = url.trim_end_matches('/');

    if clean_url.ends_with(".db")
        || clean_url.ends_with(".db.tar.gz")
        || clean_url.ends_with(".db.tar.zst")
    {
        let base_pkg_url = match clean_url.rfind('/') {
            Some(idx) => clean_url[..idx].to_string(),
            None => clean_url.to_string(),
        };
        return (clean_url.to_string(), base_pkg_url);
    }

    let repo_name = if !distribution.is_empty() && distribution != "-" && distribution != "default"
    {
        distribution
    } else {
        // Infer from last segment of URL if applicable
        clean_url.split('/').next_back().unwrap_or("core")
    };

    // If URL already contains the architecture suffix (e.g. os/x86_64)
    if clean_url.ends_with(&format!("os/{host_arch}")) || clean_url.ends_with(host_arch) {
        let db_url = format!("{clean_url}/{repo_name}.db");
        (db_url, clean_url.to_string())
    } else {
        let pkg_base = format!("{clean_url}/{repo_name}/os/{host_arch}");
        let db_url = format!("{pkg_base}/{repo_name}.db");
        (db_url, pkg_base)
    }
}

/// Fetches and verifies an ALPM repository sync database, returning normalized RemotePackages.
pub async fn update_alpm_repository(
    url: &str,
    distribution: &str,
    _components: &[String],
    public_key_path: Option<&Path>,
    _keyrings_dir: &Path,
) -> Result<Vec<RemotePackage>> {
    let client = Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;

    let host_arch = get_arch_architecture();
    let (db_url, base_pkg_url) = resolve_alpm_urls(url, distribution, host_arch);

    // 1. Fetch <repo>.db
    let response = client
        .get(&db_url)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    let db_bytes = bounded_response(response, MAX_DB_BYTES).await?;

    // 2. Cryptographic signature check if public key configured
    if let Some(key_path) = public_key_path {
        let sig_url = format!("{db_url}.sig");
        let sig_resp = client
            .get(&sig_url)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        let sig_bytes = bounded_response(sig_resp, 1024 * 1024).await?;
        verify_detached_signature(&db_bytes, &sig_bytes, key_path)?;
        println!(
            "  ✓ Verified OpenPGP signature for ALPM repository {} using {}",
            db_url,
            key_path.display()
        );
    }

    // 3. Parse database archive
    parse_alpm_db_archive(&db_bytes, &base_pkg_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::Builder;

    #[test]
    fn test_parse_alpm_desc() {
        let desc = r#"
%FILENAME%
firefox-130.0-1-x86_64.pkg.tar.zst

%NAME%
firefox

%BASE%
firefox

%VERSION%
130.0-1

%DESC%
Standalone web browser from mozilla.org

%CSIZE%
75000000

%ISIZE%
250000000

%SHA256SUM%
11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff

%ARCH%
x86_64
"#;

        let pkg = parse_alpm_desc(desc, "https://mirror.archlinux.org/extra/os/x86_64")
            .expect("Should parse ALPM desc");

        assert_eq!(pkg.name, "firefox");
        assert_eq!(pkg.version, "130.0-1");
        assert_eq!(pkg.architecture, "x86_64");
        assert_eq!(pkg.format, "alpm");
        assert_eq!(
            pkg.digest,
            "11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff"
        );
        assert_eq!(pkg.size_bytes, 75000000);
        assert_eq!(
            pkg.url,
            "https://mirror.archlinux.org/extra/os/x86_64/firefox-130.0-1-x86_64.pkg.tar.zst"
        );
    }

    #[test]
    fn test_parse_alpm_db_archive() {
        // Create an in-memory tar.gz ALPM database
        let mut tar_builder = Builder::new(Vec::new());

        let desc_content = b"%FILENAME%\ncurl-8.10.1-1-x86_64.pkg.tar.zst\n\n%NAME%\ncurl\n\n%VERSION%\n8.10.1-1\n\n%CSIZE%\n380000\n\n%SHA256SUM%\nfedcba9876543210\n\n%ARCH%\nx86_64\n";
        let mut header = tar::Header::new_gnu();
        header.set_path("curl-8.10.1-1/desc").unwrap();
        header.set_size(desc_content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar_builder.append(&header, &desc_content[..]).unwrap();

        let tar_bytes = tar_builder.into_inner().unwrap();

        let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut gz_encoder, &tar_bytes).unwrap();
        let db_bytes = gz_encoder.finish().unwrap();

        let packages =
            parse_alpm_db_archive(&db_bytes, "https://geo.mirror.pkgbuild.com/core/os/x86_64")
                .expect("Should parse tar.gz database");

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "curl");
        assert_eq!(packages[0].version, "8.10.1-1");
        assert_eq!(packages[0].format, "alpm");
        assert_eq!(packages[0].digest, "fedcba9876543210");
        assert_eq!(packages[0].size_bytes, 380000);
        assert_eq!(
            packages[0].url,
            "https://geo.mirror.pkgbuild.com/core/os/x86_64/curl-8.10.1-1-x86_64.pkg.tar.zst"
        );
    }
}
