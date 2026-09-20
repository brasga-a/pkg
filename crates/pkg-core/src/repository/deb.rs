use crate::domain::package::RemotePackage;
use crate::error::{Error, Result};
use crate::format::deb::DebAdapter;
use flate2::read::GzDecoder;
use futures::StreamExt;
use pgp::{cleartext::CleartextSignedMessage, composed::SignedPublicKey};
use reqwest::Client;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use xz2::read::XzDecoder;

/// Maps current host architecture to Debian architecture string.
fn get_debian_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "armhf",
        "x86" | "i686" => "i386",
        "riscv64" => "riscv64",
        other => other,
    }
}

/// Fetches and verifies Debian repository metadata, producing a list of packages.
pub async fn update_debian_repository(
    url: &str,
    distribution: &str,
    components: &[String],
    public_key_path: Option<&Path>,
    keyrings_dir: &Path,
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

    // 1. Fetch InRelease
    let inrelease_url = format!("{url}/dists/{distribution}/InRelease");
    let response = client
        .get(&inrelease_url)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    let inrelease_bytes = bounded_response(response, 4 * 1024 * 1024).await?;
    let inrelease_text = String::from_utf8(inrelease_bytes)
        .map_err(|e| Error::Parse(format!("Invalid InRelease text: {e}")))?;
    let resolved_key =
        resolve_or_fetch_keyring(&client, public_key_path, url, distribution, keyrings_dir).await;
    let key_path = resolved_key.ok_or_else(|| Error::SecurityViolation(format!(
        "No trusted GPG keyring found for repository {url} ({distribution}). Cryptographic signature verification is strictly required."
    )))?;
    let release = verify_inrelease_signature(&inrelease_text, &key_path)?;
    println!(
        "  ✓ Verified GPG signature for {url} ({distribution}) using {}",
        key_path.display()
    );
    let checksums = parse_release_checksums(&release)?;
    let host_arch = get_debian_architecture();
    let fetches = futures::stream::iter(components.iter().cloned().map(|component| {
        let client = &client;
        let checksums = &checksums;
        async move {
            for extension in ["xz", "gz"] {
                let relative = format!("{component}/binary-{host_arch}/Packages.{extension}");
                let Some(expected) = checksums.get(&relative) else {
                    continue;
                };
                if expected.size > MAX_INDEX_BYTES {
                    return Err(Error::LimitsExceeded(format!(
                        "Index too large: {relative}"
                    )));
                }
                let response = client
                    .get(format!("{url}/dists/{distribution}/{relative}"))
                    .send()
                    .await
                    .map_err(|e| Error::Network(e.to_string()))?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    continue;
                }
                let raw = bounded_response(response, expected.size).await?;
                expected.verify(&raw)?;
                let decoder: Box<dyn Read> = if extension == "gz" {
                    Box::new(GzDecoder::new(&raw[..]))
                } else {
                    let stream = xz2::stream::Stream::new_auto_decoder(
                        MAX_INDEX_BYTES,
                        xz2::stream::CONCATENATED,
                    )
                    .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
                    Box::new(XzDecoder::new_stream(&raw[..], stream))
                };
                let mut text = String::new();
                decoder
                    .take(MAX_INDEX_TEXT_BYTES + 1)
                    .read_to_string(&mut text)?;
                if text.len() as u64 > MAX_INDEX_TEXT_BYTES {
                    return Err(Error::LimitsExceeded(
                        "Decompressed package index is too large".into(),
                    ));
                }
                let mut packages = Vec::new();
                parse_packages_file(&text, url, distribution, &mut packages);
                for package in &packages {
                    if package.digest.len() != 64
                        || !package.digest.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(Error::SecurityViolation(format!(
                            "Invalid SHA256 for package {}",
                            package.name
                        )));
                    }
                }
                return Ok(packages);
            }
            Err(Error::SecurityViolation(format!(
                "No available SHA256-authenticated index for {component}/{host_arch}"
            )))
        }
    }))
    .buffered(4);
    futures::pin_mut!(fetches);
    let mut packages = Vec::new();
    while let Some(result) = fetches.next().await {
        packages.extend(result?);
    }
    Ok(packages)
}

