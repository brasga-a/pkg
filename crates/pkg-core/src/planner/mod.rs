//! Side-effect-free installation and removal planning (INV-010).

use std::path::Path;

use crate::activation::Activator;
use crate::domain::contracts::{ArtifactEvidence, TrustEvidence};
use crate::domain::package::PackageName;
use crate::domain::plan::{BinaryActivation, InstallPlan, RemovePlan};
use crate::error::{Error, Result};
use crate::host::HostFacts;
use crate::resolver::Resolver;
use crate::resolver::evidence::HostEvidence;
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
        Self::plan_install_with_replacements(artifact_path, layout, db, profile, is_dry_run, &[])
    }

    /// Plans an install while treating the listed installed package names as
    /// replacements in the same transaction. This lets an upgrade resolve
    /// versioned `Breaks`/`Conflicts` against the incoming generation.
    pub fn plan_install_with_replacements(
        artifact_path: &Path,
        layout: &StoreLayout,
        db: &StateDatabase,
        profile: &str,
        is_dry_run: bool,
        replaced_packages: &[PackageName],
    ) -> Result<InstallPlan> {
        Self::plan_install_with_replacements_and_options(
            artifact_path,
            layout,
            db,
            profile,
            is_dry_run,
            replaced_packages,
            &crate::engine::InstallOptions::default(),
        )
    }

    /// Plans an install while treating the listed installed package names as
    /// replacements in the same transaction and respecting customized install options.
    pub fn plan_install_with_replacements_and_options(
        artifact_path: &Path,
        layout: &StoreLayout,
        db: &StateDatabase,
        profile: &str,
        is_dry_run: bool,
        replaced_packages: &[PackageName],
        options: &crate::engine::InstallOptions,
    ) -> Result<InstallPlan> {
        StoreLayout::validate_profile(profile)?;
        let format = crate::format::detect_format(artifact_path)?;
        let adapter = crate::format::get_adapter(format);
        let package = adapter.parse_metadata(artifact_path)?;

        // Check architecture compatibility (Gate M1-A)
        let host = HostFacts::detect();
        if !package.architecture.matches_host(&host.architecture) {
            return Err(Error::ArchitectureMismatch {
                expected: host.architecture.to_string(),
                found: package.architecture.to_string(),
            });
        }

        // Resolve declared package/capability constraints before producing an
        // install plan, unless dependency resolution is explicitly skipped.
        let mut resolved_dependencies = Vec::new();
        if !options.skip_dependencies && !package.constraints.is_empty() {
            let mut host_evidence = HostEvidence::detect(&host);
            let profile_lib_dir = layout.profile_lib_dir(profile);
            if profile_lib_dir.is_dir()
                && !host_evidence
                    .library_search_paths
                    .contains(&profile_lib_dir)
            {
                host_evidence
                    .library_search_paths
                    .push(profile_lib_dir.clone());
            }
            let installed = db.normalized_packages_for_profile(profile)?;
            let resolution = Resolver::new(host_evidence)
                .with_installed_packages(installed)
                .with_repository_packages(db.normalized_remote_packages()?)
                .with_replaced_packages(replaced_packages.iter().cloned())
                .resolve(&package)
                .map_err(Error::from)?;
            resolved_dependencies = resolution.packages_to_install;
        }

        let store_id = StoreLayout::compute_derivation_id(
            &package.digest,
            &package.name,
            &package.version,
            &package.format.to_string(),
            "normalizer-v1",
            "recipes-v1",
            layout.base_dir(),
        );
        let target_store_dir = layout.store_object_dir(&store_id);
        layout.validate_store_path(&target_store_dir)?;

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

                        if binaries.iter().any(|b: &BinaryActivation| b.command == cmd) {
                            return Err(Error::MalformedArchive(format!(
                                "Duplicate executable command: {cmd}"
                            )));
                        }
                        let previous_target =
                            db.activation_target(profile, cmd, package.name.as_str())?;
                        Activator::check_destination(
                            &profile_bin.join(cmd),
                            previous_target.as_deref(),
                        )?;
                        binaries.push(BinaryActivation {
                            command: cmd.to_string(),
                            relative_store_path: entry.relative_path.clone(),
                            profile_symlink_path: profile_bin.join(cmd),
                            previous_target,
                        });
                    }
                }
            }
        }

        let ignored_scripts = package.scripts.iter().map(|s| s.name.clone()).collect();

        let remote_by_digest = match db.remote_package_by_digest(package.digest.hex())? {
            Some(remote) => Some(remote),
            None => {
                // Older snapshots stored the algorithm prefix alongside the
                // hexadecimal digest.  Accept that canonical spelling too so
                // provenance is not silently downgraded to local unsigned
                // when reopening a compatible state database.
                db.remote_package_by_digest(&package.digest.to_string())?
            }
        };
        let artifact_evidence = remote_by_digest
            .filter(|(remote, _)| {
                remote.name == package.name.as_str()
                    && remote.version == package.version.as_str()
                    && remote.format == package.format.to_string()
                    && remote.architecture == package.architecture.as_str()
                    && remote.size_bytes == package.size_bytes
            })
            .map(|(remote, snapshot_id)| ArtifactEvidence {
                schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
                source: remote.url,
                format: package.format,
                architecture: package.architecture.clone(),
                size_bytes: package.size_bytes,
                digest: package.digest.clone(),
                catalog_snapshot: Some(snapshot_id.clone()),
                trust: TrustEvidence::Repository {
                    repository_id: remote.repository_id,
                    snapshot_id,
                    metadata_verified: true,
                },
            })
            .unwrap_or_else(|| ArtifactEvidence::local(artifact_path.to_string_lossy(), &package));

        Ok(InstallPlan {
            package,
            store_id,
            target_store_dir,
            binaries,
            ignored_scripts,
            host_libraries_verified: Vec::new(),
            missing_libraries: Vec::new(),
            is_dry_run,
            artifact_evidence: Some(artifact_evidence),
            payload_manifest: None,
            adaptations: Vec::new(),
            executions: Vec::new(),
            runtimes: Vec::new(),
            resolved_dependencies,
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
        StoreLayout::validate_profile(profile)?;
        let pkg = db
            .get_package(profile, package_name)?
            .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;

        let profile_bin = layout.profile_bin_dir(profile);
        layout.validate_store_path(&pkg.store_path)?;
        let expected_targets = pkg
            .binaries
            .iter()
            .map(|bin| {
                db.activation_target(profile, bin, package_name)?
                    .ok_or_else(|| {
                        Error::Internal(format!("Missing activation ownership for {bin}"))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
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
            expected_targets,
            is_dry_run,
        })
    }
}
