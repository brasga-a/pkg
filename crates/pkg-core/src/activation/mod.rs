//! Profile-based binary activation and link management (ADR-010, INV-006, INV-015).

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::plan::InstallPlan;
use crate::error::{Error, Result};

/// Manages binary symlinks in profile bin directories.
#[derive(Debug)]
pub struct Activator;

impl Activator {
    /// Rejects an existing destination unless its link target matches recorded ownership.
    pub fn check_destination(path: &Path, expected: Option<&Path>) -> Result<()> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && expected.is_some_and(|target| {
                        fs::read_link(path).is_ok_and(|actual| actual == target)
                    }) =>
            {
                Ok(())
            }
            Ok(_) => Err(Error::ActivationConflict {
                command: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                existing_package: "unmanaged or modified file".into(),
            }),
        }
    }

    /// Activates executable binaries for an installed store object within a profile.
    pub fn activate(plan: &InstallPlan, profile_bin_dir: &Path) -> Result<()> {
        fs::create_dir_all(profile_bin_dir)?;
        for binary in &plan.binaries {
            Self::check_destination(
                &binary.profile_symlink_path,
                binary.previous_target.as_deref(),
            )?;
        }

        for binary in &plan.binaries {
            let store_target = plan.target_store_dir.join(&binary.relative_store_path);
            let link_path = &binary.profile_symlink_path;

            if link_path.is_symlink() {
                // Build a replacement beside the link, then switch it atomically.
                let temp = tempfile::tempdir_in(profile_bin_dir)?;
                let replacement = temp.path().join("link");
                std::os::unix::fs::symlink(&store_target, &replacement)?;
                Self::check_destination(link_path, binary.previous_target.as_deref())?;
                fs::rename(replacement, link_path)?;
            } else {
                // symlink fails atomically if any unmanaged entry appeared after planning.
                std::os::unix::fs::symlink(&store_target, link_path)?;
            }
        }

        Ok(())
    }

    /// Deactivates binary symlinks during package removal.
    pub fn deactivate(binaries: &[PathBuf], expected_targets: &[PathBuf]) -> Result<()> {
        if binaries.len() != expected_targets.len() {
            return Err(Error::Internal(
                "Incomplete activation ownership in removal plan".into(),
            ));
        }
        for (link_path, expected) in binaries.iter().zip(expected_targets) {
            // A user replacement is no longer pkg-owned and must remain untouched.
            if fs::read_link(link_path).is_ok_and(|actual| actual == *expected) {
                fs::remove_file(link_path)?;
            }
        }
        Ok(())
    }

    /// Activates shared libraries from a store object into the profile lib directory.
    pub fn activate_libraries(store_dir: &Path, profile_lib_dir: &Path) -> Result<()> {
        if !store_dir.exists() {
            return Ok(());
        }
        fs::create_dir_all(profile_lib_dir)?;

        let mut lib_files = Vec::new();
        let lib_subdirs = ["lib", "lib64", "usr/lib", "usr/lib64"];
        for sub in &lib_subdirs {
            let p = store_dir.join(sub);
            if p.is_dir() {
                collect_so_files(&p, &mut lib_files);
            }
        }

        for file in lib_files {
            if let Some(name) = file.file_name() {
                let link_path = profile_lib_dir.join(name);
                if link_path.is_symlink() {
                    let _ = fs::remove_file(&link_path);
                } else if link_path.exists() {
                    // Unmanaged regular file or directory: preserve it
                    continue;
                }
                let _ = std::os::unix::fs::symlink(&file, &link_path);
            }
        }

        Ok(())
    }

    /// Deactivates shared library symlinks for a store object from the profile lib directory.
    pub fn deactivate_libraries(store_dir: &Path, profile_lib_dir: &Path) -> Result<()> {
        if !profile_lib_dir.exists() {
            return Ok(());
        }
        if let Ok(entries) = fs::read_dir(profile_lib_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_symlink() {
                    if let Ok(target) = fs::read_link(&path) {
                        if target.starts_with(store_dir) {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn collect_so_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                collect_so_files(&path, files);
            } else if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.contains(".so") && path.exists() {
                    files.push(path);
                }
            }
        }
    }
}
