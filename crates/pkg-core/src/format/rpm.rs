//! RPM artifact adapter implementation (ADR-005, ADR-017).
//!
//! Parses RPM header metadata and extracts payload (CPIO stream) under safe limits (INV-004).
//! Inventories `%pre`, `%post`, `%preun`, `%postun` scriptlets without executing (INV-003).

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use xz2::read::XzDecoder;

use crate::domain::capability::{Capability, Dependency};
use crate::domain::constraint::{CapabilityConstraint, Constraint, VersionConstraint};
use crate::domain::package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion,
};
use crate::error::{Error, Result};
use crate::format::{ArtifactAdapter, ExtractionLimits, ExtractionReport};

/// Format adapter for Red Hat Package Manager (.rpm) artifacts.
#[derive(Debug, Default, Clone)]
pub struct RpmAdapter;

impl RpmAdapter {
    /// Creates a new RPM format adapter.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Computes the SHA-256 digest of the artifact file.
    fn compute_digest(path: &Path) -> Result<ArtifactDigest> {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        let hash_str = format!("{:x}", hasher.finalize());
        Ok(ArtifactDigest::sha256(hash_str))
    }

    /// Decompresses raw RPM payload bytes based on magic detection or compression headers.
    fn decompress_payload(payload: &[u8]) -> Result<Vec<u8>> {
        if payload.len() < 4 {
            return Err(Error::MalformedArchive("RPM payload too short".into()));
        }

        // Gzip magic: 0x1F 0x8B
        if payload[0] == 0x1f && payload[1] == 0x8b {
            let mut decoder = GzDecoder::new(payload);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            return Ok(decompressed);
        }

        // XZ magic: 0xFD '7' 'z' 'X' 'Z' 0x00
        if payload.len() >= 6 && &payload[..6] == b"\xfd7zXZ\x00" {
            let mut decoder = XzDecoder::new(payload);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            return Ok(decompressed);
        }

        // Zstandard magic: 0x28 0xB5 0x2F 0xFD
        if payload.len() >= 4 && payload[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
            let mut decoder = zstd::stream::read::Decoder::new(payload)?;
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed)?;
            return Ok(decompressed);
        }

        // Assume uncompressed CPIO if magic matches 070701 / 070702
        if payload.len() >= 6 && (&payload[..6] == b"070701" || &payload[..6] == b"070702") {
            return Ok(payload.to_vec());
        }

