//! ALPM (Arch Linux Package Management) artifact adapter implementation (ADR-005, ADR-017).
//!
//! Parses Arch Linux package archives (`.pkg.tar.zst`, `.pkg.tar.xz`, `.pkg.tar.gz`, `.pkg.tar`),
//! normalizes `.PKGINFO` metadata, inventories `.INSTALL` scriptlets under default-deny (INV-003),
//! and safely extracts payload files under configured limits (INV-004).

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read};
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

/// Format adapter for Arch Linux (.pkg.tar.zst / .pkg.tar.xz / .pkg.tar.gz) artifacts.
#[derive(Debug, Default, Clone)]
pub struct AlpmAdapter;

impl AlpmAdapter {
    /// Creates payload directories without traversing a pre-existing symlink.
    ///
    /// A package archive may list entries in an arbitrary order.  Walking each
    /// parent with `symlink_metadata` keeps a malicious entry from redirecting
    /// a later file write outside the staging root.
    fn create_payload_dirs(root: &Path, relative: &Path) -> Result<()> {
        let mut current = root.to_path_buf();
        for component in relative.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(Error::SecurityViolation(format!(
                        "Non-directory ALPM payload parent: {}",
                        current.display()
                    )));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Verifies a materialized link and any link chain remain below `root`.
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
                "ALPM symlink parent escapes staging: {}",
                relative.display()
            )));
        }

        for component in target.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute ALPM symlink target rejected: {} -> {}",
                        relative.display(),
                        target.display()
                    )));
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if current == root || !current.starts_with(root) {
                        return Err(Error::SecurityViolation(format!(
                            "Escaping ALPM payload link: {}",
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
                                    "Escaping ALPM payload link: {}",
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
                "Escaping ALPM payload link: {}",
                relative.display()
            )));
        }
        Ok(())
    }

    /// Creates a new ALPM format adapter.
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

    /// Decompresses raw package stream into a tar reader based on magic bytes or file extension.
    fn make_tar_reader<'a>(
        reader: Box<dyn Read + 'a>,
        path: &Path,
    ) -> Result<tar::Archive<Box<dyn Read + 'a>>> {
        let mut buf_reader = BufReader::new(reader);
        let mut magic = [0u8; 6];
        let n = buf_reader.read(&mut magic).unwrap_or(0);
        let combined = Cursor::new(magic[..n].to_vec()).chain(buf_reader);

        // Zstandard magic: 0x28 0xB5 0x2F 0xFD
        if n >= 4 && magic[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
            let decoder = zstd::stream::read::Decoder::new(combined)?;
            return Ok(tar::Archive::new(Box::new(decoder)));
        }

        // XZ magic: 0xFD '7' 'z' 'X' 'Z' 0x00
        if n >= 6 && &magic[..6] == b"\xfd7zXZ\x00" {
            let decoder = XzDecoder::new(combined);
            return Ok(tar::Archive::new(Box::new(decoder)));
        }

        // Gzip magic: 0x1F 0x8B
        if n >= 2 && magic[..2] == [0x1f, 0x8b] {
            let decoder = GzDecoder::new(combined);
            return Ok(tar::Archive::new(Box::new(decoder)));
        }

        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".tar") || name.contains(".pkg.tar") {
            return Ok(tar::Archive::new(Box::new(combined)));
        }

        Err(Error::MalformedArchive(format!(
            "Unrecognized compression format for ALPM package: {}",
            path.display()
        )))
    }

    /// Parses key-value fields from an Arch Linux `.PKGINFO` file.
    fn parse_pkginfo(
        content: &str,
    ) -> (
        HashMap<String, String>,
        Vec<String>,
        Vec<String>,
        Vec<String>,
    ) {
        let mut fields = HashMap::new();
        let mut depends = Vec::new();
        let mut provides = Vec::new();
        let mut conflicts = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((key, val)) = trimmed.split_once('=') {
                let k = key.trim();
                let v = val.trim();
                match k {
                    "depend" => depends.push(v.to_string()),
                    "provides" => provides.push(v.to_string()),
                    "conflict" => conflicts.push(v.to_string()),
                    _ => {
                        fields.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }

        (fields, depends, provides, conflicts)
    }

    /// Sanitizes an entry path from the payload tar, rejecting absolute paths and traversal (INV-004).
    fn sanitize_relative_path(path: &Path) -> Result<PathBuf> {
        let mut clean = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute path rejected in ALPM payload: {}",
                        path.display()
                    )));
                }
                Component::ParentDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Path traversal rejected in ALPM payload: {}",
                        path.display()
                    )));
                }
                Component::Normal(c) => clean.push(c),
                Component::CurDir => {}
            }
        }
        Ok(clean)
    }

    /// Checks whether an entry is an internal ALPM metadata file that should not be extracted to store payload.
    fn is_alpm_metadata_file(path: &Path) -> bool {
        let s = path.to_string_lossy();
        s == ".PKGINFO"
            || s == ".INSTALL"
            || s == ".MTREE"
            || s == ".BUILDINFO"
            || s.starts_with(".PKGINFO")
    }
}

