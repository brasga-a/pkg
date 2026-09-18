//! Profile-based binary activation and link management (ADR-010, INV-006, INV-015).

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::contracts::ActivationGeneration;
use crate::domain::plan::InstallPlan;
use crate::error::{Error, Result};
use crate::store::StoreLayout;

/// Manages binary symlinks in profile bin directories.
#[derive(Debug)]
pub struct Activator;

/// Publishes the immutable command view selected for a profile.
///
/// The legacy `profiles/<name>/bin` directory is captured as a generation on
/// first publication.  The stable `bin` path then becomes a symlink to
/// `current/bin`; existing callers keep their path while new launches resolve
/// one generation and stay pinned to it.
#[derive(Debug)]
pub struct GenerationManager;

impl GenerationManager {
    /// Detaches the active generation's bin directory into transaction-owned
    /// staging so an install can prepare a new command set without mutating a
    /// generation that may still be retained for rollback.
    pub fn detach_active_bin(
        layout: &StoreLayout,
        profile: &str,
        transaction_id: &str,
    ) -> Result<()> {
        let bin = layout.profile_bin_link(profile);
        let metadata = match fs::symlink_metadata(&bin) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_symlink() {
            return Ok(());
        }
        let detached = layout.staging_dir(transaction_id).join("activation-bin");
        if detached.exists() {
            fs::remove_dir_all(&detached)?;
        }
        crate::store::layout::ensure_directory(&detached)?;
        for entry in fs::read_dir(&bin)? {
            let entry = entry?;
            copy_entry(&entry.path(), &detached.join(entry.file_name()))?;
        }
        fs::remove_file(&bin)?;
        fs::rename(&detached, &bin)?;
        Ok(())
    }

