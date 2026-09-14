//! Side-effect-free installation and removal planning (INV-010).

use std::path::Path;

use crate::domain::plan::{BinaryActivation, InstallPlan, RemovePlan};
use crate::error::{Error, Result};
use crate::format::ArtifactAdapter;
use crate::format::deb::DebAdapter;
use crate::host::HostFacts;
use crate::state::StateDatabase;
use crate::store::StoreLayout;

/// Creates installation and removal plans without mutating disk or database state.
#[derive(Debug)]
pub struct Planner;

impl Planner {
    /// Plans installation of a local package artifact.
    ///
    /// This method is strictly side-effect free: it does not create files,
    /// allocate staging directories, or write to the state database.
    pub fn plan_install(
        artifact_path: &Path,
        layout: &StoreLayout,
        db: &StateDatabase,
        profile: &str,
        is_dry_run: bool,
    ) -> Result<InstallPlan> {
        let adapter = DebAdapter::new();
        let package = adapter.parse_metadata(artifact_path)?;

        // Check architecture compatibility (Gate M1-A)
        let host = HostFacts::detect();
        if !package.architecture.matches_host(&host.architecture) {
            return Err(Error::ArchitectureMismatch {
                expected: host.architecture.to_string(),
                found: package.architecture.to_string(),
            });
        }

        let store_id =
            StoreLayout::compute_store_id(&package.digest, &package.name, &package.version);
        let target_store_dir = layout.store_object_dir(&store_id);

        let mut binaries = Vec::new();
        let profile_bin = layout.profile_bin_dir(profile);

        for entry in &package.entries {
            if entry.is_dir || entry.is_symlink {
                continue;
            }

            if let Some(parent) = entry.relative_path.parent() {
                if parent == Path::new("usr/bin") || parent == Path::new("bin") {
                    if let Some(cmd) = entry.relative_path.file_name().and_then(|n| n.to_str()) {
                        // Check for activation conflict (INV-015, Gate M1-B)
                        if let Some(existing) =
                            db.find_conflicting_activation(profile, cmd, package.name.as_str())?
                        {
                            return Err(Error::ActivationConflict {
                                command: cmd.to_string(),
                                existing_package: existing,
                            });
                        }

                        binaries.push(BinaryActivation {
                            command: cmd.to_string(),
                            relative_store_path: entry.relative_path.clone(),
                            profile_symlink_path: profile_bin.join(cmd),
                        });
                    }
                }
            }
        }

        let ignored_scripts = package.scripts.iter().map(|s| s.name.clone()).collect();

        Ok(InstallPlan {
            package,
            store_id,
            target_store_dir,
            binaries,
            ignored_scripts,
            host_libraries_verified: Vec::new(),
            is_dry_run,
        })
    }

    /// Plans package removal from the selected profile.
    pub fn plan_remove(
        package_name: &str,
        layout: &StoreLayout,
        db: &StateDatabase,
        profile: &str,
        is_dry_run: bool,
    ) -> Result<RemovePlan> {
        let pkg = db
            .get_package(profile, package_name)?
            .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;

        let profile_bin = layout.profile_bin_dir(profile);
        let binaries_to_remove = pkg
            .binaries
            .iter()
            .map(|bin| profile_bin.join(bin))
            .collect();

        Ok(RemovePlan {
            package_name: pkg.name,
            version: pkg.version,
            store_id: pkg.store_id,
            store_path: pkg.store_path,
            binaries_to_remove,
            is_dry_run,
        })
    }
}
