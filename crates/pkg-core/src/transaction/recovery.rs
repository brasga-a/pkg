//! Startup transaction recovery and state reconciliation (INV-013, ADR-012, Gate M1-C).

use std::fs;

use crate::error::Result;
use crate::state::StateDatabase;
use crate::store::StoreLayout;

/// Reconciles interrupted transactions on startup.
#[derive(Debug)]
pub struct Recovery;

impl Recovery {
    /// Inspects the database and store for incomplete transactions and reconciles them to a clean state.
    pub fn reconcile(layout: &StoreLayout, db: &StateDatabase) -> Result<usize> {
        let incomplete = db.list_incomplete_transactions()?;
        let mut recovered = 0;

        for tx in incomplete {
            StoreLayout::validate_component(&tx.id, "transaction id")?;
            tracing::warn!(
                "Reconciling incomplete transaction '{}' (operation: {}, phase: {})",
                tx.id,
                tx.operation,
                tx.phase
            );

            // A crash after the package/state commit but before the generation
            // row or receipt was written must keep the already-published view.
            // Adopt that complete manifest instead of rolling the filesystem
            // pointer back to a generation whose DB state is no longer true.
            let adopted = adopt_committed_generation(layout, db, &tx)?;
            if adopted && tx.operation == "remove" {
                // A removed package has no remaining logical owner for its
                // host integrations. Clean those links even when the
                // generation itself was successfully adopted.
                reconcile_host_integration_if_committed(db, &tx)?;
            }
            if adopted {
                recovered += 1;
                continue;
            }

            // Clean up staging directory if it exists
            let staging = layout.staging_dir(&tx.id);
            if staging.exists() {
                let _ = fs::remove_dir_all(&staging);
            }

            // If transaction failed before committing logical package state,
            // check if an uncommitted store object directory was created
            if let Some(ref store_id) = tx.store_id {
                let store_path = layout.store_object_dir(store_id);
                // If the package is not registered in the packages table, delete the orphan store tree
                if !db.store_is_referenced_by_committed_state(store_id)? {
                    layout.validate_store_path(&store_path)?;
                    remove_links_to_store(layout, &store_path)?;
                    if store_path.exists() {
                        fs::remove_dir_all(&store_path)?;
                    }
                    db.remove_store_object_after_recovery(store_id)?;
                }
            }

            // A publication can switch filesystem pointers before its SQLite
            // commit.  Remove that uncommitted generation and restore the
            // detached legacy bin snapshot when no durable generation claims
            // it.  This handles the first install as well as upgrades.
            cleanup_uncommitted_generation(layout, db, &tx.id)?;

            restore_active_bin_pointer(layout, db, &tx.id)?;

            // For removal, restore_active_bin_pointer may re-establish the
            // previous package state when the logical DELETE happened before
            // publication. Decide ownership only after that reconciliation so
            // a rolled-back removal keeps its host integrations.
            if matches!(
                tx.operation.as_str(),
                "integrate" | "deintegrate" | "remove"
            ) {
                reconcile_host_integration_if_committed(db, &tx)?;
            }

            db.update_transaction_phase(&tx.id, "FailedClean")?;
            recovered += 1;
        }

        // Reconcile the filesystem generation pointer with the durable DB
        // record.  A crash can occur after either side of the pointer/SQLite
        // boundary; readers must settle on the last generation whose manifest
        // is complete rather than trusting a dangling `current` symlink.
        reconcile_generation_pointers(layout, db)?;
        remove_dangling_store_links(layout)?;

        // A failed upgrade may have switched links before its SQLite commit.
        // Restore links from the last committed activation rows, preserving regular
        // files that the user placed in the profile.
        for (link, target) in db.activation_targets()? {
            if !target.exists() {
                continue;
            }
            match fs::symlink_metadata(&link) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if fs::read_link(&link)? != target {
                        fs::remove_file(&link)?;
                        std::os::unix::fs::symlink(&target, &link)?;
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if let Some(parent) = link.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    std::os::unix::fs::symlink(&target, &link)?;
                }
                Err(error) => return Err(error.into()),
            }
        }

