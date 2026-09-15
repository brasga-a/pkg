//! Debian `.deb` package format adapter.
//!
//! Implements RFC 822 control parsing, archive extraction, and strict security validation
//! without invoking `dpkg` (INV-001, ADR-005, Gate M1-A).

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};

use crate::domain::capability::{Capability, Dependency};
use crate::domain::constraint::{CapabilityConstraint, Constraint, VersionConstraint, VersionOp};
use crate::domain::package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion,
};
use crate::error::{Error, Result};
use crate::format::{ArtifactAdapter, ExtractionLimits, ExtractionReport};

/// Debian package format adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebAdapter;

impl DebAdapter {
    fn create_payload_dirs(root: &Path, relative: &Path) -> Result<()> {
        let mut current = root.to_path_buf();
        for component in relative.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(Error::SecurityViolation(format!(
                        "Non-directory payload parent: {}",
                        current.display()
                    )));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Creates a new `DebAdapter`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Computes the SHA-256 digest of a file.
    pub fn compute_digest(path: &Path) -> Result<ArtifactDigest> {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let result = hasher.finalize();
        Ok(ArtifactDigest::sha256(format!("{result:x}")))
    }

    /// Decompresses a tar payload based on its extension.
    fn make_tar_reader<'a>(
        name: &str,
        reader: Box<dyn Read + 'a>,
    ) -> Result<tar::Archive<Box<dyn Read + 'a>>> {
        if name.ends_with(".gz") || name.ends_with(".tgz") {
            let decoder = flate2::read::GzDecoder::new(reader);
            Ok(tar::Archive::new(Box::new(decoder)))
        } else if name.ends_with(".xz") {
            let decoder = xz2::read::XzDecoder::new(reader);
            Ok(tar::Archive::new(Box::new(decoder)))
        } else if name.ends_with(".zst") || name.ends_with(".zstd") {
            let decoder = zstd::Decoder::new(reader).map_err(|e| {
                Error::MalformedArchive(format!("Failed to initialize zstd decoder: {e}"))
            })?;
            Ok(tar::Archive::new(Box::new(decoder)))
        } else if name.ends_with(".tar") {
            Ok(tar::Archive::new(reader))
        } else {
            Err(Error::MalformedArchive(format!(
                "Unsupported compression for archive member: {name}"
            )))
        }
    }

    /// Parses Debian RFC 822 control format key-value pairs.
    fn parse_control_fields(content: &str) -> HashMap<String, String> {
        let mut fields = HashMap::new();
        let mut current_key: Option<String> = None;
        let mut current_value = String::new();

        for line in content.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                // Continuation line
                if current_key.is_some() {
                    current_value.push('\n');
                    current_value.push_str(line.trim_start());
                }
            } else if let Some((key, value)) = line.split_once(':') {
                if let Some(prev_key) = current_key.take() {
                    fields.insert(prev_key, current_value.trim().to_string());
                    current_value.clear();
                }
                current_key = Some(key.trim().to_string());
                current_value.push_str(value.trim());
            }
        }

        if let Some(prev_key) = current_key {
            fields.insert(prev_key, current_value.trim().to_string());
        }

        fields
    }

    /// Parses a Debian `Depends` line into individual dependencies.
    fn parse_dependencies(raw_depends: &str) -> Vec<Dependency> {
        let mut deps = Vec::new();
        for item in raw_depends.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            // If alternative `a | b`, take the primary choice for dependency recording
            let primary = item.split('|').next().unwrap_or(item).trim();

            let (name, ver) = if let Some(open) = primary.find('(') {
                let name = primary[..open].trim().to_string();
                let close = primary.find(')').unwrap_or(primary.len());
                let constraint = primary[open + 1..close].trim().to_string();
                (name, Some(constraint))
            } else {
                (primary.to_string(), None)
            };

            deps.push(Dependency {
                raw: item.to_string(),
                name,
                version_constraint: ver,
                ecosystem: "debian".to_string(),
            });
        }
        deps
    }

    fn parse_single_debian_constraint(raw_item: &str) -> Option<Constraint> {
        let item = raw_item.trim();
        if item.is_empty() {
            return None;
        }

        let (name, ver_op, ver_val) = if let Some(open) = item.find('(') {
            let name = item[..open].trim();
            let close = item.find(')').unwrap_or(item.len());
            let inner = item[open + 1..close].trim();
            let mut parts = inner.split_whitespace();
            let op_str = parts.next().unwrap_or("");
            let ver_str = parts.next().unwrap_or("");

            let op = match op_str {
                ">=" => Some(VersionOp::GreaterEqual),
                "<=" => Some(VersionOp::LessEqual),
                ">>" | ">" => Some(VersionOp::Greater),
                "<<" | "<" => Some(VersionOp::Less),
                "=" => Some(VersionOp::Exact),
                _ => None,
            };

            (
                name,
                op,
                if ver_str.is_empty() {
                    None
                } else {
                    Some(ver_str)
                },
            )
        } else {
            (item, None, None)
        };

        let ver_constraint = match (ver_op, ver_val) {
            (Some(op), Some(val)) => VersionConstraint::Relational(op, PackageVersion::new(val)),
            _ => VersionConstraint::Any,
        };

        if name.starts_with("lib") && (name.contains(".so") || name.ends_with(".so")) {
            Some(Constraint::Capability(CapabilityConstraint {
                identifier: format!("lib:{name}"),
                version: ver_constraint,
                original_expression: raw_item.to_string(),
            }))
        } else if let Ok(pkg_name) = PackageName::new(name) {
            Some(Constraint::Package {
                name: pkg_name,
                version: ver_constraint,
                ecosystem: "debian".to_string(),
                original_expression: raw_item.to_string(),
            })
        } else {
            Some(Constraint::Capability(CapabilityConstraint {
                identifier: format!("feature:{name}"),
                version: ver_constraint,
                original_expression: raw_item.to_string(),
            }))
        }
    }

    fn parse_constraints(fields: &HashMap<String, String>) -> Vec<Constraint> {
        let mut constraints = Vec::new();

        if let Some(depends_raw) = fields.get("Depends") {
            for group in depends_raw.split(',') {
                let group = group.trim();
                if group.is_empty() {
                    continue;
                }
                let alternatives: Vec<&str> = group
                    .split('|')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if alternatives.len() > 1 {
                    let mut any_of = Vec::new();
                    for alt in alternatives {
                        if let Some(c) = Self::parse_single_debian_constraint(alt) {
                            any_of.push(c);
                        }
                    }
                    if !any_of.is_empty() {
                        constraints.push(Constraint::AnyOf(any_of));
                    }
                } else if let Some(alt) = alternatives.first() {
                    if let Some(c) = Self::parse_single_debian_constraint(alt) {
                        constraints.push(c);
                    }
                }
            }
        }

        if let Some(conflicts_raw) = fields.get("Conflicts").or_else(|| fields.get("Breaks")) {
            for item in conflicts_raw.split(',') {
                let item = item.trim();
                if !item.is_empty() {
                    let target = item.split_whitespace().next().unwrap_or(item).to_string();
                    constraints.push(Constraint::Conflict {
                        target,
                        original_expression: item.to_string(),
                    });
                }
            }
        }

        constraints
    }

    /// Validates a relative archive path to guarantee it cannot escape the staging root.
    pub fn sanitize_relative_path(path: &Path) -> Result<PathBuf> {
        let mut normalized = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute path rejected in archive entry: {}",
                        path.display()
                    )));
                }
                Component::ParentDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Path traversal ('..') rejected in archive entry: {}",
                        path.display()
                    )));
                }
                Component::CurDir => {}
                Component::Normal(c) => {
                    normalized.push(c);
                }
            }
        }
        // If the path was "." or "./", normalized is empty, representing the archive root itself.
        // Callers can check if normalized.as_os_str().is_empty() to skip root directory entries.
        Ok(normalized)
    }

    /// Verifies that a symlink target does not escape the destination directory.
    fn validate_symlink_target(
        entry_path: &Path,
        target: &Path,
        staging_root: &Path,
    ) -> Result<()> {
        if target.is_absolute() {
            return Err(Error::SecurityViolation(format!(
                "Absolute symlink target rejected: {} -> {}",
                entry_path.display(),
                target.display()
            )));
        }

        // Resolve target relative to entry parent
        let entry_dir = entry_path.parent().unwrap_or(Path::new(""));
        let mut resolved = staging_root.join(entry_dir);
        for comp in target.components() {
            match comp {
                Component::ParentDir => {
                    if resolved == staging_root {
                        return Err(Error::SecurityViolation(format!(
                            "Escaping symlink rejected: {} points outside staging ({})",
                            entry_path.display(),
                            target.display()
                        )));
                    }
                    resolved.pop();
                }
                Component::Normal(c) => {
                    resolved.push(c);
                }
                Component::CurDir => {}
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute component in symlink target: {}",
                        target.display()
                    )));
                }
            }
        }

        Ok(())
    }

    /// Verifies that a materialized symlink (and any chain of links) does not escape `root`.
    /// Handles existing target files as well as dangling/cross-package targets safely.
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
                "Symlink parent directory escapes staging: {}",
                relative.display()
            )));
        }

        for comp in target.components() {
            match comp {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(Error::SecurityViolation(format!(
                        "Absolute symlink target rejected: {} -> {}",
                        relative.display(),
                        target.display()
                    )));
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if current == root || !current.starts_with(root) {
                        return Err(Error::SecurityViolation(format!(
                            "Escaping payload link: {}",
                            relative.display()
                        )));
                    }
                    current.pop();
                }
                Component::Normal(c) => {
                    let next = current.join(c);
                    // If next exists and is a symlink, resolve it to detect chained escapes
                    if next.is_symlink() {
                        if let Ok(canon) = fs::canonicalize(&next) {
                            if !canon.starts_with(root) {
                                return Err(Error::SecurityViolation(format!(
                                    "Escaping payload link: {}",
                                    relative.display()
                                )));
                            }
                            current = canon;
                            continue;
                        }
                    }
                    current = next;
                }
            }
        }

        if !current.starts_with(root) {
            return Err(Error::SecurityViolation(format!(
                "Escaping payload link: {}",
                relative.display()
            )));
        }

        Ok(())
    }
}

