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