        Ok(recovered)
    }
}

fn reconcile_host_integration_if_committed(
    db: &StateDatabase,
    tx: &crate::state::TransactionRecord,
) -> Result<()> {
    let Some(details) = tx.details.as_deref() else {
        return Ok(());
    };
    let plan: crate::domain::integration::IntegrationPlan = serde_json::from_str(details)?;
    if tx.operation == "remove" && db.get_package(&plan.profile, &plan.package_name)?.is_some() {
        // The logical package state was not committed. Keep the existing
        // integration and let the normal generation recovery restore the
        // transaction's previous view.
        return Ok(());
    }
    let ownership = plan
        .actions
        .iter()
        .map(|action| crate::host::integration::IntegrationOwnership {
            kind: action.kind,
            source_path: action.source_path.clone(),
            target_path: action.target_path.clone(),
            source_digest: action.source_digest.clone(),
        })
        .collect::<Vec<_>>();
    crate::host::integration::remove(&ownership)?;
    for record in db.list_integrations(&plan.profile, Some(&plan.package_name))? {
        db.remove_integration(&plan.profile, &record.target_path)?;
    }
    Ok(())
}

fn restore_active_bin_pointer(
    layout: &StoreLayout,
    db: &StateDatabase,
    transaction_id: &str,
) -> Result<()> {
    let profiles = match fs::read_dir(layout.profiles_root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for profile_entry in profiles {
        let profile_entry = profile_entry?;
        if !profile_entry.file_type()?.is_dir() {
            continue;
        }
        let profile = profile_entry.file_name().to_string_lossy().into_owned();
        if db.active_generation(&profile)?.is_none() {
            continue;
        }
        // The remove path detaches `bin` before mutating SQLite.  If the
        // process stops before a candidate generation is published, the
        // active generation remains the durable source of truth.  Restore its
        // package/activation rows as well as the stable pointer; this covers
        // the window after the package DELETE committed but before publication.
        let active_generation = db
            .active_generation(&profile)?
            .map(|generation| generation.generation_id);
        if let Some(active_generation) = active_generation {
            if fs::symlink_metadata(layout.profile_bin_link(&profile))
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            {
                db.restore_generation_state(&profile, &active_generation)?;
                let bin = layout.profile_bin_link(&profile);
                let retained = layout
                    .profile_dir(&profile)
                    .join(format!("legacy-bin-recovery-{transaction_id}"));
                if !retained.exists() {
                    fs::rename(&bin, &retained)?;
                }
                std::os::unix::fs::symlink(std::path::Path::new("current").join("bin"), &bin)?;
            }
        }
    }
    Ok(())
}

fn adopt_committed_generation(
    layout: &StoreLayout,
    db: &StateDatabase,
    tx: &crate::state::TransactionRecord,
) -> Result<bool> {
    let candidate = format!("gen-{}", tx.id);
    let profiles = match fs::read_dir(layout.profiles_root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    for profile_entry in profiles {
        let profile_entry = profile_entry?;
        if !profile_entry.file_type()?.is_dir() {
            continue;
        }
        let profile = profile_entry.file_name().to_string_lossy().into_owned();
        let generation_dir = layout.profile_generations_dir(&profile).join(&candidate);
        let manifest_path = generation_dir.join("manifest.json");
        if !fs::symlink_metadata(&manifest_path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            continue;
        }
        let current = layout.profile_current_path(&profile);
        let points_to_candidate = fs::read_link(&current)
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy() == candidate)
            })
            .unwrap_or(false);
        if !points_to_candidate {
            continue;
        }
        let committed = match tx.operation.as_str() {
            "install" => tx.store_id.as_deref().is_some_and(|store_id| {
                db.get_package(&profile, &tx.package_name)
                    .ok()
                    .flatten()
                    .is_some_and(|package| package.store_id == store_id)
            }),
            "remove" => db
                .get_package(&profile, &tx.package_name)
                .ok()
                .flatten()
                .is_none(),
            _ => false,
        };
        if !committed {
            continue;
        }
        let staging = layout.staging_dir(&tx.id);
        if staging.exists() {
            fs::remove_dir_all(staging)?;
        }
        let generation: crate::domain::contracts::ActivationGeneration =
            serde_json::from_slice(&fs::read(&manifest_path)?)?;
        let store_ids = db
            .list_packages(&profile)?
            .into_iter()
            .map(|package| package.store_id)
            .collect::<Vec<_>>();
        let refs = store_ids.iter().map(String::as_str).collect::<Vec<_>>();
        db.record_generation(&generation, &manifest_path, &refs)?;
        db.update_transaction_phase(&tx.id, "Completed")?;
        return Ok(true);
    }
    Ok(false)
}

