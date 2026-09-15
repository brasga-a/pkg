//! Top-level pkg engine coordinating format adapters, store layout,
//! state database, transactions, and binary activations.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::activation::Activator;
use crate::domain::installed::InstalledPackage;
use crate::domain::package::{ArtifactDigest, NormalizedPackage};
use crate::domain::plan::{InstallPlan, RemovePlan};
use crate::error::{Error, Result};
use crate::format::ExtractionLimits;
use crate::host::elf::inspect_elf;
use crate::lock::ProcessLock;
use crate::planner::Planner;
use crate::state::StateDatabase;
use crate::store::StoreLayout;
use crate::transaction::Recovery;

/// Result of resolving a package target specification against the remote catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteResolution {
    /// Exactly one package was matched (either naturally or via priority resolution).
    Exact(crate::domain::package::RemotePackage),
    /// Multiple packages matched across different repositories or formats.
    Ambiguous(Vec<crate::domain::package::RemotePackage>),
    /// No matching packages were found.
    NotFound,
}

/// Detailed information about a package, either from local artifact or installed state.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
}

impl Engine {
    /// Initializes or opens an engine using the provided store layout.
    pub fn open(layout: StoreLayout) -> Result<Self> {
        let _lock = ProcessLock::acquire(&layout.lock_path())?;
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

    /// Performs a preflight inspection on an artifact without mutating state or the store.
    /// Returns the parsed package metadata and any missing ELF dynamic shared libraries.
    pub fn preflight_check(&self, artifact_path: &Path) -> Result<PreflightReport> {
        let format = crate::format::detect_format(artifact_path)?;
        let adapter = crate::format::get_adapter(format);
        let package = adapter.parse_metadata(artifact_path)?;

        let temp_dir = tempfile::tempdir()?;
        let report = adapter.extract_payload(
            artifact_path,
            temp_dir.path(),
            &ExtractionLimits::default(),
        )?;

        let mut missing_libraries = Vec::new();
        for file in &report.extracted_files {
            let full_path = temp_dir.path().join(file);
            if let Some(inspection) = inspect_elf(&full_path, Some(temp_dir.path()))? {
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

        // 2. Transaction Staging
        let staging_dir = self.layout.staging_dir(&tx_id);
        self.db.update_transaction_phase(&tx_id, "Staging")?;

        let format = crate::format::detect_format(artifact_path)?;
        let adapter = crate::format::get_adapter(format);
        let report =
            adapter.extract_payload(artifact_path, &staging_dir, &ExtractionLimits::default())?;

        // 3. Transaction Prepared (verify ELF binaries and host libraries)
        self.db.update_transaction_phase(&tx_id, "Prepared")?;
        for file in &report.extracted_files {
            let full_path = staging_dir.join(file);
            if let Some(inspection) = inspect_elf(&full_path, Some(&staging_dir))? {
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

        // 4. Transaction Promoting (atomic rename staging -> store)
        self.db.update_transaction_phase(&tx_id, "Promoting")?;
        self.layout
            .promote_staging(&staging_dir, &plan.target_store_dir)?;

        // 5. Transaction Activating (symlink executables into profile bin)
        self.db.update_transaction_phase(&tx_id, "Activating")?;
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
        Activator::activate(&plan, &self.layout.profile_bin_dir(profile))?;

        // 6. Transaction Committing (commit state DB records)
        self.db.update_transaction_phase(&tx_id, "Committing")?;
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

        // 7. Transaction Completed
        self.db.update_transaction_phase(&tx_id, "Completed")?;

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
        Activator::deactivate(&plan.binaries_to_remove, &plan.expected_targets)?;

        // Remove from database
        self.db.remove_package_from_profile(profile, package_name)?;
        if !self.db.store_is_referenced(&plan.store_id)? {
            self.layout.validate_store_path(&plan.store_path)?;
            if plan.store_path.exists() {
                fs::remove_dir_all(&plan.store_path)?;
            }
            self.db.remove_store_object(&plan.store_id)?;
        }

        self.db.update_transaction_phase(&tx_id, "Completed")?;

        Ok(plan)
    }

    /// Updates local repository snapshots using the provided configuration.
    ///
    /// Repositories (and their internal components) are fetched and verified in parallel
    /// using asynchronous tasks, followed by atomic snapshot commits to SQLite.
    pub async fn update(&self, config: &crate::repository::RepositoriesConfig) -> Result<usize> {
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let keyrings_dir = self.layout.keyrings_dir();

        // Concurrently fetch and verify all repositories
        let fetch_futures = config.repositories.iter().map(|repo| {
            let keyrings_dir = keyrings_dir.clone();
            let repo = repo.clone();
            async move {
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
            }
        });

        let results = futures::future::join_all(fetch_futures).await;

        let mut total_packages = 0;
        for res in results {
            let (repo, packages) = res?;
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
        let candidates = self.db.find_remote_candidates(spec)?;
        if candidates.is_empty() {
            return Ok(RemoteResolution::NotFound);
        }
        if candidates.len() == 1 {
            return Ok(RemoteResolution::Exact(
                candidates.into_iter().next().unwrap(),
            ));
        }

        // Use configured repository priority to break ties if possible
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
        }

        Ok(RemoteResolution::Ambiguous(candidates))
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
        if pkg.digest.len() != 64 || !pkg.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::SecurityViolation(
                "Invalid SHA256 digest in repository metadata".into(),
            ));
        }
        let _lock = ProcessLock::acquire(&self.layout.lock_path())?;
        let dest = self.layout.artifact_cache_path(&pkg.digest);
        if let Ok(metadata) = fs::symlink_metadata(&dest) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::SecurityViolation(
                    "Invalid artifact cache entry".into(),
                ));
            }
            if ArtifactDigest::from_file(&dest)?
                .hex()
                .eq_ignore_ascii_case(&pkg.digest)
                && metadata.len() == pkg.size_bytes
            {
                return Ok(dest);
            }
            fs::remove_file(&dest)?;
            return Err(Error::SecurityViolation(
                "Artifact cache size or digest mismatch".into(),
            ));
        }

        fs::create_dir_all(dest.parent().unwrap())?;
        let temporary = tempfile::NamedTempFile::new_in(dest.parent().unwrap())?;
        let downloader = crate::transport::BoundedDownloader::try_default()?;
        downloader
            .download_to_file_with_progress(&pkg.url, temporary.path(), on_progress)
            .await?;
        let hash = ArtifactDigest::from_file(temporary.path())?;
        if !hash.hex().eq_ignore_ascii_case(&pkg.digest)
            || fs::metadata(temporary.path())?.len() != pkg.size_bytes
        {
            return Err(crate::error::Error::SecurityViolation(format!(
                "Artifact size or digest mismatch: expected {}, got {}",
                pkg.digest, hash
            )));
        }
        temporary.as_file().sync_all()?;
        temporary.persist(&dest).map_err(|e| Error::Io(e.error))?;
        Ok(dest)
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
