//! Capabilities and dependencies domain models.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A capability exposed or required by a package.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// An executable binary exposed to the profile (e.g. `rg`).
    Executable(String),
    /// A shared library SONAME provided (e.g. `libz.so.1`).
    SharedLibrary(String),
    /// A virtual feature or token provided.
    Feature(String),
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executable(name) => write!(f, "bin:{name}"),
            Self::SharedLibrary(name) => write!(f, "lib:{name}"),
            Self::Feature(name) => write!(f, "feature:{name}"),
        }
    }
}

/// A package dependency requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dependency {
    /// Raw unparsed dependency expression from source package (e.g. `libc6 (>= 2.34)`).
    pub raw: String,
    /// Package or capability name.
    pub name: String,
    /// Optional version constraint (e.g. `>= 2.34`).
    pub version_constraint: Option<String>,
    /// Source ecosystem (e.g. `debian`).
    pub ecosystem: String,
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref ver) = self.version_constraint {
            write!(f, "{} ({})", self.name, ver)
        } else {
            write!(f, "{}", self.name)
        }
    }
}
