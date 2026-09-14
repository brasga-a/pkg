//! Installed package records.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::package::{
    Architecture, ArtifactDigest, PackageFormat, PackageName, PackageVersion,
};

/// An installed package tracked in pkg state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPackage {
    /// Package name.
    pub name: PackageName,
    /// Installed version.
    pub version: PackageVersion,
    /// Architecture.
    pub architecture: Architecture,
    /// Package format.
    pub format: PackageFormat,
    /// Source artifact digest.
    pub digest: ArtifactDigest,
    /// Store object identifier.
    pub store_id: String,
    /// Absolute path to the store object directory.
    pub store_path: PathBuf,
    /// ISO 8601 installation timestamp.
    pub installed_at: String,
    /// Is this package currently active in the selected profile?
    pub active: bool,
    /// Profile name where this package is registered.
    pub profile: String,
    /// Executable commands exposed by this package in the profile.
    pub binaries: Vec<String>,
}
