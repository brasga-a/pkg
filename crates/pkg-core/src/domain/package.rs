//! Package domain entities and metadata normalization.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

use crate::domain::capability::{Capability, Dependency};
use crate::domain::constraint::Constraint;
use crate::error::{Error, Result};

/// A normalized package name.
///
/// Package names are kept with their source spelling because RPM names are
/// case-sensitive and may contain uppercase letters and underscores.  The
/// common portable alphabet is ASCII alphanumeric characters plus `-`, `.`,
/// `_`, and `+`; the first character must be alphanumeric.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageName(String);

impl PackageName {
    /// Creates and validates a package name.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(Error::MalformedArchive(
                "Package name cannot be empty".into(),
            ));
        }
        // Validate characters without applying a lossy case conversion. Debian
        // and ALPM repositories normally use lowercase names, while RPM
        // repositories legitimately contain names such as `0xFFFF` and
        // `AMF-devel`.
        let first = trimmed.chars().next().unwrap();
        if !first.is_ascii_alphanumeric() {
            return Err(Error::MalformedArchive(format!(
                "Invalid package name '{trimmed}': must start with an ASCII letter or digit"
            )));
        }
        for ch in trimmed.chars() {
            if !ch.is_ascii_alphanumeric() && !matches!(ch, '-' | '.' | '_' | '+') {
                return Err(Error::MalformedArchive(format!(
                    "Invalid character '{ch}' in package name '{trimmed}'"
                )));
            }
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Returns the raw package name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Package version preserving original source distribution syntax (INV-008).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageVersion(String);

impl PackageVersion {
    /// Rejects versions that cannot safely identify a Debian store object.
    pub fn validate(&self) -> Result<()> {
        if self.0.is_empty()
            || self.0.len() > 128
            || !self
                .0
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".+~:_^-".contains(&b))
        {
            return Err(Error::MalformedArchive(format!(
                "Invalid package version: {:?}",
                self.0
            )));
        }
        Ok(())
    }

    /// Creates a package version string.
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into().trim().to_string())
    }

    /// Returns the version string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Package target architecture.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Architecture {
    /// Linux x86_64 / amd64.
    X86_64,
    /// Architecture independent (Debian 'all').
    All,
    /// Wildcard architecture.
    Any,
    /// Other architecture (e.g. arm64, riscv64).
    Other(String),
}

impl Architecture {
    /// Parses an architecture string from package metadata.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "amd64" | "x86_64" | "x86-64" => Self::X86_64,
            "all" | "noarch" => Self::All,
            "any" => Self::Any,
            other => Self::Other(other.to_string()),
        }
    }

    /// Checks if this package architecture can run on the target host architecture.
    #[must_use]
    pub fn matches_host(&self, host: &Self) -> bool {
        match (self, host) {
            (Self::All | Self::Any, _) => true,
            (Self::X86_64, Self::X86_64) => true,
            (Self::Other(a), Self::Other(b)) => canonical_arch(a) == canonical_arch(b),
            _ => false,
        }
    }

    /// Canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::X86_64 => "x86_64",
            Self::All => "all",
            Self::Any => "any",
            Self::Other(s) => s.as_str(),
        }
    }
}

fn canonical_arch(value: &str) -> &str {
    match value {
        "arm64" => "aarch64",
        "armhf" | "armv7h" | "armv7" => "arm",
        "i386" | "i486" | "i586" | "x86" => "i686",
        "ppc64el" => "ppc64le",
        other => other,
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Package archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PackageFormat {
    /// Debian archive (.deb).
    Deb,
    /// Red Hat package (.rpm).
    Rpm,
    /// Arch Linux package (.pkg.tar.zst).
    Alpm,
    /// Generic tarball.
    Tarball,
}

impl fmt::Display for PackageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deb => write!(f, "deb"),
            Self::Rpm => write!(f, "rpm"),
            Self::Alpm => write!(f, "alpm"),
            Self::Tarball => write!(f, "tarball"),
        }
    }
}

/// Cryptographic digest identifying an artifact (ADR-013).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactDigest {
    algorithm: String,
    hex: String,
}

impl ArtifactDigest {
    /// Creates a new artifact digest.
    pub fn new(algorithm: impl Into<String>, hex: impl Into<String>) -> Self {
        Self {
            algorithm: algorithm.into(),
            hex: hex.into(),
        }
    }

    /// Creates a SHA-256 digest from raw hex string.
    pub fn sha256(hex: impl Into<String>) -> Self {
        Self::new("sha256", hex)
    }

