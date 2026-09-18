//! Installation and removal plans.
//!
//! Planning is strictly side-effect free (INV-010).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::contracts::{
    AdaptationPlan, ArtifactEvidence, ExecutionPlan, PayloadManifest, RuntimeManifest,
};
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
    /// Missing host libraries permitted by user override (--ignore-missing-libs).
    #[serde(default)]
    pub missing_libraries: Vec<String>,
    /// Whether this is a dry-run execution.
    pub is_dry_run: bool,
    /// Evidence for the source artifact and trust boundary.
    #[serde(default)]
    pub artifact_evidence: Option<ArtifactEvidence>,
    /// Realized payload manifest, populated after staging for a normal install.
    #[serde(default)]
    pub payload_manifest: Option<PayloadManifest>,
    /// Explicit transformations applied before publication.
    #[serde(default)]
    pub adaptations: Vec<AdaptationPlan>,
    /// Per-command execution contracts.
    #[serde(default)]
    pub executions: Vec<ExecutionPlan>,
    /// Runtime manifests referenced by the execution contracts.
    #[serde(default)]
    pub runtimes: Vec<RuntimeManifest>,
    /// Additional normalized packages selected by the resolver for the root
    /// package's dependency closure.
    #[serde(default)]
    pub resolved_dependencies: Vec<NormalizedPackage>,
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
