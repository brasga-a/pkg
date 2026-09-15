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
        let mut id = None;
        let mut id_like = None;
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("ID=") {
                id = Some(val.trim_matches('"').trim_matches('\'').to_lowercase());
            } else if let Some(val) = line.strip_prefix("ID_LIKE=") {
                id_like = Some(val.trim_matches('"').trim_matches('\'').to_lowercase());
            }
        }
        (id, id_like)
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
