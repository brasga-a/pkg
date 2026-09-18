//! Typed, reversible user-space host integration actions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::package::ArtifactDigest;

/// A host-visible integration class supported by the rootless policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    /// A desktop entry exposed through the user's applications directory.
    DesktopEntry,
    /// An icon exposed through the user's icon theme directory.
    Icon,
    /// A MIME package description exposed through the user's MIME directory.
    MimePackage,
}

impl IntegrationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DesktopEntry => "desktop_entry",
            Self::Icon => "icon",
            Self::MimePackage => "mime_package",
        }
    }

    pub fn parse(value: &str) -> crate::error::Result<Self> {
        match value {
            "desktop_entry" => Ok(Self::DesktopEntry),
            "icon" => Ok(Self::Icon),
            "mime_package" => Ok(Self::MimePackage),
            other => Err(crate::error::Error::Internal(format!(
                "unknown integration kind '{other}'"
            ))),
        }
    }
}

/// One planned host integration action. The source is immutable pkg-owned
/// content; the target is a user-space path that can be removed by ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationAction {
    pub kind: IntegrationKind,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub source_digest: ArtifactDigest,
}

/// A deterministic integration plan for one installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationPlan {
    pub profile: String,
    pub package_name: String,
    pub store_id: String,
    pub actions: Vec<IntegrationAction>,
    pub conflicts: Vec<IntegrationConflict>,
}

/// A target that is already occupied by content not owned by this package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationConflict {
    pub kind: IntegrationKind,
    pub target_path: PathBuf,
    pub reason: String,
}
