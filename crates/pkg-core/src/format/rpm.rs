//! RPM artifact adapter implementation (ADR-005, ADR-017).
//!
//! Parses RPM header metadata and extracts payload (CPIO stream) under safe limits (INV-004).
//! Inventories `%pre`, `%post`, `%preun`, `%postun` scriptlets without executing (INV-003).

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use xz2::read::XzDecoder;

use crate::domain::capability::{Capability, Dependency};
use crate::domain::constraint::{CapabilityConstraint, Constraint, VersionConstraint, VersionOp};
use crate::domain::package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion, VersionedCapability,
};
use crate::error::{Error, Result};
use crate::format::{ArtifactAdapter, ExtractionLimits, ExtractionReport};

/// Format adapter for Red Hat Package Manager (.rpm) artifacts.
#[derive(Debug, Default, Clone)]
pub struct RpmAdapter;

impl RpmAdapter {
    fn create_payload_dirs(root: &Path, relative: &Path) -> Result<()> {
        let mut current = root.to_path_buf();
        for component in relative.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(Error::SecurityViolation(format!(
                        "Non-directory RPM payload parent: {}",
                        current.display()
                    )));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn check_link_does_not_escape(root: &Path, relative: &Path, target: &Path) -> Result<()> {
        let link_path = root.join(relative);
        let parent = link_path.parent().unwrap_or(root);
        let mut current = if parent.exists() {
            fs::canonicalize(parent)?
        } else {
            root.to_path_buf()
        };
        if !current.starts_with(root) {
            return Err(Error::SecurityViolation(format!(
                "RPM symlink parent escapes staging: {}",
                relative.display()
            )));
        }
        for component in target.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute RPM symlink target rejected: {} -> {}",
                        relative.display(),
                        target.display()
                    )));
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if current == root || !current.starts_with(root) {
                        return Err(Error::SecurityViolation(format!(
                            "Escaping RPM payload link: {}",
                            relative.display()
                        )));
                    }
                    current.pop();
                }
                Component::Normal(component) => {
                    let next = current.join(component);
                    if next.is_symlink() {
                        if let Ok(canonical) = fs::canonicalize(&next) {
                            if !canonical.starts_with(root) {
                                return Err(Error::SecurityViolation(format!(
                                    "Escaping RPM payload link: {}",
                                    relative.display()
                                )));
                            }
                            current = canonical;
                            continue;
                        }
                    }
                    current = next;
                }
            }
        }
        if !current.starts_with(root) {
            return Err(Error::SecurityViolation(format!(
                "Escaping RPM payload link: {}",
                relative.display()
            )));
        }
        Ok(())
    }

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
        // RPM CPIO payloads conventionally prefix names with `./`.  Remove
        // that exact prefix while preserving meaningful dot components such
        // as `.config`; stripping every leading dot or slash would turn
        // `../escape` and `/etc/passwd` into apparently safe paths.
        let mut trimmed = raw;
        while let Some(rest) = trimmed.strip_prefix("./") {
            trimmed = rest;
        }
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
        let mut pending_links = Vec::new();

        fs::create_dir_all(destination)?;

        let skip_payload = |cursor: &mut Cursor<&[u8]>, size: u64| -> Result<()> {
            let padding = (4 - (size % 4)) % 4;
            let skip = size.checked_add(padding).ok_or_else(|| {
                Error::LimitsExceeded("CPIO payload offset overflowed u64".into())
            })?;
            let end = cursor.position().checked_add(skip).ok_or_else(|| {
                Error::LimitsExceeded("CPIO payload offset overflowed u64".into())
            })?;
            if end > cpio_bytes.len() as u64 {
                return Err(Error::MalformedArchive("CPIO payload is truncated".into()));
            }
            cursor.set_position(end);
            Ok(())
        };

        loop {
            // Read 110-byte CPIO header
            let mut hdr_buf = [0u8; 110];
            if let Err(e) = cursor.read_exact(&mut hdr_buf) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Err(Error::MalformedArchive(
                        "CPIO archive ended before TRAILER!!!".into(),
                    ));
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

            let parse_hex = |field: &[u8], label: &str| -> Result<u64> {
                let value = std::str::from_utf8(field)
                    .map_err(|_| Error::MalformedArchive(format!("Invalid CPIO {label} field")))?;
                u64::from_str_radix(value, 16).map_err(|_| {
                    Error::MalformedArchive(format!("Invalid CPIO {label} field: {value:?}"))
                })
            };
            let mode = u32::try_from(parse_hex(&hdr_buf[14..22], "mode")?)
                .map_err(|_| Error::MalformedArchive("CPIO mode overflows u32".into()))?;
            let filesize = parse_hex(&hdr_buf[54..62], "file size")?;
            let namesize = usize::try_from(parse_hex(&hdr_buf[94..102], "name size")?)
                .map_err(|_| Error::LimitsExceeded("CPIO name size overflows usize".into()))?;

            if namesize == 0 {
                return Err(Error::MalformedArchive(
                    "CPIO entry has an empty name".into(),
                ));
            }
            if namesize > 1024 * 1024 {
                return Err(Error::LimitsExceeded(
                    "CPIO filename exceeds 1 MiB limit".into(),
                ));
            }

            // Read filename
            let mut name_buf = vec![0u8; namesize];
            cursor.read_exact(&mut name_buf)?;
            let raw_name = std::str::from_utf8(&name_buf)
                .map_err(|_| Error::MalformedArchive("CPIO filename is not UTF-8".into()))?;
            let raw_name = raw_name.strip_suffix('\0').unwrap_or(raw_name);

            // Align cursor to 4-byte boundary after header + name
            let name_pad = (4 - ((110 + namesize) % 4)) % 4;
            if name_pad > 0 {
                let mut pad = vec![0u8; name_pad];
                cursor.read_exact(&mut pad)?;
            }

            if raw_name == "TRAILER!!!" {
                if filesize != 0 {
                    return Err(Error::MalformedArchive(
                        "CPIO TRAILER!!! has non-zero payload".into(),
                    ));
                }
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

            total_bytes = total_bytes
                .checked_add(filesize)
                .ok_or_else(|| Error::LimitsExceeded("CPIO total size overflowed u64".into()))?;
            if total_bytes > limits.max_total_bytes {
                return Err(Error::LimitsExceeded(format!(
                    "Total extracted bytes exceeded limit of {}",
                    limits.max_total_bytes
                )));
            }

            let relative_path = Self::sanitize_relative_path(raw_name)?;
            if relative_path.as_os_str().is_empty() {
                // Skip root or empty paths
                skip_payload(&mut cursor, filesize)?;
                continue;
            }

            if seen_paths.contains(&relative_path) {
                return Err(Error::MalformedArchive(format!(
                    "Duplicate entry in RPM payload: {}",
                    relative_path.display()
                )));
            }
            seen_paths.insert(relative_path.clone());

            let file_type = mode & 0o170000;
            let is_dir = file_type == 0o040000;
            let is_symlink = file_type == 0o120000;
            let is_file = file_type == 0o100000;
            if !is_dir && !is_symlink && !is_file {
                return Err(Error::MalformedArchive(format!(
                    "Unsupported CPIO payload entry type: {}",
                    relative_path.display()
                )));
            }

            if is_dir {
                if filesize != 0 {
                    return Err(Error::MalformedArchive(format!(
                        "Directory has non-zero payload: {}",
                        relative_path.display()
                    )));
                }
                Self::create_payload_dirs(destination, &relative_path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        destination.join(&relative_path),
                        fs::Permissions::from_mode(mode & 0o7777),
                    )?;
                }
            } else if is_symlink {
                let link_size = usize::try_from(filesize).map_err(|_| {
                    Error::LimitsExceeded("CPIO symlink size overflows usize".into())
                })?;
                let mut link_buf = vec![0u8; link_size];
                cursor.read_exact(&mut link_buf)?;
                let link_target_str = std::str::from_utf8(&link_buf).map_err(|_| {
                    Error::MalformedArchive("CPIO symlink target is not UTF-8".into())
                })?;
                let link_target_str = link_target_str
                    .strip_suffix('\0')
                    .unwrap_or(link_target_str);
                if link_target_str.is_empty() {
                    return Err(Error::MalformedArchive(format!(
                        "Empty CPIO symlink target: {}",
                        relative_path.display()
                    )));
                }
                let link_target = Path::new(link_target_str);

                let sanitized_target =
                    super::sanitize_symlink_target(&relative_path, link_target, destination)?;
                pending_links.push((relative_path.clone(), sanitized_target));
                symlinks_count += 1;
            } else if is_file {
                if let Some(parent) = relative_path.parent() {
                    Self::create_payload_dirs(destination, parent)?;
                }
                let file_size = usize::try_from(filesize)
                    .map_err(|_| Error::LimitsExceeded("CPIO file size overflows usize".into()))?;
                let mut file_buf = vec![0u8; file_size];
                cursor.read_exact(&mut file_buf)?;
                let target_path = destination.join(&relative_path);
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target_path)?
                    .write_all(&file_buf)?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let file_mode = mode & 0o7777;
                    fs::set_permissions(&target_path, fs::Permissions::from_mode(file_mode))?;
                }
            }

            // Align cursor to 4-byte boundary after file data
            let file_pad = (4 - (filesize % 4)) % 4;
            if file_pad > 0 {
                let mut pad = vec![
                    0u8;
                    usize::try_from(file_pad).map_err(|_| {
                        Error::LimitsExceeded("CPIO padding overflows usize".into())
                    })?
                ];
                cursor.read_exact(&mut pad)?;
            }

            extracted_files.push(relative_path);
        }

        for (relative, target) in &pending_links {
            if let Some(parent) = relative.parent() {
                Self::create_payload_dirs(destination, parent)?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, destination.join(relative))?;
        }
        let root = fs::canonicalize(destination)?;
        for (relative, target) in &pending_links {
            Self::check_link_does_not_escape(&root, relative, target)?;
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

        // Preserve the source spelling. RPM repositories legitimately use
        // uppercase and underscore-containing names, and package identity is
        // case-sensitive at the RPM layer.
        let raw_name = pkg.metadata.get_name().unwrap_or("");
        let name = PackageName::new(raw_name)?;

        let raw_version = pkg.metadata.get_version().unwrap_or("0.0.0");
        let raw_release = pkg.metadata.get_release().unwrap_or("1");
        let version = PackageVersion::new(format!("{raw_version}-{raw_release}"));
        version.validate()?;

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
        let mut versioned_provides = Vec::new();
        if let Ok(prov_entries) = pkg.metadata.get_provides() {
            for prov in prov_entries {
                let p_name = prov.name.as_str();
                let capability = if p_name.ends_with(".so") || p_name.contains(".so.") {
                    Capability::SharedLibrary(p_name.to_string())
                } else if !p_name.starts_with('/') {
                    Capability::Feature(p_name.to_string())
                } else {
                    continue;
                };
                if !prov.version.is_empty() {
                    let version = PackageVersion::new(prov.version.as_str());
                    if version.validate().is_ok() {
                        versioned_provides.push(VersionedCapability {
                            capability,
                            version,
                        });
                        continue;
                    }
                }
                provides.push(capability);
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

                let comparator = req.flags.comparator_str();
                let raw = if comparator.is_empty() || req.version.is_empty() {
                    req.name.clone()
                } else {
                    format!("{} {} {}", req.name, comparator, req.version)
                };
                let version_constraint = match (comparator, req.version.is_empty()) {
                    ("=", false) => VersionConstraint::Relational(
                        VersionOp::Exact,
                        PackageVersion::new(req.version.as_str()),
                    ),
                    (">", false) => VersionConstraint::Relational(
                        VersionOp::Greater,
                        PackageVersion::new(req.version.as_str()),
                    ),
                    (">=", false) => VersionConstraint::Relational(
                        VersionOp::GreaterEqual,
                        PackageVersion::new(req.version.as_str()),
                    ),
                    ("<", false) => VersionConstraint::Relational(
                        VersionOp::Less,
                        PackageVersion::new(req.version.as_str()),
                    ),
                    ("<=", false) => VersionConstraint::Relational(
                        VersionOp::LessEqual,
                        PackageVersion::new(req.version.as_str()),
                    ),
                    _ => VersionConstraint::Any,
                };
                dependencies.push(Dependency {
                    raw: raw.clone(),
                    name: req_name.to_string(),
                    version_constraint: (!req.version.is_empty()).then(|| req.version.clone()),
                    ecosystem: "rpm".to_string(),
                });

                if req_name.ends_with(".so") || req_name.contains(".so.") {
                    constraints.push(Constraint::Capability(CapabilityConstraint {
                        identifier: format!("lib:{req_name}"),
                        version: version_constraint.clone(),
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
                        version: version_constraint.clone(),
                        ecosystem: "rpm".to_string(),
                        original_expression: raw,
                    });
                } else {
                    constraints.push(Constraint::Capability(CapabilityConstraint {
                        identifier: format!("feature:{req_name}"),
                        version: version_constraint,
                        original_expression: raw,
                    }));
                }
            }
        }

        if let Ok(conflicts) = pkg.metadata.get_conflicts() {
            for c in conflicts {
                let comparator = c.flags.comparator_str();
                let version = match (comparator, c.version.is_empty()) {
                    ("=", false) => VersionConstraint::Relational(
                        VersionOp::Exact,
                        PackageVersion::new(c.version.as_str()),
                    ),
                    ("<", false) => VersionConstraint::Relational(
                        VersionOp::Less,
                        PackageVersion::new(c.version.as_str()),
                    ),
                    ("<=", false) => VersionConstraint::Relational(
                        VersionOp::LessEqual,
                        PackageVersion::new(c.version.as_str()),
                    ),
                    (">", false) => VersionConstraint::Relational(
                        VersionOp::Greater,
                        PackageVersion::new(c.version.as_str()),
                    ),
                    (">=", false) => VersionConstraint::Relational(
                        VersionOp::GreaterEqual,
                        PackageVersion::new(c.version.as_str()),
                    ),
                    _ => VersionConstraint::Any,
                };
                let original_expression = if comparator.is_empty() || c.version.is_empty() {
                    c.name.clone()
                } else {
                    format!("{} {} {}", c.name, comparator, c.version)
                };
                constraints.push(Constraint::Conflict {
                    target: c.name.clone(),
                    version,
                    original_expression,
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
                let symlink_target = if is_symlink {
                    f.linkto().map(PathBuf::from)
                } else {
                    None
                };

                // If executable in bin dir, add executable capability
                if !is_dir && !is_symlink {
                    if let Some(parent) = clean_path.parent() {
                        if parent == Path::new("usr/bin") || parent == Path::new("bin") {
                            if let Some(cmd) = clean_path.file_name().and_then(|n| n.to_str()) {
                                provides.push(Capability::Executable(cmd.to_string()));
                            }
                        }
                    }
                    if let Some(filename) = clean_path.file_name().and_then(|n| n.to_str())
                        && filename.contains(".so")
                    {
                        let capability = Capability::SharedLibrary(filename.to_string());
                        if !provides.contains(&capability)
                            && !versioned_provides
                                .iter()
                                .any(|entry| entry.capability == capability)
                        {
                            provides.push(capability);
                        }
                    }
                }

                entries.push(PackageEntry {
                    relative_path: clean_path.to_path_buf(),
                    is_dir,
                    is_symlink,
                    mode: f.mode().raw_mode() as u32,
                    size: f.size() as u64,
                    symlink_target,
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
            versioned_provides,
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

#[cfg(test)]
mod tests {
    use super::RpmAdapter;
    use crate::error::Error;
    use std::path::PathBuf;

    #[test]
    fn sanitize_relative_path_rejects_escape_and_preserves_dotfiles() {
        assert!(matches!(
            RpmAdapter::sanitize_relative_path("../escape"),
            Err(Error::SecurityViolation(_))
        ));
        assert!(matches!(
            RpmAdapter::sanitize_relative_path("/etc/passwd"),
            Err(Error::SecurityViolation(_))
        ));
        assert_eq!(
            RpmAdapter::sanitize_relative_path("./usr/bin/tool").unwrap(),
            PathBuf::from("usr/bin/tool")
        );
        assert_eq!(
            RpmAdapter::sanitize_relative_path(".config/app").unwrap(),
            PathBuf::from(".config/app")
        );
    }
}
