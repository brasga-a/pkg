use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoriesConfig {
    #[serde(rename = "repository", default)]
    pub repositories: Vec<RepositoryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryConfig {
    /// Unique identifier for this repository (e.g., "debian-bookworm-main")
    pub id: String,

    /// Repository format/ecosystem ("deb", "rpm", "alpm"). Defaults to "deb".
    #[serde(default = "default_repo_format")]
    pub format: String,

    /// Base URL of the repository (e.g., "http://deb.debian.org/debian")
    pub url: String,

    /// The distribution suite (e.g., "bookworm", "stable")
    pub distribution: String,

    /// List of components to fetch (e.g., ["main", "contrib", "non-free"])
    #[serde(default)]
    pub components: Vec<String>,

    /// Path to a GPG public key or keyring to verify InRelease
    pub public_key_path: Option<PathBuf>,

    /// Optional repository priority for resolving ambiguous packages (higher = preferred).
    #[serde(default)]
    pub priority: Option<u32>,
}

fn default_repo_format() -> String {
    "deb".to_string()
}

impl RepositoriesConfig {
    /// Loads the repository configuration from the specified TOML file.
    pub fn load_from_file(path: &std::path::Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "Failed to read repositories config {}: {}",
                path.display(),
                e
            )))
        })?;

        let config: RepositoriesConfig = toml::from_str(&content).map_err(|e| {
            Error::Parse(format!(
                "Failed to parse repositories TOML {}: {}",
                path.display(),
                e
            ))
        })?;

        // Basic validation
        for repo in &config.repositories {
            if repo.id.trim().is_empty() {
                return Err(Error::Parse("Repository ID cannot be empty".to_string()));
            }
            if repo.url.trim().is_empty() {
                return Err(Error::Parse(format!(
                    "Repository '{}' has empty URL",
                    repo.id
                )));
            }
        }

        Ok(config)
    }
}
