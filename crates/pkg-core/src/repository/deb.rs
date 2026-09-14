use std::path::Path;
use reqwest::Client;
use std::fs;
use pgp::{SignedPublicKey, cleartext::CleartextSignedMessage};
use pgp::Deserializable;
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

    // 2. Verify GPG Signature if a key is provided
    if let Some(key_path) = public_key_path {
        verify_inrelease_signature(&inrelease_text, key_path)?;
    } else {
        tracing::warn!("Skipping GPG signature verification for repository {}", url);
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

fn verify_inrelease_signature(signed_text: &str, key_path: &Path) -> Result<()> {
    let key_bytes = fs::read(key_path).map_err(|e| Error::Io(e))?;
    // We attempt to parse the public key. Depending on keyring format, this might need more robust parsing.
    let key = SignedPublicKey::from_bytes(&key_bytes[..])
        .map_err(|e| Error::Parse(format!("Failed to parse public key: {:?}", e)))?;

    let msg = CleartextSignedMessage::from_string(signed_text)
        .map_err(|e| Error::Parse(format!("Failed to parse InRelease signed message: {:?}", e)))?;

    msg.0.verify(&key).map_err(|e| Error::Parse(format!("GPG signature verification failed: {:?}", e)))?;
    
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
