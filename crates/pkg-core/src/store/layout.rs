//! Isolated rootless store layout and directory management (ADR-003, ADR-014).

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::package::{ArtifactDigest, PackageName, PackageVersion};
use crate::error::Result;

/// Filesystem layout for pkg-owned isolated stores, profiles, and state.
#[derive(Debug, Clone)]
pub struct StoreLayout {
    base_dir: PathBuf,
}

impl StoreLayout {
    /// Creates a store layout rooted at the specified base directory.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Resolves the default rootless base directory according to XDG conventions.
    ///
    /// Checks `PKG_DATA_DIR`, then `XDG_DATA_HOME/pkg`, then `~/.local/share/pkg`.
    #[must_use]
    pub fn default_rootless() -> Self {
        if let Ok(val) = std::env::var("PKG_DATA_DIR") {
            if !val.trim().is_empty() {
                return Self::new(PathBuf::from(val));
            }
        }

        if let Ok(val) = std::env::var("XDG_DATA_HOME") {
            if !val.trim().is_empty() {
                return Self::new(PathBuf::from(val).join("pkg"));
            }
        }

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        Self::new(PathBuf::from(home).join(".local").join("share").join("pkg"))
    }

    /// Base root directory for all pkg-managed user data.
    #[must_use]
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Directory containing immutable store objects.
    #[must_use]
    pub fn store_dir(&self) -> PathBuf {
        self.base_dir.join("store")
    }

    /// Directory containing transaction staging areas.
    ///
    /// Staging is purposely placed on the same filesystem as `store`
    /// to guarantee atomic promotion via `rename` (INV-011, ADR-012).
    #[must_use]
    pub fn staging_root(&self) -> PathBuf {
        self.base_dir.join("staging")
    }

    /// Staging directory for a specific transaction.
    #[must_use]
    pub fn staging_dir(&self, transaction_id: &str) -> PathBuf {
        self.staging_root().join(transaction_id)
    }

    /// Store object path for a specific store ID.
    #[must_use]
    pub fn store_object_dir(&self, store_id: &str) -> PathBuf {
        self.store_dir().join(store_id)
    }

    /// Returns the active profiles directory (`profiles/`).
    pub fn profiles_dir(&self) -> PathBuf {
        self.base_dir.join("profiles")
    }

    /// Returns the package and metadata cache directory (`cache/`).
    pub fn cache_dir(&self) -> PathBuf {
        self.base_dir.join("cache")
    }

    /// Returns the trusted GPG keyrings directory (`keyrings/`).
    pub fn keyrings_dir(&self) -> PathBuf {
        self.base_dir.join("keyrings")
    }

    /// Returns the specific path for a cached artifact addressed by its digest.
    pub fn artifact_cache_path(&self, digest: &str) -> PathBuf {
        // e.g., cache/artifacts/sha256/abc123def...
        self.cache_dir().join("artifacts").join("sha256").join(digest)
    }

    /// Directory containing profiles.
    #[must_use]
    pub fn profiles_root(&self) -> PathBuf {
        self.base_dir.join("profiles")
    }

    /// Root directory for a specific profile (e.g. `default`).
    #[must_use]
    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.profiles_root().join(profile)
    }

    /// Exposed executable commands directory for a profile.
    #[must_use]
    pub fn profile_bin_dir(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("bin")
    }

    /// Directory holding state databases and metadata.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.base_dir.join("state")
    }

    /// SQLite state database path.
    #[must_use]
    pub fn db_path(&self) -> PathBuf {
        self.state_dir().join("pkg.db")
    }

    /// Single-writer process lock path (INV-012).
    #[must_use]
    pub fn lock_path(&self) -> PathBuf {
        self.base_dir.join("pkg.lock")
    }

    /// Computes a unique deterministic store ID for an artifact.
    #[must_use]
    pub fn compute_store_id(
        digest: &ArtifactDigest,
        name: &PackageName,
        version: &PackageVersion,
    ) -> String {
        let prefix = if digest.hex().len() >= 12 {
            &digest.hex()[..12]
        } else {
            digest.hex()
        };
        format!("{prefix}-{name}-{version}")
    }

    /// Creates all necessary top-level layout directories if they do not exist.
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.store_dir())?;
        fs::create_dir_all(self.staging_root())?;
        fs::create_dir_all(self.state_dir())?;
        fs::create_dir_all(self.profiles_root())?;
        Ok(())
    }

    /// Atomically promotes a prepared staging directory to its final immutable store location.
    pub fn promote_staging(&self, staging_path: &Path, final_store_path: &Path) -> Result<()> {
        if let Some(parent) = final_store_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // If final store already exists (e.g. reinstall), remove it first or replace
        if final_store_path.exists() {
            fs::remove_dir_all(final_store_path)?;
        }
        fs::rename(staging_path, final_store_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_id_computation() {
        let digest = ArtifactDigest::sha256("abcdef1234567890abcdef123456");
        let name = PackageName::new("ripgrep").unwrap();
        let ver = PackageVersion::new("14.1.0");
        let store_id = StoreLayout::compute_store_id(&digest, &name, &ver);
        assert_eq!(store_id, "abcdef123456-ripgrep-14.1.0");
    }
}
