//! RPM-MD repository metadata adapter.
//!
//! Synchronizes remote RPM repositories (Fedora, RHEL, openSUSE, etc.) by fetching
//! `repodata/repomd.xml`, validating integrity and cryptographic signatures, and streaming
//! `primary.xml.gz` package definitions into normalized `RemotePackage` entities.

use crate::domain::capability::Capability;
use crate::domain::constraint::{CapabilityConstraint, Constraint, VersionConstraint, VersionOp};
use crate::domain::package::{PackageName, PackageVersion, RemotePackage, VersionedCapability};
use crate::error::{Error, Result};
use crate::repository::crypto::{bounded_response, verify_detached_signature};
use flate2::read::GzDecoder;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Cursor};
use std::path::Path;

const MAX_REPOMD_BYTES: u64 = 8 * 1024 * 1024; // 8MB
const MAX_PRIMARY_BYTES: u64 = 256 * 1024 * 1024; // 256MB

/// Metadata pointer for primary.xml inside repomd.xml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepomdPrimary {
    pub location_href: String,
    pub checksum_type: String,
    pub checksum: String,
    pub size_bytes: u64,
}

/// Parses repomd.xml to locate the primary.xml metadata file and its expected checksum.
pub fn parse_repomd_xml(xml: &str) -> Result<RepomdPrimary> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut in_primary_data = false;
    let mut in_checksum = false;
    let mut in_size = false;

    let mut location_href = None;
    let mut checksum_type = None;
    let mut checksum = None;
    let mut size_bytes = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"data" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"type" && attr.value.as_ref() == b"primary" {
                            in_primary_data = true;
                        }
                    }
                }
                b"location" if in_primary_data => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            location_href = Some(String::from_utf8_lossy(&attr.value).to_string());
                        }
                    }
                }
                b"checksum" if in_primary_data => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"type" {
                            checksum_type = Some(String::from_utf8_lossy(&attr.value).to_string());
                        }
                    }
                    in_checksum = true;
                }
                b"size" if in_primary_data => {
                    in_size = true;
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if in_primary_data && e.local_name().as_ref() == b"location" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            location_href = Some(String::from_utf8_lossy(&attr.value).to_string());
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if in_checksum && in_primary_data {
                    checksum = Some(String::from_utf8_lossy(e.as_ref()).trim().to_string());
                } else if in_size && in_primary_data {
                    size_bytes = String::from_utf8_lossy(e.as_ref())
                        .trim()
                        .parse::<u64>()
                        .ok();
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"data" => {
                    if in_primary_data {
                        break;
                    }
                }
                b"checksum" => in_checksum = false,
                b"size" => in_size = false,
                _ => {}
            },
            Err(e) => return Err(Error::Parse(format!("Failed to parse repomd.xml: {e}"))),
            _ => {}
        }
        buf.clear();
    }

    let location_href = location_href.ok_or_else(|| {
        Error::Parse("repomd.xml missing <location href> for primary data".into())
    })?;
    let checksum = checksum
        .ok_or_else(|| Error::Parse("repomd.xml missing <checksum> for primary data".into()))?;
    let checksum_type = checksum_type.unwrap_or_else(|| "sha256".to_string());
    let size_bytes = size_bytes.unwrap_or(0);

    Ok(RepomdPrimary {
        location_href,
        checksum_type,
        checksum,
        size_bytes,
    })
}

