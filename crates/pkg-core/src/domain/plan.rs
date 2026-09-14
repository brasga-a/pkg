//! Installation and removal plans.
//!
//! Planning is strictly side-effect free (INV-010).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::package::{NormalizedPackage, PackageName, PackageVersion};

/// An executable binary activation link within a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryActivation {
    /// The binary command name (e.g. `rg`).
    pub command: String,
    /// Path inside the store object relative to store object root (e.g. `usr/bin/rg`).
    pub relative_store_path: PathBuf,
    /// Target destination in the profile bin directory.
    pub profile_symlink_path: PathBuf,
    /// Previously recorded target that this activation is allowed to replace.
    pub previous_target: Option<PathBuf>,
}

/// A side-effect-free plan for installing a package into the pkg-owned store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    /// Normalized package metadata.
    pub package: NormalizedPackage,
    /// Unique store identifier (`<digest-prefix>-<name>-<version>`).
    pub store_id: String,
    /// Target directory where the package payload will be stored.
    pub target_store_dir: PathBuf,
    /// Executables to activate in the target profile.
    pub binaries: Vec<BinaryActivation>,
    /// Lifecycle scripts present in the archive that will be ignored by policy (INV-003).
    pub ignored_scripts: Vec<String>,
    /// Host libraries checked and verified present for ELF binaries.
    pub host_libraries_verified: Vec<String>,
    /// Whether this is a dry-run execution.
    pub is_dry_run: bool,
}

/// A side-effect-free plan for removing a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovePlan {
    /// Package name being removed.
    pub package_name: PackageName,
    /// Version being removed.
    pub version: PackageVersion,
    /// Active store ID.
    pub store_id: String,
    /// Physical store object directory.
    pub store_path: PathBuf,
    /// Binary symlinks to unlink in the profile.
    pub binaries_to_remove: Vec<PathBuf>,
    /// Recorded targets corresponding to binaries_to_remove.
    pub expected_targets: Vec<PathBuf>,
    /// Whether this is a dry-run execution.
    pub is_dry_run: bool,
}
