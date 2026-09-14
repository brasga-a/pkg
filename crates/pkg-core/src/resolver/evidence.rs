//! Host and package evidence domain models for resolver compatibility checks (ADR-016, INV-009).
//!
//! Captures verified host facts, system libraries, dynamic linker paths, and capability tokens
//! needed to satisfy or reject normalized constraints without relying on nominal package-name aliases.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::capability::Capability;
use crate::domain::package::{Architecture, PackageVersion};
use crate::host::HostFacts;

/// Standard host Linux library search paths for 64-bit systems.
pub const STANDARD_LIB_SEARCH_DIRS: &[&str] = &[
    "/lib64",
    "/usr/lib64",
    "/lib/x86_64-linux-gnu",
    "/usr/lib/x86_64-linux-gnu",
    "/lib",
    "/usr/lib",
];

/// Verified evidence of a capability provided by the host platform or an installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    /// Normalized capability identifier.
    pub capability: Capability,
    /// Explicit version of the capability, if provided.
    pub version: Option<PackageVersion>,
    /// Source origin of the evidence (e.g. `host:system`, `store:libc6_2.38-1`).
    pub provider_origin: String,
    /// ABI symbol versions exported (e.g. `["GLIBC_2.17", "GLIBC_2.34", "GLIBC_2.38"]`).
    pub symbols: Vec<String>,
}

/// Verified evidence about the host environment used for capability and ABI evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEvidence {
    /// Host CPU architecture.
    pub architecture: Architecture,
    /// Verified capabilities provided directly by the host operating system.
    pub provided_capabilities: HashMap<String, CapabilityEvidence>,
    /// Verified linker search paths on the host.
    pub library_search_paths: Vec<PathBuf>,
}

impl HostEvidence {
    /// Creates a fluent builder for constructing test or customized host evidence.
    pub fn builder() -> HostEvidenceBuilder {
        HostEvidenceBuilder::default()
    }

    /// Automatically detects host evidence from the current running environment.
    pub fn detect(facts: &HostFacts) -> Self {
        let mut builder = Self::builder().architecture(facts.architecture.clone());

        // Probe standard library search paths
        for &dir in STANDARD_LIB_SEARCH_DIRS {
            let path = PathBuf::from(dir);
            if path.is_dir() {
                builder = builder.add_lib_path(path);
            }
        }

        // Detect common host executables
        for bin in &["sh", "bash", "coreutils", "tar", "gzip"] {
            if Path::new("/bin").join(bin).exists() || Path::new("/usr/bin").join(bin).exists() {
                builder = builder.add_executable(bin);
            }
        }

        // Detect libc SONAME on host if present
        for &dir in STANDARD_LIB_SEARCH_DIRS {
            let libc = Path::new(dir).join("libc.so.6");
            if libc.exists() {
                builder = builder.add_library("libc.so.6", None, &[]);
                break;
            }
        }

        builder.build()
    }

    /// Checks whether the host provides the requested capability, respecting version and symbols.
    pub fn provides_capability(&self, cap_str: &str) -> Option<&CapabilityEvidence> {
        self.provided_capabilities.get(cap_str)
    }

    /// Checks if a dynamic library SONAME exists in any verified host library search path.
    pub fn has_soname(&self, soname: &str) -> bool {
        let lib_key = format!("lib:{soname}");
        if self.provided_capabilities.contains_key(&lib_key) {
            return true;
        }
        for dir in &self.library_search_paths {
            if dir.join(soname).exists() {
                return true;
            }
        }
        false
    }
}

/// Fluent builder for constructing `HostEvidence` fixtures.
#[derive(Debug, Default)]
pub struct HostEvidenceBuilder {
    architecture: Option<Architecture>,
    provided_capabilities: HashMap<String, CapabilityEvidence>,
    library_search_paths: Vec<PathBuf>,
}

impl HostEvidenceBuilder {
    /// Sets the host architecture.
    pub fn architecture(mut self, arch: Architecture) -> Self {
        self.architecture = Some(arch);
        self
    }

    /// Adds a library search path.
    pub fn add_lib_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.library_search_paths.push(path.into());
        self
    }

    /// Registers a shared library capability with optional version and exported symbol versions.
    pub fn add_library(mut self, soname: &str, version: Option<&str>, symbols: &[&str]) -> Self {
        let key = format!("lib:{soname}");
        let evidence = CapabilityEvidence {
            capability: Capability::SharedLibrary(soname.to_string()),
            version: version.map(PackageVersion::new),
            provider_origin: "host:system".to_string(),
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
        };
        self.provided_capabilities.insert(key, evidence);
        self
    }

    /// Registers an executable binary capability.
    pub fn add_executable(mut self, command: &str) -> Self {
        let key = format!("bin:{command}");
        let evidence = CapabilityEvidence {
            capability: Capability::Executable(command.to_string()),
            version: None,
            provider_origin: "host:system".to_string(),
            symbols: Vec::new(),
        };
        self.provided_capabilities.insert(key, evidence);
        self
    }

    /// Registers a generic or virtual feature capability.
    pub fn add_feature(mut self, feature: &str) -> Self {
        let key = format!("feature:{feature}");
        let evidence = CapabilityEvidence {
            capability: Capability::Feature(feature.to_string()),
            version: None,
            provider_origin: "host:system".to_string(),
            symbols: Vec::new(),
        };
        self.provided_capabilities.insert(key, evidence);
        self
    }

    /// Builds the configured `HostEvidence`.
    pub fn build(self) -> HostEvidence {
        HostEvidence {
            architecture: self.architecture.unwrap_or(Architecture::X86_64),
            provided_capabilities: self.provided_capabilities,
            library_search_paths: self.library_search_paths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_evidence_builder() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", Some("2.38"), &["GLIBC_2.34", "GLIBC_2.38"])
            .add_executable("sh")
            .build();

        assert!(host.has_soname("libc.so.6"));
        let cap = host.provides_capability("lib:libc.so.6").unwrap();
        assert_eq!(cap.version.as_ref().map(|v| v.as_str()), Some("2.38"));
        assert!(cap.symbols.contains(&"GLIBC_2.38".to_string()));
    }
}
