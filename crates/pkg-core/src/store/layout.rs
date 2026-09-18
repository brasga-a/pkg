//! Isolated rootless store layout and directory management (ADR-003, ADR-014).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::domain::contracts::RuntimeManifest;
use crate::domain::package::{ArtifactDigest, PackageName, PackageVersion};
use crate::error::{Error, Result};
use sha2::{Digest, Sha256};

/// Filesystem layout for pkg-owned isolated stores, profiles, and state.
#[derive(Debug, Clone)]
pub struct StoreLayout {
    base_dir: PathBuf,
}

impl StoreLayout {
    /// Creates a store layout rooted at the specified base directory.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        Self {
            base_dir: if base_dir.is_absolute() {
                base_dir
            } else {
                std::env::current_dir()
                    .expect("current working directory unavailable")
                    .join(base_dir)
            },
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

    /// Root for immutable per-command runtime manifests and views.
    #[must_use]
    pub fn runtimes_dir(&self) -> PathBuf {
        self.base_dir.join("runtimes")
    }

    /// Directory for a single runtime identity.
    #[must_use]
    pub fn runtime_dir(&self, runtime_id: &str) -> PathBuf {
        self.runtimes_dir()
            .join(runtime_id.trim_start_matches("sha256:"))
    }

    /// Command-local loader view containing only the selected provider files.
    #[must_use]
    pub fn runtime_lib_dir(&self, runtime_id: &str) -> PathBuf {
        self.runtime_dir(runtime_id).join("lib")
    }