        Err(Error::MalformedArchive(
            "Unsupported or unrecognized RPM payload compression".into(),
        ))
    }

    /// Sanitizes an entry path from CPIO archive, rejecting absolute paths and traversal (INV-004).
    fn sanitize_relative_path(raw: &str) -> Result<PathBuf> {
        let trimmed = raw.trim_start_matches('.').trim_start_matches('/');
        let path = Path::new(trimmed);
        for comp in path.components() {
            match comp {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute path rejected in RPM payload: {raw}"
                    )));
                }
                Component::ParentDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Path traversal rejected in RPM payload: {raw}"
                    )));
                }
                Component::Normal(_) | Component::CurDir => {}
            }
        }
        Ok(path.to_path_buf())
    }

    /// Extracts CPIO SVR4 archive entries safely under configured limits.
    fn extract_cpio_stream(
        cpio_bytes: &[u8],
        destination: &Path,
        limits: &ExtractionLimits,
    ) -> Result<ExtractionReport> {
        let mut cursor = Cursor::new(cpio_bytes);
        let mut extracted_files = Vec::new();
        let mut total_bytes = 0u64;
        let mut entries_count = 0usize;
        let mut symlinks_count = 0usize;
        let mut seen_paths = HashSet::new();

        loop {
            // Read 110-byte CPIO header
            let mut hdr_buf = [0u8; 110];
            if let Err(e) = cursor.read_exact(&mut hdr_buf) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(Error::MalformedArchive(format!(
                    "Failed to read CPIO header: {e}"
                )));
            }

            let magic = &hdr_buf[0..6];
            if magic != b"070701" && magic != b"070702" {
                return Err(Error::MalformedArchive(format!(
                    "Invalid CPIO magic: {:?}",
                    std::str::from_utf8(magic).unwrap_or("")
                )));
            }

            let mode_hex = std::str::from_utf8(&hdr_buf[14..22]).unwrap_or("0");
            let filesize_hex = std::str::from_utf8(&hdr_buf[54..62]).unwrap_or("0");
            let namesize_hex = std::str::from_utf8(&hdr_buf[94..102]).unwrap_or("0");

            let mode = u32::from_str_radix(mode_hex, 16).unwrap_or(0);
            let filesize = u64::from_str_radix(filesize_hex, 16).unwrap_or(0);
            let namesize = usize::from_str_radix(namesize_hex, 16).unwrap_or(0);

            if namesize == 0 {
                break;
            }

            // Read filename
            let mut name_buf = vec![0u8; namesize];
            cursor.read_exact(&mut name_buf)?;
            let raw_name = std::str::from_utf8(&name_buf)
                .unwrap_or("")
                .trim_end_matches('\0');

            // Align cursor to 4-byte boundary after header + name
            let name_pad = (4 - ((110 + namesize) % 4)) % 4;
            if name_pad > 0 {
                let mut pad = vec![0u8; name_pad];
                cursor.read_exact(&mut pad)?;
            }

            if raw_name == "TRAILER!!!" {
                break;
            }

            entries_count += 1;
            if entries_count > limits.max_entries {
                return Err(Error::LimitsExceeded(format!(
                    "CPIO entry count exceeded limit of {}",
                    limits.max_entries
                )));
            }

            if filesize > limits.max_single_file_bytes {
                return Err(Error::LimitsExceeded(format!(
                    "File size {filesize} exceeds limit {}",
                    limits.max_single_file_bytes
                )));
            }

            total_bytes += filesize;
            if total_bytes > limits.max_total_bytes {
                return Err(Error::LimitsExceeded(format!(
                    "Total extracted bytes exceeded limit of {}",
                    limits.max_total_bytes
                )));
            }

            let relative_path = Self::sanitize_relative_path(raw_name)?;
            if relative_path.as_os_str().is_empty() {
                // Skip root or empty paths
                let file_pad = (4 - (filesize % 4)) % 4;
                let skip = filesize + file_pad;
                let cur = cursor.position();
                cursor.set_position(cur + skip);
                continue;
            }

            if seen_paths.contains(&relative_path) {
                return Err(Error::MalformedArchive(format!(
                    "Duplicate entry in RPM payload: {}",
                    relative_path.display()
                )));
            }
            seen_paths.insert(relative_path.clone());

            let target_path = destination.join(&relative_path);
            let is_dir = (mode & 0o170000) == 0o040000;
            let is_symlink = (mode & 0o170000) == 0o120000;

            if is_dir {
                fs::create_dir_all(&target_path)?;
            } else if is_symlink {
                let mut link_buf = vec![0u8; filesize as usize];
                cursor.read_exact(&mut link_buf)?;
                let link_target_str = std::str::from_utf8(&link_buf).unwrap_or("");
                let link_target = Path::new(link_target_str);

                // Reject escaping symlinks
                if link_target.is_absolute() || link_target_str.contains("..") {
                    return Err(Error::SecurityViolation(format!(
                        "Unsafe symlink in RPM: {} -> {link_target_str}",
                        relative_path.display()
                    )));
                }

                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(link_target, &target_path)?;
                symlinks_count += 1;
            } else {
                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut file_buf = vec![0u8; filesize as usize];
                cursor.read_exact(&mut file_buf)?;
                fs::write(&target_path, &file_buf)?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let file_mode = mode & 0o7777;
                    let _ =
                        fs::set_permissions(&target_path, fs::Permissions::from_mode(file_mode));
                }
            }

            // Align cursor to 4-byte boundary after file data
            let file_pad = (4 - (filesize % 4)) % 4;
            if file_pad > 0 && !is_symlink {
                let mut pad = vec![0u8; file_pad as usize];
                cursor.read_exact(&mut pad)?;
            }

            extracted_files.push(relative_path);
        }

        Ok(ExtractionReport {
            extracted_files,
            total_bytes,
            entries_count,
            symlinks_count,
        })
    }
}