impl ArtifactAdapter for AlpmAdapter {
    fn parse_metadata(&self, path: &Path) -> Result<NormalizedPackage> {
        let file = File::open(path)?;
        let size_bytes = file.metadata()?.len();
        let digest = Self::compute_digest(path)?;

        let mut archive = Self::make_tar_reader(Box::new(file), path)?;
        let mut pkginfo_content: Option<String> = None;
        let mut install_content: Option<String> = None;
        let mut entries = Vec::new();
        let mut entry_provides = Vec::new();
        let mut seen_paths = HashSet::new();

        for entry_res in archive.entries()? {
            let mut entry = entry_res.map_err(|e| {
                Error::MalformedArchive(format!("Malformed tar entry in ALPM package: {e}"))
            })?;

            let raw_path = entry.path()?.to_path_buf();
            let name_str = raw_path.to_string_lossy().to_string();

            if name_str == ".PKGINFO" {
                let mut buf = String::new();
                entry.read_to_string(&mut buf)?;
                pkginfo_content = Some(buf);
                continue;
            } else if name_str == ".INSTALL" {
                let mut buf = String::new();
                entry.read_to_string(&mut buf)?;
                install_content = Some(buf);
                continue;
            } else if Self::is_alpm_metadata_file(&raw_path) {
                continue;
            }

            let relative_path = Self::sanitize_relative_path(&raw_path)?;
            if relative_path.as_os_str().is_empty() {
                continue;
            }

            if seen_paths.contains(&relative_path) {
                return Err(Error::MalformedArchive(format!(
                    "Duplicate entry in ALPM payload: {}",
                    relative_path.display()
                )));
            }
            seen_paths.insert(relative_path.clone());

            let is_dir = entry.header().entry_type().is_dir();
            let is_symlink = entry.header().entry_type().is_symlink();
            if !is_dir && !is_symlink && !entry.header().entry_type().is_file() {
                return Err(Error::MalformedArchive(format!(
                    "Unsupported ALPM payload entry type: {}",
                    relative_path.display()
                )));
            }
            let symlink_target = if is_symlink {
                entry.link_name()?.map(|p| p.to_path_buf())
            } else {
                None
            };
            let mode = entry.header().mode().unwrap_or(0o644);
            let size = entry.header().size().unwrap_or(0);

            // If binary in usr/bin or bin, export Executable capability
            if !is_dir && !is_symlink {
                if let Some(parent) = relative_path.parent() {
                    if parent == Path::new("usr/bin") || parent == Path::new("bin") {
                        if let Some(cmd) = relative_path.file_name().and_then(|n| n.to_str()) {
                            entry_provides.push(Capability::Executable(cmd.to_string()));
                        }
                    }
                }
                if let Some(filename) = relative_path.file_name().and_then(|n| n.to_str())
                    && filename.contains(".so")
                {
                    entry_provides.push(Capability::SharedLibrary(filename.to_string()));
                }
            }

            entries.push(PackageEntry {
                relative_path,
                is_dir,
                is_symlink,
                symlink_target,
                mode,
                size,
            });
        }

        let pkginfo = pkginfo_content.ok_or_else(|| {
            Error::MalformedArchive(format!(
                "Missing '.PKGINFO' in ALPM package {}",
                path.display()
            ))
        })?;

        let (fields, raw_depends, raw_provides, raw_conflicts) = Self::parse_pkginfo(&pkginfo);

        let pkgname = fields
            .get("pkgname")
            .ok_or_else(|| Error::MalformedArchive("Missing 'pkgname' in .PKGINFO".into()))?;
        let name = PackageName::new(pkgname)?;

        let pkgver = fields
            .get("pkgver")
            .ok_or_else(|| Error::MalformedArchive("Missing 'pkgver' in .PKGINFO".into()))?;
        let version = PackageVersion::new(pkgver);
        version.validate()?;

        let arch_str = fields.get("arch").map(|s| s.as_str()).unwrap_or("x86_64");
        let architecture = Architecture::parse(arch_str);

        let description = fields.get("pkgdesc").cloned();
        let installed_size = fields.get("size").and_then(|s| s.parse::<u64>().ok());

        // Inventory lifecycle scripts under default-deny (INV-003)
        let mut scripts = Vec::new();
        if let Some(inst) = install_content {
            if !inst.trim().is_empty() {
                scripts.push(LifecycleScript {
                    name: "install".to_string(),
                    content: inst,
                });
            }
        }

        // Collect capabilities
        let mut provides = entry_provides;
        let mut versioned_provides = Vec::new();
        for p in raw_provides {
            let (name, version) = p
                .split_once('=')
                .map(|(name, version)| (name.trim(), Some(version.trim())))
                .unwrap_or((p.as_str(), None));
            let capability = if name.ends_with(".so") || name.contains(".so.") {
                Capability::SharedLibrary(name.to_string())
            } else if !p.starts_with('/') {
                Capability::Feature(name.to_string())
            } else {
                continue;
            };
            if let Some(version) = version {
                let version = PackageVersion::new(version);
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

        // Collect dependencies and normalized constraints
        let mut dependencies = Vec::new();
        let mut constraints = Vec::new();

        for dep_str in raw_depends {
            let (d_name, v_op, v_str) = if let Some(idx) = dep_str.find(">=") {
                (
                    &dep_str[..idx],
                    Some(VersionOp::GreaterEqual),
                    Some(&dep_str[idx + 2..]),
                )
            } else if let Some(idx) = dep_str.find("<=") {
                (
                    &dep_str[..idx],
                    Some(VersionOp::LessEqual),
                    Some(&dep_str[idx + 2..]),
                )
            } else if let Some(idx) = dep_str.find('>') {
                (
                    &dep_str[..idx],
                    Some(VersionOp::Greater),
                    Some(&dep_str[idx + 1..]),
                )
            } else if let Some(idx) = dep_str.find('<') {
                (
                    &dep_str[..idx],
                    Some(VersionOp::Less),
                    Some(&dep_str[idx + 1..]),
                )
            } else if let Some(idx) = dep_str.find('=') {
                (
                    &dep_str[..idx],
                    Some(VersionOp::Exact),
                    Some(&dep_str[idx + 1..]),
                )
            } else {
                (dep_str.as_str(), None, None)
            };

            let ver_constraint = match (v_op, v_str) {
                (Some(op), Some(v)) => VersionConstraint::Relational(op, PackageVersion::new(v)),
                _ => VersionConstraint::Any,
            };

            dependencies.push(Dependency {
                raw: dep_str.clone(),
                name: d_name.to_string(),
                version_constraint: v_str.map(|s| s.to_string()),
                ecosystem: "alpm".to_string(),
            });

            if d_name.ends_with(".so") || d_name.contains(".so.") {
                constraints.push(Constraint::Capability(CapabilityConstraint {
                    identifier: format!("lib:{d_name}"),
                    version: ver_constraint,
                    original_expression: dep_str,
                }));
            } else if let Ok(pkg_name) = PackageName::new(d_name) {
                constraints.push(Constraint::Package {
                    name: pkg_name,
                    version: ver_constraint,
                    ecosystem: "alpm".to_string(),
                    original_expression: dep_str,
                });
            } else {
                constraints.push(Constraint::Capability(CapabilityConstraint {
                    identifier: format!("feature:{d_name}"),
                    version: ver_constraint,
                    original_expression: dep_str,
                }));
            }
        }

        for c in raw_conflicts {
            let (target, version) = parse_alpm_name_version(&c);
            constraints.push(Constraint::Conflict {
                target: target.to_string(),
                version,
                original_expression: c,
            });
        }

        Ok(NormalizedPackage {
            name,
            version,
            architecture,
            format: PackageFormat::Alpm,
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
        let file = File::open(path)?;
        let mut archive = Self::make_tar_reader(Box::new(file), path)?;

        let mut extracted_files = Vec::new();
        let mut total_bytes = 0u64;
        let mut entries_count = 0usize;
        let mut symlinks_count = 0usize;
        let mut seen_paths = HashSet::new();
        let mut pending_links = Vec::new();

        fs::create_dir_all(destination)?;

        for entry_res in archive.entries()? {
            let mut entry = entry_res.map_err(|e| {
                Error::MalformedArchive(format!("Malformed tar entry in ALPM package: {e}"))
            })?;

            let raw_path = entry.path()?.to_path_buf();
            if Self::is_alpm_metadata_file(&raw_path) {
                continue;
            }

            let relative_path = Self::sanitize_relative_path(&raw_path)?;
            if relative_path.as_os_str().is_empty() {
                continue;
            }

            if seen_paths.contains(&relative_path) {
                return Err(Error::MalformedArchive(format!(
                    "Duplicate entry in ALPM payload: {}",
                    relative_path.display()
                )));
            }
            seen_paths.insert(relative_path.clone());

            entries_count += 1;
            if entries_count > limits.max_entries {
                return Err(Error::LimitsExceeded(format!(
                    "ALPM entry count exceeded limit of {}",
                    limits.max_entries
                )));
            }

            let size = entry.header().size().unwrap_or(0);
            if size > limits.max_single_file_bytes {
                return Err(Error::LimitsExceeded(format!(
                    "File size {size} exceeds single-file limit {}",
                    limits.max_single_file_bytes
                )));
            }

            total_bytes = total_bytes.checked_add(size).ok_or_else(|| {
                Error::LimitsExceeded("ALPM total extracted bytes overflowed u64".into())
            })?;
            if total_bytes > limits.max_total_bytes {
                return Err(Error::LimitsExceeded(format!(
                    "Total extracted bytes exceeded limit of {}",
                    limits.max_total_bytes
                )));
            }

            let entry_type = entry.header().entry_type();

            if entry_type.is_dir() {
                Self::create_payload_dirs(destination, &relative_path)?;
            } else if entry_type.is_symlink() {
                let link_target = entry.link_name()?.ok_or_else(|| {
                    Error::MalformedArchive(format!(
                        "Missing symlink target for {}",
                        relative_path.display()
                    ))
                })?;

                let sanitized_target =
                    super::sanitize_symlink_target(&relative_path, &link_target, destination)?;
                // Defer materialization until regular files are written.  This
                // prevents archive order from redirecting a write through a
                // previously-created symlink.
                pending_links.push((relative_path.clone(), sanitized_target));
                symlinks_count += 1;
            } else if entry_type.is_file() {
                if let Some(parent) = relative_path.parent() {
                    Self::create_payload_dirs(destination, parent)?;
                }

                let target_path = destination.join(&relative_path);
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target_path)?;
                io::copy(&mut entry, &mut output)?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = entry.header().mode().unwrap_or(0o644);
                    fs::set_permissions(&target_path, fs::Permissions::from_mode(mode))?;
                }
            } else {
                return Err(Error::MalformedArchive(format!(
                    "Unsupported ALPM payload entry type: {}",
                    relative_path.display()
                )));
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

fn parse_alpm_name_version(raw: &str) -> (&str, VersionConstraint) {
    let raw = raw.trim();
    let (name, op, version) = [
        (">=", VersionOp::GreaterEqual),
        ("<=", VersionOp::LessEqual),
        ("=", VersionOp::Exact),
        (">", VersionOp::Greater),
        ("<", VersionOp::Less),
    ]
    .into_iter()
    .find_map(|(token, op)| {
        raw.split_once(token)
            .map(|(name, version)| (name.trim(), Some(op), version.trim()))
    })
    .unwrap_or((raw, None, ""));
    let constraint = op
        .zip((!version.is_empty()).then(|| PackageVersion::new(version)))
        .map(|(op, version)| VersionConstraint::Relational(op, version))
        .unwrap_or(VersionConstraint::Any);
    (name, constraint)
}
