//! Package format adapters.

pub mod deb;

use std::path::{Path, PathBuf};

use crate::domain::package::NormalizedPackage;
use crate::error::Result;

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
