use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const FALLBACK_REGISTRY_JSON: &str = r#"{
  "schema_version": "1.0",
  "updated_at": "2026-09-18T22:00:00Z",
  "repositories": [
    {
      "id": "arch-core",
      "name": "Arch Linux Core",
      "distro": "arch",
      "format": "alpm",
      "url": "https://geo.mirror.pkgbuild.com",
      "distribution": "core",
      "components": [],
      "priority": 100,
      "description": "Core packages for Arch Linux",
      "default_for": ["arch"]
    },
    {
      "id": "arch-extra",
      "name": "Arch Linux Extra",
      "distro": "arch",
      "format": "alpm",
      "url": "https://geo.mirror.pkgbuild.com",
      "distribution": "extra",
      "components": [],
      "priority": 90,
      "description": "Extra packages for Arch Linux",
      "default_for": ["arch"]
    },
    {
      "id": "arch-multilib",
      "name": "Arch Linux Multilib",
      "distro": "arch",
      "format": "alpm",
      "url": "https://geo.mirror.pkgbuild.com",
      "distribution": "multilib",
      "components": [],
      "priority": 80,
      "description": "32-bit applications and libraries on 64-bit Arch Linux",
      "default_for": []
    },
    {
      "id": "ubuntu-noble",
      "name": "Ubuntu 24.04 LTS (Noble Numbat)",
      "distro": "ubuntu",
      "format": "deb",
      "url": "http://archive.ubuntu.com/ubuntu",
      "distribution": "noble",
      "components": ["main", "universe", "restricted", "multiverse"],
      "priority": 100,
      "description": "Ubuntu 24.04 LTS official repository",
      "default_for": ["ubuntu:24.04", "ubuntu:noble"]
    },
    {
      "id": "ubuntu-resolute",
      "name": "Ubuntu 26.04 LTS (Resolute Raccoon)",
      "distro": "ubuntu",
      "format": "deb",
      "url": "http://archive.ubuntu.com/ubuntu",
      "distribution": "resolute",
      "components": ["main", "universe", "restricted", "multiverse"],
      "priority": 90,
      "description": "Ubuntu 26.04 LTS official repository",
      "default_for": ["ubuntu:26.04", "ubuntu:resolute"]
    },
    {
      "id": "ubuntu-jammy",
      "name": "Ubuntu 22.04 LTS (Jammy Jellyfish)",
      "distro": "ubuntu",
      "format": "deb",
      "url": "http://archive.ubuntu.com/ubuntu",
      "distribution": "jammy",
      "components": ["main", "universe", "restricted", "multiverse"],
      "priority": 80,
      "description": "Ubuntu 22.04 LTS official repository",
      "default_for": ["ubuntu:22.04", "ubuntu:jammy"]
    },
    {
      "id": "debian-bookworm",
      "name": "Debian 12 (Bookworm)",
      "distro": "debian",
      "format": "deb",
      "url": "http://deb.debian.org/debian",
      "distribution": "bookworm",
      "components": ["main", "contrib", "non-free"],
      "priority": 100,
      "description": "Debian 12 official repository",
      "default_for": ["debian:12", "debian:bookworm"]
    },
    {
      "id": "debian-trixie",
      "name": "Debian 13 (Trixie)",
      "distro": "debian",
      "format": "deb",
      "url": "http://deb.debian.org/debian",
      "distribution": "trixie",
      "components": ["main", "contrib", "non-free"],
      "priority": 90,
      "description": "Debian 13 official repository",
      "default_for": ["debian:13", "debian:trixie"]
    },
    {
      "id": "fedora-41",
      "name": "Fedora 41",
      "distro": "fedora",
      "format": "rpm",
      "url": "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/41/Everything/x86_64/os",
      "distribution": "41",
      "components": [],
      "priority": 100,
      "description": "Fedora 41 official repository",
      "default_for": ["fedora:41"]
    },
    {
      "id": "fedora-42",
      "name": "Fedora 42 (Rawhide)",
      "distro": "fedora",
      "format": "rpm",
      "url": "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/42/Everything/x86_64/os",
      "distribution": "42",
      "components": [],
      "priority": 90,
      "description": "Fedora 42 official repository",
      "default_for": ["fedora:42", "fedora:rawhide"]
    }
  ]
}"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CuratedRepository {
    pub id: String,
    pub name: String,
    pub distro: String,
    #[serde(default = "default_repo_format")]
    pub format: String,
    pub url: String,
    pub distribution: String,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub public_key_path: Option<PathBuf>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CuratedRegistry {
    pub schema_version: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub repositories: Vec<CuratedRepository>,
}

impl CuratedRegistry {
    /// Loads the embedded fallback registry without network calls.
    #[must_use]
    pub fn fallback() -> Self {
        serde_json::from_str(FALLBACK_REGISTRY_JSON).expect("embedded registry is valid json")
    }

