//! Host environment facts and architecture detection.

pub mod elf;
pub mod integration;

use crate::domain::package::Architecture;

/// Host platform facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFacts {
    /// Operating system name (e.g. "linux").
    pub os: String,
    /// Host CPU architecture.
    pub architecture: Architecture,
    /// Distribution identifier from /etc/os-release ID (e.g. "ubuntu", "debian", "arch", "fedora").
    pub distro_id: Option<String>,
    /// Distribution family alias from /etc/os-release ID_LIKE (e.g. "debian" for Ubuntu).
    pub distro_id_like: Option<String>,
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
        let (distro_id, distro_id_like) = Self::detect_distro();
        Self {
            os,
            architecture: arch,
            distro_id,
            distro_id_like,
        }
    }

    /// Helper to parse distribution identifiers from standard os-release files.
    #[must_use]
    pub fn detect_distro() -> (Option<String>, Option<String>) {
        Self::parse_os_release_paths(&["/etc/os-release", "/usr/lib/os-release"])
    }

    /// Internal parser for os-release content across candidate file paths.
    #[must_use]
    pub fn parse_os_release_paths(paths: &[&str]) -> (Option<String>, Option<String>) {
        for path in paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                let res = Self::parse_os_release_str(&content);
                if res.0.is_some() || res.1.is_some() {
                    return res;
                }
            }
        }
        (None, None)
    }

    /// Parses os-release formatted text into (ID, ID_LIKE).
    #[must_use]
    pub fn parse_os_release_str(content: &str) -> (Option<String>, Option<String>) {
        let values = Self::parse_os_release_values(content);
        (values.get("ID").cloned(), values.get("ID_LIKE").cloned())
    }

    /// Parses all stable fields from os-release.  Keeping this separate from
    /// the historical `(ID, ID_LIKE)` helper lets runtime manifests retain
    /// release evidence without changing callers that construct `HostFacts`.
    #[must_use]
    pub fn parse_os_release_values(content: &str) -> std::collections::BTreeMap<String, String> {
        let mut values = std::collections::BTreeMap::new();
        for line in content.lines() {
            let line = line.trim();
            let Some((key, raw)) = line.split_once('=') else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            let value = raw.trim_matches('"').trim_matches('\'').to_string();
            if !value.is_empty() {
                values.insert(key.to_string(), value.to_lowercase());
            }
        }
        values
    }

    /// Returns release metadata suitable for persisting in a runtime
    /// manifest, including ID, ID_LIKE, VERSION_ID and VERSION_CODENAME.
    #[must_use]
    pub fn release_metadata() -> std::collections::BTreeMap<String, String> {
        for path in ["/etc/os-release", "/usr/lib/os-release"] {
            if let Ok(content) = std::fs::read_to_string(path) {
                let values = Self::parse_os_release_values(&content);
                if !values.is_empty() {
                    return values
                        .into_iter()
                        .filter(|(key, _)| {
                            matches!(
                                key.as_str(),
                                "ID" | "ID_LIKE" | "VERSION_ID" | "VERSION_CODENAME"
                            )
                        })
                        .map(|(key, value)| (format!("os_release_{key}"), value))
                        .collect();
                }
            }
        }
        std::collections::BTreeMap::new()
    }

    /// Detects the host libc and dynamic loader from the running executable
    /// without invoking any payload or external command.
    #[must_use]
    pub fn loader_metadata() -> std::collections::BTreeMap<String, String> {
        let mut values = std::collections::BTreeMap::new();
        let loader = [
            "/lib64/ld-linux-x86-64.so.2",
            "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
            "/lib/ld-linux-aarch64.so.1",
            "/lib/ld-musl-x86_64.so.1",
        ]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file());
        if let Some(loader) = loader {
            values.insert("loader_path".into(), loader.into());
            values.insert(
                "libc".into(),
                if loader.contains("musl") {
                    "musl".into()
                } else {
                    "glibc".into()
                },
            );
        }
        values
    }

    /// Returns default repository configuration for the host system using the curated registry.
    #[must_use]
    pub fn default_repositories_config() -> crate::repository::RepositoriesConfig {
        crate::repository::RepositoriesConfig::default_for_host()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_os_release_ubuntu() {
        let sample = r#"
NAME="Ubuntu"
VERSION="26.04 LTS"
ID=ubuntu
ID_LIKE=debian
"#;
        let (id, id_like) = HostFacts::parse_os_release_str(sample);
        assert_eq!(id.as_deref(), Some("ubuntu"));
        assert_eq!(id_like.as_deref(), Some("debian"));
    }

    #[test]
    fn test_parse_os_release_arch() {
        let sample = r#"
NAME="Arch Linux"
ID=arch
"#;
        let (id, id_like) = HostFacts::parse_os_release_str(sample);
        assert_eq!(id.as_deref(), Some("arch"));
        assert_eq!(id_like, None);
    }
}
