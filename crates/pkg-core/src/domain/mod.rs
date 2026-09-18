//! Core domain models for the `pkg` package manager.

pub mod capability;
pub mod constraint;
pub mod contracts;
pub mod installed;
pub mod integration;
pub mod package;
pub mod plan;
pub mod relocation;
pub mod version;

pub use capability::{Capability, Dependency};
pub use constraint::{CapabilityConstraint, Constraint, VersionConstraint, VersionOp};
pub use contracts::{
    ActivationGeneration, AdaptationPlan, ArtifactEvidence, CONTRACT_SCHEMA_VERSION,
    ElfManifestEvidence, ExecutionPlan, LaunchStrategy, ManifestEntry, PayloadManifest,
    ProviderEvidence, RuntimeManifest, TransactionReceipt, TrustEvidence,
};
pub use installed::InstalledPackage;
pub use integration::{IntegrationAction, IntegrationConflict, IntegrationKind, IntegrationPlan};
pub use package::{
    Architecture, ArtifactDigest, LifecycleScript, NormalizedPackage, PackageEntry, PackageFormat,
    PackageName, PackageVersion,
};
pub use plan::{BinaryActivation, InstallPlan, RemovePlan};
pub use relocation::{
    PackageFhsIndex, RelocationReport, relocate_extracted_text_files,
    relocate_extracted_text_files_with_report, relocate_text_content,
};
pub use version::{VersionEcosystem, compare_versions};
