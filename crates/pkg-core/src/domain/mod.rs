//! Core domain models for the `pkg` package manager.

pub mod capability;
pub mod constraint;
pub mod installed;
pub mod package;
pub mod plan;
pub mod relocation;
pub mod version;

pub use capability::{Capability, Dependency};
pub use constraint::{CapabilityConstraint, Constraint, VersionConstraint, VersionOp};
pub use installed::InstalledPackage;
pub use package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion,
};
pub use plan::{BinaryActivation, InstallPlan, RemovePlan};
pub use relocation::{PackageFhsIndex, relocate_extracted_text_files, relocate_text_content};
pub use version::{VersionEcosystem, compare_versions};
