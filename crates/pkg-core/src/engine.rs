//! Top-level pkg engine coordinating format adapters, store layout,
//! state database, transactions, and binary activations.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::activation::Activator;
use crate::domain::installed::InstalledPackage;
use crate::domain::package::{ArtifactDigest, NormalizedPackage};
use crate::domain::plan::{InstallPlan, RemovePlan};
use crate::error::{Error, Result};
use crate::format::ExtractionLimits;
use crate::host::elf::inspect_elf_with_extra_paths;
use crate::lock::ProcessLock;
use crate::planner::Planner;
use crate::state::StateDatabase;
use crate::store::StoreLayout;
use crate::transaction::Recovery;

/// Result of resolving a package target specification against the remote catalog.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteResolution {
    /// Exactly one package was matched (either naturally or via priority resolution).
    Exact(crate::domain::package::RemotePackage),
    /// Multiple packages matched across different repositories or formats.
    Ambiguous(Vec<crate::domain::package::RemotePackage>),
    /// No matching packages were found.
    NotFound,
}

/// One installed package with the best newer candidate from the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpgradeCandidate {
    pub installed: InstalledPackage,
    pub candidate: crate::domain::package::RemotePackage,
}

/// Detailed information about a package, either from local artifact or installed state.
#[derive(Debug, Clone, Serialize)]
pub enum PackageInfo {
    /// Information parsed from a local archive file.
    LocalArtifact(NormalizedPackage),
    /// Information retrieved from the installed state database.
    Installed(InstalledPackage),
}

/// Options for configuring package installation.
#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// If true, missing shared libraries detected in ELF binaries will not abort installation.
    pub allow_missing_libraries: bool,
}

/// Report returned by preflight inspection of an uninstalled package artifact.
#[derive(Debug, Clone, Serialize)]
pub struct PreflightReport {
    /// Parsed normalized metadata of the package.
    pub package: NormalizedPackage,
    /// Unresolved host dynamic shared libraries required by ELF executables.
    pub missing_libraries: Vec<String>,
}

/// The core package engine.
#[derive(Debug)]
pub struct Engine {
    layout: StoreLayout,
    db: StateDatabase,
    failpoint: Option<String>,
}

/// Cleans transaction-owned staging when an install exits with an error.
/// Promoted objects are left for the normal recovery pass, which has the
/// complete receipt and can distinguish committed references from orphans.
struct InstallFailureGuard<'a> {
    db: &'a StateDatabase,
    transaction_id: String,
    staging_dir: PathBuf,
    armed: bool,
}

impl<'a> InstallFailureGuard<'a> {
    fn new(db: &'a StateDatabase, transaction_id: String, staging_dir: PathBuf) -> Self {
        Self {
            db,
            transaction_id,
            staging_dir,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for InstallFailureGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.staging_dir.exists() {
            let _ = fs::remove_dir_all(&self.staging_dir);
        }
        let _ = self
            .db
            .update_transaction_phase(&self.transaction_id, "Failed");
    }
}

impl Engine {
    /// Initializes or opens an engine using the provided store layout.
    pub fn open(layout: StoreLayout) -> Result<Self> {
        Self::open_with_failpoint(layout, None)
    }

    /// Opens an engine with an optional deterministic phase failpoint.
    ///
    /// This hook is intentionally explicit and is used by recovery tests to
    /// exercise the crash boundaries without relying on process termination.
    pub fn open_with_failpoint(layout: StoreLayout, failpoint: Option<&str>) -> Result<Self> {
        let _lock = ProcessLock::acquire(&layout.lock_path())?;
        layout.ensure_dirs()?;
        let db = StateDatabase::open(&layout.db_path())?;

        // Reconcile any interrupted transactions from previous runs (INV-013)
        let recovered = Recovery::reconcile(&layout, &db)?;
        if recovered > 0 {
            tracing::info!("Recovered {recovered} interrupted transaction(s) on startup");
        }

        Ok(Self {
            layout,
            db,
            failpoint: failpoint.map(str::to_string),
        })
    }