fn cleanup_uncommitted_generation(
    layout: &StoreLayout,
    db: &StateDatabase,
    transaction_id: &str,
) -> Result<()> {
    let candidates = [
        format!("gen-{transaction_id}"),
        format!("legacy-{transaction_id}"),
    ];
    let legacy_name = format!("legacy-bin-{transaction_id}");
    let profiles = match fs::read_dir(layout.profiles_root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for profile_entry in profiles {
        let profile_entry = profile_entry?;
        if !profile_entry.file_type()?.is_dir() {
            continue;
        }
        let profile = profile_entry.file_name().to_string_lossy().into_owned();
        let Some((candidate, generation_dir)) = candidates.iter().find_map(|candidate| {
            let path = layout.profile_generations_dir(&profile).join(candidate);
            path.exists().then_some((candidate.as_str(), path))
        }) else {
            continue;
        };
        let keep = db
            .active_generation(&profile)?
            .is_some_and(|active| active.generation_id == candidate);
        if keep {
            continue;
        }
        let active = db.active_generation(&profile)?;
        let current = layout.profile_current_path(&profile);
        let points_to_candidate = fs::read_link(&current)
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy() == candidate)
            })
            .unwrap_or(false);
        if points_to_candidate {
            fs::remove_file(&current)?;
        }
        fs::remove_dir_all(&generation_dir)?;

        let bin = layout.profile_bin_link(&profile);
        let legacy = layout.profile_dir(&profile).join(&legacy_name);
        if let Some(active) = active {
            // The old generation is authoritative.  Recreate both stable
            // pointers from it and discard the detached copy used by the
            // failed transaction.
            let current_name = fs::read_link(&current).ok().and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
            if current_name.as_deref() != Some(active.generation_id.as_str()) {
                let _ = fs::remove_file(&current);
                let tmp = layout
                    .profile_dir(&profile)
                    .join(format!(".current.recovery-{transaction_id}"));
                let _ = fs::remove_file(&tmp);
                std::os::unix::fs::symlink(
                    std::path::Path::new("generations").join(&active.generation_id),
                    &tmp,
                )?;
                fs::rename(tmp, &current)?;
            }
            if let Ok(metadata) = fs::symlink_metadata(&bin) {
                if metadata.file_type().is_symlink() {
                    fs::remove_file(&bin)?;
                } else if metadata.is_dir() {
                    fs::remove_dir_all(&bin)?;
                } else {
                    fs::remove_file(&bin)?;
                }
            }
            std::os::unix::fs::symlink(std::path::Path::new("current").join("bin"), &bin)?;
            if legacy.exists() {
                fs::remove_dir_all(legacy)?;
            }
        } else if legacy.exists() {
            if fs::symlink_metadata(&bin).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                fs::remove_file(&bin)?;
            }
            if !bin.exists() {
                fs::rename(legacy, bin)?;
            }
        }
    }
    Ok(())
}