    pub fn publish_after_activation(
        layout: &StoreLayout,
        profile: &str,
        transaction_id: &str,
        plan: &InstallPlan,
    ) -> Result<String> {
        StoreLayout::validate_profile(profile)?;
        StoreLayout::validate_component(transaction_id, "transaction id")?;
        let profile_dir = layout.profile_dir(profile);
        let generations_dir = layout.profile_generations_dir(profile);
        crate::store::layout::ensure_directory(&profile_dir)?;
        crate::store::layout::ensure_directory(&generations_dir)?;

        let previous_generation = fs::read_link(layout.profile_current_path(profile))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        let generation_id = format!("gen-{transaction_id}");
        let generation_dir = generations_dir.join(&generation_id);
        if generation_dir.exists() {
            return Err(Error::SecurityViolation(format!(
                "Generation already exists: {}",
                generation_dir.display()
            )));
        }
        let staging = layout.staging_dir(transaction_id).join("generation");
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        let staging_bin = staging.join("bin");
        crate::store::layout::ensure_directory(&staging_bin)?;

        // Copy the complete visible command set, including unmanaged entries,
        // before replacing the package's own commands.  Symlink targets remain
        // literal and are never followed during the copy.
        let active_bin = layout.profile_bin_dir(profile);
        if let Ok(entries) = fs::read_dir(&active_bin) {
            for entry in entries {
                let entry = entry?;
                copy_entry(&entry.path(), &staging_bin.join(entry.file_name()))?;
            }
        }

        let mut commands = std::collections::BTreeMap::new();
        for entry in fs::read_dir(&staging_bin)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let target = fs::read_link(entry.path()).unwrap_or_else(|_| entry.path());
            commands.insert(name, target);
        }
        let replacement_commands = plan
            .runtimes
            .iter()
            .map(|runtime| runtime.execution.command.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut owned_paths =
            previous_owned_paths(layout, profile, previous_generation.as_deref())?
                .into_iter()
                .filter(|path| {
                    path.strip_prefix("bin/")
                        .ok()
                        .and_then(|name| name.to_str())
                        .is_none_or(|name| !replacement_commands.contains(name))
                })
                .collect::<Vec<_>>();
        for binary in &plan.binaries {
            let path = PathBuf::from("bin").join(&binary.command);
            if !owned_paths.contains(&path) {
                owned_paths.push(path);
            }
        }
        let runtime_ids = complete_runtime_ids(
            layout,
            profile,
            previous_generation.as_deref(),
            &commands,
            &replacement_commands,
            plan.runtimes
                .iter()
                .map(|runtime| runtime.runtime_id.clone()),
        )?;
        let runtimes_verified = runtime_ids.iter().all(|runtime_id| {
            fs::read(layout.runtime_dir(runtime_id).join("manifest.json"))
                .ok()
                .and_then(|bytes| {
                    serde_json::from_slice::<crate::domain::contracts::RuntimeManifest>(&bytes).ok()
                })
                .is_some_and(|runtime| runtime.verified)
        });
        let generation_verified = commands.keys().all(|command| {
            runtime_ids.iter().any(|runtime_id| {
                fs::read(layout.runtime_dir(runtime_id).join("manifest.json"))
                    .ok()
                    .and_then(|bytes| {
                        serde_json::from_slice::<crate::domain::contracts::RuntimeManifest>(&bytes)
                            .ok()
                    })
                    .is_some_and(|runtime| runtime.execution.command == *command)
            })
        }) && runtimes_verified;
        let generation = ActivationGeneration {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            generation_id: generation_id.clone(),
            profile: profile.to_string(),
            previous_generation,
            commands,
            runtimes: runtime_ids,
            owned_paths,
            verified: generation_verified,
        };
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&generation)?,
        )?;
        fs::rename(&staging, &generation_dir)?;

        // Both pointers are switched with rename, so a reader sees either the
        // previous generation or the complete new one.
        let current = layout.profile_current_path(profile);
        let current_tmp = profile_dir.join(format!(".current.{transaction_id}"));
        let _ = fs::remove_file(&current_tmp);
        std::os::unix::fs::symlink(Path::new("generations").join(&generation_id), &current_tmp)?;
        fs::rename(&current_tmp, &current)?;

        let bin = layout.profile_bin_link(profile);
        let bin_tmp = profile_dir.join(format!(".bin.{transaction_id}"));
        let _ = fs::remove_file(&bin_tmp);
        // If this is the first generation, preserve the legacy directory as a
        // retained snapshot rather than deleting user-owned entries.
        if fs::symlink_metadata(&bin).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            let legacy = profile_dir.join(format!("legacy-bin-{transaction_id}"));
            fs::rename(&bin, legacy)?;
        }
        std::os::unix::fs::symlink(Path::new("current").join("bin"), &bin_tmp)?;
        fs::rename(&bin_tmp, &bin)?;

        // Ensure runtime documents remain available even when publication is
        // invoked by a caller other than Engine::install.
        for runtime in &plan.runtimes {
            layout.write_runtime_manifest(runtime)?;
        }
        Ok(generation_id)
    }

    /// Returns the generation currently selected by the profile, if present.
    pub fn current(layout: &StoreLayout, profile: &str) -> Result<Option<String>> {
        let path = layout.profile_current_path(profile);
        match fs::read_link(path) {
            Ok(target) => Ok(target.file_name().map(|v| v.to_string_lossy().into_owned())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Atomically selects an already prepared generation.
    pub fn switch_to(layout: &StoreLayout, profile: &str, generation_id: &str) -> Result<()> {
        StoreLayout::validate_profile(profile)?;
        StoreLayout::validate_component(generation_id, "generation id")?;
        let generation_dir = layout.profile_generations_dir(profile).join(generation_id);
        let manifest_path = generation_dir.join("manifest.json");
        if !fs::symlink_metadata(&manifest_path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            return Err(Error::PackageNotFound(format!(
                "generation {generation_id}"
            )));
        }
        let manifest: ActivationGeneration = serde_json::from_slice(&fs::read(&manifest_path)?)?;
        if manifest.profile != profile || manifest.generation_id != generation_id {
            return Err(Error::SecurityViolation(format!(
                "Generation manifest identity mismatch: {}",
                manifest_path.display()
            )));
        }
        for command in manifest.commands.keys() {
            StoreLayout::validate_component(command, "generation command")?;
        }
        let profile_dir = layout.profile_dir(profile);
        crate::store::layout::ensure_directory(&profile_dir)?;
        let current = layout.profile_current_path(profile);
        let current_tmp = profile_dir.join(format!(".current.rollback-{generation_id}"));
        let _ = fs::remove_file(&current_tmp);
        std::os::unix::fs::symlink(Path::new("generations").join(generation_id), &current_tmp)?;
        fs::rename(&current_tmp, &current)?;

        let bin = layout.profile_bin_link(profile);
        match fs::symlink_metadata(&bin) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let tmp = profile_dir.join(format!(".bin.rollback-{generation_id}"));
                let _ = fs::remove_file(&tmp);
                std::os::unix::fs::symlink(Path::new("current").join("bin"), &tmp)?;
                fs::rename(tmp, &bin)?;
            }
            Ok(_) => {
                return Err(Error::ActivationConflict {
                    command: "bin".into(),
                    existing_package: "unmanaged profile directory".into(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::os::unix::fs::symlink(Path::new("current").join("bin"), &bin)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// Publishes a fresh generation after an explicit removal, preserving the
    /// previous manifest as a historical snapshot.
    pub fn publish_after_removal(
        layout: &StoreLayout,
        profile: &str,
        transaction_id: &str,
    ) -> Result<String> {
        StoreLayout::validate_profile(profile)?;
        StoreLayout::validate_component(transaction_id, "transaction id")?;
        let profile_dir = layout.profile_dir(profile);
        let generations_dir = layout.profile_generations_dir(profile);
        crate::store::layout::ensure_directory(&profile_dir)?;
        crate::store::layout::ensure_directory(&generations_dir)?;
        let previous_generation = fs::read_link(layout.profile_current_path(profile))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        let generation_id = format!("gen-{transaction_id}");
        let generation_dir = generations_dir.join(&generation_id);
        let staging = layout.staging_dir(transaction_id).join("generation");
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        let staging_bin = staging.join("bin");
        crate::store::layout::ensure_directory(&staging_bin)?;
        if let Ok(entries) = fs::read_dir(layout.profile_bin_dir(profile)) {
            for entry in entries {
                let entry = entry?;
                copy_entry(&entry.path(), &staging_bin.join(entry.file_name()))?;
            }
        }
        let mut commands = std::collections::BTreeMap::new();
        for entry in fs::read_dir(&staging_bin)? {
            let entry = entry?;
            commands.insert(
                entry.file_name().to_string_lossy().into_owned(),
                fs::read_link(entry.path()).unwrap_or_else(|_| entry.path()),
            );
        }
        let runtime_ids = complete_runtime_ids(
            layout,
            profile,
            previous_generation.as_deref(),
            &commands,
            &std::collections::BTreeSet::new(),
            std::iter::empty(),
        )?;
        let generation_verified = previous_generation
            .as_deref()
            .and_then(|generation_id| {
                fs::read(
                    layout
                        .profile_generations_dir(profile)
                        .join(generation_id)
                        .join("manifest.json"),
                )
                .ok()
                .and_then(|bytes| {
                    serde_json::from_slice::<ActivationGeneration>(&bytes)
                        .ok()
                        .map(|generation| generation.verified)
                })
            })
            .unwrap_or(false)
            && commands.keys().all(|command| {
                runtime_ids.iter().any(|runtime_id| {
                    fs::read(layout.runtime_dir(runtime_id).join("manifest.json"))
                        .ok()
                        .and_then(|bytes| {
                            serde_json::from_slice::<crate::domain::contracts::RuntimeManifest>(
                                &bytes,
                            )
                            .ok()
                        })
                        .is_some_and(|runtime| {
                            runtime.execution.command == *command && runtime.verified
                        })
                })
            });
        let owned_paths = previous_owned_paths(layout, profile, previous_generation.as_deref())?
            .into_iter()
            .filter(|path| {
                commands.contains_key(
                    path.strip_prefix("bin/")
                        .ok()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default(),
                )
            })
            .collect();
        let generation = ActivationGeneration {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            generation_id: generation_id.clone(),
            profile: profile.to_string(),
            previous_generation,
            commands,
            runtimes: runtime_ids,
            owned_paths,
            verified: generation_verified,
        };
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&generation)?,
        )?;
        fs::rename(&staging, &generation_dir)?;
        let current = layout.profile_current_path(profile);
        let current_tmp = profile_dir.join(format!(".current.{transaction_id}"));
        let _ = fs::remove_file(&current_tmp);
        std::os::unix::fs::symlink(Path::new("generations").join(&generation_id), &current_tmp)?;
        fs::rename(&current_tmp, &current)?;
        let bin = layout.profile_bin_link(profile);
        let bin_tmp = profile_dir.join(format!(".bin.{transaction_id}"));
        let _ = fs::remove_file(&bin_tmp);
        if fs::symlink_metadata(&bin).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
            fs::rename(
                &bin,
                profile_dir.join(format!("legacy-bin-{transaction_id}")),
            )?;
        }
        std::os::unix::fs::symlink(Path::new("current").join("bin"), &bin_tmp)?;
        fs::rename(&bin_tmp, &bin)?;
        Ok(generation_id)
    }

    /// Converts an existing regular legacy `profile/bin` directory into an
    /// explicitly unverified generation.  No package/runtime evidence is
    /// fabricated; the command links are retained until their artifacts are
    /// reacquired and replanned.
    pub fn publish_legacy(
        layout: &StoreLayout,
        profile: &str,
        transaction_id: &str,
    ) -> Result<String> {
        StoreLayout::validate_profile(profile)?;
        StoreLayout::validate_component(transaction_id, "transaction id")?;
        let profile_dir = layout.profile_dir(profile);
        let generations_dir = layout.profile_generations_dir(profile);
        crate::store::layout::ensure_directory(&profile_dir)?;
        crate::store::layout::ensure_directory(&generations_dir)?;
        let generation_id = format!("legacy-{transaction_id}");
        let generation_dir = generations_dir.join(&generation_id);
        let staging = layout.staging_dir(transaction_id).join("legacy-generation");
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        let staging_bin = staging.join("bin");
        crate::store::layout::ensure_directory(&staging_bin)?;
        if let Ok(entries) = fs::read_dir(layout.profile_bin_dir(profile)) {
            for entry in entries {
                let entry = entry?;
                copy_entry(&entry.path(), &staging_bin.join(entry.file_name()))?;
            }
        }
        let mut commands = std::collections::BTreeMap::new();
        for entry in fs::read_dir(&staging_bin)? {
            let entry = entry?;
            commands.insert(
                entry.file_name().to_string_lossy().into_owned(),
                fs::read_link(entry.path()).unwrap_or_else(|_| entry.path()),
            );
        }
        let generation = ActivationGeneration {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            generation_id: generation_id.clone(),
            profile: profile.to_string(),
            previous_generation: None,
            commands,
            runtimes: Vec::new(),
            owned_paths: Vec::new(),
            verified: false,
        };
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&generation)?,
        )?;
        fs::rename(&staging, &generation_dir)?;

        let current = layout.profile_current_path(profile);
        let current_tmp = profile_dir.join(format!(".current.{transaction_id}"));
        let _ = fs::remove_file(&current_tmp);
        std::os::unix::fs::symlink(Path::new("generations").join(&generation_id), &current_tmp)?;
        if fs::symlink_metadata(&current).is_ok() {
            fs::remove_file(&current)?;
        }
        fs::rename(&current_tmp, &current)?;
        let bin = layout.profile_bin_link(profile);
        if fs::symlink_metadata(&bin).is_ok_and(|m| !m.file_type().is_symlink()) {
            fs::rename(
                &bin,
                profile_dir.join(format!("legacy-bin-{transaction_id}")),
            )?;
        }
        let bin_tmp = profile_dir.join(format!(".bin.{transaction_id}"));
        let _ = fs::remove_file(&bin_tmp);
        std::os::unix::fs::symlink(Path::new("current").join("bin"), &bin_tmp)?;
        fs::rename(&bin_tmp, &bin)?;
        Ok(generation_id)
    }
}

fn copy_entry(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(source)?, destination)?;
    } else if metadata.is_dir() {
        crate::store::layout::ensure_directory(destination)?;
        for child in fs::read_dir(source)? {
            let child = child?;
            copy_entry(&child.path(), &destination.join(child.file_name()))?;
        }
    } else if metadata.is_file() {
        fs::copy(source, destination)?;
        std::fs::set_permissions(destination, metadata.permissions())?;
    } else {
        return Err(Error::SecurityViolation(format!(
            "Unsupported profile entry: {}",
            source.display()
        )));
    }
    Ok(())
}