    /// Fetches the curated registry from `pkg.atlantic.sh/repositories` with fallback.
    pub async fn fetch_remote_or_fallback() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(4))
            .build();
        if let Ok(client) = client {
            if let Ok(resp) = client
                .get("https://pkg.atlantic.sh/repositories")
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Ok(registry) = serde_json::from_slice::<CuratedRegistry>(&bytes) {
                            return registry;
                        }
                    }
                }
            }
        }
        Self::fallback()
    }

    /// Finds a repository by ID (case-insensitive).
    #[must_use]
    pub fn find_by_id(&self, id: &str) -> Option<&CuratedRepository> {
        self.repositories
            .iter()
            .find(|r| r.id.eq_ignore_ascii_case(id))
    }

    /// Selects the repositories that should be enabled by default for a host distribution.
    #[must_use]
    pub fn default_for_distro(
        &self,
        distro_id: &str,
        codename: Option<&str>,
        version_id: Option<&str>,
    ) -> Vec<CuratedRepository> {
        let distro = distro_id.to_lowercase();
        let codename = codename.map(|s| s.to_lowercase());
        let version_id = version_id.map(|s| s.to_lowercase());

        // 1. Try exact matches in default_for (e.g. "ubuntu:resolute", "ubuntu:26.04", "arch")
        let mut matched = Vec::new();
        for repo in &self.repositories {
            let matches_tag = repo.default_for.iter().any(|tag| {
                let tag_lower = tag.to_lowercase();
                if let Some(ref c) = codename {
                    if tag_lower == format!("{distro}:{c}") {
                        return true;
                    }
                }
                if let Some(ref v) = version_id {
                    if tag_lower == format!("{distro}:{v}") {
                        return true;
                    }
                }
                tag_lower == distro
            });

            if matches_tag {
                matched.push(repo.clone());
            }
        }

        if !matched.is_empty() {
            return matched;
        }

        // 2. Fallback heuristic by distro family
        if distro.contains("arch") || distro.contains("manjaro") || distro.contains("endeavour") {
            return self
                .repositories
                .iter()
                .filter(|r| r.id == "arch-core" || r.id == "arch-extra")
                .cloned()
                .collect();
        }

        if distro.contains("ubuntu") || distro.contains("pop") || distro.contains("mint") {
            return self
                .repositories
                .iter()
                .filter(|r| r.id == "ubuntu-noble")
                .cloned()
                .collect();
        }

        if distro.contains("debian") {
            return self
                .repositories
                .iter()
                .filter(|r| r.id == "debian-bookworm")
                .cloned()
                .collect();
        }

        if distro.contains("fedora") || distro.contains("rhel") || distro.contains("centos") {
            return self
                .repositories
                .iter()
                .filter(|r| r.id == "fedora-41")
                .cloned()
                .collect();
        }

        // 3. Ultimate fallback: Arch Linux core + extra
        self.repositories
            .iter()
            .filter(|r| r.id == "arch-core" || r.id == "arch-extra")
            .cloned()
            .collect()
    }
}

impl From<CuratedRepository> for RepositoryConfig {
    fn from(curated: CuratedRepository) -> Self {
        RepositoryConfig {
            id: curated.id,
            format: curated.format,
            url: curated.url,
            distribution: curated.distribution,
            components: curated.components,
            public_key_path: curated.public_key_path,
            priority: curated.priority,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoriesConfig {
    #[serde(rename = "repository", default)]
    pub repositories: Vec<RepositoryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Builds the default repository configuration tailored for the current host machine.
    #[must_use]
    pub fn default_for_host() -> Self {
        let registry = CuratedRegistry::fallback();
        Self::default_for_host_with_registry(&registry)
    }

    /// Builds the default repository configuration for the current host machine using the provided registry.
    #[must_use]
    pub fn default_for_host_with_registry(registry: &CuratedRegistry) -> Self {
        let (distro_id, distro_id_like) = crate::host::HostFacts::detect_distro();
        let metadata = crate::host::HostFacts::release_metadata();
        let codename = metadata
            .get("os_release_VERSION_CODENAME")
            .map(|s| s.as_str());
        let version_id = metadata.get("os_release_VERSION_ID").map(|s| s.as_str());

        let mut matched = if let Some(ref d) = distro_id {
            registry.default_for_distro(d, codename, version_id)
        } else {
            Vec::new()
        };

        if matched.is_empty() {
            if let Some(ref dl) = distro_id_like {
                matched = registry.default_for_distro(dl, codename, version_id);
            }
        }

        if matched.is_empty() {
            matched = registry.default_for_distro("", codename, version_id);
        }

        let repos = matched.into_iter().map(RepositoryConfig::from).collect();
        RepositoriesConfig {
            repositories: repos,
        }
    }

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

impl Default for RepositoriesConfig {
    fn default() -> Self {
        Self::default_for_host()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_registry_deserialization() {
        let registry = CuratedRegistry::fallback();
        assert_eq!(registry.schema_version, "1.0");
        assert!(!registry.repositories.is_empty());
        assert!(registry.find_by_id("arch-core").is_some());
        assert!(registry.find_by_id("ubuntu-noble").is_some());
    }

    #[test]
    fn test_distro_matching_arch() {
        let registry = CuratedRegistry::fallback();
        let repos = registry.default_for_distro("arch", None, None);
        let ids: Vec<&str> = repos.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["arch-core", "arch-extra"]);
    }

    #[test]
    fn test_distro_matching_ubuntu() {
        let registry = CuratedRegistry::fallback();
        let repos = registry.default_for_distro("ubuntu", Some("noble"), Some("24.04"));
        let ids: Vec<&str> = repos.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["ubuntu-noble"]);

        let repos2 = registry.default_for_distro("ubuntu", Some("resolute"), Some("26.04"));
        let ids2: Vec<&str> = repos2.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids2, vec!["ubuntu-resolute"]);
    }

    #[test]
    fn test_distro_matching_fedora() {
        let registry = CuratedRegistry::fallback();
        let repos = registry.default_for_distro("fedora", None, Some("41"));
        let ids: Vec<&str> = repos.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["fedora-41"]);
    }

    #[test]
    fn test_default_config_to_toml() {
        let registry = CuratedRegistry::fallback();
        let repos = registry.default_for_distro("arch", None, None);
        let config = RepositoriesConfig {
            repositories: repos.into_iter().map(RepositoryConfig::from).collect(),
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("[[repository]]"));
        assert!(toml_str.contains("id = \"arch-core\""));
    }
}
