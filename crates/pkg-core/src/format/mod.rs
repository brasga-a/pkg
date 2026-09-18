pub mod alpm;
pub mod deb;
pub mod rpm;

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::domain::package::{NormalizedPackage, PackageFormat};
use crate::error::{Error, Result};

/// Detects package format based on magic bytes and filename heuristics.
pub fn detect_format(path: &Path) -> Result<PackageFormat> {
    if let Ok(mut file) = File::open(path) {
        let mut magic = [0u8; 8];
        let n = file.read(&mut magic).unwrap_or(0);

        // RPM magic: 0xED 0xAB 0xEE 0xDB
        if n >= 4 && magic[..4] == [0xed, 0xab, 0xee, 0xdb] {
            return Ok(PackageFormat::Rpm);
        }

        // Debian archive starts with "!<arch>\n"
        if n >= 7 && &magic[..7] == b"!<arch>" {
            return Ok(PackageFormat::Deb);
        }

        // ALPM packages are compressed tar archives (commonly .pkg.tar.zst or .pkg.tar.xz)
        // Zstandard magic: 0x28 0xB5 0x2F 0xFD
        if n >= 4 && magic[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
            return Ok(PackageFormat::Alpm);
        }

        // XZ magic: 0xFD '7' 'z' 'X' 'Z' 0x00
        if n >= 6 && &magic[..6] == b"\xfd7zXZ\x00" {
            return Ok(PackageFormat::Alpm);
        }
    }

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.contains(".pkg.tar") {
        return Ok(PackageFormat::Alpm);
    }
    if name.ends_with(".rpm") {
        return Ok(PackageFormat::Rpm);
    }
    if name.ends_with(".deb") {
        return Ok(PackageFormat::Deb);
    }

    Err(Error::MalformedArchive(format!(
        "Unsupported or unrecognized package format for {}",
        path.display()
    )))
}

/// Instantiates the appropriate format adapter for the detected package format.
#[must_use]
pub fn get_adapter(format: PackageFormat) -> Box<dyn ArtifactAdapter> {
    match format {
        PackageFormat::Deb => Box::new(deb::DebAdapter::new()),
        PackageFormat::Rpm => Box::new(rpm::RpmAdapter::new()),
        PackageFormat::Alpm | PackageFormat::Tarball => Box::new(alpm::AlpmAdapter::new()),
    }
}

/// Safety limits on archive extraction to protect against zip bombs, exhaustion, and traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionLimits {
    /// Maximum allowed number of entries in the archive.
    pub max_entries: usize,
    /// Maximum total uncompressed payload bytes across all entries.
    pub max_total_bytes: u64,
    /// Maximum uncompressed bytes for any single file entry.
    pub max_single_file_bytes: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_entries: 50_000,
            max_total_bytes: 1_073_741_824,     // 1 GiB
            max_single_file_bytes: 268_435_456, // 256 MiB
        }
    }
}

/// Statistics and inventory of extracted package contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionReport {
    /// List of paths relative to the extraction destination.
    pub extracted_files: Vec<PathBuf>,
    /// Total bytes written to disk.
    pub total_bytes: u64,
    /// Number of entries extracted.
    pub entries_count: usize,
    /// Number of symbolic links created.
    pub symlinks_count: usize,
}

/// Format-specific artifact reader and extractor.
pub trait ArtifactAdapter {
    /// Inspects and parses metadata from an artifact without mutating the system or invoking external tools.
    fn parse_metadata(&self, path: &Path) -> Result<NormalizedPackage>;

    /// Extracts the package payload into the staging destination under safe limits.
    fn extract_payload(
        &self,
        path: &Path,
        destination: &Path,
        limits: &ExtractionLimits,
    ) -> Result<ExtractionReport>;
}

/// Sanitizes and validates a symlink target inside a package extraction staging directory.
///
/// If `target` is absolute (e.g. `/etc/R/Makeconf`), it is safely relativized with respect
/// to the package payload root (`staging_root`), ensuring that internal cross-directory references
/// are preserved within the isolated store directory while preventing escapes to the host filesystem.
///
/// Returns the relative sanitized target path, or `Error::SecurityViolation` if the target
/// escapes `staging_root`.
pub fn sanitize_symlink_target(
    entry_path: &Path,
    target: &Path,
    staging_root: &Path,
) -> Result<PathBuf> {
    use std::path::Component;

    let entry_dir = entry_path.parent().unwrap_or(Path::new(""));

    let rel_target = if target.is_absolute() {
        // Step 1: Normalize absolute target components relative to root
        let mut normalized_target = PathBuf::new();
        for comp in target.components() {
            match comp {
                Component::Prefix(_) | Component::RootDir => {}
                Component::CurDir => {}
                Component::ParentDir => {
                    if !normalized_target.pop() {
                        return Err(Error::SecurityViolation(format!(
                            "Absolute symlink target escapes package root: {} -> {}",
                            entry_path.display(),
                            target.display()
                        )));
                    }
                }
                Component::Normal(c) => {
                    normalized_target.push(c);
                }
            }
        }

        // Step 2: Compute relative path from entry_dir to normalized_target
        let from_comps: Vec<_> = entry_dir
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s),
                _ => None,
            })
            .collect();

        let to_comps: Vec<_> = normalized_target
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s),
                _ => None,
            })
            .collect();

        let common_len = from_comps
            .iter()
            .zip(to_comps.iter())
            .take_while(|(a, b)| a == b)
            .count();

        let mut computed = PathBuf::new();
        for _ in common_len..from_comps.len() {
            computed.push("..");
        }
        for comp in &to_comps[common_len..] {
            computed.push(comp);
        }
        if computed.as_os_str().is_empty() {
            computed.push(".");
        }
        computed
    } else {
        target.to_path_buf()
    };

    // Step 3: Validate that rel_target does not escape staging_root
    let mut resolved = staging_root.join(entry_dir);
    for comp in rel_target.components() {
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

    Ok(rel_target)
}