impl ArtifactAdapter for DebAdapter {
    fn parse_metadata(&self, path: &Path) -> Result<NormalizedPackage> {
        let file = File::open(path)?;
        let size_bytes = file.metadata()?.len();
        let digest = Self::compute_digest(path)?;

        let mut archive = ar::Archive::new(file);

        let mut debian_binary_found = false;
        let mut control_tar_data: Option<(String, Vec<u8>)> = None;
        let mut data_tar_data: Option<(String, Vec<u8>)> = None;

        while let Some(entry_res) = archive.next_entry() {
            let mut entry = entry_res
                .map_err(|e| Error::MalformedArchive(format!("Failed to read ar entry: {e}")))?;

            let raw_id = std::str::from_utf8(entry.header().identifier())
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .to_string();

            if raw_id == "debian-binary" {
                if debian_binary_found {
                    return Err(Error::MalformedArchive(
                        "Duplicate 'debian-binary' entry in ar container".into(),
                    ));
                }
                let mut buf = String::new();
                entry.read_to_string(&mut buf)?;
                let trimmed = buf.trim();
                if trimmed != "2.0" {
                    return Err(Error::MalformedArchive(format!(
                        "Invalid debian-binary version: expected '2.0', found '{trimmed}'"
                    )));
                }
                debian_binary_found = true;
            } else if raw_id.starts_with("control.tar") {
                if control_tar_data.is_some() {
                    return Err(Error::MalformedArchive(
                        "Duplicate 'control.tar.*' entry in ar container".into(),
                    ));
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                control_tar_data = Some((raw_id, bytes));
            } else if raw_id.starts_with("data.tar") {
                if data_tar_data.is_some() {
                    return Err(Error::MalformedArchive(
                        "Duplicate 'data.tar.*' entry in ar container".into(),
                    ));
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                data_tar_data = Some((raw_id, bytes));
            }
        }

        if !debian_binary_found {
            return Err(Error::MalformedArchive(
                "Missing 'debian-binary' entry in ar container".into(),
            ));
        }

        let (control_name, control_bytes) = control_tar_data.ok_or_else(|| {
            Error::MalformedArchive("Missing 'control.tar.*' in .deb archive".into())
        })?;

        let (data_name, data_bytes) = data_tar_data.ok_or_else(|| {
            Error::MalformedArchive("Missing 'data.tar.*' in .deb archive".into())
        })?;

        // Parse control.tar.*
        let mut control_tar =
            Self::make_tar_reader(&control_name, Box::new(Cursor::new(control_bytes)))?;
        let mut control_content: Option<String> = None;
        let mut scripts = Vec::new();

        let control_entries = control_tar.entries().map_err(|e| {
            Error::MalformedArchive(format!("Failed to read control.tar entries: {e}"))
        })?;

        for entry_res in control_entries {
            let mut entry = entry_res.map_err(|e| {
                Error::MalformedArchive(format!("Malformed control.tar entry: {e}"))
            })?;

            let file_name = {
                let entry_path = entry.path().map_err(|e| {
                    Error::MalformedArchive(format!("Invalid entry path in control.tar: {e}"))
                })?;
                entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };

            if file_name == "control" {
                let mut text = String::new();
                entry.read_to_string(&mut text)?;
                control_content = Some(text);
            } else if matches!(
                file_name.as_str(),
                "preinst" | "postinst" | "prerm" | "postrm" | "config"
            ) {
                // Inventory maintainer script (INV-003, ADR-011)
                let mut text = String::new();
                entry.read_to_string(&mut text)?;
                scripts.push(LifecycleScript {
                    name: file_name,
                    content: text,
                });
            }
        }

        let control_text = control_content.ok_or_else(|| {
            Error::MalformedArchive("Missing 'control' file inside control.tar".into())
        })?;

        let fields = Self::parse_control_fields(&control_text);

        let package_raw = fields.get("Package").ok_or_else(|| {
            Error::MalformedArchive("Missing 'Package' field in control file".into())
        })?;
        let name = PackageName::new(package_raw)?;

        let version_raw = fields.get("Version").ok_or_else(|| {
            Error::MalformedArchive("Missing 'Version' field in control file".into())
        })?;
        let version = PackageVersion::new(version_raw);
        version.validate()?;

        let arch_raw = fields.get("Architecture").ok_or_else(|| {
            Error::MalformedArchive("Missing 'Architecture' field in control file".into())
        })?;
        let architecture = Architecture::parse(arch_raw);

        let description = fields.get("Description").cloned();
        let installed_size = fields
            .get("Installed-Size")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|kb| kb * 1024);

        let dependencies = fields
            .get("Depends")
            .map(|d| Self::parse_dependencies(d))
            .unwrap_or_default();
        let constraints = Self::parse_constraints(&fields);

        // Parse data.tar.* entries for inventory
        let mut data_tar = Self::make_tar_reader(&data_name, Box::new(Cursor::new(data_bytes)))?;
        let mut entries = Vec::new();
        let mut provides = Vec::new();

        if let Some(provides_raw) = fields.get("Provides") {
            for p in provides_raw.split(',') {
                let p = p.trim();
                if !p.is_empty() {
                    let name = p.split_whitespace().next().unwrap_or(p);
                    provides.push(Capability::Feature(name.to_string()));
                }
            }
        }
        let mut seen_paths = HashSet::new();

        let data_entries = data_tar.entries().map_err(|e| {
            Error::MalformedArchive(format!("Failed to read data.tar entries: {e}"))
        })?;

        for entry_res in data_entries {
            let entry = entry_res
                .map_err(|e| Error::MalformedArchive(format!("Malformed data.tar entry: {e}")))?;

            let raw_path = entry
                .path()
                .map_err(|e| Error::MalformedArchive(format!("Invalid path in data.tar: {e}")))?;

            // Sanitize path: will error if traversal or absolute
            let relative_path = Self::sanitize_relative_path(&raw_path)?;
            if relative_path.as_os_str().is_empty() {
                continue;
            }
            if !seen_paths.insert(relative_path.clone()) {
                return Err(Error::MalformedArchive(format!(
                    "Duplicate payload path: {}",
                    relative_path.display()
                )));
            }
            let header = entry.header();
            let entry_type = header.entry_type();
            let is_dir = entry_type.is_dir();
            let is_symlink = entry_type.is_symlink();
            let symlink_target = if is_symlink {
                header.link_name().ok().flatten().map(|p| p.into_owned())
            } else {
                None
            };
            let mode = header.mode().unwrap_or(0o644);
            let size = header.size().unwrap_or(0);

            // If binary in usr/bin or bin, inventory executable capability
            if !is_dir && !is_symlink {
                if let Some(parent) = relative_path.parent() {
                    if parent == Path::new("usr/bin") || parent == Path::new("bin") {
                        if let Some(cmd) = relative_path.file_name().and_then(|n| n.to_str()) {
                            provides.push(Capability::Executable(cmd.to_string()));
                        }
                    }
                }
            }

            // If shared library, inventory SharedLibrary capability
            if !is_dir {
                if let Some(filename) = relative_path.file_name().and_then(|n| n.to_str()) {
                    if filename.contains(".so") {
                        provides.push(Capability::SharedLibrary(filename.to_string()));
                    }
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

        Ok(NormalizedPackage {
            name,
            version,
            architecture,
            format: PackageFormat::Deb,
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
        let file = File::open(path)?;
        let mut archive = ar::Archive::new(file);

        let mut data_tar_data: Option<(String, Vec<u8>)> = None;

        while let Some(entry_res) = archive.next_entry() {
            let mut entry = entry_res
                .map_err(|e| Error::MalformedArchive(format!("Failed to read ar entry: {e}")))?;

            let raw_id = std::str::from_utf8(entry.header().identifier())
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .to_string();

            if raw_id.starts_with("data.tar") {
                if data_tar_data.is_some() {
                    return Err(Error::MalformedArchive(
                        "Duplicate 'data.tar.*' entry in ar container".into(),
                    ));
                }
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                data_tar_data = Some((raw_id, bytes));
            }
        }

        let (data_name, data_bytes) = data_tar_data.ok_or_else(|| {
            Error::MalformedArchive("Missing 'data.tar.*' in .deb archive".into())
        })?;

        let mut data_tar = Self::make_tar_reader(&data_name, Box::new(Cursor::new(data_bytes)))?;

        let mut extracted_files = Vec::new();
        let mut total_bytes = 0u64;
        let mut entries_count = 0usize;
        let mut symlinks_count = 0usize;
        let mut seen = HashSet::new();
        let mut pending_links = Vec::new();

        fs::create_dir_all(destination)?;

        let entries = data_tar.entries().map_err(|e| {
            Error::MalformedArchive(format!("Failed to read data.tar entries: {e}"))
        })?;

        for entry_res in entries {
            entries_count += 1;
            if entries_count > limits.max_entries {
                return Err(Error::LimitsExceeded(format!(
                    "Maximum entry count ({}) exceeded",
                    limits.max_entries
                )));
            }

            let mut entry = entry_res
                .map_err(|e| Error::MalformedArchive(format!("Malformed data.tar entry: {e}")))?;

            let raw_path = entry
                .path()
                .map_err(|e| Error::MalformedArchive(format!("Invalid path in data.tar: {e}")))?;

            let relative_path = Self::sanitize_relative_path(&raw_path)?;
            if relative_path.as_os_str().is_empty() {
                continue;
            }
            let target_path = destination.join(&relative_path);
            if !seen.insert(relative_path.clone()) {
                return Err(Error::MalformedArchive(format!(
                    "Duplicate payload path: {}",
                    relative_path.display()
                )));
            }

            let (is_dir, is_symlink, symlink_target_opt, file_size, file_mode) = {
                let header = entry.header();
                let entry_type = header.entry_type();
                if !entry_type.is_dir() && !entry_type.is_symlink() && !entry_type.is_file() {
                    return Err(Error::MalformedArchive(format!(
                        "Unsupported payload entry type: {}",
                        relative_path.display()
                    )));
                }
                let link_name = if entry_type.is_symlink() {
                    let link = header
                        .link_name()
                        .map_err(|e| {
                            Error::MalformedArchive(format!("Malformed symlink header: {e}"))
                        })?
                        .ok_or_else(|| {
                            Error::MalformedArchive("Missing symlink target in header".into())
                        })?;
                    Some(link.into_owned())
                } else {
                    None
                };
                (
                    entry_type.is_dir(),
                    entry_type.is_symlink(),
                    link_name,
                    header.size().unwrap_or(0),
                    header.mode().unwrap_or(0o644),
                )
            };

            if is_dir {
                Self::create_payload_dirs(destination, &relative_path)?;
            } else if is_symlink {
                symlinks_count += 1;
                let link_name = symlink_target_opt
                    .ok_or_else(|| Error::MalformedArchive("Missing symlink target".into()))?;

                Self::validate_symlink_target(&relative_path, &link_name, destination)?;

                // Materialize links only after all regular files, so archive order
                // cannot redirect writes through an earlier symlink.
                pending_links.push((relative_path.clone(), link_name));
                extracted_files.push(relative_path);
            } else {
                if file_size > limits.max_single_file_bytes {
                    return Err(Error::LimitsExceeded(format!(
                        "File '{}' size ({} bytes) exceeds single file limit ({} bytes)",
                        relative_path.display(),
                        file_size,
                        limits.max_single_file_bytes
                    )));
                }

                total_bytes = total_bytes.checked_add(file_size).ok_or_else(|| {
                    Error::LimitsExceeded("Total extraction bytes overflowed u64".into())
                })?;

                if total_bytes > limits.max_total_bytes {
                    return Err(Error::LimitsExceeded(format!(
                        "Total extracted size ({} bytes) exceeds total extraction limit ({} bytes)",
                        total_bytes, limits.max_total_bytes
                    )));
                }

                if let Some(parent) = relative_path.parent() {
                    Self::create_payload_dirs(destination, parent)?;
                }

                let mut out_file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target_path)?;
                io::copy(&mut entry, &mut out_file)?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    // Retain user permissions and execution bits
                    fs::set_permissions(&target_path, fs::Permissions::from_mode(file_mode))?;
                }

                extracted_files.push(relative_path);
            }
        }

        for (relative, target) in &pending_links {
            if let Some(parent) = relative.parent() {
                Self::create_payload_dirs(destination, parent)?;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_relative_path() {
        let valid = Path::new("usr/bin/ripgrep");
        assert_eq!(
            DebAdapter::sanitize_relative_path(valid).unwrap(),
            PathBuf::from("usr/bin/ripgrep")
        );

        let dot_slash = Path::new("./usr/bin/ripgrep");
        assert_eq!(
            DebAdapter::sanitize_relative_path(dot_slash).unwrap(),
            PathBuf::from("usr/bin/ripgrep")
        );

        let root_dot = Path::new("./");
        assert_eq!(
            DebAdapter::sanitize_relative_path(root_dot).unwrap(),
            PathBuf::new()
        );

        let absolute = Path::new("/etc/shadow");
        assert!(DebAdapter::sanitize_relative_path(absolute).is_err());

        let traversal = Path::new("usr/bin/../../etc/passwd");
        assert!(DebAdapter::sanitize_relative_path(traversal).is_err());
    }

    #[test]
    fn test_parse_control_fields() {
        let text = "Package: test-pkg\nVersion: 1.0.0\nArchitecture: amd64\nDepends: libc6 (>= 2.34)\nDescription: A test package\n with multiline description\n";
        let fields = DebAdapter::parse_control_fields(text);
        assert_eq!(fields.get("Package").unwrap(), "test-pkg");
        assert_eq!(fields.get("Version").unwrap(), "1.0.0");
        assert_eq!(fields.get("Architecture").unwrap(), "amd64");
        assert_eq!(fields.get("Depends").unwrap(), "libc6 (>= 2.34)");
        assert!(
            fields
                .get("Description")
                .unwrap()
                .contains("multiline description")
        );
    }
}
