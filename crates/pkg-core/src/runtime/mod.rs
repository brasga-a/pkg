//! Runtime adaptation, launcher generation, and scoped process environment (ADR-010, INV-006).

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::domain::plan::BinaryActivation;
use crate::error::Result;

/// Scoped environment variable mutation for a command launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedEnvVar {
    pub name: String,
    /// Store-relative paths to prepend (e.g. `usr/lib/python3.14/site-packages`).
    pub paths: Vec<PathBuf>,
}

/// Plan for generating a runtime launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLauncherPlan {
    /// Command name (e.g. `reflector`).
    pub command: String,
    /// Original script path relative to store root (e.g. `usr/bin/reflector`).
    pub target_script_rel_path: PathBuf,
    /// Absolute path to the resolved interpreter (e.g. `/usr/bin/python3`).
    pub interpreter: String,
    /// Scoped environment variables to inject.
    pub env_vars: Vec<ScopedEnvVar>,
}

/// Inspects extracted binaries and generates scoped launchers where needed.
pub fn prepare_launchers(
    staging_dir: &Path,
    binaries: &mut [BinaryActivation],
    extracted_files: &mut Vec<PathBuf>,
) -> Result<()> {
    // 1. Collect all python site-packages / dist-packages relative directory paths
    let python_module_dirs = detect_python_module_dirs(extracted_files);

    // 2. For each binary, check if runtime adaptation is required
    for binary in binaries.iter_mut() {
        let script_full_path = staging_dir.join(&binary.relative_store_path);
        if let Some(plan) = inspect_script(
            &binary.command,
            &binary.relative_store_path,
            &script_full_path,
            &python_module_dirs,
        ) {
            let launcher_dir = staging_dir.join(".pkg-launcher");
            fs::create_dir_all(&launcher_dir)?;
            let launcher_rel_path = PathBuf::from(".pkg-launcher").join(&binary.command);
            let launcher_file = staging_dir.join(&launcher_rel_path);

            let content = generate_launcher_script(&plan);
            fs::write(&launcher_file, content)?;

            let mut perms = fs::metadata(&launcher_file)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&launcher_file, perms)?;

            binary.relative_store_path = launcher_rel_path.clone();
            if !extracted_files.contains(&launcher_rel_path) {
                extracted_files.push(launcher_rel_path);
            }
        }
    }

    Ok(())
}

/// Detects any Python `site-packages` or `dist-packages` directory paths in the payload.
#[must_use]
pub fn detect_python_module_dirs(extracted_files: &[PathBuf]) -> Vec<PathBuf> {
    let mut set = BTreeSet::new();
    for file in extracted_files {
        let mut curr = file.as_path();
        while let Some(parent) = curr.parent() {
            if let Some(file_name) = parent.file_name() {
                if file_name == "site-packages" || file_name == "dist-packages" {
                    set.insert(parent.to_path_buf());
                    break;
                }
            }
            curr = parent;
        }
    }
    set.into_iter().collect()
}

/// Inspects a file to determine if it requires a runtime launcher.
#[must_use]
pub fn inspect_script(
    command: &str,
    relative_store_path: &Path,
    full_path: &Path,
    python_module_dirs: &[PathBuf],
) -> Option<ScriptLauncherPlan> {
    // Only inspect regular files or symlinks resolving to regular files
    let Ok(bytes) = fs::read(full_path) else {
        return None;
    };
    if bytes.len() < 2 || bytes[0] != b'#' || bytes[1] != b'!' {
        return None;
    }

    // Read first line as shebang
    let newline_idx = bytes
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(bytes.len());
    let first_line = String::from_utf8_lossy(&bytes[..newline_idx]);
    let shebang = first_line.trim_start_matches("#!").trim();
    if shebang.is_empty() {
        return None;
    }

    let parts: Vec<&str> = shebang.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // Determine interpreter
    let (raw_interp, is_env) = if parts[0].ends_with("/env") && parts.len() > 1 {
        (parts[1], true)
    } else {
        (parts[0], false)
    };

    let interp_file_name = Path::new(raw_interp)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Case 1: Python script
    if interp_file_name.starts_with("python") {
        let host_interp_exists = !is_env && Path::new(raw_interp).is_file();
        let needs_interpreter_normalization = !host_interp_exists;
        let needs_pythonpath = !python_module_dirs.is_empty();

        if needs_interpreter_normalization || needs_pythonpath {
            let resolved_interp = resolve_python_interpreter(raw_interp);
            let mut env_vars = Vec::new();
            if needs_pythonpath {
                env_vars.push(ScopedEnvVar {
                    name: "PYTHONPATH".to_string(),
                    paths: python_module_dirs.to_vec(),
                });
            }
            return Some(ScriptLauncherPlan {
                command: command.to_string(),
                target_script_rel_path: relative_store_path.to_path_buf(),
                interpreter: resolved_interp,
                env_vars,
            });
        }
    }

    // Case 2: Other script with non-existent absolute interpreter
    if !is_env && !Path::new(raw_interp).is_file() {
        if let Some(resolved) = resolve_general_interpreter(interp_file_name) {
            return Some(ScriptLauncherPlan {
                command: command.to_string(),
                target_script_rel_path: relative_store_path.to_path_buf(),
                interpreter: resolved,
                env_vars: Vec::new(),
            });
        }
    }

    None
}