    /// Opens an engine for a read-only plan. Existing state is opened read-only;
    /// a missing state database uses an in-memory schema, so an empty data root
    /// is never initialized merely to produce a preview.
    pub fn open_for_dry_run(layout: StoreLayout) -> Result<Self> {
        let db = if layout.db_path().is_file() {
            StateDatabase::open_read_only(&layout.db_path())?
        } else {
            StateDatabase::open_in_memory()?
        };
        Ok(Self {
            layout,
            db,
            failpoint: None,
        })
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

    fn maybe_failpoint(&self, phase: &str) -> Result<()> {
        if self.failpoint.as_deref() == Some(phase) {
            return Err(Error::Internal(format!(
                "deterministic transaction failpoint reached: {phase}"
            )));
        }
        Ok(())
    }

    /// Inspects pkg-owned state without changing the store or database.
    pub fn doctor(&self, profile: Option<&str>) -> Result<crate::doctor::DoctorReport> {
        crate::doctor::Doctor::inspect(&self.layout, &self.db, profile)
    }

    /// Repairs only recoverable pkg-owned transaction state, then returns a
    /// fresh diagnostic report. Recovery never deletes paths whose ownership
    /// cannot be proved by the durable transaction record.
    pub fn doctor_repair(&self, profile: Option<&str>) -> Result<crate::doctor::DoctorReport> {
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        crate::doctor::Doctor::inspect(&self.layout, &self.db, profile)
    }

    /// Plans explicit user-space desktop, icon and MIME integrations for an
    /// installed package without touching the host.
    pub fn plan_integrations(
        &self,
        profile: &str,
        package_name: &str,
    ) -> Result<crate::domain::integration::IntegrationPlan> {
        let package = self
            .db
            .get_package(profile, package_name)?
            .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;
        crate::host::integration::plan(
            profile,
            package.name.as_str(),
            &package.store_id,
            &package.store_path,
        )
    }

    /// Materializes typed, reversible user-space integration links.
    pub fn integrate(
        &self,
        profile: &str,
        package_name: &str,
        is_dry_run: bool,
    ) -> Result<crate::domain::integration::IntegrationPlan> {
        if is_dry_run {
            return self.plan_integrations(profile, package_name);
        }
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        let plan = self.plan_integrations(profile, package_name)?;
        let tx_id = format!(
            "tx-integrate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let plan_details = serde_json::to_string(&plan)?;
        self.db.record_transaction_start(
            &tx_id,
            "integrate",
            "Planned",
            package_name,
            Some(&plan.store_id),
            Some(&plan_details),
        )?;
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        self.maybe_failpoint("Integrating")?;
        let _created = match crate::host::integration::apply(&plan) {
            Ok(created) => created,
            Err(error) => {
                self.db.update_transaction_phase(&tx_id, "Failed")?;
                return Err(error);
            }
        };
        self.maybe_failpoint("Integrated")?;
        for action in &plan.actions {
            self.db
                .record_integration(&crate::state::IntegrationRecord {
                    profile: profile.to_string(),
                    package_name: package_name.to_string(),
                    store_id: plan.store_id.clone(),
                    kind: action.kind.as_str().to_string(),
                    source_path: action.source_path.clone(),
                    target_path: action.target_path.clone(),
                    source_digest: action.source_digest.to_string(),
                })?;
        }
        self.db.update_transaction_phase(&tx_id, "Completed")?;
        Ok(plan)
    }

    /// Removes only integration links owned by a package and preserves a
    /// user replacement as an explicit conflict.
    pub fn deintegrate(
        &self,
        profile: &str,
        package_name: &str,
        is_dry_run: bool,
    ) -> Result<crate::domain::integration::IntegrationPlan> {
        if is_dry_run {
            let package = self
                .db
                .get_package(profile, package_name)?
                .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;
            let records = self.db.list_integrations(profile, Some(package_name))?;
            let plan =
                integration_plan_from_records(profile, package_name, &package.store_id, records)?;
            return Ok(plan);
        }
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        let package = self
            .db
            .get_package(profile, package_name)?
            .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;
        let records = self.db.list_integrations(profile, Some(package_name))?;
        let plan = integration_plan_from_records(
            profile,
            package_name,
            &package.store_id,
            records.clone(),
        )?;
        let ownership = records
            .iter()
            .map(integration_ownership)
            .collect::<Result<Vec<_>>>()?;
        let tx_id = format!(
            "tx-deintegrate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let plan_details = serde_json::to_string(&plan)?;
        self.db.record_transaction_start(
            &tx_id,
            "deintegrate",
            "Planned",
            package_name,
            Some(&plan.store_id),
            Some(&plan_details),
        )?;
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        self.maybe_failpoint("Deintegrating")?;
        if let Err(error) = crate::host::integration::remove(&ownership) {
            self.db.update_transaction_phase(&tx_id, "Failed")?;
            return Err(error);
        }
        self.maybe_failpoint("Deintegrated")?;
        for record in records {
            self.db.remove_integration(profile, &record.target_path)?;
        }
        self.db.update_transaction_phase(&tx_id, "Completed")?;
        Ok(plan)
    }

    /// Creates an empty, isolated profile for a task or user workflow.
    pub fn create_profile(&self, profile: &str) -> Result<()> {
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        StoreLayout::validate_profile(profile)?;
        crate::store::layout::ensure_directory(&self.layout.profiles_root())?;
        crate::store::layout::ensure_directory(&self.layout.profile_dir(profile))?;
        crate::store::layout::ensure_directory(&self.layout.profile_generations_dir(profile))?;
        Ok(())
    }

    /// Lists profiles found in the managed filesystem or committed database.
    pub fn list_profiles(&self) -> Result<Vec<String>> {
        let mut profiles = std::collections::BTreeSet::new();
        for profile in self.db.list_profiles()? {
            profiles.insert(profile);
        }
        if let Ok(entries) = fs::read_dir(self.layout.profiles_root()) {
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    StoreLayout::validate_profile(&name)?;
                    profiles.insert(name);
                }
            }
        }
        Ok(profiles.into_iter().collect())
    }

    /// Drops a profile after proving that every remaining entry is pkg-owned.
    /// Unknown regular files cause a hard failure and are never deleted.
    pub fn drop_profile(&self, profile: &str) -> Result<()> {
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        StoreLayout::validate_profile(profile)?;
        for transaction in self.db.list_incomplete_transactions()? {
            if transaction.package_name == profile {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "profile '{profile}' has incomplete transaction '{}'",
                    transaction.id
                )));
            }
        }
        let profile_dir = self.layout.profile_dir(profile);
        if !profile_dir.exists() {
            self.db.remove_profile_state(profile)?;
            return Ok(());
        }
        validate_profile_tree_for_removal(&profile_dir)?;
        fs::remove_dir_all(&profile_dir)?;
        self.db.remove_profile_state(profile)?;
        self.prune_orphan_runtimes()?;
        Ok(())
    }

    fn prune_orphan_runtimes(&self) -> Result<()> {
        for (runtime_id, manifest_path) in self.db.unreferenced_runtime_manifests()? {
            let expected = self.layout.runtime_dir(&runtime_id).join("manifest.json");
            if manifest_path != expected {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest path mismatch: {}",
                    manifest_path.display()
                )));
            }
            let runtime_dir = self.layout.runtime_dir(&runtime_id);
            if let Ok(metadata) = fs::symlink_metadata(&runtime_dir) {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(Error::SecurityViolation(format!(
                        "invalid runtime directory: {}",
                        runtime_dir.display()
                    )));
                }
                fs::remove_dir_all(&runtime_dir)?;
            }
            self.db.remove_runtime_manifest(&runtime_id)?;
        }
        Ok(())
    }

    /// Produces an installation plan without mutating disk or state database (INV-010).
    pub fn plan_install(&self, artifact_path: &Path, profile: &str) -> Result<InstallPlan> {
        self.plan_install_with_replacements(artifact_path, profile, &[])
    }

    /// Produces an installation plan while treating selected installed names
    /// as replacements in the same upgrade transaction.
    pub fn plan_install_with_replacements(
        &self,
        artifact_path: &Path,
        profile: &str,
        replaced_packages: &[crate::domain::package::PackageName],
    ) -> Result<InstallPlan> {
        Planner::plan_install_with_replacements(
            artifact_path,
            &self.layout,
            &self.db,
            profile,
            true,
            replaced_packages,
        )
    }

    /// Performs a preflight inspection on an artifact without mutating state or the store.
    /// Uses the default profile for library resolution.
    pub fn preflight_check(&self, artifact_path: &Path) -> Result<PreflightReport> {
        self.preflight_check_with_profile(artifact_path, "default")
    }

    /// Performs a preflight inspection on an artifact for a specific profile.
    pub fn preflight_check_with_profile(
        &self,
        artifact_path: &Path,
        profile: &str,
    ) -> Result<PreflightReport> {
        let format = crate::format::detect_format(artifact_path)?;
        let adapter = crate::format::get_adapter(format);
        let package = adapter.parse_metadata(artifact_path)?;

        let temp_dir = tempfile::tempdir()?;
        let report = adapter.extract_payload(
            artifact_path,
            temp_dir.path(),
            &ExtractionLimits::default(),
        )?;

        let extra_lib_dirs = self.profile_lib_search_paths(profile)?;
        let mut missing_libraries = Vec::new();
        for file in &report.extracted_files {
            let full_path = temp_dir.path().join(file);
            if let Some(inspection) =
                inspect_elf_with_extra_paths(&full_path, Some(temp_dir.path()), &extra_lib_dirs)?
            {
                for lib in inspection.missing_libraries {
                    if !missing_libraries.contains(&lib) {
                        missing_libraries.push(lib);
                    }
                }
            }
        }

        Ok(PreflightReport {
            package,
            missing_libraries,
        })
    }

    /// Installs a local package artifact into the store with customized options.
    pub fn install_with_options(
        &self,
        artifact_path: &Path,
        profile: &str,
        is_dry_run: bool,
        options: InstallOptions,
    ) -> Result<InstallPlan> {
        self.install_with_options_replacing(artifact_path, profile, is_dry_run, options, &[])
    }

    /// Installs an artifact while treating selected installed names as
    /// replacements in the same transaction.
    pub fn install_with_options_replacing(
        &self,
        artifact_path: &Path,
        profile: &str,
        is_dry_run: bool,
        options: InstallOptions,
        replaced_packages: &[crate::domain::package::PackageName],
    ) -> Result<InstallPlan> {
        if is_dry_run {
            return self.plan_install_with_replacements(artifact_path, profile, replaced_packages);
        }

        // Acquire process lock to prevent concurrent state mutations (INV-012)
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;

        // Reconcile transactions under lock
        let _ = Recovery::reconcile(&self.layout, &self.db)?;

        // Plan installation
        let mut plan = Planner::plan_install_with_replacements(
            artifact_path,
            &self.layout,
            &self.db,
            profile,
            false,
            replaced_packages,
        )?;
        let old_pkg = self.db.get_package(profile, plan.package.name.as_str())?;
        let old_binaries = self
            .db
            .get_activated_binaries(profile, plan.package.name.as_str())?;

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
        // Persist the receipt before creating staging files.  SQLite and the
        // filesystem are separate durability domains; this record freezes the
        // plan identity and previous generation so startup recovery can make
        // a decision even if the process stops during extraction or
        // promotion.
        let previous_generation = self
            .db
            .active_generation(profile)?
            .map(|generation| generation.generation_id);
        let planned_receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "install".into(),
            plan_identity: crate::domain::contracts::digest_serialized(&plan),
            previous_generation,
            new_generation: None,
            created_objects: vec![plan.store_id.clone()],
            reused_objects: Vec::new(),
            expected_digests: std::collections::BTreeMap::new(),
            phase: "Planned".into(),
        };
        self.db
            .record_transaction_receipt(&planned_receipt, "install planned")?;

        // 2. Transaction Staging
        let staging_dir = self.layout.staging_dir(&tx_id);
        let failure_guard = InstallFailureGuard::new(&self.db, tx_id.clone(), staging_dir.clone());
        self.db.update_transaction_phase(&tx_id, "Staging")?;
        self.maybe_failpoint("Staging")?;

        let format = crate::format::detect_format(artifact_path)?;
        let adapter = crate::format::get_adapter(format);
        let mut report =
            adapter.extract_payload(artifact_path, &staging_dir, &ExtractionLimits::default())?;

        // 3. Transaction Prepared (verify ELF binaries and host libraries)
        self.db.update_transaction_phase(&tx_id, "Prepared")?;
        self.maybe_failpoint("Prepared")?;
        let extra_lib_dirs = self.profile_lib_search_paths(profile)?;
        for file in &report.extracted_files {
            let full_path = staging_dir.join(file);
            if let Some(inspection) =
                inspect_elf_with_extra_paths(&full_path, Some(&staging_dir), &extra_lib_dirs)?
            {
                if !inspection.missing_libraries.is_empty() {
                    if !options.allow_missing_libraries {
                        return Err(Error::IncompatibleHost(format!(
                            "{} requires missing libraries: {}",
                            file.display(),
                            inspection.missing_libraries.join(", ")
                        )));
                    } else {
                        for lib in &inspection.missing_libraries {
                            if !plan.missing_libraries.contains(lib) {
                                plan.missing_libraries.push(lib.clone());
                            }
                        }
                    }
                }
                for lib in inspection.resolved_libraries {
                    if !plan.host_libraries_verified.contains(&lib) {
                        plan.host_libraries_verified.push(lib);
                    }
                }
            }
        }

        // Relocate FHS paths in text files (e.g. shell scripts, .desktop, .service)
        let relocation = crate::domain::relocation::relocate_extracted_text_files_with_report(
            &staging_dir,
            &plan.target_store_dir,
            &report.extracted_files,
        )?;
        for generated in &relocation.generated_files {
            if !report.extracted_files.contains(generated) {
                report.extracted_files.push(generated.clone());
            }
        }
        if !relocation.modified_files.is_empty() {
            plan.adaptations
                .push(crate::domain::contracts::AdaptationPlan {
                    recipe_id: "pkg.text-fhs-relocation".into(),
                    recipe_version: "1.0.0".into(),
                    preconditions: vec![
                    "UTF-8 text payload; every rewritten FHS component exists in the same payload"
                        .into(),
                ],
                    inputs: relocation.modified_files.clone(),
                    outputs: relocation.modified_files.clone(),
                    postconditions: vec![
                        "Only indexed package FHS references were rewritten".into(),
                    ],
                });
        }
        if !relocation.generated_files.is_empty() {
            let inputs = relocation
                .generated_files
                .iter()
                .map(|path| {
                    let mut source = path.clone();
                    source.set_extension("ucf");
                    source
                })
                .collect();
            plan.adaptations
                .push(crate::domain::contracts::AdaptationPlan {
                    recipe_id: "pkg.ucf-default".into(),
                    recipe_version: "1.0.0".into(),
                    preconditions: vec![
                        "The exact sibling .ucf source exists and the destination is absent".into(),
                    ],
                    inputs,
                    outputs: relocation.generated_files.clone(),
                    postconditions: vec![
                        "Only the exact sibling default file was materialized".into(),
                    ],
                });
        }

        // Adapt runtime environment and prepare launchers for scripts (e.g. Python PYTHONPATH/shebang normalization)
        let module_dirs = crate::runtime::detect_python_module_dirs(&report.extracted_files);
        let launcher_plans = crate::runtime::prepare_launchers_with_plans(
            &staging_dir,
            &mut plan.binaries,
            &mut report.extracted_files,
        )?;
        for launcher in launcher_plans.values() {
            let input = launcher.target_script_rel_path.clone();
            let output = PathBuf::from(".pkg-launcher").join(&launcher.command);
            plan.adaptations.push(crate::domain::contracts::AdaptationPlan {
                recipe_id: "pkg.interpreter-adapter.v1".into(),
                recipe_version: "1.0.0".into(),
                preconditions: vec![
                    "script has a supported shebang and a host interpreter with a stable absolute path".into(),
                ],
                inputs: vec![input],
                outputs: vec![output],
                postconditions: vec![format!(
                    "interpreter={} args={:?} environment-scoped",
                    launcher.interpreter, launcher.interpreter_args
                )],
            });
        }

        // Freeze the per-command execution contracts before promotion.  These
        // manifests are evidence for recovery and future generations; they
        // are never inferred after activation.
        let (mut executions, mut runtimes) = crate::runtime::build_execution_contracts(
            &staging_dir,
            &plan.target_store_dir,
            &plan.binaries,
            &extra_lib_dirs,
            &module_dirs,
        )?;
        for execution in &mut executions {
            execution.adaptations = plan.adaptations.clone();
        }
        for runtime in &mut runtimes {
            runtime.execution.adaptations = plan.adaptations.clone();
            runtime.runtime_id = crate::runtime::runtime_identity(runtime);
        }

        // Compute retention roots before the native-runner transformation.
        // The runner configuration embeds the final runtime ID, so every
        // field that participates in that ID (including indirect providers)
        // must be present before the runner is written into staging.
        for runtime in &mut runtimes {
            let mut references = runtime
                .references
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            for provider in runtime.library_view.values() {
                let promoted = provider
                    .strip_prefix(&staging_dir)
                    .map(|relative| plan.target_store_dir.join(relative))
                    .unwrap_or_else(|_| provider.clone());
                if let Some(root) = store_object_root_for_path(&promoted, &self.layout.store_dir())
                {
                    references.insert(root.to_string_lossy().into_owned());
                }
            }
            runtime.references = references.into_iter().collect();
            runtime.runtime_id = crate::runtime::runtime_identity(runtime);
        }

        // Native-runner commands get a deterministic profile entrypoint that
        // applies their own runtime view.  The runtime identity is derived
        // from the final entrypoint contract and the exact static bootstrap
        // bytes; the sidecar path is materialization detail only.
        for runtime in &mut runtimes {
            if matches!(
                runtime.execution.strategy,
                crate::domain::contracts::LaunchStrategy::NativeRunner
            ) {
                let target = runtime.execution.executable.clone();
                let launcher_rel = PathBuf::from(".pkg-launcher")
                    .join(format!("{}-native", runtime.execution.command));
                runtime.execution.executable = launcher_rel.clone();
                // The launcher itself is the static bootstrap.  Keeping a
                // shell interpreter here would load a dynamic host program
                // before the runner can sanitize loader controls.
                runtime.execution.interpreter = None;
                runtime.runner_target = Some(target.clone());
                runtime.runner_digest = Some(crate::domain::contracts::digest_bytes(
                    crate::runtime::native_runner_bytes(),
                ));
                runtime.runtime_id = crate::runtime::runtime_identity(runtime);
                let launcher = staging_dir.join(&launcher_rel);
                if let Some(parent) = launcher.parent() {
                    fs::create_dir_all(parent)?;
                }
                crate::runtime::write_native_runner(
                    &launcher,
                    &target,
                    &self.layout.runtime_lib_dir(&runtime.runtime_id),
                )?;
                if !report.extracted_files.contains(&launcher_rel) {
                    report.extracted_files.push(launcher_rel.clone());
                }
                let config_rel = launcher_rel.with_extension("conf");
                if !report.extracted_files.contains(&config_rel) {
                    report.extracted_files.push(config_rel);
                }
                if let Some(binary) = plan
                    .binaries
                    .iter_mut()
                    .find(|binary| binary.command == runtime.execution.command)
                {
                    binary.relative_store_path = launcher_rel;
                }
            }
        }
        executions = runtimes
            .iter()
            .map(|runtime| runtime.execution.clone())
            .collect();
        plan.executions = executions;
        plan.runtimes = runtimes;

        // Freeze the realized payload and static ELF evidence after all
        // generated entrypoints have been inventoried.
        let mut payload_manifest =
            crate::domain::contracts::PayloadManifest::from_tree(&staging_dir)
                .map_err(Error::Io)?;
        for relative in &report.extracted_files {
            let path = staging_dir.join(relative);
            if let Some(inspection) =
                inspect_elf_with_extra_paths(&path, Some(&staging_dir), &extra_lib_dirs)?
            {
                payload_manifest
                    .elf_evidence
                    .push(crate::domain::contracts::ElfManifestEvidence {
                        relative_path: relative.clone(),
                        interpreter: inspection.interpreter,
                        machine: inspection.machine,
                        class_bits: inspection.class_bits,
                        little_endian: inspection.little_endian,
                        needed_libraries: inspection.needed_libraries,
                        soname: inspection.soname,
                        symbol_versions: inspection.symbol_versions,
                        defined_symbol_versions: inspection.defined_symbol_versions,
                    });
            }
        }
        payload_manifest
            .elf_evidence
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for adaptation in &plan.adaptations {
            for output in &adaptation.outputs {
                if let Some(entry) = payload_manifest
                    .entries
                    .iter_mut()
                    .find(|entry| &entry.relative_path == output)
                {
                    entry.generated_by = Some(adaptation.recipe_id.clone());
                }
            }
        }
        for entry in &mut payload_manifest.entries {
            let path = entry.relative_path.to_string_lossy();
            if path.starts_with(".pkg-launcher/") {
                entry.generated_by = Some(
                    if path.ends_with("-native") || path.ends_with("-native.conf") {
                        "pkg.native-runner.v1".into()
                    } else {
                        "pkg.interpreter-adapter.v1".into()
                    },
                );
            }
        }
        payload_manifest.tree_digest =
            crate::domain::contracts::digest_serialized(&payload_manifest.entries);
        plan.payload_manifest = Some(payload_manifest);

        // 4. Transaction Promoting (atomic rename staging -> store)
        self.db.update_transaction_phase(&tx_id, "Promoting")?;
        self.maybe_failpoint("Promoting")?;
        self.layout
            .promote_staging(&staging_dir, &plan.target_store_dir)?;
        if let Some(expected_manifest) = &plan.payload_manifest
            && !expected_manifest.matches_realized_tree(&plan.target_store_dir)?
        {
            return Err(Error::SecurityViolation(format!(
                "realized store tree does not match payload manifest: {}",
                plan.target_store_dir.display()
            )));
        }

        // Runtime manifests are immutable, content-addressed records.  They
        // live outside package payloads so one realization can be reused by
        // commands with different provider selections.
        for runtime in &mut plan.runtimes {
            // ELF inspection runs against the transaction staging tree.  Once
            // that tree is promoted, rewrite only those package-local paths to
            // the immutable store object before materializing the runtime
            // view; host providers keep their concrete host paths.
            let selected: std::collections::BTreeMap<String, PathBuf> = runtime
                .library_view
                .iter()
                .map(|(name, path)| {
                    let promoted = path
                        .strip_prefix(&staging_dir)
                        .map(|relative| plan.target_store_dir.join(relative))
                        .unwrap_or_else(|_| path.clone());
                    // Resolve compatibility links (notably profile/lib)
                    // before materializing the immutable runtime view.  A
                    // runtime must point directly at the selected store or
                    // host file, so removing an unrelated profile alias
                    // cannot break an already published command.
                    let promoted = fs::canonicalize(&promoted).unwrap_or(promoted);
                    (name.clone(), promoted)
                })
                .collect();
            // Runtime references are the retention roots for every selected
            // provider, including providers reached through the legacy
            // profile/lib compatibility view.  Recording only the consumer's
            // store object would let GC collect an indirect provider after
            // its package row is removed even though this runtime still needs
            // the exact file.
            let mut references = runtime
                .references
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            for provider in selected.values() {
                if let Some(root) = store_object_root_for_path(provider, &self.layout.store_dir()) {
                    references.insert(root.to_string_lossy().into_owned());
                }
            }
            runtime.references = references.into_iter().collect();
            // References are part of the immutable runtime contract.  Adding
            // an indirect provider root therefore changes the content
            // identity before the runtime view is materialized.
            runtime.runtime_id = crate::runtime::runtime_identity(runtime);
            runtime.library_view = self
                .layout
                .materialize_runtime_view(&runtime.runtime_id, &selected)?;
            let manifest_path = self.layout.write_runtime_manifest(runtime)?;
            self.db.record_runtime_manifest(
                &runtime.runtime_id,
                &manifest_path,
                &crate::domain::contracts::digest_serialized(runtime),
            )?;
        }

        // 5. Transaction Activating (symlink executables into profile bin)
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        self.maybe_failpoint("Activating")?;
        crate::activation::GenerationManager::detach_active_bin(&self.layout, profile, &tx_id)?;
        let stale = old_binaries
            .iter()
            .filter(|command| {
                !plan
                    .binaries
                    .iter()
                    .any(|binary| &binary.command == *command)
            })
            .map(|command| {
                Ok((
                    self.layout.profile_bin_dir(profile).join(command),
                    self.db
                        .activation_target(profile, command, plan.package.name.as_str())?
                        .ok_or_else(|| {
                            Error::Internal(format!("Missing activation ownership for {command}"))
                        })?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let (stale_paths, stale_targets): (Vec<_>, Vec<_>) = stale.into_iter().unzip();
        Activator::deactivate(&stale_paths, &stale_targets)?;
        if let Some(ref old) = old_pkg {
            if old.store_path != plan.target_store_dir {
                Activator::deactivate_libraries(
                    &old.store_path,
                    &self.layout.profile_lib_dir(profile),
                )?;
            }
        }
        Activator::activate(&plan, &self.layout.profile_bin_dir(profile))?;
        Activator::activate_libraries(
            &plan.target_store_dir,
            &self.layout.profile_lib_dir(profile),
        )?;

        // Capture the complete visible command set as an immutable activation
        // generation.  The stable profile/bin path is switched only after the
        // generation manifest and all command links are ready.
        let generation_id = crate::activation::GenerationManager::publish_after_activation(
            &self.layout,
            profile,
            &tx_id,
            &plan,
        )?;

        // 6. Transaction Committing (commit state DB records)
        self.db.update_transaction_phase(&tx_id, "Committing")?;
        self.maybe_failpoint("Committing")?;
        let activations = plan
            .binaries
            .iter()
            .map(|binary| {
                (
                    binary.command.as_str(),
                    binary.profile_symlink_path.as_path(),
                )
            })
            .collect::<Vec<_>>();
        self.db.commit_install_state(
            profile,
            &plan.package,
            &plan.store_id,
            &plan.target_store_dir,
            &report.extracted_files,
            &activations,
        )?;

        let generation_manifest_path = self
            .layout
            .profile_generations_dir(profile)
            .join(&generation_id)
            .join("manifest.json");
        let generation: crate::domain::contracts::ActivationGeneration =
            serde_json::from_slice(&fs::read(&generation_manifest_path)?)?;
        let mut generation_store_ids = self
            .db
            .list_packages(profile)?
            .into_iter()
            .map(|package| package.store_id)
            .collect::<Vec<_>>();
        if !generation_store_ids.iter().any(|id| id == &plan.store_id) {
            generation_store_ids.push(plan.store_id.clone());
        }
        let generation_store_refs = generation_store_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.db.record_generation(
            &generation,
            &generation_manifest_path,
            &generation_store_refs,
        )?;

        let receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "install".into(),
            plan_identity: crate::domain::contracts::digest_serialized(&plan),
            previous_generation: generation.previous_generation.clone(),
            new_generation: Some(generation_id),
            created_objects: vec![plan.store_id.clone()],
            reused_objects: Vec::new(),
            expected_digests: std::collections::BTreeMap::from([(
                plan.store_id.clone(),
                plan.payload_manifest
                    .as_ref()
                    .map(|manifest| manifest.tree_digest.clone())
                    .unwrap_or_default(),
            )]),
            phase: "Completed".into(),
        };
        self.db
            .record_transaction_receipt(&receipt, "install completed")?;

        // 7. Transaction Completed
        self.db.update_transaction_phase(&tx_id, "Completed")?;

        failure_guard.disarm();

        Ok(plan)
    }

    /// Installs a local package artifact into the store.
    pub fn install(
        &self,
        artifact_path: &Path,
        profile: &str,
        is_dry_run: bool,
    ) -> Result<InstallPlan> {
        self.install_with_options(
            artifact_path,
            profile,
            is_dry_run,
            InstallOptions::default(),
        )
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
        let integration_records = self.db.list_integrations(profile, Some(package_name))?;
        let integration_ownership = integration_records
            .iter()
            .map(integration_ownership)
            .collect::<Result<Vec<_>>>()?;
        let integration_plan = integration_plan_from_records(
            profile,
            package_name,
            &plan.store_id,
            integration_records.clone(),
        )?;
        let integration_details = serde_json::to_string(&integration_plan)?;
        // Reject user replacements before detaching the package generation.
        crate::host::integration::validate_removal(&integration_ownership)?;

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
            Some(&integration_details),
        )?;
        let previous_generation = self
            .db
            .active_generation(profile)?
            .map(|generation| generation.generation_id);
        let planned_receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "remove".into(),
            plan_identity: crate::domain::contracts::digest_serialized(&plan),
            previous_generation,
            new_generation: None,
            created_objects: Vec::new(),
            reused_objects: Vec::new(),
            expected_digests: std::collections::BTreeMap::new(),
            phase: "Planned".into(),
        };
        self.db
            .record_transaction_receipt(&planned_receipt, "remove planned")?;

        // Keep the transaction recoverable if any filesystem or database
        // mutation below fails.  Removal has no payload staging tree, but the
        // same guard still records the failed phase so startup recovery can
        // restore the previous generation and activation rows.
        let failure_guard =
            InstallFailureGuard::new(&self.db, tx_id.clone(), self.layout.staging_dir(&tx_id));

        // Work on a transaction-owned copy so a retained generation is never
        // mutated in place while removing one of its packages.
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        self.maybe_failpoint("Activating")?;
        crate::activation::GenerationManager::detach_active_bin(&self.layout, profile, &tx_id)?;

        // Deactivate profile binary symlinks and shared library symlinks
        Activator::deactivate(&plan.binaries_to_remove, &plan.expected_targets)?;
        Activator::deactivate_libraries(&plan.store_path, &self.layout.profile_lib_dir(profile))?;

        // Remove from database
        self.db.remove_package_from_profile(profile, package_name)?;
        let generation_id = crate::activation::GenerationManager::publish_after_removal(
            &self.layout,
            profile,
            &tx_id,
        )?;
        let generation_manifest_path = self
            .layout
            .profile_generations_dir(profile)
            .join(&generation_id)
            .join("manifest.json");
        let generation: crate::domain::contracts::ActivationGeneration =
            serde_json::from_slice(&fs::read(&generation_manifest_path)?)?;
        self.db
            .record_generation(&generation, &generation_manifest_path, &[])?;
        crate::host::integration::remove(&integration_ownership)?;
        self.maybe_failpoint("HostIntegrationRemoved")?;
        for record in &integration_records {
            self.db.remove_integration(profile, &record.target_path)?;
        }
        if !self
            .db
            .store_is_referenced_by_committed_state(&plan.store_id)?
        {
            self.layout.validate_store_path(&plan.store_path)?;
            if plan.store_path.exists() {
                fs::remove_dir_all(&plan.store_path)?;
            }
            self.db
                .remove_store_object_after_committed_check(&plan.store_id)?;
        }

        self.db.update_transaction_phase(&tx_id, "Completed")?;

        failure_guard.disarm();

        Ok(plan)
    }

    /// Scans pkg-owned store objects and reports unreachable candidates.
    pub fn gc_scan(&self) -> Result<crate::gc::GcReport> {
        crate::gc::GarbageCollector::scan(&self.layout, &self.db)
    }

    /// Collects unreachable pkg-owned store objects under the process lock.
    pub fn gc(&self, dry_run: bool) -> Result<crate::gc::GcReport> {
        if dry_run {
            return self.gc_scan();
        }
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        self.maybe_failpoint("GcBeforeCollect")?;
        crate::gc::GarbageCollector::collect(&self.layout, &self.db)
    }

    /// Restores a retained profile generation and its recorded package state.
    pub fn rollback(&self, profile: &str, generation_id: Option<&str>) -> Result<String> {
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        StoreLayout::validate_profile(profile)?;
        let previous_generation = self
            .db
            .active_generation(profile)?
            .map(|generation| generation.generation_id);
        let target = match generation_id {
            Some(id) => id.to_string(),
            None => self
                .db
                .active_generation(profile)?
                .and_then(|generation| generation.previous_generation)
                .ok_or_else(|| Error::PackageNotFound("previous generation".into()))?,
        };
        let generation = self
            .db
            .list_generations(profile)?
            .into_iter()
            .find(|generation| generation.generation_id == target)
            .ok_or_else(|| Error::PackageNotFound(format!("generation {target}")))?;
        let tx_id = format!(
            "tx-rollback-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        self.db.record_transaction_start(
            &tx_id,
            "rollback",
            "Planned",
            profile,
            None,
            Some(&target),
        )?;
        let planned_receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "rollback".into(),
            plan_identity: crate::domain::contracts::digest_serialized(&target),
            previous_generation: previous_generation.clone(),
            new_generation: Some(target.clone()),
            created_objects: Vec::new(),
            reused_objects: Vec::new(),
            expected_digests: std::collections::BTreeMap::new(),
            phase: "Planned".into(),
        };
        self.db
            .record_transaction_receipt(&planned_receipt, "rollback planned")?;
        let failure_guard =
            InstallFailureGuard::new(&self.db, tx_id.clone(), self.layout.staging_dir(&tx_id));
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        self.maybe_failpoint("Activating")?;
        crate::activation::GenerationManager::switch_to(&self.layout, profile, &target)?;
        self.db.update_transaction_phase(&tx_id, "Committing")?;
        self.maybe_failpoint("Committing")?;
        self.db.restore_generation_state(profile, &target)?;
        let receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "rollback".into(),
            plan_identity: crate::domain::contracts::digest_serialized(&target),
            previous_generation,
            new_generation: Some(target.clone()),
            created_objects: Vec::new(),
            reused_objects: Vec::new(),
            expected_digests: std::collections::BTreeMap::new(),
            phase: "Completed".into(),
        };
        self.db
            .record_transaction_receipt(&receipt, "generation rollback completed")?;
        self.db.update_transaction_phase(&tx_id, "Completed")?;
        failure_guard.disarm();
        Ok(generation.generation_id)
    }

    /// Runs a command through the immutable runtime manifest selected by the
    /// active profile generation.  This gives callers an explicit execution
    /// entry point even when the profile's compatibility symlink is invoked
    /// by another process.
    pub fn run_command(
        &self,
        profile: &str,
        command: &str,
        args: &[std::ffi::OsString],
    ) -> Result<std::process::ExitStatus> {
        let (runtime, store_root) = self.runtime_for_command(profile, command)?;
        crate::runtime::execute_command(&runtime, &store_root, args)
    }

    /// Runs a command and captures stdout/stderr for structured CLI output.
    pub fn run_command_capture(
        &self,
        profile: &str,
        command: &str,
        args: &[std::ffi::OsString],
    ) -> Result<std::process::Output> {
        let (runtime, store_root) = self.runtime_for_command(profile, command)?;
        crate::runtime::execute_command_capture(&runtime, &store_root, args)
    }

    /// Executes a command by replacing the current Unix process, preserving
    /// signal delivery and exit status exactly.
    #[cfg(unix)]
    pub fn exec_command(
        &self,
        profile: &str,
        command: &str,
        args: &[std::ffi::OsString],
    ) -> Result<()> {
        let (runtime, store_root) = self.runtime_for_command(profile, command)?;
        crate::runtime::exec_command(&runtime, &store_root, args)
    }

    fn runtime_for_command(
        &self,
        profile: &str,
        command: &str,
    ) -> Result<(crate::domain::contracts::RuntimeManifest, PathBuf)> {
        StoreLayout::validate_profile(profile)?;
        StoreLayout::validate_component(command, "command")?;
        let generation = self
            .db
            .active_generation(profile)?
            .ok_or_else(|| Error::PackageNotFound(format!("active generation for {profile}")))?;
        let manifest_path = self
            .layout
            .profile_generations_dir(profile)
            .join(&generation.generation_id)
            .join("manifest.json");
        if !fs::symlink_metadata(&manifest_path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            return Err(Error::TransactionRecoveryRequired(format!(
                "active generation manifest is unavailable: {}",
                manifest_path.display()
            )));
        }
        let activation: crate::domain::contracts::ActivationGeneration =
            serde_json::from_slice(&fs::read(manifest_path)?)?;
        if activation.profile != profile || activation.generation_id != generation.generation_id {
            return Err(Error::TransactionRecoveryRequired(
                "active generation manifest identity mismatch".into(),
            ));
        }
        if !activation.commands.contains_key(command) {
            return Err(Error::PackageNotFound(format!(
                "runtime command '{command}' in generation {}",
                generation.generation_id
            )));
        }
        let generation_bin = self
            .layout
            .profile_generations_dir(profile)
            .join(&generation.generation_id)
            .join("bin");
        let exposed_command = generation_bin.join(command);
        let exposed_target = match fs::symlink_metadata(&exposed_command) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let link = fs::read_link(&exposed_command)?;
                if link.is_absolute() {
                    link
                } else {
                    exposed_command
                        .parent()
                        .unwrap_or(&generation_bin)
                        .join(link)
                }
            }
            Ok(metadata) if metadata.is_file() => exposed_command.clone(),
            Ok(_) => {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "generation command is not a regular file or symlink: {}",
                    exposed_command.display()
                )));
            }
            Err(error) => return Err(error.into()),
        };
        let exposed_target = lexical_normalize_path(&exposed_target);
        for runtime_id in activation.runtimes {
            let Some((runtime_path, expected_digest)) =
                self.db.runtime_manifest_record(&runtime_id)?
            else {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest is not recorded: {runtime_id}"
                )));
            };
            if !fs::symlink_metadata(&runtime_path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest is unavailable: {}",
                    runtime_path.display()
                )));
            }
            if !path_is_within(&runtime_path, &self.layout.runtimes_dir()) {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest is outside the pkg runtime root: {}",
                    runtime_path.display()
                )));
            }
            let runtime: crate::domain::contracts::RuntimeManifest =
                serde_json::from_slice(&fs::read(runtime_path)?)?;
            if runtime.runtime_id != runtime_id {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest identity mismatch for {runtime_id}"
                )));
            }
            if crate::domain::contracts::digest_serialized(&runtime) != expected_digest {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "runtime manifest digest mismatch for {runtime_id}"
                )));
            }
            if runtime.execution.command != command {
                continue;
            }
            let store_root = runtime
                .references
                .iter()
                .map(PathBuf::from)
                .find(|path| {
                    path_is_within(path, &self.layout.store_dir())
                        && self.layout.validate_store_path(path).is_ok()
                        && path.is_dir()
                })
                .ok_or_else(|| {
                    Error::TransactionRecoveryRequired(format!(
                        "runtime {runtime_id} has no valid realized store reference"
                    ))
                })?;
            if runtime.execution.executable.is_absolute()
                || runtime
                    .execution
                    .executable
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
            {
                return Err(Error::SecurityViolation(
                    "runtime executable must be store-relative".into(),
                ));
            }
            let runtime_target =
                lexical_normalize_path(&store_root.join(&runtime.execution.executable));
            if runtime_target != exposed_target {
                return Err(Error::TransactionRecoveryRequired(format!(
                    "generation command target does not match runtime contract: {command}"
                )));
            }
            return Ok((runtime, store_root));
        }
        Err(Error::PackageNotFound(format!(
            "runtime command '{command}' in generation {}",
            generation.generation_id
        )))
    }

    /// Captures an existing legacy profile activation without claiming that
    /// its old runtime is verified.  The retained generation can later be
    /// replaced by a normal install/replan once original artifacts are
    /// available.
    pub fn migrate_legacy_profile(&self, profile: &str) -> Result<String> {
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;
        if let Some(active) = self.db.active_generation(profile)? {
            let manifest = self
                .layout
                .profile_generations_dir(profile)
                .join(&active.generation_id)
                .join("manifest.json");
            if fs::symlink_metadata(&manifest)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                return Ok(active.generation_id);
            }
        }
        let bin = self.layout.profile_bin_dir(profile);
        if !bin.exists() && !bin.is_symlink() {
            return Err(Error::PackageNotFound(format!(
                "legacy profile bin for {profile}"
            )));
        }
        let tx_id = format!(
            "tx-migrate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        self.db
            .record_transaction_start(&tx_id, "migrate", "Planned", profile, None, None)?;
        let planned_receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "migrate".into(),
            plan_identity: "sha256:pending".into(),
            previous_generation: None,
            new_generation: None,
            created_objects: Vec::new(),
            reused_objects: Vec::new(),
            expected_digests: std::collections::BTreeMap::new(),
            phase: "Planned".into(),
        };
        self.db
            .record_transaction_receipt(&planned_receipt, "legacy migration planned")?;
        let failure_guard =
            InstallFailureGuard::new(&self.db, tx_id.clone(), self.layout.staging_dir(&tx_id));
        self.db.update_transaction_phase(&tx_id, "Activating")?;
        self.maybe_failpoint("Activating")?;
        let generation_id =
            crate::activation::GenerationManager::publish_legacy(&self.layout, profile, &tx_id)?;
        let manifest_path = self
            .layout
            .profile_generations_dir(profile)
            .join(&generation_id)
            .join("manifest.json");
        let generation: crate::domain::contracts::ActivationGeneration =
            serde_json::from_slice(&fs::read(&manifest_path)?)?;
        let store_ids = self
            .db
            .list_packages(profile)?
            .into_iter()
            .map(|package| package.store_id)
            .collect::<Vec<_>>();
        let refs = store_ids.iter().map(String::as_str).collect::<Vec<_>>();
        self.db
            .record_generation(&generation, &manifest_path, &refs)?;
        let receipt = crate::domain::contracts::TransactionReceipt {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            transaction_id: tx_id.clone(),
            operation: "migrate".into(),
            plan_identity: crate::domain::contracts::digest_serialized(&generation),
            previous_generation: None,
            new_generation: Some(generation_id.clone()),
            created_objects: Vec::new(),
            reused_objects: store_ids,
            expected_digests: std::collections::BTreeMap::new(),
            phase: "Completed".into(),
        };
        self.db
            .record_transaction_receipt(&receipt, "legacy activation captured")?;
        self.db.update_transaction_phase(&tx_id, "Completed")?;
        failure_guard.disarm();
        Ok(generation_id)
    }

    /// Updates local repository snapshots using the provided configuration.
    ///
    /// Repositories (and their internal components) are fetched and verified in parallel
    /// using tokio::spawn tasks, followed by atomic snapshot commits to SQLite.
    ///
    /// If one repository fails (e.g. network timeout or signature mismatch), any successful
    /// repositories are still safely committed, providing resilience. If ALL configured
    /// repositories fail, an error is returned.
    pub async fn update(&self, config: &crate::repository::RepositoriesConfig) -> Result<usize> {
        let keyrings_dir = self.layout.keyrings_dir();

        // Concurrently fetch and verify all repositories using tokio::spawn
        let handles: Vec<_> = config
            .repositories
            .iter()
            .map(|repo| {
                let keyrings_dir = keyrings_dir.clone();
                let repo = repo.clone();
                tokio::spawn(async move {
                    let key_path = repo.public_key_path.as_deref();
                    let mut packages = match repo.format.to_lowercase().as_str() {
                        "rpm" => {
                            crate::repository::rpm_md::update_rpm_repository(
                                &repo.url,
                                &repo.distribution,
                                &repo.components,
                                key_path,
                                &keyrings_dir,
                            )
                            .await?
                        }
                        "alpm" => {
                            crate::repository::alpm_sync::update_alpm_repository(
                                &repo.url,
                                &repo.distribution,
                                &repo.components,
                                key_path,
                                &keyrings_dir,
                            )
                            .await?
                        }
                        _ => {
                            crate::repository::deb::update_debian_repository(
                                &repo.url,
                                &repo.distribution,
                                &repo.components,
                                key_path,
                                &keyrings_dir,
                            )
                            .await?
                        }
                    };

                    for pkg in &mut packages {
                        pkg.repository_id = repo.id.clone();
                    }

                    Ok::<
                        (
                            crate::repository::RepositoryConfig,
                            Vec<crate::domain::package::RemotePackage>,
                        ),
                        Error,
                    >((repo, packages))
                })
            })
            .collect();

        let mut successes = Vec::new();
        let mut failures = Vec::new();

        for handle in handles {
            match handle.await {
                Ok(Ok((repo, packages))) => successes.push((repo, packages)),
                Ok(Err(err)) => failures.push(err),
                Err(join_err) => {
                    failures.push(Error::Internal(format!("Repository task join error: {join_err}")));
                }
            }
        }

        // Fail-closed only if all configured repositories failed
        if successes.is_empty() && !config.repositories.is_empty() {
            return Err(failures.into_iter().next().unwrap_or_else(|| {
                Error::Internal("All repository synchronizations failed".to_string())
            }));
        }

        for err in &failures {
            tracing::warn!("Repository sync warning: {err}");
            eprintln!("Warning: Failed to sync repository: {err}");
        }

        // Network acquisition is intentionally outside the writer lock.  Once
        // every snapshot has been fetched and verified, serialize only the
        // short SQLite publication window.
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let _ = Recovery::reconcile(&self.layout, &self.db)?;

        let mut total_packages = 0;
        for (repo, packages) in successes {
            self.db.commit_repository_snapshot(
                &repo.id,
                &repo.format,
                &repo.url,
                &repo.distribution,
                &packages,
            )?;
            total_packages += packages.len();
        }

        Ok(total_packages)
    }

    /// Searches for remote packages matching a query (substring) in active snapshots.
    pub fn search(&self, query: &str) -> Result<Vec<crate::domain::package::RemotePackage>> {
        self.db.search_remote_packages(query)
    }

    /// Searches for remote packages with optional format and repository filters.
    pub fn search_filtered(
        &self,
        query: &str,
        format_filter: Option<&str>,
        repo_filter: Option<&str>,
    ) -> Result<Vec<crate::domain::package::RemotePackage>> {
        self.db
            .search_remote_packages_filtered(query, format_filter, repo_filter)
    }

    /// Looks up a remote package by exact name in active snapshots.
    pub fn get_remote_package(
        &self,
        name: &str,
    ) -> Result<Option<crate::domain::package::RemotePackage>> {
        self.db.get_remote_package(name)
    }

    /// Resolves a remote package target specification against the catalog.
    ///
    /// Supports:
    /// - `repository_id/name` (e.g. `fedora-41/curl`, `arch-extra/curl`)
    /// - `name:format` (e.g. `curl:rpm`, `curl:alpm`, `curl:deb`)
    /// - `name@version` (e.g. `curl@8.9.1-1.fc41`)
    /// - `name` (exact name with repository priority tie-breaking)
    pub fn resolve_remote_package(
        &self,
        spec: &str,
        config: Option<&crate::repository::RepositoriesConfig>,
    ) -> Result<RemoteResolution> {
        let host = crate::host::HostFacts::detect();
        self.resolve_remote_package_with_host(spec, config, Some(&host))
    }

    /// Resolves a remote package target specification against the catalog with optional host facts.
    pub fn resolve_remote_package_with_host(
        &self,
        spec: &str,
        config: Option<&crate::repository::RepositoriesConfig>,
        host: Option<&crate::host::HostFacts>,
    ) -> Result<RemoteResolution> {
        let mut candidates = self.db.find_remote_candidates(spec)?;
        if candidates.is_empty() {
            return Ok(RemoteResolution::NotFound);
        }
        if candidates.len() == 1 {
            return Ok(RemoteResolution::Exact(
                candidates.into_iter().next().unwrap(),
            ));
        }

        // Priority 1: Match host distribution if target was not explicitly qualified with repository ('/')
        if !spec.contains('/') {
            if let Some(h) = host {
                if let Some(ref distro) = h.distro_id {
                    let host_matches: Vec<_> = candidates
                        .iter()
                        .filter(|c| c.repository_id.to_lowercase().contains(distro))
                        .cloned()
                        .collect();
                    if host_matches.len() == 1 {
                        return Ok(RemoteResolution::Exact(
                            host_matches.into_iter().next().unwrap(),
                        ));
                    }
                    if !host_matches.is_empty() {
                        candidates = host_matches;
                    } else if let Some(ref id_like) = h.distro_id_like {
                        let like_matches: Vec<_> = candidates
                            .iter()
                            .filter(|c| c.repository_id.to_lowercase().contains(id_like))
                            .cloned()
                            .collect();
                        if like_matches.len() == 1 {
                            return Ok(RemoteResolution::Exact(
                                like_matches.into_iter().next().unwrap(),
                            ));
                        }
                        if !like_matches.is_empty() {
                            candidates = like_matches;
                        }
                    }
                }
            }
        }

        // Priority 2: Use configured repository priority to break ties if possible
        if let Some(cfg) = config {
            let priority_map: std::collections::HashMap<&str, u32> = cfg
                .repositories
                .iter()
                .filter_map(|r| r.priority.map(|p| (r.id.as_str(), p)))
                .collect();

            let mut max_priority = None;
            let mut best_candidates = Vec::new();

            for cand in &candidates {
                let p = priority_map
                    .get(cand.repository_id.as_str())
                    .copied()
                    .unwrap_or(0);
                match max_priority {
                    None => {
                        max_priority = Some(p);
                        best_candidates.push(cand.clone());
                    }
                    Some(max_p) if p > max_p => {
                        max_priority = Some(p);
                        best_candidates.clear();
                        best_candidates.push(cand.clone());
                    }
                    Some(max_p) if p == max_p => {
                        best_candidates.push(cand.clone());
                    }
                    _ => {}
                }
            }

            if best_candidates.len() == 1 {
                return Ok(RemoteResolution::Exact(
                    best_candidates.into_iter().next().unwrap(),
                ));
            }
            if !best_candidates.is_empty() {
                candidates = best_candidates;
            }
        }

        // Priority 3: Compare versions if all candidates have the same name and format
        if candidates.len() > 1 {
            let first_fmt = candidates[0].format.clone();
            let first_name = candidates[0].name.clone();
            let all_same = candidates
                .iter()
                .all(|c| c.format == first_fmt && c.name == first_name);
            if all_same {
                let ecosystem = match first_fmt.as_str() {
                    "deb" => Some(crate::domain::version::VersionEcosystem::Debian),
                    "rpm" => Some(crate::domain::version::VersionEcosystem::Rpm),
                    "alpm" => Some(crate::domain::version::VersionEcosystem::Alpm),
                    _ => None,
                };
                if let Some(eco) = ecosystem {
                    candidates.sort_by(|a, b| {
                        crate::domain::version::compare_versions(&b.version, &a.version, eco)
                    });
                    if candidates.len() == 1
                        || crate::domain::version::compare_versions(
                            &candidates[0].version,
                            &candidates[1].version,
                            eco,
                        ) == std::cmp::Ordering::Greater
                    {
                        return Ok(RemoteResolution::Exact(
                            candidates.into_iter().next().unwrap(),
                        ));
                    }
                }
            }
        }

        Ok(RemoteResolution::Ambiguous(candidates))
    }

    /// Resolves a dependency package target specification against the catalog with contextual preference.
    ///
    /// Contextual resolution prioritizes:
    /// 1. Origin repository (`preferred_repo`, e.g. `ubuntu-noble`)
    /// 2. Compatible format (`preferred_format`, e.g. `deb`)
    /// 3. Repository priority from configuration
    /// 4. Highest version
    pub fn resolve_dependency_package(
        &self,
        spec: &str,
        preferred_repo: Option<&str>,
        preferred_format: Option<&str>,
        config: Option<&crate::repository::RepositoriesConfig>,
    ) -> Result<RemoteResolution> {
        let capability_request =
            spec.contains(".so") || spec.starts_with("lib:") || spec.starts_with("bin:");
        let mut candidates = if capability_request {
            let identifier = if spec.starts_with("lib:") || spec.starts_with("bin:") {
                spec.to_string()
            } else if spec.contains(".so") {
                format!("lib:{spec}")
            } else {
                format!("feature:{spec}")
            };
            self.db
                .search_remote_packages_filtered("", None, None)?
                .into_iter()
                .filter(|candidate| {
                    candidate
                        .provides
                        .iter()
                        .any(|provided| provided.to_string() == identifier)
                        || candidate
                            .versioned_provides
                            .iter()
                            .any(|provided| provided.capability.to_string() == identifier)
                })
                .collect()
        } else {
            self.db.find_remote_candidates(spec)?
        };
        if candidates.is_empty() {
            return Ok(RemoteResolution::NotFound);
        }
        if candidates.len() == 1 {
            let cand = candidates.into_iter().next().unwrap();
            if !capability_request
                && let Some(fmt) = preferred_format
                && cand.format != fmt
            {
                return Ok(RemoteResolution::NotFound);
            }
            return Ok(RemoteResolution::Exact(cand));
        }

        // Priority 1: Match preferred repository (parent package origin)
        if let Some(repo) = preferred_repo {
            let repo_matches: Vec<_> = candidates
                .iter()
                .filter(|c| c.repository_id == repo)
                .cloned()
                .collect();
            if repo_matches.len() == 1 {
                return Ok(RemoteResolution::Exact(
                    repo_matches.into_iter().next().unwrap(),
                ));
            }
            if !repo_matches.is_empty() {
                candidates = repo_matches;
            }
        }

        // Priority 2: Filter by preferred format if provided to avoid cross-distro format mixing
        if !capability_request && let Some(fmt) = preferred_format {
            let format_matches: Vec<_> = candidates
                .iter()
                .filter(|c| c.format == fmt)
                .cloned()
                .collect();
            if format_matches.is_empty() {
                return Ok(RemoteResolution::NotFound);
            }
            if format_matches.len() == 1 {
                return Ok(RemoteResolution::Exact(
                    format_matches.into_iter().next().unwrap(),
                ));
            }
            candidates = format_matches;
        }

        // Priority 3: Repository priority tie-breaking from configuration
        if let Some(cfg) = config {
            let priority_map: std::collections::HashMap<&str, u32> = cfg
                .repositories
                .iter()
                .filter_map(|r| r.priority.map(|p| (r.id.as_str(), p)))
                .collect();

            let mut max_priority = None;
            let mut best_candidates = Vec::new();

            for cand in &candidates {
                let p = priority_map
                    .get(cand.repository_id.as_str())
                    .copied()
                    .unwrap_or(0);
                match max_priority {
                    None => {
                        max_priority = Some(p);
                        best_candidates.push(cand.clone());
                    }
                    Some(max_p) if p > max_p => {
                        max_priority = Some(p);
                        best_candidates.clear();
                        best_candidates.push(cand.clone());
                    }
                    Some(max_p) if p == max_p => {
                        best_candidates.push(cand.clone());
                    }
                    _ => {}
                }
            }

            if best_candidates.len() == 1 {
                return Ok(RemoteResolution::Exact(
                    best_candidates.into_iter().next().unwrap(),
                ));
            }
            if !best_candidates.is_empty() {
                candidates = best_candidates;
            }
        }

        // Priority 4: Compare versions if all candidates have the same name and format
        if candidates.len() > 1 {
            let first_fmt = candidates[0].format.clone();
            let first_name = candidates[0].name.clone();
            let all_same = candidates
                .iter()
                .all(|c| c.format == first_fmt && c.name == first_name);
            if all_same {
                let ecosystem = match first_fmt.as_str() {
                    "deb" => Some(crate::domain::version::VersionEcosystem::Debian),
                    "rpm" => Some(crate::domain::version::VersionEcosystem::Rpm),
                    "alpm" => Some(crate::domain::version::VersionEcosystem::Alpm),
                    _ => None,
                };
                if let Some(eco) = ecosystem {
                    candidates.sort_by(|a, b| {
                        crate::domain::version::compare_versions(&b.version, &a.version, eco)
                    });
                    if candidates.len() == 1
                        || crate::domain::version::compare_versions(
                            &candidates[0].version,
                            &candidates[1].version,
                            eco,
                        ) == std::cmp::Ordering::Greater
                    {
                        return Ok(RemoteResolution::Exact(
                            candidates.into_iter().next().unwrap(),
                        ));
                    }
                }
            }
        }

        Ok(RemoteResolution::Ambiguous(candidates))
    }

    /// Returns search paths for shared libraries in the profile.
    /// This includes `profile_lib_dir` and store object library directories of installed packages.
    pub fn profile_lib_search_paths(&self, profile: &str) -> Result<Vec<PathBuf>> {
        let mut dirs = Vec::new();
        let profile_lib = self.layout.profile_lib_dir(profile);
        if profile_lib.is_dir() {
            dirs.push(profile_lib);
        }

        if let Ok(installed) = self.db.list_packages(profile) {
            for pkg in installed {
                if pkg.store_path.is_dir() {
                    collect_lib_dirs(&pkg.store_path, &mut dirs);
                }
            }
        }
        Ok(dirs)
    }

    /// Downloads a remote package to the digest-addressed artifact cache.
    pub async fn download_remote(
        &self,
        pkg: &crate::domain::package::RemotePackage,
    ) -> Result<std::path::PathBuf> {
        self.download_remote_with_progress(pkg, |_, _| {}).await
    }

    /// Downloads a remote package to the digest-addressed artifact cache with progress reporting.
    pub async fn download_remote_with_progress<F>(
        &self,
        pkg: &crate::domain::package::RemotePackage,
        on_progress: F,
    ) -> Result<std::path::PathBuf>
    where
        F: FnMut(u64, Option<u64>) + Send + Sync,
    {
        let spec = crate::acquisition::ArtifactAcquisitionSpec::new(&self.layout, pkg.clone())?;
        Ok(crate::acquisition::acquire_artifact(spec, on_progress)
            .await?
            .path)
    }

    /// Looks up an installed package by name in the specified profile.
    pub fn get_installed_package(
        &self,
        profile: &str,
        name: &str,
    ) -> Result<Option<InstalledPackage>> {
        self.db.get_package(profile, name)
    }

    /// Lists locally installed packages in the specified profile.
    pub fn list(&self, profile: &str) -> Result<Vec<InstalledPackage>> {
        self.db.list_packages(profile)
    }

    /// Finds installed packages that expose an exact command in a profile.
    pub fn query_command(&self, profile: &str, command: &str) -> Result<Vec<InstalledPackage>> {
        StoreLayout::validate_component(command, "command")?;
        Ok(self
            .db
            .list_packages(profile)?
            .into_iter()
            .filter(|package| package.binaries.iter().any(|binary| binary == command))
            .collect())
    }

    /// Computes newer candidates without downloading or changing state.
    pub fn plan_upgrade(
        &self,
        profile: &str,
        package_name: Option<&str>,
    ) -> Result<Vec<UpgradeCandidate>> {
        let host = crate::host::HostFacts::detect();
        let installed = self.db.list_packages(profile)?;
        let mut upgrades = Vec::new();
        for package in installed {
            if package_name.is_some_and(|name| name != package.name.as_str()) {
                continue;
            }
            let candidates = self.db.find_remote_candidates(package.name.as_str())?;
            let installed_ecosystem = match package.format {
                crate::domain::package::PackageFormat::Deb => {
                    crate::domain::version::VersionEcosystem::Debian
                }
                crate::domain::package::PackageFormat::Rpm => {
                    crate::domain::version::VersionEcosystem::Rpm
                }
                crate::domain::package::PackageFormat::Alpm => {
                    crate::domain::version::VersionEcosystem::Alpm
                }
                crate::domain::package::PackageFormat::Tarball => continue,
            };
            let mut best = None;
            for candidate in candidates {
                let ecosystem = match candidate.format.as_str() {
                    "deb" => crate::domain::version::VersionEcosystem::Debian,
                    "rpm" => crate::domain::version::VersionEcosystem::Rpm,
                    "alpm" => crate::domain::version::VersionEcosystem::Alpm,
                    _ => installed_ecosystem,
                };
                if !crate::domain::package::Architecture::parse(&candidate.architecture)
                    .matches_host(&host.architecture)
                {
                    continue;
                }
                if crate::domain::version::compare_versions(
                    &candidate.version,
                    package.version.as_str(),
                    ecosystem,
                ) != std::cmp::Ordering::Greater
                {
                    continue;
                }
                let replace =
                    best.as_ref()
                        .is_none_or(|current: &crate::domain::package::RemotePackage| {
                            let ordering = crate::domain::version::compare_versions(
                                &candidate.version,
                                &current.version,
                                ecosystem,
                            );
                            ordering == std::cmp::Ordering::Greater
                                || (ordering == std::cmp::Ordering::Equal
                                    && (
                                        candidate.repository_id.as_str(),
                                        candidate.format.as_str(),
                                        candidate.digest.as_str(),
                                    ) < (
                                        current.repository_id.as_str(),
                                        current.format.as_str(),
                                        current.digest.as_str(),
                                    ))
                        });
                if replace {
                    best = Some(candidate);
                }
            }
            if let Some(candidate) = best {
                upgrades.push(UpgradeCandidate {
                    installed: package,
                    candidate,
                });
            }
        }
        Ok(upgrades)
    }

    /// Inspects package information from either an artifact path or an installed package name.
    pub fn info(&self, name_or_path: &str, profile: &str) -> Result<PackageInfo> {
        let path = Path::new(name_or_path);
        if path.exists() && path.is_file() {
            let format = crate::format::detect_format(path)?;
            let adapter = crate::format::get_adapter(format);
            let meta = adapter.parse_metadata(path)?;
            Ok(PackageInfo::LocalArtifact(meta))
        } else if let Some(installed) = self.db.get_package(profile, name_or_path)? {
            Ok(PackageInfo::Installed(installed))
        } else {
            Err(Error::PackageNotFound(name_or_path.to_string()))
        }
    }
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    path.is_absolute() && path.strip_prefix(root).is_ok()
}