const MAX_INDEX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_INDEX_TEXT_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug)]
struct IndexChecksum {
    digest: String,
    size: u64,
}

impl IndexChecksum {
    fn verify(&self, bytes: &[u8]) -> Result<()> {
        use sha2::{Digest, Sha256};
        let actual = format!("{:x}", Sha256::digest(bytes));
        if bytes.len() as u64 != self.size || !actual.eq_ignore_ascii_case(&self.digest) {
            return Err(Error::SecurityViolation(
                "Package index differs from signed InRelease size or SHA256".into(),
            ));
        }
        Ok(())
    }
}

fn parse_release_checksums(
    release: &str,
) -> Result<std::collections::HashMap<String, IndexChecksum>> {
    let mut checksums = std::collections::HashMap::new();
    let mut in_sha256 = false;
    for line in release.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_sha256 = line == "SHA256:";
            continue;
        }
        if !in_sha256 {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3
            || fields[0].len() != 64
            || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Parse("Invalid InRelease SHA256 entry".into()));
        }
        let size = fields[1]
            .parse()
            .map_err(|_| Error::Parse("Invalid InRelease index size".into()))?;
        if checksums
            .insert(
                fields[2].into(),
                IndexChecksum {
                    digest: fields[0].into(),
                    size,
                },
            )
            .is_some()
        {
            return Err(Error::Parse("Duplicate InRelease SHA256 entry".into()));
        }
    }
    if checksums.is_empty() {
        return Err(Error::SecurityViolation(
            "InRelease has no SHA256 index hashes".into(),
        ));
    }
    Ok(checksums)
}

