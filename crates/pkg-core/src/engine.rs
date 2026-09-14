//! Top-level pkg engine coordinating format adapters, store layout,
//! state database, transactions, and binary activations.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::activation::Activator;
use crate::domain::installed::InstalledPackage;
use crate::domain::package::NormalizedPackage;
use crate::domain::plan::{InstallPlan, RemovePlan};
use crate::error::{Error, Result};
use crate::format::deb::DebAdapter;
use crate::format::{ArtifactAdapter, ExtractionLimits};
use crate::host::elf::inspect_elf;
use crate::lock::ProcessLock;
use crate::planner::Planner;
use crate::state::{NewStoreObject, StateDatabase};
use crate::store::StoreLayout;
use crate::transaction::Recovery;

/// Detailed information about a package, either from local artifact or installed state.
#[derive(Debug, Clone)]
pub enum PackageInfo {
    /// Information parsed from a local archive file.
    LocalArtifact(NormalizedPackage),
    /// Information retrieved from the installed state database.
    Installed(InstalledPackage),
}

/// The core package engine.
#[derive(Debug)]
pub struct Engine {
    layout: StoreLayout,
    db: StateDatabase,
}

impl Engine {
    /// Initializes or opens an engine using the provided store layout.
    pub fn open(layout: StoreLayout) -> Result<Self> {
        layout.ensure_dirs()?;
        let db = StateDatabase::open(&layout.db_path())?;

        // Reconcile any interrupted transactions from previous runs (INV-013)
        let recovered = Recovery::reconcile(&layout, &db)?;
        if recovered > 0 {
            tracing::info!("Recovered {recovered} interrupted transaction(s) on startup");
        }

        Ok(Self { layout, db })
    }

    /// Access the underlying store layout.
    #[must_use]
    pub fn layout(&self) -> &StoreLayout {
        &self.layout
    }

    /// Access the underlying state database.
    #[must_use]
    pub fn db(&self) -> &StateDatabase {
        &self.db
    }

    /// Produces an installation plan without mutating disk or state database (INV-010).
    pub fn plan_install(&self, artifact_path: &Path, profile: &str) -> Result<InstallPlan> {
        Planner::plan_install(artifact_path, &self.layout, &self.db, profile, true)
    }

    /// Installs a local package artifact into the store.
    pub fn install(
        &self,
        artifact_path: &Path,
        profile: &str,
        is_dry_run: bool,
    ) -> Result<InstallPlan> {
        if is_dry_run {
            return self.plan_install(artifact_path, profile);
        }

        // Acquire process lock to prevent concurrent state mutations (INV-012)
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;

        // Reconcile transactions under lock
        let _ = Recovery::reconcile(&self.layout, &self.db)?;

        // Plan installation
        let mut plan =
            Planner::plan_install(artifact_path, &self.layout, &self.db, profile, false)?;

        let tx_id = format!(
            "tx-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        // 1. Transaction Planned
        self.db.record_transaction_start(
            &tx_id,
            "install",
            "Planned",
            plan.package.name.as_str(),
            Some(&plan.store_id),
            None,
        )?;

        // 2. Transaction Staging
        let staging_dir = self.layout.staging_dir(&tx_id);
        self.db.update_transaction_phase(&tx_id, "Staging")?;

        let adapter = DebAdapter::new();
        let report =
            adapter.extract_payload(artifact_path, &staging_dir, &ExtractionLimits::default())?;

        // 3. Transaction Prepared (verify ELF binaries and host libraries)
        self.db.update_transaction_phase(&tx_id, "Prepared")?;
        for file in &report.extracted_files {
            let full_path = staging_dir.join(file);
            if let Ok(Some(inspection)) = inspect_elf(&full_path, Some(&staging_dir)) {
                for lib in inspection.resolved_libraries {
                    if !plan.host_libraries_verified.contains(&lib) {
                        plan.host_libraries_verified.push(lib);
                    }
                }
            }
        }

        // 4. Transaction Promoting (atomic rename staging -> store)
        self.db.update_transaction_phase(&tx_id, "Promoting")?;
        self.layout
            .promote_staging(&staging_dir, &plan.target_store_dir)?;

        // 5. Transaction Activating (symlink executables into profile bin)
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        Activator::activate(&plan, &self.layout.profile_bin_dir(profile))?;

        // 6. Transaction Committing (commit state DB records)
        self.db.update_transaction_phase(&tx_id, "Committing")?;
        self.db.record_store_object(&NewStoreObject {
            store_id: &plan.store_id,
            name: &plan.package.name,
            version: &plan.package.version,
            architecture: &plan.package.architecture,
            format: plan.package.format,
            digest: &plan.package.digest,
            store_path: &plan.target_store_dir,
            files: &report.extracted_files,
        })?;

        for bin in &plan.binaries {
            self.db.record_activation(
                profile,
                &bin.command,
                &plan.store_id,
                plan.package.name.as_str(),
                &bin.profile_symlink_path,
            )?;
        }

        self.db.record_package(
            profile,
            &plan.package.name,
            &plan.package.version,
            &plan.store_id,
        )?;

        // 7. Transaction Completed
        self.db.update_transaction_phase(&tx_id, "Completed")?;

        Ok(plan)
    }

    /// Produces a removal plan without mutating disk or state database (INV-010).
    pub fn plan_remove(&self, package_name: &str, profile: &str) -> Result<RemovePlan> {
        Planner::plan_remove(package_name, &self.layout, &self.db, profile, true)
    }

    /// Removes an installed package from a profile and cleans its store object.
    pub fn remove(
        &self,
        package_name: &str,
        profile: &str,
        is_dry_run: bool,
    ) -> Result<RemovePlan> {
        if is_dry_run {
            return self.plan_remove(package_name, profile);
        }

        // Acquire process lock
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;

        // Reconcile transactions under lock
        let _ = Recovery::reconcile(&self.layout, &self.db)?;

        // Plan removal
        let plan = Planner::plan_remove(package_name, &self.layout, &self.db, profile, false)?;

        let tx_id = format!(
            "tx-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        self.db.record_transaction_start(
            &tx_id,
            "remove",
            "Planned",
            package_name,
            Some(&plan.store_id),
            None,
        )?;

        // Deactivate profile binary symlinks
        Activator::deactivate(&plan.binaries_to_remove)?;

        // Remove from database
        self.db.remove_package_from_profile(profile, package_name)?;
        self.db.remove_store_object(&plan.store_id)?;

        // Physical store cleanup
        if plan.store_path.exists() {
            let _ = fs::remove_dir_all(&plan.store_path);
        }

        self.db.update_transaction_phase(&tx_id, "Completed")?;

        Ok(plan)
    }

    /// Lists all installed packages in the specified profile.
    pub fn list(&self, profile: &str) -> Result<Vec<InstalledPackage>> {
        self.db.list_packages(profile)
    }

    /// Inspects package information from either an artifact path or an installed package name.
    pub fn info(&self, name_or_path: &str, profile: &str) -> Result<PackageInfo> {
        let path = Path::new(name_or_path);
        if path.exists() && path.is_file() {
            let adapter = DebAdapter::new();
            let meta = adapter.parse_metadata(path)?;
            Ok(PackageInfo::LocalArtifact(meta))
        } else if let Some(installed) = self.db.get_package(profile, name_or_path)? {
            Ok(PackageInfo::Installed(installed))
        } else {
            Err(Error::PackageNotFound(name_or_path.to_string()))
        }
    }
}
