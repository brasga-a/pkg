//! Core domain models for the `pkg` package manager.

pub mod capability;
pub mod installed;
pub mod package;
pub mod plan;

pub use capability::{Capability, Dependency};
pub use installed::InstalledPackage;
pub use package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion,
};
pub use plan::{BinaryActivation, InstallPlan, RemovePlan};
