//! Static ELF binary inspection and library compatibility check.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::{Error, Result};

/// Standard host Linux library search directories.
const STANDARD_LIB_DIRS: &[&str] = &[
    "/lib64",
    "/usr/lib64",
    "/lib/x86_64-linux-gnu",
    "/usr/lib/x86_64-linux-gnu",
    "/lib",
    "/usr/lib",
];

/// Results of inspecting an ELF executable or shared library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfInspection {
    /// Dynamic libraries requested via `DT_NEEDED`.
    pub needed_libraries: Vec<String>,
    /// Libraries successfully resolved on the host or inside the package staging root.
    pub resolved_libraries: Vec<String>,
    /// Libraries that could not be located in standard host paths.
    pub missing_libraries: Vec<String>,
}

/// Checks if a file starts with the ELF magic header (`\x7fELF`).
pub fn is_elf(path: &Path) -> bool {
    let mut buf = [0u8; 4];
    if let Ok(mut f) = File::open(path) {
        if f.read_exact(&mut buf).is_ok() {
            return buf == [0x7f, b'E', b'L', b'F'];
        }
    }
    false
}

/// Inspects an ELF binary, extracts its `DT_NEEDED` dependencies, and checks compatibility.
pub fn inspect_elf(elf_path: &Path, staging_root: Option<&Path>) -> Result<Option<ElfInspection>> {
    inspect_elf_with_extra_paths(elf_path, staging_root, &[])
}

/// Inspects an ELF binary with additional search paths (e.g. store objects and profile lib dir).
pub fn inspect_elf_with_extra_paths(
    elf_path: &Path,
    staging_root: Option<&Path>,
    extra_search_dirs: &[std::path::PathBuf],
) -> Result<Option<ElfInspection>> {
    if !is_elf(elf_path) {
        return Ok(None);
    }

    let bytes = std::fs::read(elf_path)?;
    let elf = match goblin::elf::Elf::parse(&bytes) {
        Ok(e) => e,
        Err(e) => {
            return Err(Error::MalformedArchive(format!(
                "Invalid ELF binary '{}': {e}",
                elf_path.display()
            )));
        }
    };

    let mut needed = Vec::new();
    let mut resolved = Vec::new();
    let mut missing = Vec::new();

    for &lib in &elf.libraries {
        needed.push(lib.to_string());

        let mut found = false;
        // Resolve paths in the same order the dynamic loader uses for package-local
        // binaries: RUNPATH (or RPATH when RUNPATH is absent), then host defaults.
        if let Some(staging) = staging_root {
            let origin = elf_path.parent().unwrap_or(staging);
            let search_paths = if !elf.runpaths.is_empty() {
                &elf.runpaths
            } else {
                &elf.rpaths
            };
            for raw_path in search_paths {
                for path in raw_path.split(':') {
                    let expanded = path
                        .replace("${ORIGIN}", &origin.to_string_lossy())
                        .replace("$ORIGIN", &origin.to_string_lossy());
                    let candidate = if Path::new(&expanded).is_absolute() {
                        Path::new(&expanded).to_path_buf()
                    } else {
                        origin.join(expanded)
                    };
                    if candidate.join(lib).exists() {
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
        }

        // Check host standard directories
        if !found {
            for &dir in STANDARD_LIB_DIRS {
                if Path::new(dir).join(lib).exists() {
                    found = true;
                    break;
                }
            }
        }

        // Check extra search directories (e.g. profile lib or installed store packages)
        if !found {
            for dir in extra_search_dirs {
                if dir.join(lib).exists() {
                    found = true;
                    break;
                }
            }
        }

        if found {
            resolved.push(lib.to_string());
        } else {
            missing.push(lib.to_string());
        }
    }

    Ok(Some(ElfInspection {
        needed_libraries: needed,
        resolved_libraries: resolved,
        missing_libraries: missing,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_elf_on_non_elf() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"echo hello").unwrap();
        assert!(!is_elf(temp.path()));
    }
}
