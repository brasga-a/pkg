//! Text relocation module for FHS paths in extracted package payloads.
//!
//! Relocates hardcoded FHS paths (`/usr/share/`, `/usr/lib/`, `/opt/`, `/etc/`,
//! `/usr/bin/`, etc.) in text files (scripts, `.desktop`, `.service`, configs)
//! so that packages installed into isolated store directories resolve their
//! bundled resources correctly without modifying host system paths.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Index of relative FHS components extracted in the package payload.
#[derive(Debug, Default, Clone)]
pub struct PackageFhsIndex {
    /// Top-level entries under `usr/share/` (e.g. `discord`, `applications`, `icons`)
    pub usr_share: BTreeSet<String>,
    /// Top-level entries under `usr/lib/`
    pub usr_lib: BTreeSet<String>,
    /// Top-level entries under `usr/lib64/`
    pub usr_lib64: BTreeSet<String>,
    /// Top-level entries under `usr/libexec/`
    pub usr_libexec: BTreeSet<String>,
    /// Top-level entries under `usr/include/`
    pub usr_include: BTreeSet<String>,
    /// Top-level entries under `usr/bin/` (e.g. `discord`, `reflector`)
    pub usr_bin: BTreeSet<String>,
    /// Top-level entries under `usr/sbin/`
    pub usr_sbin: BTreeSet<String>,
    /// Top-level entries under `opt/` (e.g. `discord`, `google`)
    pub opt: BTreeSet<String>,
    /// Top-level entries under `etc/` (e.g. `xdg`, `libvirt`, `default`)
    pub etc: BTreeSet<String>,
    /// Top-level entries under `bin/`
    pub bin: BTreeSet<String>,
    /// Top-level entries under `sbin/`
    pub sbin: BTreeSet<String>,
}

