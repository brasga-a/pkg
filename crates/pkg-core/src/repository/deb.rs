use std::path::Path;
use reqwest::Client;
use std::fs;
use pgp::{cleartext::CleartextSignedMessage, composed::SignedPublicKey};
use crate::error::{Error, Result};
use crate::domain::package::RemotePackage;
use flate2::read::GzDecoder;
use xz2::read::XzDecoder;
use std::io::Read;

/// Fetches and verifies Debian repository metadata, producing a list of packages.
pub async fn update_debian_repository(
    url: &str,
    distribution: &str,
    components: &[String],
    public_key_path: Option<&Path>,
    keyrings_dir: &Path,
) -> Result<Vec<RemotePackage>> {
    let client = Client::builder()
        .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;

    // 1. Fetch InRelease
    let inrelease_url = format!("{url}/dists/{distribution}/InRelease");
    let response = client.get(&inrelease_url).send().await
        .map_err(|e| Error::Network(e.to_string()))?;
        
    let inrelease_text = response.text().await
        .map_err(|e| Error::Network(e.to_string()))?;

    // 2. Resolve keyring and verify GPG Signature
    let resolved_key = resolve_or_fetch_keyring(&client, public_key_path, url, distribution, keyrings_dir).await;
    if let Some(ref key_path) = resolved_key {
        verify_inrelease_signature(&inrelease_text, key_path)?;
        println!("  ✓ Verified GPG signature for {url} ({distribution}) using {}", key_path.display());
    } else {
        tracing::warn!("Skipping GPG signature verification for repository {} (no keyring found)", url);
    }

    // For now, we will simply fetch the Packages file for each component, architecture amd64
    let mut packages = Vec::new();
    
    for component in components {
        let pkgs_url = format!("{url}/dists/{distribution}/{component}/binary-amd64/Packages.xz");
        let pkgs_response = client.get(&pkgs_url).send().await;
        
        let mut raw_bytes = Vec::new();
        if let Ok(res) = pkgs_response {
            if res.status().is_success() {
                raw_bytes = res.bytes().await.map_err(|e| Error::Network(e.to_string()))?.to_vec();
            }
        }
        
        if raw_bytes.is_empty() {
            // Try .gz if .xz fails
            let pkgs_gz_url = format!("{url}/dists/{distribution}/{component}/binary-amd64/Packages.gz");
            let res = client.get(&pkgs_gz_url).send().await.map_err(|e| Error::Network(e.to_string()))?;
            let res = res.error_for_status().map_err(|e| Error::Network(e.to_string()))?;
            let gz_bytes = res.bytes().await.map_err(|e| Error::Network(e.to_string()))?.to_vec();
            
            let mut decoder = GzDecoder::new(&gz_bytes[..]);
            let mut text = String::new();
            decoder.read_to_string(&mut text).map_err(Error::Io)?;
            parse_packages_file(&text, url, distribution, &mut packages);
        } else {
            let stream = xz2::stream::Stream::new_auto_decoder(u64::MAX, xz2::stream::CONCATENATED)
                .map_err(|e| Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;
            let mut decoder = XzDecoder::new_stream(&raw_bytes[..], stream);
            let mut text = String::new();
            decoder.read_to_string(&mut text).map_err(Error::Io)?;
            parse_packages_file(&text, url, distribution, &mut packages);
        }
    }

    Ok(packages)
}

async fn resolve_or_fetch_keyring(
    client: &Client,
    key_path: Option<&Path>,
    url: &str,
    distribution: &str,
    keyrings_dir: &Path,
) -> Option<std::path::PathBuf> {
    if let Some(path) = key_path {
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }

    // Auto-detect standard host keyrings
    let host_candidates = [
        Path::new("/usr/share/keyrings/ubuntu-archive-keyring.gpg"),
        Path::new("/etc/apt/trusted.gpg.d/ubuntu-keyring-2018-archive.gpg"),
        Path::new("/usr/share/keyrings/debian-archive-keyring.gpg"),
    ];

    for candidate in &host_candidates {
        if candidate.exists() {
            let lower = url.to_lowercase();
            let c_lower = candidate.to_string_lossy().to_lowercase();
            if (lower.contains("ubuntu") && c_lower.contains("ubuntu"))
                || (lower.contains("debian") && c_lower.contains("debian"))
            {
                return Some(candidate.to_path_buf());
            }
        }
    }

    // If debian and not on host, fetch Debian official archive key automatically
    if url.contains("debian.org") {
        let debian_key = keyrings_dir.join(format!("debian-{distribution}.asc"));
        if debian_key.exists() {
            return Some(debian_key);
        }
        let key_url = "https://ftp-master.debian.org/keys/archive-key-12.asc";
        if let Ok(res) = client.get(key_url).send().await {
            if let Ok(bytes) = res.bytes().await {
                let _ = tokio::fs::create_dir_all(keyrings_dir).await;
                if tokio::fs::write(&debian_key, &bytes).await.is_ok() {
                    return Some(debian_key);
                }
            }
        }
    }

    None
}

fn verify_key_or_subkeys(msg: &CleartextSignedMessage, key: &SignedPublicKey) -> bool {
    if msg.verify(key).is_ok() {
        return true;
    }
    for subkey in &key.public_subkeys {
        if msg.verify(subkey).is_ok() {
            return true;
        }
    }
    false
}

fn verify_inrelease_signature(signed_text: &str, key_path: &Path) -> Result<()> {
    let key_bytes = fs::read(key_path).map_err(Error::Io)?;
    let (msg, _) = CleartextSignedMessage::from_string(signed_text)
        .map_err(|e| Error::Parse(format!("Failed to parse InRelease signed message: {:?}", e)))?;

    // Try parsing as multi-key binary keyring (filtering out GPG Tag::Trust packets)
    use pgp::packet::PacketParser;
    use pgp::composed::signed_key::SignedPublicKeyParser;
    use pgp::types::Tag;

    let p_parser = PacketParser::new(&key_bytes[..]);
    let filtered_packets = p_parser.filter(|p| {
        if let Ok(pkt) = p {
            pkt.tag() != Tag::Trust
        } else {
            true
        }
    }).peekable();

    let keys_parser = SignedPublicKeyParser::from_packets(filtered_packets);
    let mut verified = false;

    for key_res in keys_parser {
        if let Ok(key) = key_res {
            if verify_key_or_subkeys(&msg, &key) {
                verified = true;
                break;
            }
        }
    }

    if !verified {
        // Also try armored format (.asc) if binary didn't yield verification
        let armored_iter = pgp::composed::signed_key::from_armor_many(&key_bytes[..]);
        if let Ok((iter, _)) = armored_iter {
            for key_res in iter {
                if let Ok(pgp::composed::signed_key::PublicOrSecret::Public(key)) = key_res {
                    if verify_key_or_subkeys(&msg, &key) {
                        verified = true;
                        break;
                    }
                }
            }
        }
    }

    if !verified {
        return Err(Error::Parse(format!(
            "GPG signature verification failed for {}: no matching trusted key found in keyring",
            key_path.display()
        )));
    }

    Ok(())
}

fn parse_packages_file(text: &str, base_url: &str, _dist: &str, packages: &mut Vec<RemotePackage>) {
    let mut name = String::new();
    let mut version = String::new();
    let mut arch = String::new();
    let mut filename = String::new();
    let mut sha256 = String::new();
    let mut size: u64 = 0;

    for line in text.lines() {
        if line.is_empty() {
            if !name.is_empty() && !filename.is_empty() {
                packages.push(RemotePackage {
                    repository_id: format!("{}-{}", base_url, name), // just a mock ID base
                    name: name.clone(),
                    version: version.clone(),
                    architecture: arch.clone(),
                    format: "deb".to_string(),
                    digest: sha256.clone(),
                    size_bytes: size,
                    url: format!("{}/{}", base_url, filename),
                });
            }
            name.clear();
            version.clear();
            arch.clear();
            filename.clear();
            sha256.clear();
            size = 0;
            continue;
        }

        if let Some(stripped) = line.strip_prefix("Package: ") {
            name = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("Version: ") {
            version = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("Architecture: ") {
            arch = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("Filename: ") {
            filename = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("SHA256: ") {
            sha256 = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("Size: ") {
            size = stripped.parse().unwrap_or(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_debian_keyring() {
        let key_path = Path::new("/tmp/debian-key-12.asc");
        if !key_path.exists() {
            return;
        }
        let inrelease_path = Path::new("/tmp/debian-inrelease.txt");
        if !inrelease_path.exists() {
            return;
        }
        let inrelease_text = fs::read_to_string(inrelease_path).unwrap();
        let res = verify_inrelease_signature(&inrelease_text, key_path);
        println!("Debian verification result: {:?}", res);
        assert!(res.is_ok());
    }
}