/// Resolves an available Python 3 interpreter on the host system.
#[must_use]
pub fn resolve_python_interpreter(requested: &str) -> String {
    // If requested path exists directly, use it
    if Path::new(requested).is_file() {
        return requested.to_string();
    }

    // Prefer python3 on host
    let candidates = ["/usr/bin/python3", "/bin/python3", "/usr/local/bin/python3"];
    for cand in candidates {
        if Path::new(cand).is_file() {
            return cand.to_string();
        }
    }

    // Check PATH
    if let Some(path_os) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_os) {
            let p = dir.join("python3");
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }

    // Fallback
    "/usr/bin/python3".to_string()
}

/// Resolves standard host interpreters (e.g. bash, sh).
#[must_use]
pub fn resolve_general_interpreter(name: &str) -> Option<String> {
    match name {
        "bash" => {
            let candidates = ["/usr/bin/bash", "/bin/bash"];
            candidates
                .into_iter()
                .find(|p| Path::new(p).is_file())
                .map(|s| s.to_string())
        }
        "sh" => {
            let candidates = ["/bin/sh", "/usr/bin/sh"];
            candidates
                .into_iter()
                .find(|p| Path::new(p).is_file())
                .map(|s| s.to_string())
        }
        other => {
            if let Some(path_os) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&path_os) {
                    let p = dir.join(other);
                    if p.is_file() {
                        return Some(p.to_string_lossy().into_owned());
                    }
                }
            }
            None
        }
    }
}

/// Generates a POSIX-compliant symlink-safe shell launcher script.
#[must_use]
pub fn generate_launcher_script(plan: &ScriptLauncherPlan) -> String {
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script.push_str("# Generated by pkg runtime adapter (scoped launcher)\n");
    script.push_str("TARGET=\"$0\"\n");
    script.push_str("case \"$TARGET\" in\n");
    script.push_str("    */*) ;;\n");
    script.push_str("    *) TARGET=\"$(command -v -- \"$TARGET\")\" ;;\n");
    script.push_str("esac\n");
    script.push_str("while [ -L \"$TARGET\" ]; do\n");
    script.push_str("    DIR=\"$(CDPATH= cd -- \"$(dirname -- \"$TARGET\")\" && pwd)\"\n");
    script.push_str("    TARGET=\"$(readlink \"$TARGET\")\"\n");
    script.push_str("    case \"$TARGET\" in\n");
    script.push_str("        /*) ;;\n");
    script.push_str("        *) TARGET=\"$DIR/$TARGET\" ;;\n");
    script.push_str("    esac\n");
    script.push_str("done\n");
    script.push_str("LAUNCHER_DIR=\"$(CDPATH= cd -- \"$(dirname -- \"$TARGET\")\" && pwd)\"\n");
    script.push_str("BASE_DIR=\"$(CDPATH= cd -- \"$LAUNCHER_DIR/..\" && pwd)\"\n\n");

    for env_var in &plan.env_vars {
        if !env_var.paths.is_empty() {
            let expanded_paths: Vec<String> = env_var
                .paths
                .iter()
                .map(|p| format!("$BASE_DIR/{}", p.display()))
                .collect();
            let joined = expanded_paths.join(":");
            script.push_str(&format!(
                "export {}=\"{}${{{}:+:${}}}\"\n",
                env_var.name, joined, env_var.name, env_var.name
            ));
        }
    }

    script.push_str(&format!(
        "exec {} \"$BASE_DIR/{}\" \"$@\"\n",
        plan.interpreter,
        plan.target_script_rel_path.display()
    ));

    script
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_python_module_dirs() {
        let extracted = vec![
            PathBuf::from("usr/bin/reflector"),
            PathBuf::from("usr/lib/python3.14/site-packages/Reflector.py"),
            PathBuf::from("usr/lib/python3.14/site-packages/reflector/main.py"),
            PathBuf::from("etc/xdg/reflector/reflector.conf"),
        ];
        let dirs = detect_python_module_dirs(&extracted);
        assert_eq!(
            dirs,
            vec![PathBuf::from("usr/lib/python3.14/site-packages")]
        );
    }

    #[test]
    fn test_generate_launcher_script() {
        let plan = ScriptLauncherPlan {
            command: "reflector".to_string(),
            target_script_rel_path: PathBuf::from("usr/bin/reflector"),
            interpreter: "/usr/bin/python3".to_string(),
            env_vars: vec![ScopedEnvVar {
                name: "PYTHONPATH".to_string(),
                paths: vec![PathBuf::from("usr/lib/python3.14/site-packages")],
            }],
        };
        let script = generate_launcher_script(&plan);
        assert!(script.contains("export PYTHONPATH=\"$BASE_DIR/usr/lib/python3.14/site-packages${PYTHONPATH:+:$PYTHONPATH}\""));
        assert!(script.contains("exec /usr/bin/python3 \"$BASE_DIR/usr/bin/reflector\" \"$@\""));
        assert!(script.contains("LAUNCHER_DIR="));
    }
}
