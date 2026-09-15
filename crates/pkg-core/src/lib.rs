//! `pkg-core`: Foundation and shared domain/application types for the `pkg` package manager.

pub mod activation;
pub mod domain;
pub mod engine;
pub mod error;
pub mod format;
pub mod host;
pub mod lock;
pub mod planner;
pub mod repository;
pub mod resolver;
pub mod state;
pub mod store;
pub mod transaction;
pub mod transport;

pub use engine::{Engine, PackageInfo, RemoteResolution};
pub use error::{Error, Result};
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