fn integration_ownership(
    record: &crate::state::IntegrationRecord,
) -> Result<crate::host::integration::IntegrationOwnership> {
    Ok(crate::host::integration::IntegrationOwnership {
        kind: crate::domain::integration::IntegrationKind::parse(&record.kind)?,
        source_path: record.source_path.clone(),
        target_path: record.target_path.clone(),
        source_digest: parse_digest(&record.source_digest),
    })
}

fn integration_plan_from_records(
    profile: &str,
    package_name: &str,
    store_id: &str,
    records: Vec<crate::state::IntegrationRecord>,
) -> Result<crate::domain::integration::IntegrationPlan> {
    let mut actions = Vec::new();
    for record in records {
        actions.push(crate::domain::integration::IntegrationAction {
            kind: crate::domain::integration::IntegrationKind::parse(&record.kind)?,
            source_path: record.source_path,
            target_path: record.target_path,
            source_digest: parse_digest(&record.source_digest),
        });
    }
    Ok(crate::domain::integration::IntegrationPlan {
        profile: profile.to_string(),
        package_name: package_name.to_string(),
        store_id: store_id.to_string(),
        actions,
        conflicts: Vec::new(),
    })
}

fn parse_digest(value: &str) -> ArtifactDigest {
    value.split_once(':').map_or_else(
        || ArtifactDigest::sha256(value),
        |(algorithm, hex)| ArtifactDigest::new(algorithm, hex),
    )
}

