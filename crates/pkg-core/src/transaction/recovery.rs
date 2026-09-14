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
                if !db.store_is_referenced(store_id)? {
                    layout.validate_store_path(&store_path)?;
                    remove_links_to_store(layout, &store_path)?;
                    if store_path.exists() {
                        fs::remove_dir_all(&store_path)?;
                    }
                    db.remove_store_object(store_id)?;
                }
            }

            db.update_transaction_phase(&tx.id, "FailedClean")?;
            recovered += 1;
        }

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