    /// Materializes a runtime view as literal symlinks to exact providers.
    pub fn materialize_runtime_view(
        &self,
        runtime_id: &str,
        providers: &std::collections::BTreeMap<String, PathBuf>,
    ) -> Result<std::collections::BTreeMap<String, PathBuf>> {
        let id = runtime_id.trim_start_matches("sha256:");
        Self::validate_component(id, "runtime id")?;
        ensure_directory(&self.runtimes_dir())?;
        ensure_directory(&self.runtime_dir(id))?;
        let dir = self.runtime_lib_dir(id);
        ensure_directory(&dir)?;
        let mut view = std::collections::BTreeMap::new();
        for (name, provider) in providers {
            Self::validate_component(name, "runtime library name")?;
            if !provider.exists() {
                return Err(Error::IncompatibleHost(format!(
                    "Selected runtime provider is unavailable: {}",
                    provider.display()
                )));
            }
            let destination = dir.join(name);
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if fs::read_link(&destination)? != *provider {
                        return Err(Error::SecurityViolation(format!(
                            "Runtime provider collision: {}",
                            destination.display()
                        )));
                    }
                }
                Ok(_) => {
                    return Err(Error::SecurityViolation(format!(
                        "Runtime view destination is not a symlink: {}",
                        destination.display()
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let nonce = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let temp = dir.join(format!(".{name}.{nonce}.tmp"));
                    let _ = fs::remove_file(&temp);
                    std::os::unix::fs::symlink(provider, &temp)?;
                    fs::rename(temp, &destination)?;
                }
                Err(error) => return Err(error.into()),
            }
            view.insert(name.clone(), destination);
        }
        Ok(view)
    }

    /// Writes an immutable runtime manifest exactly once.
    pub fn write_runtime_manifest(&self, manifest: &RuntimeManifest) -> Result<PathBuf> {
        let runtime_id = manifest.runtime_id.trim_start_matches("sha256:");
        Self::validate_component(runtime_id, "runtime id")?;
        ensure_directory(&self.runtimes_dir())?;
        ensure_directory(&self.runtime_dir(runtime_id))?;
        let dir = self.runtime_dir(runtime_id);
        ensure_directory(&dir)?;
        let path = dir.join("manifest.json");
        let bytes = serde_json::to_vec_pretty(manifest)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(Error::SecurityViolation(format!(
                    "Runtime manifest path is not a regular file: {}",
                    path.display()
                )));
            }
            Ok(_) => {
                let existing = fs::read(&path)?;
                if existing != bytes {
                    return Err(Error::SecurityViolation(format!(
                        "Runtime manifest identity collision: {}",
                        path.display()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Use a transaction-local temporary file and a hard-link
                // publication so a concurrent writer cannot replace an
                // already published immutable manifest.
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let temp = dir.join(format!(".manifest.{}.{}.tmp", std::process::id(), nonce));
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                match fs::hard_link(&temp, &path) {
                    Ok(()) => {
                        fs::remove_file(temp)?;
                    }
                    Err(link_error) if link_error.kind() == std::io::ErrorKind::AlreadyExists => {
                        fs::remove_file(temp)?;
                        let existing = fs::read(&path)?;
                        if existing != bytes {
                            return Err(Error::SecurityViolation(format!(
                                "Runtime manifest identity collision: {}",
                                path.display()
                            )));
                        }
                    }
                    Err(link_error) => {
                        let _ = fs::remove_file(&temp);
                        return Err(link_error.into());
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
        Ok(path)
    }

    /// Validates identifiers used as one filesystem path component.
    pub fn validate_component(value: &str, label: &str) -> Result<()> {
        if value.is_empty()
            || matches!(value, "." | "..")
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(Error::SecurityViolation(format!(
                "Invalid {label}: {value:?}"
            )));
        }
        Ok(())
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
        self.cache_dir()
            .join("artifacts")
            .join("sha256")
            .join(digest)
    }

    /// Directory containing profiles.
    #[must_use]
    pub fn profiles_root(&self) -> PathBuf {
        self.base_dir.join("profiles")
    }

    /// Directory containing immutable generations for a profile.
    #[must_use]
    pub fn profile_generations_dir(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("generations")
    }

    /// Current generation pointer for a profile.
    #[must_use]
    pub fn profile_current_path(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("current")
    }

    /// Stable command directory exposed to users.  It is a symlink to the
    /// selected generation's `bin` directory after generation publication.
    #[must_use]
    pub fn profile_bin_link(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("bin")
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

    /// Exposed shared libraries directory for a profile.
    #[must_use]
    pub fn profile_lib_dir(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("lib")
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

    /// Computes the realization identity from every input that can affect the
    /// transformed tree.  The legacy helper above remains available for
    /// callers that only need the historical display identifier.
    #[must_use]
    pub fn compute_derivation_id(
        digest: &ArtifactDigest,
        name: &PackageName,
        version: &PackageVersion,
        format: &str,
        normalizer_version: &str,
        recipe_version: &str,
        embedded_prefix: &Path,
    ) -> String {
        let input = format!(
            "artifact={digest}|name={name}|version={version}|format={format}|normalizer={normalizer_version}|recipes={recipe_version}|prefix={}",
            embedded_prefix.display()
        );
        let digest = format!("{:x}", Sha256::digest(input.as_bytes()));
        format!("{}-{name}-{version}", &digest[..16])
    }

    /// Creates all necessary top-level layout directories if they do not exist.
    pub fn ensure_dirs(&self) -> Result<()> {
        // Every managed namespace must be a real directory.  In particular,
        // rejecting a pre-existing symlink here prevents a later extraction,
        // runtime publication, or database write from following it outside
        // the configured root.
        ensure_directory(&self.base_dir)?;
        ensure_directory(&self.store_dir())?;
        ensure_directory(&self.staging_root())?;
        ensure_directory(&self.runtimes_dir())?;
        ensure_directory(&self.state_dir())?;
        ensure_directory(&self.profiles_root())?;
        Ok(())
    }

    /// Atomically promotes a prepared staging directory to its final immutable store location.
    pub fn promote_staging(&self, staging_path: &Path, final_store_path: &Path) -> Result<()> {
        self.validate_store_path(final_store_path)?;
        // Immutable objects may already be shared with another profile.
        if final_store_path.exists() {
            if !same_store_tree(staging_path, final_store_path)? {
                return Err(Error::SecurityViolation(format!(
                    "Existing store object differs from verified staging: {}",
                    final_store_path.display()
                )));
            }
            fs::remove_dir_all(staging_path)?;
            return Ok(());
        }
        fs::rename(staging_path, final_store_path)?;
        Ok(())
    }

    /// Only direct, non-symlink children of the store may be promoted or removed.
    pub fn validate_store_path(&self, path: &Path) -> Result<()> {
        let relative = path.strip_prefix(self.store_dir()).map_err(|_| {
            Error::SecurityViolation(format!("Path outside store: {}", path.display()))
        })?;
        let mut components = relative.components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
            || fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink())
        {
            return Err(Error::SecurityViolation(format!(
                "Invalid store path: {}",
                path.display()
            )));
        }
        Ok(())
    }

    /// Profile names are single path components, never filesystem paths.
    pub fn validate_profile(profile: &str) -> Result<()> {
        Self::validate_component(profile, "profile")
    }
}

/// Creates a directory only when an existing path is a real directory.  This
/// prevents a user-controlled symlink in the runtime namespace from redirecting
/// writes outside the pkg root.
pub(crate) fn ensure_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(Error::SecurityViolation(format!(
                "Runtime path is not a directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::SecurityViolation(format!(
                    "Runtime path is not a directory: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

// Reusing a shared object must not bypass verification of the newly prepared payload.
fn same_store_tree(prepared: &Path, existing: &Path) -> Result<bool> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let left = fs::symlink_metadata(prepared)?;
    let right = fs::symlink_metadata(existing)?;
    if left.file_type() != right.file_type() {
        return Ok(false);
    }
    if left.file_type().is_symlink() {
        return Ok(fs::read_link(prepared)? == fs::read_link(existing)?);
    }
    if left.is_dir() {
        let names = |path: &Path| -> Result<Vec<std::ffi::OsString>> {
            let mut names = fs::read_dir(path)?
                .map(|entry| entry.map(|e| e.file_name()))
                .collect::<std::io::Result<Vec<_>>>()?;
            names.sort();
            Ok(names)
        };
        let children = names(prepared)?;
        if children != names(existing)? {
            return Ok(false);
        }
        for child in children {
            if !same_store_tree(&prepared.join(&child), &existing.join(child))? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if !left.is_file()
        || left.len() != right.len()
        || left.permissions().mode() != right.permissions().mode()
    {
        return Ok(false);
    }
    let mut left = fs::File::open(prepared)?;
    let mut right = fs::File::open(existing)?;
    let mut a = vec![0; 65536];
    let mut b = vec![0; 65536];
    loop {
        let n = left.read(&mut a)?;
        if n == 0 {
            return Ok(right.read(&mut b[..1])? == 0);
        }
        right.read_exact(&mut b[..n])?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
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

    #[test]
    fn derivation_id_changes_when_recipe_or_prefix_changes() {
        let digest = ArtifactDigest::sha256("abcdef1234567890");
        let name = PackageName::new("tool").unwrap();
        let version = PackageVersion::new("1.0");
        let first = StoreLayout::compute_derivation_id(
            &digest,
            &name,
            &version,
            "deb",
            "normalizer-v1",
            "recipes-v1",
            Path::new("/tmp/a"),
        );
        let second = StoreLayout::compute_derivation_id(
            &digest,
            &name,
            &version,
            "deb",
            "normalizer-v1",
            "recipes-v2",
            Path::new("/tmp/a"),
        );
        let third = StoreLayout::compute_derivation_id(
            &digest,
            &name,
            &version,
            "deb",
            "normalizer-v1",
            "recipes-v1",
            Path::new("/tmp/b"),
        );
        assert_ne!(first, second);
        assert_ne!(first, third);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_dirs_rejects_symlinked_managed_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("pkg");
        std::fs::create_dir_all(&base).unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("store")).unwrap();

        let error = StoreLayout::new(&base).ensure_dirs().unwrap_err();
        assert!(matches!(error, Error::SecurityViolation(_)));
        assert!(outside.read_dir().unwrap().next().is_none());
    }
}