fn validate_profile_tree_for_removal(profile_dir: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(profile_dir)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::SecurityViolation(format!(
            "profile path is not a managed directory: {}",
            profile_dir.display()
        )));
    }
    for entry in fs::read_dir(profile_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        match name.as_str() {
            "current" | "bin" => {
                if !metadata.file_type().is_symlink() {
                    return Err(Error::SecurityViolation(format!(
                        "profile entry is not a managed symlink: {}",
                        path.display()
                    )));
                }
            }
            "lib" => validate_managed_link_dir(&path)?,
            "generations" => validate_generations_dir(&path)?,
            name if name.starts_with("legacy-bin-") => validate_generation_bin(&path)?,
            _ => {
                return Err(Error::ActivationConflict {
                    command: name,
                    existing_package: "unmanaged profile content".into(),
                });
            }
        }
    }
    Ok(())
}

fn validate_generations_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::SecurityViolation(format!(
            "generations path is not a managed directory: {}",
            path.display()
        )));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let generation = entry.path();
        StoreLayout::validate_component(&entry.file_name().to_string_lossy(), "generation id")?;
        let metadata = fs::symlink_metadata(&generation)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::SecurityViolation(format!(
                "generation entry is not a managed directory: {}",
                generation.display()
            )));
        }
        for child in fs::read_dir(&generation)? {
            let child = child?;
            let child_path = child.path();
            let child_name = child.file_name().to_string_lossy().into_owned();
            let child_metadata = fs::symlink_metadata(&child_path)?;
            match child_name.as_str() {
                "manifest.json" => {
                    if !child_metadata.is_file() || child_metadata.file_type().is_symlink() {
                        return Err(Error::SecurityViolation(format!(
                            "generation manifest is not a managed regular file: {}",
                            child_path.display()
                        )));
                    }
                }
                "bin" => validate_generation_bin(&child_path)?,
                _ => {
                    return Err(Error::ActivationConflict {
                        command: child_name,
                        existing_package: "unmanaged generation content".into(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_generation_bin(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::SecurityViolation(format!(
            "generation bin is not a managed directory: {}",
            path.display()
        )));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if !fs::symlink_metadata(entry.path())?.file_type().is_symlink() {
            return Err(Error::ActivationConflict {
                command: entry.file_name().to_string_lossy().into_owned(),
                existing_package: "unmanaged generation command".into(),
            });
        }
    }
    Ok(())
}

fn validate_managed_link_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::SecurityViolation(format!(
            "profile library path is not a managed directory: {}",
            path.display()
        )));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if !fs::symlink_metadata(entry.path())?.file_type().is_symlink() {
            return Err(Error::ActivationConflict {
                command: entry.file_name().to_string_lossy().into_owned(),
                existing_package: "unmanaged profile library".into(),
            });
        }
    }
    Ok(())
}

/// Resolves a selected provider (which may be a symlink in profile/lib) to
/// the direct store-object root that owns it.  Host paths intentionally return
/// `None`; they are validated at launch but are not pkg-owned GC roots.
fn store_object_root_for_path(path: &Path, store_root: &Path) -> Option<PathBuf> {
    // During planning the target store path may still be in staging and not
    // exist yet.  Canonicalize existing aliases, otherwise use the validated
    // absolute path lexically; the caller has already derived it from the
    // transaction's target store root.
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let relative = resolved.strip_prefix(store_root).ok()?;
    let store_id = relative.components().next()?.as_os_str().to_str()?;
    StoreLayout::validate_component(store_id, "store id").ok()?;
    Some(store_root.join(store_id))
}

fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn collect_lib_dirs(base: &Path, dirs: &mut Vec<PathBuf>) {
    let candidates = ["lib", "lib64", "usr/lib", "usr/lib64"];
    for cand in &candidates {
        let p = base.join(cand);
        if p.is_dir() {
            if !dirs.contains(&p) {
                dirs.push(p.clone());
            }
            if let Ok(entries) = std::fs::read_dir(&p) {
                for entry in entries.flatten() {
                    let sub = entry.path();
                    if sub.is_dir() && !sub.is_symlink() && !dirs.contains(&sub) {
                        dirs.push(sub);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn dry_run_engine_does_not_initialize_an_empty_root() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("pkg");
        let layout = StoreLayout::new(&root);
        let _engine = Engine::open_for_dry_run(layout).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn doctor_repair_reconciles_known_incomplete_transaction() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        let engine = Engine::open(layout.clone()).unwrap();
        engine
            .db()
            .record_transaction_start("tx-doctor-repair", "install", "Staging", "tool", None, None)
            .unwrap();
        fs::create_dir_all(layout.staging_dir("tx-doctor-repair")).unwrap();

        let before = engine.doctor(None).unwrap();
        assert_eq!(before.exit_code(), 3);
        let after = engine.doctor_repair(None).unwrap();
        assert!(after.is_healthy());
        assert!(!layout.staging_dir("tx-doctor-repair").exists());
    }
}