    /// Computes the SHA-256 digest of a file.
    pub fn from_file(path: &Path) -> Result<Self> {
        use sha2::{Digest, Sha256};
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = std::io::Read::read(&mut file, &mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(Self::sha256(format!("{:x}", hasher.finalize())))
    }

    /// Returns the digest hex string.
    #[must_use]
    pub fn hex(&self) -> &str {
        &self.hex
    }

    /// Returns the algorithm name.
    #[must_use]
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }
}

impl fmt::Display for ArtifactDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.hex)
    }
}

/// A maintainer lifecycle script present in the package control archive.
///
/// In accordance with INV-003 and ADR-011, scripts are inventoried as data
/// but never executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleScript {
    /// Script name (e.g. preinst, postinst, prerm, postrm).
    pub name: String,
    /// Script body/content.
    pub content: String,
}

/// A verified, installed package present in the local store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub format: String,
    pub digest: String,
    pub store_id: String,
    pub store_path: PathBuf,
    pub installed_at: String, // ISO8601 string
    pub active: bool,
    pub profile: String,
    pub binaries: Vec<String>,
}

/// A package available in a remote repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePackage {
    pub repository_id: String,
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub format: String,
    pub digest: String,
    pub size_bytes: u64,
    pub url: String,
    /// Normalized dependency constraints from the repository snapshot.
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// Capabilities explicitly provided by repository metadata.
    #[serde(default)]
    pub provides: Vec<Capability>,
    /// Capabilities with an explicit provider version from repository metadata.
    /// Keeping this separate from the package version prevents an unversioned
    /// virtual provide from accidentally satisfying a relational requirement.
    #[serde(default)]
    pub versioned_provides: Vec<VersionedCapability>,
}

/// Metadata describing a single file entry in the package payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageEntry {
    /// Target path inside the store relative to store root.
    pub relative_path: PathBuf,
    /// Is this entry a directory?
    pub is_dir: bool,
    /// Is this entry a symlink?
    pub is_symlink: bool,
    /// Symlink target path if it is a symlink.
    pub symlink_target: Option<PathBuf>,
    /// Unix file permission mode.
    pub mode: u32,
    /// Size in bytes.
    pub size: u64,
}

/// A fully parsed and normalized package description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedPackage {
    /// Normalized package name.
    pub name: PackageName,
    /// Source package version.
    pub version: PackageVersion,
    /// Target architecture.
    pub architecture: Architecture,
    /// Source archive format.
    pub format: PackageFormat,
    /// Cryptographic digest of source artifact.
    pub digest: ArtifactDigest,
    /// Size of source artifact in bytes.
    pub size_bytes: u64,
    /// Optional package description.
    pub description: Option<String>,
    /// Declared dependencies.
    pub dependencies: Vec<Dependency>,
    /// Normalized constraint requirements (ADR-009).
    #[serde(default)]
    pub constraints: Vec<crate::domain::constraint::Constraint>,
    /// Provided capabilities (e.g. binaries).
    pub provides: Vec<Capability>,
    /// Capabilities whose source metadata carries an explicit provider
    /// version (for example Debian/RPM versioned `Provides`).  The legacy
    /// `provides` list remains the unversioned form for compatibility.
    #[serde(default)]
    pub versioned_provides: Vec<VersionedCapability>,
    /// Inventoried maintainer scripts (not executed).
    pub scripts: Vec<LifecycleScript>,
    /// Package file entries.
    pub entries: Vec<PackageEntry>,
    /// Installed uncompressed size estimate in bytes.
    pub installed_size: Option<u64>,
}

/// A capability with an explicit native package version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedCapability {
    pub capability: Capability,
    pub version: PackageVersion,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_architecture_noarch_matches_x86_64_host() {
        let package = Architecture::parse("noarch");
        assert_eq!(package, Architecture::All);
        let host = Architecture::X86_64;
        assert!(package.matches_host(&host));

        assert!(Architecture::parse("all").matches_host(&host));
        assert!(Architecture::parse("any").matches_host(&host));
        assert!(Architecture::parse("x86_64").matches_host(&host));
        assert!(Architecture::parse("amd64").matches_host(&host));
        assert!(!Architecture::parse("aarch64").matches_host(&host));
        assert!(Architecture::parse("arm64").matches_host(&Architecture::parse("aarch64")));
        assert!(Architecture::parse("i386").matches_host(&Architecture::parse("i686")));
    }
}
