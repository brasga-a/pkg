//! `pkg-core`: Foundation and shared domain/application types for the `pkg` package manager.

pub mod acquisition;
pub mod activation;
pub mod doctor;
pub mod domain;
pub mod engine;
pub mod error;
pub mod format;
pub mod gc;
pub mod host;
pub mod lock;
pub mod planner;
pub mod repository;
pub mod resolver;
pub mod runtime;
pub mod state;
pub mod store;
pub mod transaction;
pub mod transport;

pub use acquisition::{
    ArtifactAcquisitionResult, ArtifactAcquisitionSource, ArtifactAcquisitionSpec, acquire_artifact,
};
pub use doctor::{DoctorFinding, DoctorReport, FindingLevel};
pub use engine::{
    Engine, InstallOptions, PackageInfo, PreflightReport, RemoteResolution, UpgradeCandidate,
};
pub use error::{Error, Result};
pub use gc::{GarbageCollector, GcCandidate, GcReport};
pub use resolver::{ResolutionError, ResolutionPlan, Resolver};
pub use store::StoreLayout;

/// Returns the library core version.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_version() {
        assert!(!version().is_empty());
    }
}