impl PackageFhsIndex {
    /// Builds a `PackageFhsIndex` by inspecting the list of extracted package relative paths.
    #[must_use]
    pub fn from_extracted_files(extracted_files: &[PathBuf]) -> Self {
        let mut index = Self::default();

        for path in extracted_files {
            let normalized = path.to_string_lossy().replace('\\', "/");
            let s = normalized.trim_start_matches("./").trim_start_matches('/');

            if let Some(rest) = s.strip_prefix("usr/share/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_share.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("usr/lib/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_lib.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("usr/lib64/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_lib64.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("usr/libexec/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_libexec.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("usr/include/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_include.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("usr/bin/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_bin.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("usr/sbin/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.usr_sbin.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("opt/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.opt.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("etc/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.etc.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("bin/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.bin.insert(comp.to_string());
                    }
                }
            } else if let Some(rest) = s.strip_prefix("sbin/") {
                if let Some(comp) = rest.split('/').next() {
                    if !comp.is_empty() {
                        index.sbin.insert(comp.to_string());
                    }
                }
            }
        }

        index
    }
}

#[derive(Debug)]
struct PrefixRule<'a> {
    prefix_with_slash: &'static str,
    prefix_no_slash: &'static str,
    target_rel: &'static str,
    known: &'a BTreeSet<String>,
    allow_dollar: bool,
    allow_bare_dir: bool,
}

/// Relocates hardcoded FHS paths inside a text string if any matches are found.
///
/// Returns `Some(relocated_text)` if modifications occurred, or `None` if the text was unchanged.
#[must_use]
pub fn relocate_text_content(
    content: &str,
    target_store_dir: &Path,
    index: &PackageFhsIndex,
) -> Option<String> {
    let target_store_str = target_store_dir.to_string_lossy();
    let target_store_base = target_store_str.trim_end_matches('/');

    let rules = [
        PrefixRule {
            prefix_with_slash: "/usr/libexec/",
            prefix_no_slash: "/usr/libexec",
            target_rel: "usr/libexec/",
            known: &index.usr_libexec,
            allow_dollar: true,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/usr/lib64/",
            prefix_no_slash: "/usr/lib64",
            target_rel: "usr/lib64/",
            known: &index.usr_lib64,
            allow_dollar: true,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/usr/share/",
            prefix_no_slash: "/usr/share",
            target_rel: "usr/share/",
            known: &index.usr_share,
            allow_dollar: true,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/usr/include/",
            prefix_no_slash: "/usr/include",
            target_rel: "usr/include/",
            known: &index.usr_include,
            allow_dollar: false,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/usr/lib/",
            prefix_no_slash: "/usr/lib",
            target_rel: "usr/lib/",
            known: &index.usr_lib,
            allow_dollar: true,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/usr/bin/",
            prefix_no_slash: "/usr/bin",
            target_rel: "usr/bin/",
            known: &index.usr_bin,
            allow_dollar: false,
            allow_bare_dir: false,
        },
        PrefixRule {
            prefix_with_slash: "/usr/sbin/",
            prefix_no_slash: "/usr/sbin",
            target_rel: "usr/sbin/",
            known: &index.usr_sbin,
            allow_dollar: false,
            allow_bare_dir: false,
        },
        PrefixRule {
            prefix_with_slash: "/opt/",
            prefix_no_slash: "/opt",
            target_rel: "opt/",
            known: &index.opt,
            allow_dollar: true,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/etc/",
            prefix_no_slash: "/etc",
            target_rel: "etc/",
            known: &index.etc,
            allow_dollar: true,
            allow_bare_dir: true,
        },
        PrefixRule {
            prefix_with_slash: "/sbin/",
            prefix_no_slash: "/sbin",
            target_rel: "sbin/",
            known: &index.sbin,
            allow_dollar: false,
            allow_bare_dir: false,
        },
        PrefixRule {
            prefix_with_slash: "/bin/",
            prefix_no_slash: "/bin",
            target_rel: "bin/",
            known: &index.bin,
            allow_dollar: false,
            allow_bare_dir: false,
        },
    ];

    let mut result = String::with_capacity(content.len());
    let mut modified = false;

    for (line_idx, line) in content.split_inclusive('\n').enumerate() {
        // Preserve shebang line on line 0
        if line_idx == 0 && line.starts_with("#!") {
            result.push_str(line);
            continue;
        }

        let relocated_line = relocate_line(line, target_store_base, &rules);
        if relocated_line != line {
            modified = true;
        }
        result.push_str(&relocated_line);
    }

    if modified { Some(result) } else { None }
}

fn relocate_line(line: &str, target_store_base: &str, rules: &[PrefixRule<'_>]) -> String {
    let mut out = String::with_capacity(line.len() + 64);
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Skip if already relocated with target_store_base
        if out.ends_with(target_store_base) {
            let ch = line[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let mut matched = false;

        for rule in rules {
            // Case 1: Check prefix with trailing slash (e.g. "/usr/share/")
            if line[i..].starts_with(rule.prefix_with_slash) {
                let after = &line[i + rule.prefix_with_slash.len()..];
                let is_variable =
                    rule.allow_dollar && !rule.known.is_empty() && after.starts_with('$');

                let comp = extract_path_component(after);
                let is_known = !comp.is_empty() && rule.known.contains(comp);

                if is_variable || is_known {
                    out.push_str(target_store_base);
                    out.push('/');
                    out.push_str(rule.target_rel);
                    i += rule.prefix_with_slash.len();
                    matched = true;
                    break;
                }
            }

            // Case 2: Check prefix without trailing slash when followed by delimiter (e.g. "/usr/share"")
            if rule.allow_bare_dir && line[i..].starts_with(rule.prefix_no_slash) {
                let after = &line[i + rule.prefix_no_slash.len()..];
                let next_char = after.chars().next();
                let is_delimiter = matches!(
                    next_char,
                    Some('"' | '\'' | ' ' | '\t' | ';' | ')' | ',' | '\r' | '\n') | None
                );

                if is_delimiter && !rule.known.is_empty() {
                    out.push_str(target_store_base);
                    out.push('/');
                    out.push_str(rule.target_rel.trim_end_matches('/'));
                    i += rule.prefix_no_slash.len();
                    matched = true;
                    break;
                }
            }
        }

        if !matched {
            let ch = line[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    out
}

fn extract_path_component(s: &str) -> &str {
    let mut end = s.len();
    for (idx, ch) in s.char_indices() {
        if matches!(
            ch,
            '/' | '"'
                | '\''
                | ' '
                | '\t'
                | ':'
                | ';'
                | ','
                | ')'
                | ']'
                | '}'
                | '>'
                | '<'
                | '='
                | '|'
                | '&'
                | '\r'
                | '\n'
        ) {
            end = idx;
            break;
        }
    }
    &s[..end]
}

/// Scans extracted files in `staging_dir` and performs text relocation on applicable text files.
///
/// Returns the number of files modified.
pub fn relocate_extracted_text_files(
    staging_dir: &Path,
    target_store_dir: &Path,
    extracted_files: &[PathBuf],
) -> Result<usize> {
    let index = PackageFhsIndex::from_extracted_files(extracted_files);
    let mut modified_count = 0;

    for rel_path in extracted_files {
        let full_path = staging_dir.join(rel_path);
        let Ok(meta) = fs::symlink_metadata(&full_path) else {
            continue;
        };

        if !meta.is_file() || meta.file_type().is_symlink() {
            continue;
        }

        // Skip large files (> 10 MiB)
        if meta.len() > 10 * 1024 * 1024 {
            continue;
        }

        let Ok(bytes) = fs::read(&full_path) else {
            continue;
        };

        // Binary check: skip files with null bytes in the first 4096 bytes
        if bytes.iter().take(4096).any(|&b| b == 0) {
            continue;
        }

        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };

        if let Some(new_text) = relocate_text_content(text, target_store_dir, &index) {
            let perms = meta.permissions();
            fs::write(&full_path, new_text.as_bytes())?;
            fs::set_permissions(&full_path, perms)?;
            modified_count += 1;
        }
    }

    Ok(modified_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discord_script_relocation() {
        let extracted_files = vec![
            PathBuf::from("usr/bin/discord"),
            PathBuf::from("usr/share/discord/updater_bootstrap"),
            PathBuf::from("usr/share/applications/discord.desktop"),
        ];
        let index = PackageFhsIndex::from_extracted_files(&extracted_files);
        assert!(index.usr_bin.contains("discord"));
        assert!(index.usr_share.contains("discord"));

        let script = r#"#!/bin/sh
BOOTSTRAP_SUFFIX=discord/updater_bootstrap
bootstrap=/usr/share/$BOOTSTRAP_SUFFIX
if [ ! -x "$bootstrap" ]; then
    bootstrap=/opt/$BOOTSTRAP_SUFFIX
fi
app_dir=`"$bootstrap" "$config_home/$DIR"`
"#;

        let target_store = Path::new("/home/user/.local/share/pkg/store/abc-discord-1.0");
        let relocated =
            relocate_text_content(script, target_store, &index).expect("should relocate");

        assert!(relocated.starts_with("#!/bin/sh\n"));
        assert!(relocated.contains("bootstrap=/home/user/.local/share/pkg/store/abc-discord-1.0/usr/share/$BOOTSTRAP_SUFFIX"));
        // /opt is not in package, so it is untouched
        assert!(relocated.contains("bootstrap=/opt/$BOOTSTRAP_SUFFIX"));
    }

    #[test]
    fn test_desktop_and_service_relocation() {
        let extracted_files = vec![
            PathBuf::from("usr/bin/reflector"),
            PathBuf::from("etc/xdg/reflector/reflector.conf"),
        ];
        let index = PackageFhsIndex::from_extracted_files(&extracted_files);

        let service = r#"[Unit]
Description=Reflector
[Service]
ExecStart=/usr/bin/reflector @/etc/xdg/reflector/reflector.conf
"#;

        let target_store = Path::new("/home/user/.local/share/pkg/store/xyz-reflector-1.0");
        let relocated =
            relocate_text_content(service, target_store, &index).expect("should relocate");

        assert!(relocated.contains("ExecStart=/home/user/.local/share/pkg/store/xyz-reflector-1.0/usr/bin/reflector @/home/user/.local/share/pkg/store/xyz-reflector-1.0/etc/xdg/reflector/reflector.conf"));
    }

    #[test]
    fn test_virt_manager_python_script_relocation() {
        let extracted_files = vec![
            PathBuf::from("usr/bin/virt-manager"),
            PathBuf::from("usr/share/virt-manager/virtManager/main.py"),
        ];
        let index = PackageFhsIndex::from_extracted_files(&extracted_files);

        let script = r#"#!/usr/bin/python3
import sys
sys.path.insert(0, "/usr/share/virt-manager")
from virtManager import virtmanager
"#;

        let target_store = Path::new("/home/user/.local/share/pkg/store/123-virt-manager-1.0");
        let relocated =
            relocate_text_content(script, target_store, &index).expect("should relocate");

        // Shebang is preserved!
        assert!(relocated.starts_with("#!/usr/bin/python3\n"));
        assert!(relocated.contains(r#"sys.path.insert(0, "/home/user/.local/share/pkg/store/123-virt-manager-1.0/usr/share/virt-manager")"#));
    }

    #[test]
    fn test_host_paths_preserved() {
        let extracted_files = vec![
            PathBuf::from("usr/bin/mytool"),
            PathBuf::from("etc/mytool/config.toml"),
        ];
        let index = PackageFhsIndex::from_extracted_files(&extracted_files);

        let script = r#"#!/usr/bin/env bash
if [ -f /etc/os-release ]; then
    cat /etc/passwd
fi
export PATH=/usr/bin:$PATH
exec /usr/bin/env python3 /etc/mytool/config.toml
"#;

        let target_store = Path::new("/home/user/.local/share/pkg/store/456-mytool-1.0");
        let relocated =
            relocate_text_content(script, target_store, &index).expect("should relocate");

        // Shebang untouched
        assert!(relocated.starts_with("#!/usr/bin/env bash\n"));
        // Host /etc/os-release untouched
        assert!(relocated.contains("if [ -f /etc/os-release ]; then"));
        // Host /etc/passwd untouched
        assert!(relocated.contains("cat /etc/passwd"));
        // PATH=/usr/bin untouched
        assert!(relocated.contains("export PATH=/usr/bin:$PATH"));
        // /usr/bin/env untouched
        assert!(relocated.contains("exec /usr/bin/env python3"));
        // Package /etc/mytool/config.toml IS relocated
        assert!(
            relocated.contains(
                "/home/user/.local/share/pkg/store/456-mytool-1.0/etc/mytool/config.toml"
            )
        );
    }
}
