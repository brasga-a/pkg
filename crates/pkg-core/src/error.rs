//! Typed error definitions for `pkg-core`.

use thiserror::Error;

/// Core error types for domain, store, and package management operations.
#[derive(Debug, Error)]
pub enum Error {
    /// An I/O error occurred during an internal operation.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// SQLite state database error.
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// The package archive is malformed or cannot be parsed.
    #[error("Malformed archive: {0}")]
    MalformedArchive(String),

    /// Archive entry violates extraction security (path traversal, absolute paths, escaping symlinks).
    #[error("Security violation: archive entry rejected: {0}")]
    SecurityViolation(String),

    /// Archive extraction exceeded configured resource limits.
    #[error("Extraction limits exceeded: {0}")]
    LimitsExceeded(String),

    /// Package architecture does not match host architecture.
    #[error("Architecture mismatch: package is '{found}', host is '{expected}'")]
    ArchitectureMismatch {
        /// Expected host architecture.
        expected: String,
        /// Package's target architecture.
        found: String,
    },

    /// Package is incompatible with the host environment (e.g. missing required ELF libraries).
    #[error("Incompatible host: {0}")]
    IncompatibleHost(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Parse error: {0}")]
    Parse(String),

    /// Binary command name collision with an already active package.
    #[error(
        "Activation conflict: command '{command}' is already provided by package '{existing_package}'"
    )]
    ActivationConflict {
        /// Conflicting binary command name.
        command: String,
        /// Package currently providing the command.
        existing_package: String,
    },

    /// The requested package was not found in installed state or repositories.
    #[error("Package not found: '{0}'")]
    PackageNotFound(String),

    /// The package is already installed.
    #[error("Package '{name}' version '{version}' is already installed")]
    PackageAlreadyInstalled {
        /// Package name.
        name: String,
        /// Package version.
        version: String,
    },

    /// A lifecycle script is present and required but blocked by policy (INV-003).
    #[error("Maintainer script blocked by policy: script '{script}' ({reason})")]
    ScriptBlocked {
        /// Script name (e.g. preinst, postinst).
        script: String,
        /// Reason description.
        reason: String,
    },

    /// Failed to acquire single-writer process lock.
    #[error("Lock error: {0}")]
    LockError(String),

    /// Transaction recovery required before proceeding.
    #[error("Transaction recovery required: {0}")]
    TransactionRecoveryRequired(String),

    /// JSON serialization or deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Dependency resolution failed with human-readable explanation chain (INV-020).
    #[error("Dependency resolution failed:\n{0}")]
    ResolutionFailed(Box<crate::resolver::explanation::ExplanationChain>),

    /// A generic internal domain error.
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<crate::resolver::ResolutionError> for Error {
    fn from(err: crate::resolver::ResolutionError) -> Self {
        Self::ResolutionFailed(Box::new(err.chain))
    }
}

/// A specialized Result type for `pkg-core` operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::SecurityViolation("escapes store".to_string());
        assert_eq!(
            err.to_string(),
            "Security violation: archive entry rejected: escapes store"
        );
    }

    #[test]
    fn test_activation_conflict_display() {
        let err = Error::ActivationConflict {
            command: "ripgrep".to_string(),
            existing_package: "rg-pkg".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Activation conflict: command 'ripgrep' is already provided by package 'rg-pkg'"
        );
    }
}
