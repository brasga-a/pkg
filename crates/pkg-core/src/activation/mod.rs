//! Profile-based binary activation and link management (ADR-010, INV-006, INV-015).

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::plan::InstallPlan;
use crate::error::Result;

/// Manages binary symlinks in profile bin directories.
#[derive(Debug)]
pub struct Activator;

impl Activator {
    /// Activates executable binaries for an installed store object within a profile.
    pub fn activate(plan: &InstallPlan, profile_bin_dir: &Path) -> Result<()> {
        fs::create_dir_all(profile_bin_dir)?;

        for binary in &plan.binaries {
            let store_target = plan.target_store_dir.join(&binary.relative_store_path);
            let link_path = &binary.profile_symlink_path;

            // Remove existing link if updating or reinstalling
            if link_path.is_symlink() || link_path.exists() {
                let _ = fs::remove_file(link_path);
            }

            #[cfg(unix)]
            std::os::unix::fs::symlink(&store_target, link_path)?;
        }

        Ok(())
    }

    /// Deactivates binary symlinks during package removal.
    pub fn deactivate(binaries: &[PathBuf]) -> Result<()> {
        for link_path in binaries {
            if link_path.is_symlink() || link_path.exists() {
                let _ = fs::remove_file(link_path);
            }
        }
        Ok(())
    }
}