async fn bounded_response(response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    let response = response
        .error_for_status()
        .map_err(|e| Error::Network(e.to_string()))?;
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(Error::LimitsExceeded(
            "Repository response exceeds allowed size".into(),
        ));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Network(e.to_string()))?;
        if body.len() as u64 + chunk.len() as u64 > limit {
            return Err(Error::LimitsExceeded(
                "Repository response exceeds allowed size".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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

    // If ubuntu and not on host, fetch Ubuntu official archive keyring automatically
    if url.to_lowercase().contains("ubuntu") {
        let ubuntu_key = keyrings_dir.join("ubuntu-archive-keyring.gpg");
        if ubuntu_key.exists() {
            return Some(ubuntu_key);
        }
        let key_urls = [
            format!(
                "{}/project/ubuntu-archive-keyring.gpg",
                url.trim_end_matches('/')
            ),
            "http://archive.ubuntu.com/ubuntu/project/ubuntu-archive-keyring.gpg".to_string(),
        ];
        for key_url in &key_urls {
            if let Ok(res) = client.get(key_url).send().await {
                if res.status().is_success() {
                    if let Ok(bytes) = res.bytes().await {
                        let _ = tokio::fs::create_dir_all(keyrings_dir).await;
                        if tokio::fs::write(&ubuntu_key, &bytes).await.is_ok() {
                            return Some(ubuntu_key);
                        }
                    }
                }
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
            if res.status().is_success() {
                if let Ok(bytes) = res.bytes().await {
                    let _ = tokio::fs::create_dir_all(keyrings_dir).await;
                    if tokio::fs::write(&debian_key, &bytes).await.is_ok() {
                        return Some(debian_key);
                    }
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

fn verify_inrelease_signature(signed_text: &str, key_path: &Path) -> Result<String> {
    let key_bytes = fs::read(key_path).map_err(Error::Io)?;
    let (msg, _) = CleartextSignedMessage::from_string(signed_text)
        .map_err(|e| Error::Parse(format!("Failed to parse InRelease signed message: {:?}", e)))?;

    // Try parsing as multi-key binary keyring (filtering out GPG Tag::Trust packets)
    use pgp::composed::signed_key::SignedPublicKeyParser;
    use pgp::packet::PacketParser;
    use pgp::types::Tag;

    let p_parser = PacketParser::new(&key_bytes[..]);
    let filtered_packets = p_parser
        .filter(|p| {
            if let Ok(pkt) = p {
                pkt.tag() != Tag::Trust
            } else {
                true
            }
        })
        .peekable();

    let keys_parser = SignedPublicKeyParser::from_packets(filtered_packets);
    let mut verified = false;

    for key in keys_parser.flatten() {
        if verify_key_or_subkeys(&msg, &key) {
            verified = true;
            break;
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

    Ok(msg.signed_text())
}

/// Parses a Debian `Packages` index into normalized remote package records.
///
/// This public adapter is also used by the reproducible benchmark harness so
/// repository parsing can be measured without network access or signature
/// acquisition.
pub fn parse_packages_index(text: &str, base_url: &str, dist: &str) -> Vec<RemotePackage> {
    let mut packages = Vec::new();
    parse_packages_file(text, base_url, dist, &mut packages);
    packages
}

fn parse_packages_file(text: &str, base_url: &str, _dist: &str, packages: &mut Vec<RemotePackage>) {
    let mut name = String::new();
    let mut version = String::new();
    let mut arch = String::new();
    let mut filename = String::new();
    let mut sha256 = String::new();
    let mut size: u64 = 0;
    let mut depends = String::new();
    let mut conflicts = String::new();
    let mut provides = String::new();

    for line in text.lines() {
        if line.is_empty() {
            if !name.is_empty() && !filename.is_empty() {
                let metadata = HashMap::from([
                    ("Depends".to_string(), depends.clone()),
                    ("Conflicts".to_string(), conflicts.clone()),
                    ("Provides".to_string(), provides.clone()),
                ]);
                let (provides, versioned_provides) = DebAdapter::parse_provides(&metadata);
                packages.push(RemotePackage {
                    repository_id: format!("{}-{}", base_url, name), // just a mock ID base
                    name: name.clone(),
                    version: version.clone(),
                    architecture: arch.clone(),
                    format: "deb".to_string(),
                    digest: sha256.clone(),
                    size_bytes: size,
                    url: format!("{}/{}", base_url, filename),
                    constraints: DebAdapter::parse_constraints(&metadata),
                    provides,
                    versioned_provides,
                });
            }
            name.clear();
            version.clear();
            arch.clear();
            filename.clear();
            sha256.clear();
            size = 0;
            depends.clear();
            conflicts.clear();
            provides.clear();
            continue;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            // Multiline continuation field (e.g. Description), ignore
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
        } else if let Some(stripped) = line.strip_prefix("Depends: ") {
            depends = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("Pre-Depends: ") {
            if depends.is_empty() {
                depends = stripped.to_string();
            } else {
                depends.push_str(", ");
                depends.push_str(stripped);
            }
        } else if let Some(stripped) = line.strip_prefix("Conflicts: ") {
            conflicts = stripped.to_string();
        } else if let Some(stripped) = line.strip_prefix("Breaks: ") {
            if conflicts.is_empty() {
                conflicts = stripped.to_string();
            } else {
                conflicts.push_str(", ");
                conflicts.push_str(stripped);
            }
        } else if let Some(stripped) = line.strip_prefix("Provides: ") {
            provides = stripped.to_string();
        }
    }

    // Flush last package if file didn't end with an empty line
    if !name.is_empty() && !filename.is_empty() {
        let metadata = HashMap::from([
            ("Depends".to_string(), depends.clone()),
            ("Conflicts".to_string(), conflicts.clone()),
            ("Provides".to_string(), provides.clone()),
        ]);
        let (provides, versioned_provides) = DebAdapter::parse_provides(&metadata);
        packages.push(RemotePackage {
            repository_id: format!("{}-{}", base_url, name),
            name: name.clone(),
            version: version.clone(),
            architecture: arch.clone(),
            format: "deb".to_string(),
            digest: sha256.clone(),
            size_bytes: size,
            url: format!("{}/{}", base_url, filename),
            constraints: DebAdapter::parse_constraints(&metadata),
            provides,
            versioned_provides,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_signed_fixture() {
        let temp = tempfile::tempdir().unwrap();
        let key = temp.path().join("key.asc");
        fs::write(
            &key,
            include_bytes!("../../../../tests/fixtures/repository/test-key.asc"),
        )
        .unwrap();
        let release = verify_inrelease_signature(
            include_str!("../../../../tests/fixtures/repository/InRelease"),
            &key,
        )
        .unwrap();
        assert!(release.contains("SHA256:"));
    }

    #[test]
    fn release_checksums_require_sha256_and_unique_entries() {
        assert!(
            parse_release_checksums("Suite: test\nMD5Sum:\n deadbeef 5 Packages.gz\n").is_err()
        );
        let entry = format!(" {} 3 main/binary-amd64/Packages.gz\n", "0".repeat(64));
        assert!(parse_release_checksums(&format!("SHA256:\n{entry}{entry}")).is_err());
        assert!(
            IndexChecksum {
                digest: "0".repeat(64),
                size: 3
            }
            .verify(b"two")
            .is_err()
        );
    }

    #[test]
    fn test_parse_packages_file_with_multiline_fields() {
        let sample = r#"Package: curl
Version: 7.88.1-10+deb12u5
Architecture: amd64
Maintainer: Debian cURL Maintainers <pkg-curl-devel@lists.alioth.debian.org>
Installed-Size: 435
Depends: libc6 (>= 2.34), libcurl4 (= 7.88.1-10+deb12u5), zlib1g (>= 1:1.1.4)
Filename: pool/main/c/curl/curl_7.88.1-10+deb12u5_amd64.deb
Size: 209860
SHA256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
Description: command line tool for transferring data with URL syntax
 curl is a command line tool for transferring data with URL syntax, supporting
 DICT, FILE, FTP, FTPS, GOPHER, GOPHERS, HTTP, HTTPS, IMAP, IMAPS, LDAP,
 LDAPS, MQTT, POP3, POP3S, RTMP, RTMPS, RTSP, SCP, SFTP, SMB, SMBS, SMTP,
 SMTPS, TELNET, TFTP, WS and WSS.

Package: wget
Version: 1.21.3-1+b2
Architecture: amd64
Filename: pool/main/w/wget/wget_1.21.3-1+b2_amd64.deb
Size: 991284
SHA256: 01ba4719c80b6fe911b091a7c05124b64eeece964e09c058ef8f9805daca546b
"#;
        let mut packages = Vec::new();
        parse_packages_file(
            sample,
            "http://deb.debian.org/debian",
            "bookworm",
            &mut packages,
        );
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "curl");
        assert_eq!(packages[0].version, "7.88.1-10+deb12u5");
        assert_eq!(packages[0].architecture, "amd64");
        assert_eq!(packages[0].size_bytes, 209860);
        assert_eq!(
            packages[0].digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            packages[0].url,
            "http://deb.debian.org/debian/pool/main/c/curl/curl_7.88.1-10+deb12u5_amd64.deb"
        );

        assert_eq!(packages[1].name, "wget");
        assert_eq!(packages[1].version, "1.21.3-1+b2");
        assert_eq!(packages[1].size_bytes, 991284);
    }

    #[test]
    fn packages_file_preserves_versioned_provides() {
        let sample = "Package: provider\nVersion: 1\nArchitecture: amd64\nProvides: virtual-api (= 2.4), unversioned-api\nFilename: pool/provider.deb\nSize: 1\nSHA256: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        let mut packages = Vec::new();
        parse_packages_file(
            sample,
            "https://example.invalid/repo",
            "stable",
            &mut packages,
        );
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].provides.len(), 1);
        assert_eq!(packages[0].versioned_provides.len(), 1);
        assert_eq!(packages[0].versioned_provides[0].version.as_str(), "2.4");
    }
}