impl ArtifactAdapter for RpmAdapter {
    fn parse_metadata(&self, path: &Path) -> Result<NormalizedPackage> {
        let file = File::open(path)?;
        let size_bytes = file.metadata()?.len();

        let pkg = rpm::Package::open(path).map_err(|e| {
            Error::MalformedArchive(format!(
                "Failed to parse RPM archive {}: {e}",
                path.display()
            ))
        })?;

        let digest = Self::compute_digest(path)?;

        let raw_name = pkg.metadata.get_name().unwrap_or("").to_lowercase();
        let name = PackageName::new(raw_name)?;

        let raw_version = pkg.metadata.get_version().unwrap_or("0.0.0");
        let raw_release = pkg.metadata.get_release().unwrap_or("1");
        let version = PackageVersion::new(format!("{raw_version}-{raw_release}"));

        let raw_arch = pkg.metadata.get_arch().unwrap_or("x86_64");
        let architecture = Architecture::parse(raw_arch);

        let description = pkg
            .metadata
            .get_description()
            .ok()
            .map(|s| s.to_string())
            .or_else(|| pkg.metadata.get_summary().ok().map(|s| s.to_string()));

        let installed_size = pkg.metadata.get_installed_size().ok();

        // Parse scriptlets under default-deny (INV-003)
        let mut scripts = Vec::new();
        if let Ok(pre) = pkg.metadata.get_pre_install_script() {
            if !pre.script.trim().is_empty() {
                scripts.push(LifecycleScript {
                    name: "prein".to_string(),
                    content: pre.script.clone(),
                });
            }
        }
        if let Ok(post) = pkg.metadata.get_post_install_script() {
            if !post.script.trim().is_empty() {
                scripts.push(LifecycleScript {
                    name: "postin".to_string(),
                    content: post.script.clone(),
                });
            }
        }
        if let Ok(preun) = pkg.metadata.get_pre_uninstall_script() {
            if !preun.script.trim().is_empty() {
                scripts.push(LifecycleScript {
                    name: "preun".to_string(),
                    content: preun.script.clone(),
                });
            }
        }
        if let Ok(postun) = pkg.metadata.get_post_uninstall_script() {
            if !postun.script.trim().is_empty() {
                scripts.push(LifecycleScript {
                    name: "postun".to_string(),
                    content: postun.script.clone(),
                });
            }
        }

        // Collect provides as capabilities (ADR-016)
        let mut provides = Vec::new();
        if let Ok(prov_entries) = pkg.metadata.get_provides() {
            for prov in prov_entries {
                let p_name = prov.name.as_str();
                if p_name.ends_with(".so") || p_name.contains(".so.") {
                    provides.push(Capability::SharedLibrary(p_name.to_string()));
                } else if !p_name.starts_with('/') {
                    provides.push(Capability::Feature(p_name.to_string()));
                }
            }
        }

        // Collect dependencies and normalized constraints (ADR-009)
        let mut dependencies = Vec::new();
        let mut constraints = Vec::new();

        if let Ok(req_entries) = pkg.metadata.get_requires() {
            for req in req_entries {
                let req_name = req.name.as_str();
                // Filter out internal rpmlib requirements
                if req_name.starts_with("rpmlib(") {
                    continue;
                }

                let raw = req.name.clone();
                dependencies.push(Dependency {
                    raw: raw.clone(),
                    name: req_name.to_string(),
                    version_constraint: None,
                    ecosystem: "rpm".to_string(),
                });

                if req_name.ends_with(".so") || req_name.contains(".so.") {
                    constraints.push(Constraint::Capability(CapabilityConstraint {
                        identifier: format!("lib:{req_name}"),
                        version: VersionConstraint::Any,
                        original_expression: raw,
                    }));
                } else if req_name.starts_with('/') {
                    // Path requirement (e.g. /bin/sh) -> binary capability
                    let bin_name = req_name.rsplit('/').next().unwrap_or(req_name);
                    constraints.push(Constraint::Capability(CapabilityConstraint {
                        identifier: format!("bin:{bin_name}"),
                        version: VersionConstraint::Any,
                        original_expression: raw,
                    }));
                } else if let Ok(pkg_name) = PackageName::new(req_name) {
                    constraints.push(Constraint::Package {
                        name: pkg_name,
                        version: VersionConstraint::Any,
                        ecosystem: "rpm".to_string(),
                        original_expression: raw,
                    });
                } else {
                    constraints.push(Constraint::Capability(CapabilityConstraint {
                        identifier: format!("feature:{req_name}"),
                        version: VersionConstraint::Any,
                        original_expression: raw,
                    }));
                }
            }
        }

        if let Ok(conflicts) = pkg.metadata.get_conflicts() {
            for c in conflicts {
                constraints.push(Constraint::Conflict {
                    target: c.name.clone(),
                    original_expression: c.name,
                });
            }
        }

        // Build file entries list
        let mut entries = Vec::new();
        if let Ok(file_entries) = pkg.metadata.get_file_entries() {
            for f in file_entries {
                let p = f.path();
                let clean_path = p.strip_prefix("/").unwrap_or(&p);
                let is_dir = f.mode().file_type() == rpm::FileType::Dir;
                let is_symlink = f.mode().file_type() == rpm::FileType::SymbolicLink;

                // If executable in bin dir, add executable capability
                if !is_dir && !is_symlink {
                    if let Some(parent) = clean_path.parent() {
                        if parent == Path::new("usr/bin") || parent == Path::new("bin") {
                            if let Some(cmd) = clean_path.file_name().and_then(|n| n.to_str()) {
                                provides.push(Capability::Executable(cmd.to_string()));
                            }
                        }
                    }
                }

                entries.push(PackageEntry {
                    relative_path: clean_path.to_path_buf(),
                    is_dir,
                    is_symlink,
                    symlink_target: None,
                    mode: f.mode().raw_mode() as u32,
                    size: f.size() as u64,
                });
            }
        }

        Ok(NormalizedPackage {
            name,
            version,
            architecture,
            format: PackageFormat::Rpm,
            digest,
            size_bytes,
            description,
            dependencies,
            constraints,
            provides,
            scripts,
            entries,
            installed_size,
        })
    }

    fn extract_payload(
        &self,
        path: &Path,
        destination: &Path,
        limits: &ExtractionLimits,
    ) -> Result<ExtractionReport> {
        let pkg = rpm::Package::open(path).map_err(|e| {
            Error::MalformedArchive(format!("Failed to open RPM for extraction: {e}"))
        })?;

        let uncompressed_cpio = Self::decompress_payload(&pkg.payload)?;
        Self::extract_cpio_stream(&uncompressed_cpio, destination, limits)
    }
}