fn previous_owned_paths(
    layout: &StoreLayout,
    profile: &str,
    generation_id: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let Some(generation_id) = generation_id else {
        return Ok(Vec::new());
    };
    let manifest_path = layout
        .profile_generations_dir(profile)
        .join(generation_id)
        .join("manifest.json");
    if !fs::symlink_metadata(&manifest_path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        return Ok(Vec::new());
    }
    let generation: ActivationGeneration = serde_json::from_slice(&fs::read(manifest_path)?)?;
    if generation.profile != profile || generation.generation_id != generation_id {
        return Err(Error::TransactionRecoveryRequired(
            "previous generation manifest identity mismatch".into(),
        ));
    }
    Ok(generation.owned_paths)
}

/// Carries forward runtime references for every command that remains visible
/// in a new generation.  A generation is a complete command set, so retaining
/// only runtimes created by the current package would make older commands
/// impossible to execute after an unrelated install.
fn complete_runtime_ids<I>(
    layout: &StoreLayout,
    profile: &str,
    previous_generation: Option<&str>,
    commands: &std::collections::BTreeMap<String, PathBuf>,
    replacement_commands: &std::collections::BTreeSet<String>,
    new_runtime_ids: I,
) -> Result<Vec<String>>
where
    I: IntoIterator<Item = String>,
{
    let mut runtime_ids = std::collections::BTreeSet::new();
    if let Some(previous_generation) = previous_generation {
        let manifest_path = layout
            .profile_generations_dir(profile)
            .join(previous_generation)
            .join("manifest.json");
        if fs::symlink_metadata(&manifest_path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            let previous: ActivationGeneration =
                serde_json::from_slice(&fs::read(&manifest_path)?)?;
            for runtime_id in previous.runtimes {
                let runtime = read_runtime_manifest(layout, &runtime_id)?;
                let command = &runtime.execution.command;
                if commands.contains_key(command) && !replacement_commands.contains(command) {
                    runtime_ids.insert(runtime_id);
                }
            }
        }
    }

    for runtime_id in new_runtime_ids {
        let runtime = read_runtime_manifest(layout, &runtime_id)?;
        if !commands.contains_key(&runtime.execution.command) {
            return Err(Error::TransactionRecoveryRequired(format!(
                "runtime command is absent from generation: {}",
                runtime.execution.command
            )));
        }
        runtime_ids.insert(runtime_id);
    }
    Ok(runtime_ids.into_iter().collect())
}

fn read_runtime_manifest(
    layout: &StoreLayout,
    runtime_id: &str,
) -> Result<crate::domain::contracts::RuntimeManifest> {
    let id = runtime_id.trim_start_matches("sha256:");
    StoreLayout::validate_component(id, "runtime id")?;
    let path = layout.runtime_dir(runtime_id).join("manifest.json");
    if !fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
    {
        return Err(Error::TransactionRecoveryRequired(format!(
            "runtime manifest is unavailable: {}",
            path.display()
        )));
    }
    let runtime: crate::domain::contracts::RuntimeManifest =
        serde_json::from_slice(&fs::read(path)?)?;
    if runtime.runtime_id != runtime_id {
        return Err(Error::TransactionRecoveryRequired(format!(
            "runtime manifest identity mismatch: {runtime_id}"
        )));
    }
    Ok(runtime)
}

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
        crate::store::layout::ensure_directory(profile_bin_dir)?;
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
        crate::store::layout::ensure_directory(profile_lib_dir)?;

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