/// Parses primary.xml streaming to produce normalized `RemotePackage` entities.
pub fn parse_primary_xml<R: BufRead>(
    reader: R,
    host_arch: &str,
    base_url: &str,
) -> Result<Vec<RemotePackage>> {
    let mut xml_reader = Reader::from_reader(reader);
    xml_reader.config_mut().trim_text(true);

    let mut packages = Vec::new();
    let mut buf = Vec::new();

    // Package temporary state
    let mut in_package = false;
    let mut in_name = false;
    let mut in_arch = false;
    let mut in_checksum = false;

    let mut cur_name = String::new();
    let mut cur_arch = String::new();
    let mut cur_ver = String::new();
    let mut cur_rel = String::new();
    let mut cur_epoch = String::new();
    let mut cur_checksum = String::new();
    let mut is_cur_pkgid = false;
    let mut cur_size = 0u64;
    let mut cur_href = String::new();
    let mut in_requires = false;
    let mut in_provides = false;
    let mut cur_constraints: Vec<Constraint> = Vec::new();
    let mut cur_provides: Vec<Capability> = Vec::new();
    let mut cur_versioned_provides: Vec<VersionedCapability> = Vec::new();

    loop {
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"package" => {
                    in_package = true;
                    cur_name.clear();
                    cur_arch.clear();
                    cur_ver.clear();
                    cur_rel.clear();
                    cur_epoch.clear();
                    cur_checksum.clear();
                    is_cur_pkgid = false;
                    cur_size = 0;
                    cur_href.clear();
                    in_requires = false;
                    in_provides = false;
                    cur_constraints.clear();
                    cur_provides.clear();
                    cur_versioned_provides.clear();
                }
                b"name" if in_package => in_name = true,
                b"arch" if in_package => in_arch = true,
                b"version" if in_package => {
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"ver" => cur_ver = String::from_utf8_lossy(&attr.value).to_string(),
                            b"rel" => cur_rel = String::from_utf8_lossy(&attr.value).to_string(),
                            b"epoch" => {
                                cur_epoch = String::from_utf8_lossy(&attr.value).to_string()
                            }
                            _ => {}
                        }
                    }
                }
                b"checksum" if in_package => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"pkgid"
                            && attr.value.as_ref().eq_ignore_ascii_case(b"yes")
                        {
                            is_cur_pkgid = true;
                        }
                    }
                    in_checksum = true;
                }
                b"size" if in_package => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"package" {
                            cur_size = String::from_utf8_lossy(&attr.value)
                                .parse::<u64>()
                                .unwrap_or(0);
                        }
                    }
                }
                b"location" if in_package => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"href" {
                            cur_href = String::from_utf8_lossy(&attr.value).to_string();
                        }
                    }
                }
                b"requires" if in_package => in_requires = true,
                b"provides" if in_package => in_provides = true,
                b"entry" if in_package => {
                    // Fedora's primary.xml commonly emits dependency entries
                    // as regular start/end elements instead of self-closing
                    // tags.  Parse both forms so dependency metadata is not
                    // silently dropped.
                    parse_rpm_entry(
                        &e,
                        in_requires,
                        in_provides,
                        &mut cur_constraints,
                        &mut cur_provides,
                        &mut cur_versioned_provides,
                    );
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if in_package {
                    match e.local_name().as_ref() {
                        b"version" => {
                            for attr in e.attributes().flatten() {
                                match attr.key.as_ref() {
                                    b"ver" => {
                                        cur_ver = String::from_utf8_lossy(&attr.value).to_string()
                                    }
                                    b"rel" => {
                                        cur_rel = String::from_utf8_lossy(&attr.value).to_string()
                                    }
                                    b"epoch" => {
                                        cur_epoch = String::from_utf8_lossy(&attr.value).to_string()
                                    }
                                    _ => {}
                                }
                            }
                        }
                        b"size" => {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"package" {
                                    cur_size = String::from_utf8_lossy(&attr.value)
                                        .parse::<u64>()
                                        .unwrap_or(0);
                                }
                            }
                        }
                        b"location" => {
                            for attr in e.attributes().flatten() {
                                if attr.key.as_ref() == b"href" {
                                    cur_href = String::from_utf8_lossy(&attr.value).to_string();
                                }
                            }
                        }
                        b"entry" => {
                            parse_rpm_entry(
                                &e,
                                in_requires,
                                in_provides,
                                &mut cur_constraints,
                                &mut cur_provides,
                                &mut cur_versioned_provides,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(e)) => {
                let text = String::from_utf8_lossy(e.as_ref()).trim().to_string();
                if in_name && in_package {
                    cur_name = text;
                } else if in_arch && in_package {
                    cur_arch = text;
                } else if in_checksum && in_package && (is_cur_pkgid || cur_checksum.is_empty()) {
                    cur_checksum = text;
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"package" => {
                    in_package = false;
                    // Architecture filter
                    let arch_matches = cur_arch == host_arch
                        || cur_arch == "noarch"
                        || cur_arch == "all"
                        || cur_arch == "any"
                        || host_arch == "any";

                    if arch_matches
                        && !cur_name.is_empty()
                        && !cur_href.is_empty()
                        && !cur_checksum.is_empty()
                    {
                        let version = if !cur_epoch.is_empty()
                            && cur_epoch != "0"
                            && !cur_ver.contains(':')
                        {
                            format!("{}:{}-{}", cur_epoch, cur_ver, cur_rel)
                        } else if !cur_rel.is_empty() {
                            format!("{}-{}", cur_ver, cur_rel)
                        } else {
                            cur_ver.clone()
                        };

                        let package_url = if cur_href.starts_with("http://")
                            || cur_href.starts_with("https://")
                        {
                            cur_href.clone()
                        } else {
                            format!("{}/{}", base_url.trim_end_matches('/'), cur_href)
                        };

                        packages.push(RemotePackage {
                            repository_id: String::new(),
                            name: cur_name.clone(),
                            version,
                            architecture: cur_arch.clone(),
                            format: "rpm".to_string(),
                            digest: cur_checksum.clone(),
                            size_bytes: cur_size,
                            url: package_url,
                            constraints: cur_constraints.clone(),
                            provides: cur_provides.clone(),
                            versioned_provides: cur_versioned_provides.clone(),
                        });
                    }
                }
                b"name" => in_name = false,
                b"arch" => in_arch = false,
                b"checksum" => in_checksum = false,
                b"requires" => in_requires = false,
                b"provides" => in_provides = false,
                _ => {}
            },
            Err(e) => return Err(Error::Parse(format!("Failed to parse primary.xml: {e}"))),
            _ => {}
        }
        buf.clear();
    }

    Ok(packages)
}

fn parse_rpm_entry(
    event: &quick_xml::events::BytesStart<'_>,
    in_requires: bool,
    in_provides: bool,
    constraints: &mut Vec<Constraint>,
    provides: &mut Vec<Capability>,
    versioned_provides: &mut Vec<VersionedCapability>,
) {
    let mut name = None;
    let mut flags = None;
    let mut version = None;
    for attr in event.attributes().flatten() {
        let value = String::from_utf8_lossy(&attr.value).to_string();
        match attr.key.as_ref() {
            b"name" => name = Some(value),
            b"flags" => flags = Some(value),
            b"ver" => version = Some(value),
            b"rel" => {
                if let Some(existing) = version.as_mut() {
                    existing.push('-');
                    existing.push_str(&value);
                } else {
                    version = Some(value);
                }
            }
            _ => {}
        }
    }
    let Some(name) = name else { return };
    if in_provides {
        let capability = if name.contains(".so") {
            Capability::SharedLibrary(name)
        } else {
            Capability::Feature(name)
        };
        if let Some(version) = version.filter(|value| !value.is_empty()) {
            let version = PackageVersion::new(version);
            if version.validate().is_ok() {
                versioned_provides.push(VersionedCapability {
                    capability,
                    version,
                });
                return;
            }
        }
        provides.push(capability);
        return;
    }
    if !in_requires {
        return;
    }
    let version_constraint = match (flags.as_deref(), version) {
        (Some("EQ"), Some(v)) => {
            VersionConstraint::Relational(VersionOp::Exact, PackageVersion::new(v))
        }
        (Some("GE"), Some(v)) => {
            VersionConstraint::Relational(VersionOp::GreaterEqual, PackageVersion::new(v))
        }
        (Some("GT"), Some(v)) => {
            VersionConstraint::Relational(VersionOp::Greater, PackageVersion::new(v))
        }
        (Some("LE"), Some(v)) => {
            VersionConstraint::Relational(VersionOp::LessEqual, PackageVersion::new(v))
        }
        (Some("LT"), Some(v)) => {
            VersionConstraint::Relational(VersionOp::Less, PackageVersion::new(v))
        }
        _ => VersionConstraint::Any,
    };
    let original = name.clone();
    if name.contains(".so") {
        constraints.push(Constraint::Capability(CapabilityConstraint {
            identifier: format!("lib:{name}"),
            version: version_constraint,
            original_expression: original,
        }));
    } else if name.starts_with('/') {
        let binary = name.rsplit('/').next().unwrap_or(&name);
        constraints.push(Constraint::Capability(CapabilityConstraint {
            identifier: format!("bin:{binary}"),
            version: version_constraint,
            original_expression: name,
        }));
    } else if let Ok(package) = PackageName::new(&name) {
        constraints.push(Constraint::Package {
            name: package,
            version: version_constraint,
            ecosystem: "rpm".into(),
            original_expression: original,
        });
    } else {
        constraints.push(Constraint::Capability(CapabilityConstraint {
            identifier: format!("feature:{name}"),
            version: version_constraint,
            original_expression: original,
        }));
    }
}

/// Fetches and verifies an RPM-MD repository, producing normalized RemotePackages.
pub async fn update_rpm_repository(
    url: &str,
    _distribution: &str,
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

    let base_url = url.trim_end_matches('/');

    // 1. Fetch repodata/repomd.xml
    let repomd_url = format!("{base_url}/repodata/repomd.xml");
    let response = client
        .get(&repomd_url)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    // If redirected by a mirror manager, derive effective_base_url from the final location of repomd.xml
    let effective_base_url =
        if let Some(stripped) = response.url().as_str().strip_suffix("/repodata/repomd.xml") {
            stripped.to_string()
        } else {
            base_url.to_string()
        };

    let repomd_bytes = bounded_response(response, MAX_REPOMD_BYTES).await?;

    // 2. Cryptographic signature verification if public key configured
    if let Some(key_path) = public_key_path {
        let asc_url = format!("{effective_base_url}/repodata/repomd.xml.asc");
        let sig_response = client
            .get(&asc_url)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        let sig_bytes = bounded_response(sig_response, 1024 * 1024).await?;
        verify_detached_signature(&repomd_bytes, &sig_bytes, key_path)?;
        println!(
            "  ✓ Verified OpenPGP signature for RPM repository {effective_base_url} using {}",
            key_path.display()
        );
    }

    // 3. Parse repomd.xml to locate primary.xml
    let repomd_text = String::from_utf8(repomd_bytes)
        .map_err(|e| Error::Parse(format!("Invalid repomd.xml UTF-8: {e}")))?;
    let primary_meta = parse_repomd_xml(&repomd_text)?;

    // 4. Download primary.xml archive
    let primary_url = format!("{effective_base_url}/{}", primary_meta.location_href);
    let primary_resp = client
        .get(&primary_url)
        .send()
        .await
        .map_err(|e| Error::Network(e.to_string()))?;

    let primary_raw = bounded_response(primary_resp, MAX_PRIMARY_BYTES).await?;

    // Verify SHA-256 of primary archive
    let actual_digest = format!("{:x}", Sha256::digest(&primary_raw));
    if !actual_digest.eq_ignore_ascii_case(&primary_meta.checksum) {
        return Err(Error::SecurityViolation(format!(
            "Primary metadata digest mismatch: expected {}, got {}",
            primary_meta.checksum, actual_digest
        )));
    }

    // 5. Decompress primary stream
    let host_arch = std::env::consts::ARCH;
    let decoder: Box<dyn BufRead> = if primary_meta.location_href.ends_with(".gz") {
        Box::new(BufReader::new(GzDecoder::new(Cursor::new(primary_raw))))
    } else if primary_meta.location_href.ends_with(".zst") {
        Box::new(BufReader::new(
            zstd::stream::Decoder::new(Cursor::new(primary_raw))
                .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?,
        ))
    } else {
        Box::new(BufReader::new(Cursor::new(primary_raw)))
    };

    // 6. Parse packages
    parse_primary_xml(decoder, host_arch, &effective_base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repomd_xml_valid() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<repomd xmlns="http://linux.duke.edu/metadata/repo">
  <revision>1726000000</revision>
  <data type="other">
    <location href="repodata/other.xml.gz"/>
  </data>
  <data type="primary">
    <checksum type="sha256">abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890</checksum>
    <location href="repodata/test-primary.xml.gz"/>
    <size>54321</size>
  </data>
</repomd>"#;

        let res = parse_repomd_xml(xml).expect("Should parse valid repomd.xml");
        assert_eq!(res.location_href, "repodata/test-primary.xml.gz");
        assert_eq!(
            res.checksum,
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        );
        assert_eq!(res.size_bytes, 54321);
    }

    #[test]
    fn test_parse_primary_xml_packages() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata xmlns="http://linux.duke.edu/metadata/common" packages="2">
  <package type="rpm">
    <name>curl</name>
    <arch>x86_64</arch>
    <version epoch="0" ver="8.9.1" rel="1.fc41"/>
    <checksum type="sha256" pkgid="YES">d1g3st123</checksum>
    <size package="350000"/>
    <location href="Packages/c/curl-8.9.1-1.fc41.x86_64.rpm"/>
  </package>
  <package type="rpm">
    <name>libcurl</name>
    <arch>noarch</arch>
    <version epoch="1" ver="8.9.1" rel="1.fc41"/>
    <checksum type="sha256" pkgid="YES">d1g3st456</checksum>
    <size package="250000"/>
    <location href="Packages/l/libcurl-8.9.1-1.fc41.noarch.rpm"/>
  </package>
  <package type="rpm">
    <name>unrelated-arm</name>
    <arch>aarch64</arch>
    <version epoch="0" ver="1.0" rel="1"/>
    <checksum type="sha256" pkgid="YES">armdigest</checksum>
    <location href="Packages/u/unrelated.rpm"/>
  </package>
</metadata>"#;

        let pkgs = parse_primary_xml(
            Cursor::new(xml.as_bytes()),
            "x86_64",
            "https://mirror.example.com",
        )
        .expect("Should parse primary XML");

        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "curl");
        assert_eq!(pkgs[0].version, "8.9.1-1.fc41");
        assert_eq!(pkgs[0].architecture, "x86_64");
        assert_eq!(pkgs[0].format, "rpm");
        assert_eq!(pkgs[0].digest, "d1g3st123");
        assert_eq!(pkgs[0].size_bytes, 350000);
        assert_eq!(
            pkgs[0].url,
            "https://mirror.example.com/Packages/c/curl-8.9.1-1.fc41.x86_64.rpm"
        );

        assert_eq!(pkgs[1].name, "libcurl");
        assert_eq!(pkgs[1].version, "1:8.9.1-1.fc41");
        assert_eq!(pkgs[1].architecture, "noarch");
    }

    #[test]
    fn primary_xml_preserves_versioned_provides() {
        let xml = r#"<metadata>
  <package type="rpm">
    <name>provider</name>
    <arch>x86_64</arch>
    <version epoch="0" ver="1.0" rel="1"/>
    <checksum type="sha256" pkgid="YES">aaaaaaaa</checksum>
    <size package="10"/>
    <location href="Packages/p/provider.rpm"/>
    <format><provides><entry name="virtual-api" flags="EQ" ver="2.4"/><entry name="unversioned-api"/></provides></format>
  </package>
</metadata>"#;
        let packages = parse_primary_xml(
            Cursor::new(xml.as_bytes()),
            "x86_64",
            "https://example.invalid",
        )
        .unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].provides.len(), 1);
        assert_eq!(packages[0].versioned_provides.len(), 1);
        assert_eq!(packages[0].versioned_provides[0].version.as_str(), "2.4");
    }
}