fn reconcile_generation_pointers(layout: &StoreLayout, db: &StateDatabase) -> Result<()> {
    let profiles = match fs::read_dir(layout.profiles_root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for profile_entry in profiles {
        let profile_entry = profile_entry?;
        if !profile_entry.file_type()?.is_dir() {
            continue;
        }
        let profile = profile_entry.file_name().to_string_lossy().into_owned();
        StoreLayout::validate_profile(&profile)?;
        let Some(active) = db.active_generation(&profile)? else {
            continue;
        };
        let generation_dir = layout
            .profile_generations_dir(&profile)
            .join(&active.generation_id);
        let manifest = generation_dir.join("manifest.json");
        if !fs::symlink_metadata(&manifest)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            return Err(crate::error::Error::TransactionRecoveryRequired(format!(
                "Active generation manifest is missing: {}",
                manifest.display()
            )));
        }
        let current = layout.profile_current_path(&profile);
        let current_name = fs::read_link(&current).ok().and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        if current_name.as_deref() != Some(active.generation_id.as_str()) {
            let temp = layout
                .profile_dir(&profile)
                .join(format!(".current.recovery-{}", active.generation_id));
            let _ = fs::remove_file(&temp);
            std::os::unix::fs::symlink(
                std::path::Path::new("generations").join(&active.generation_id),
                &temp,
            )?;
            fs::rename(temp, &current)?;
        }
        let bin = layout.profile_bin_link(&profile);
        let bin_target = std::path::Path::new("current").join("bin");
        match fs::symlink_metadata(&bin) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if fs::read_link(&bin)? != bin_target {
                    fs::remove_file(&bin)?;
                    std::os::unix::fs::symlink(&bin_target, &bin)?;
                }
            }
            Ok(_) => {
                return Err(crate::error::Error::TransactionRecoveryRequired(format!(
                    "Profile bin path is unmanaged: {}",
                    bin.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::os::unix::fs::symlink(&bin_target, &bin)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Removes only dangling symlinks whose literal target is inside pkg's own
/// store.  This covers a failed publication that switched a generation before
/// its store object was rolled back; unmanaged host paths remain untouched.
fn remove_dangling_store_links(layout: &StoreLayout) -> Result<()> {
    let profiles = match fs::read_dir(layout.profiles_root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for profile in profiles {
        let profile = profile?;
        let bin = profile.path().join("bin");
        let entries = match fs::read_dir(&bin) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !fs::symlink_metadata(&path)?.file_type().is_symlink() {
                continue;
            }
            let target = fs::read_link(&path)?;
            if target.starts_with(layout.store_dir()) && !target.exists() {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

/// Removes links created for an uncommitted store object, including binaries
/// that did not exist in the previously committed package version.
fn remove_links_to_store(layout: &StoreLayout, store_path: &std::path::Path) -> Result<()> {
    let profiles = layout.profiles_root();
    let entries = match fs::read_dir(&profiles) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for profile in entries {
        let profile = profile?;
        let bin_dir = profile.path().join("bin");
        let links = match fs::read_dir(&bin_dir) {
            Ok(links) => links,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for link in links {
            let link = link?;
            let path = link.path();
            if fs::symlink_metadata(&path)?.file_type().is_symlink()
                && fs::read_link(&path)
                    .map(|target| target == store_path || target.starts_with(store_path))
                    .unwrap_or(false)
            {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::integration::{IntegrationAction, IntegrationKind, IntegrationPlan};
    use crate::domain::package::{
        Architecture, ArtifactDigest, PackageFormat, PackageName, PackageVersion,
    };
    use crate::state::{NewStoreObject, TransactionRecord};
    use tempfile::tempdir;

    #[test]
    fn recovers_host_link_after_interrupted_integration() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();

        let source = temp
            .path()
            .join("store-object/usr/share/applications/tool.desktop");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            b"[Desktop Entry]\nType=Application\nName=Tool\nExec=tool\n",
        )
        .unwrap();
        let target = temp.path().join("xdg/applications/tool.desktop");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&source, &target).unwrap();

        let plan = IntegrationPlan {
            profile: "default".into(),
            package_name: "tool".into(),
            store_id: "store-id".into(),
            actions: vec![IntegrationAction {
                kind: IntegrationKind::DesktopEntry,
                source_path: fs::canonicalize(&source).unwrap(),
                target_path: target.clone(),
                source_digest: ArtifactDigest::sha256("deadbeef"),
            }],
            conflicts: Vec::new(),
        };
        let details = serde_json::to_string(&plan).unwrap();
        db.record_transaction_start(
            "tx-integrate-recovery",
            "integrate",
            "Integrated",
            "tool",
            None,
            Some(&details),
        )
        .unwrap();

        let recovered = Recovery::reconcile(&layout, &db).unwrap();
        assert_eq!(recovered, 1);
        assert!(!target.exists());
        assert!(db.list_incomplete_transactions().unwrap().is_empty());
    }

    #[test]
    fn recovers_host_link_after_committed_remove_boundary() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();

        let source = temp
            .path()
            .join("store-object/usr/share/applications/tool.desktop");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            b"[Desktop Entry]\nType=Application\nName=Tool\nExec=tool\n",
        )
        .unwrap();
        let target = temp.path().join("xdg/applications/tool.desktop");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&source, &target).unwrap();

        let plan = IntegrationPlan {
            profile: "default".into(),
            package_name: "tool".into(),
            store_id: "store-id".into(),
            actions: vec![IntegrationAction {
                kind: IntegrationKind::DesktopEntry,
                source_path: fs::canonicalize(&source).unwrap(),
                target_path: target.clone(),
                source_digest: ArtifactDigest::sha256("deadbeef"),
            }],
            conflicts: Vec::new(),
        };
        let details = serde_json::to_string(&plan).unwrap();
        db.record_transaction_start(
            "tx-remove-recovery",
            "remove",
            "Activating",
            "tool",
            None,
            Some(&details),
        )
        .unwrap();

        let recovered = Recovery::reconcile(&layout, &db).unwrap();
        assert_eq!(recovered, 1);
        assert!(!target.exists());
        assert!(db.list_incomplete_transactions().unwrap().is_empty());
    }

    #[test]
    fn keeps_host_link_when_remove_did_not_commit_package_state() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();

        let source = temp
            .path()
            .join("store-object/usr/share/applications/tool.desktop");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            b"[Desktop Entry]\nType=Application\nName=Tool\nExec=tool\n",
        )
        .unwrap();
        let target = temp.path().join("xdg/applications/tool.desktop");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&source, &target).unwrap();

        let name = PackageName::new("tool").unwrap();
        let version = PackageVersion::new("1.0.0");
        let store_path = temp.path().join("store-object");
        db.record_store_object(&NewStoreObject {
            store_id: "store-id",
            name: &name,
            version: &version,
            architecture: &Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: &ArtifactDigest::sha256("artifact"),
            store_path: &store_path,
            files: &[std::path::PathBuf::from(
                "usr/share/applications/tool.desktop",
            )],
        })
        .unwrap();
        db.record_package("default", &name, &version, "store-id")
            .unwrap();

        let plan = IntegrationPlan {
            profile: "default".into(),
            package_name: "tool".into(),
            store_id: "store-id".into(),
            actions: vec![IntegrationAction {
                kind: IntegrationKind::DesktopEntry,
                source_path: fs::canonicalize(&source).unwrap(),
                target_path: target.clone(),
                source_digest: ArtifactDigest::sha256("deadbeef"),
            }],
            conflicts: Vec::new(),
        };
        let tx = TransactionRecord {
            id: "tx-remove-before-commit".into(),
            operation: "remove".into(),
            phase: "Activating".into(),
            package_name: "tool".into(),
            store_id: Some("store-id".into()),
            created_at: String::new(),
            updated_at: String::new(),
            details: Some(serde_json::to_string(&plan).unwrap()),
        };

        reconcile_host_integration_if_committed(&db, &tx).unwrap();
        assert!(target.exists());
    }
}
