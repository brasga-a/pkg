//! Static ELF binary inspection and library compatibility check.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::domain::package::Architecture;
use crate::error::{Error, Result};

/// Returns the ELF machine and class expected for a concrete host/package
/// architecture.  `All`/`Any` deliberately return `None`: those metadata
/// values describe architecture-independent payloads, so the actual ELF
/// object still has to be checked against the host at inspection time.
pub fn expected_elf_abi(architecture: &Architecture) -> Option<(u16, u8)> {
    match architecture {
        Architecture::X86_64 => Some((goblin::elf::header::EM_X86_64, 64)),
        Architecture::Other(name) if matches!(name.as_str(), "aarch64" | "arm64") => {
            Some((goblin::elf::header::EM_AARCH64, 64))
        }
        Architecture::Other(name) if name == "riscv64" => Some((goblin::elf::header::EM_RISCV, 64)),
        Architecture::Other(name)
            if matches!(name.as_str(), "arm" | "armhf" | "armv7" | "armv7h") =>
        {
            Some((goblin::elf::header::EM_ARM, 32))
        }
        Architecture::Other(name)
            if matches!(name.as_str(), "i686" | "i386" | "i486" | "i586" | "x86") =>
        {
            Some((goblin::elf::header::EM_386, 32))
        }
        Architecture::Other(name) if matches!(name.as_str(), "ppc64le" | "ppc64el") => {
            Some((goblin::elf::header::EM_PPC64, 64))
        }
        Architecture::All | Architecture::Any | Architecture::Other(_) => None,
    }
}

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
    /// Exact path selected for each resolved SONAME.
    pub resolved_paths: BTreeMap<String, std::path::PathBuf>,
    /// SONAME exported by this object, when it is a shared object.
    pub soname: Option<String>,
    /// GNU symbol versions required from each DT_NEEDED object.
    pub symbol_versions: BTreeMap<String, Vec<String>>,
    /// GNU symbol versions exported by this object.
    pub defined_symbol_versions: Vec<String>,
    /// ELF interpreter requested through `PT_INTERP`, when present.
    pub interpreter: Option<String>,
    /// ELF machine identifier (`e_machine`).
    pub machine: u16,
    /// ELF class in bits (32 or 64).
    pub class_bits: u8,
    /// Whether the ELF object uses little-endian encoding.
    pub little_endian: bool,
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

    let expected_machine = match std::env::consts::ARCH {
        "x86_64" => goblin::elf::header::EM_X86_64,
        "aarch64" => goblin::elf::header::EM_AARCH64,
        "riscv64" => goblin::elf::header::EM_RISCV,
        _ => 0,
    };
    if expected_machine != 0 && elf.header.e_machine != expected_machine {
        return Err(Error::ArchitectureMismatch {
            expected: Architecture::parse(std::env::consts::ARCH).to_string(),
            found: format!("ELF machine {}", elf.header.e_machine),
        });
    }
    let expected_class_bits = match std::env::consts::ARCH {
        "x86_64" | "aarch64" | "riscv64" => 64,
        _ => {
            if elf.is_64 {
                64
            } else {
                32
            }
        }
    };
    if elf.is_64 != (expected_class_bits == 64) {
        return Err(Error::IncompatibleHost(format!(
            "ELF class {} is unsupported on this {}-bit host",
            if elf.is_64 { 64 } else { 32 },
            expected_class_bits
        )));
    }
    if let Some(interpreter) = elf.interpreter {
        let interpreter_path = Path::new(interpreter);
        let available = interpreter_path.exists()
            || staging_root.is_some_and(|root| {
                root.join(
                    interpreter_path
                        .strip_prefix("/")
                        .unwrap_or(interpreter_path),
                )
                .exists()
            });
        if !available {
            return Err(Error::IncompatibleHost(format!(
                "ELF interpreter is unavailable: {interpreter}"
            )));
        }
    }

    let mut needed = Vec::new();
    let mut resolved = Vec::new();
    let mut missing = Vec::new();
    let mut resolved_paths = BTreeMap::new();

    // The staging tree is transaction-owned.  A package-local RPATH, an
    // explicit pathname, or a symlink must never turn a provider lookup into
    // a read outside that tree.  Host paths remain admissible when they are
    // not rooted in staging (for example `/lib64/libc.so.6`).
    let canonical_staging_root = staging_root
        .filter(|root| root.exists())
        .map(std::fs::canonicalize)
        .transpose()
        .map_err(Error::from)?;
    let existing_candidate = |candidate: &Path| -> Result<Option<std::path::PathBuf>> {
        if !candidate.exists() {
            return Ok(None);
        }
        let canonical = std::fs::canonicalize(candidate)?;
        if let (Some(staging), Some(canonical_root)) = (staging_root, &canonical_staging_root) {
            let rooted_in_staging = candidate.starts_with(staging);
            if rooted_in_staging && !canonical.starts_with(canonical_root) {
                return Err(Error::SecurityViolation(format!(
                    "ELF dependency path escapes staging root: {}",
                    candidate.display()
                )));
            }
        }
        Ok(Some(canonical))
    };

    let mut symbol_versions = BTreeMap::new();
    if let Some(verneed) = &elf.verneed {
        for need in verneed.iter() {
            let Some(file) = elf.dynstrtab.get_at(need.vn_file) else {
                continue;
            };
            let versions = need
                .iter()
                .filter_map(|aux| elf.dynstrtab.get_at(aux.vna_name))
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !versions.is_empty() {
                symbol_versions.insert(file.to_string(), versions);
            }
        }
    }
    let mut defined_symbol_versions = Vec::new();
    if let Some(verdef) = &elf.verdef {
        for definition in verdef.iter() {
            for aux in definition.iter() {
                if let Some(name) = elf.dynstrtab.get_at(aux.vda_name) {
                    if !defined_symbol_versions
                        .iter()
                        .any(|existing| existing == name)
                    {
                        defined_symbol_versions.push(name.to_string());
                    }
                }
            }
        }
    }

    for &lib in &elf.libraries {
        needed.push(lib.to_string());

        let mut found = false;
        let mut selected_path = None;
        let resolve_explicit = |root: Option<&Path>, origin: &Path, name: &str| {
            let requested = Path::new(name);
            if requested.is_absolute() {
                // Absolute DT_NEEDED paths are interpreted relative to the
                // package root first.  Falling back to the host path keeps
                // native system objects usable while never rewriting a
                // package path into an arbitrary directory.
                if let Some(staging) = root {
                    let package_path =
                        staging.join(requested.strip_prefix("/").unwrap_or(requested));
                    if let Some(path) = existing_candidate(&package_path)? {
                        return Ok(Some(path));
                    }
                }
                existing_candidate(requested)
            } else {
                let path = origin.join(requested);
                existing_candidate(&path)
            }
        };

        // A pathname in DT_NEEDED is already an exact request.  Do not append
        // it to a directory (which would turn `lib/foo.so` into
        // `lib/lib/foo.so`).
        if lib.contains('/') {
            let origin = elf_path.parent().unwrap_or_else(|| Path::new("."));
            if let Some(path) = resolve_explicit(staging_root, origin, lib)? {
                found = true;
                selected_path = Some(path);
            }
        }
        // Resolve paths in the same order the dynamic loader uses: RUNPATH
        // (or RPATH when RUNPATH is absent), then package-local directories,
        // then explicit profile/host search paths.  RPATH resolution also
        // applies to host providers; skipping it would miss transitive
        // libraries shipped beside a verified host object.
        if !found {
            let origin = elf_path.parent().unwrap_or_else(|| Path::new("."));
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
                    let candidate = candidate.join(lib);
                    if let Some(candidate) = existing_candidate(&candidate)? {
                        found = true;
                        selected_path = Some(candidate);
                        break;
                    }
                }
                if found {
                    break;
                }
            }
            // Package-local libraries are admissible providers and are
            // searched before host/profile paths.  Only the selected file is
            // recorded; the whole directory is never exported to a process.
            if !found && let Some(staging) = staging_root {
                for relative_dir in ["lib", "lib64", "usr/lib", "usr/lib64"] {
                    let candidate = staging.join(relative_dir).join(lib);
                    if let Some(candidate) = existing_candidate(&candidate)? {
                        found = true;
                        selected_path = Some(candidate.clone());
                        resolved_paths.insert(lib.to_string(), candidate);
                        break;
                    }
                }
            }
        }

        // A command-specific runtime view is an explicit search path and must
        // take precedence over host defaults. It is populated only with
        // providers selected by the planner; it is not a profile-wide library
        // directory.
        if !found {
            for dir in extra_search_dirs {
                if let Some(path) = existing_candidate(&dir.join(lib))? {
                    found = true;
                    selected_path = Some(path.clone());
                    resolved_paths.insert(lib.to_string(), path);
                    break;
                }
            }
        }

        // Finally check host standard directories.
        if !found {
            for &dir in STANDARD_LIB_DIRS {
                if let Some(path) = existing_candidate(&Path::new(dir).join(lib))? {
                    found = true;
                    selected_path = Some(path.clone());
                    resolved_paths.insert(lib.to_string(), path);
                    break;
                }
            }
        }

        if found {
            if let Some(path) = selected_path {
                resolved_paths.insert(lib.to_string(), path);
            }
            resolved.push(lib.to_string());
        } else {
            missing.push(lib.to_string());
        }
    }

    Ok(Some(ElfInspection {
        needed_libraries: needed,
        resolved_libraries: resolved,
        missing_libraries: missing,
        resolved_paths,
        soname: elf.soname.map(str::to_string),
        symbol_versions,
        defined_symbol_versions,
        interpreter: elf.interpreter.map(str::to_string),
        machine: elf.header.e_machine,
        class_bits: if elf.is_64 { 64 } else { 32 },
        little_endian: elf.little_endian,
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
