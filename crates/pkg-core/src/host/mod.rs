//! Host environment facts and architecture detection.

pub mod elf;

use crate::domain::package::Architecture;

/// Host platform facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFacts {
    /// Operating system name (e.g. "linux").
    pub os: String,
    /// Host CPU architecture.
    pub architecture: Architecture,
}

impl HostFacts {
    /// Detects current host environment facts.
    #[must_use]
    pub fn detect() -> Self {
        let os = std::env::consts::OS.to_string();
        let arch = match std::env::consts::ARCH {
            "x86_64" => Architecture::X86_64,
            other => Architecture::Other(other.to_string()),
        };
        Self {
            os,
            architecture: arch,
        }
    }
}
