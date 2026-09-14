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
        // Check staging root first if provided (package-local library)
        if let Some(staging) = staging_root {
            for sub in &["usr/lib", "lib", "usr/lib64", "lib64"] {
                if staging.join(sub).join(lib).exists() {
                    found = true;
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
