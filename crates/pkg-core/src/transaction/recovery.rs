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
                if db.get_package("default", &tx.package_name)?.is_none() && store_path.exists() {
                    let _ = fs::remove_dir_all(&store_path);
                    let _ = db.remove_store_object(store_id);
                }
            }

            db.update_transaction_phase(&tx.id, "FailedClean")?;
            recovered += 1;
        }

        Ok(recovered)
    }
}
