//! CLI entry point for `pkg` package manager.

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use pkg_core::domain::package::NormalizedPackage;
use pkg_core::{
    ArtifactAcquisitionResult, ArtifactAcquisitionSource, ArtifactAcquisitionSpec, Engine,
    InstallOptions, PackageInfo, RemoteResolution, StoreLayout,
};

static JSON_EMITTED: AtomicBool = AtomicBool::new(false);

#[derive(Parser, Debug)]
#[command(
    name = "pkg",
    version,
    about = "A universal, cross-distribution package manager for Linux",
    long_about = "pkg is a cross-distribution Linux package manager that installs supported applications into a pkg-owned isolated store."
)]
struct Cli {
    /// Enable verbose diagnostic logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Custom base directory for pkg state and store (overrides default rootless path)
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// Target profile name
    #[arg(long, global = true, default_value = "default")]
    profile: String,

    /// Emit one deterministic machine-readable JSON document on stdout
    #[arg(long, global = true)]
    json: bool,

    /// Disable all prompts and fail immediately on unresolved choices
    #[arg(long, global = true)]
    non_interactive: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Install one or more packages from local artifacts (.deb, .rpm, .pkg.tar.zst) or remote catalog
    Install {
        /// Path(s) to local package artifact(s) or remote package target(s) (e.g. name, repo/name)
        #[arg(required = true, num_args = 1..)]
        targets: Vec<PathBuf>,

        /// Produce and display the install plan without modifying disk or state
        #[arg(long)]
        dry_run: bool,

        /// Automatically answer yes to all confirmation prompts
        #[arg(short = 'y', long = "yes")]
        yes: bool,

        /// Bypass missing host shared libraries verification
        #[arg(long = "ignore-missing-libs")]
        ignore_missing_libs: bool,

        /// Skip resolving and installing declared dependencies
        #[arg(long = "no-deps", alias = "skip-deps")]
        no_deps: bool,

        /// Interactively select from matching package candidates
        #[arg(short = 'i', long = "interactive")]
        interactive: bool,
    },

    /// Remove an installed package from the active profile
    Remove {
        /// Package name to remove
        name: String,

        /// Produce and display the removal plan without modifying disk or state
        #[arg(long)]
        dry_run: bool,
    },

    /// Show or collect unreachable pkg-owned store objects
    Gc {
        /// Only report candidates without modifying the store or database
        #[arg(long)]
        dry_run: bool,
    },

    /// Inspect pkg-owned state and report consistency or recovery findings
    Doctor {
        /// Inspect every profile instead of only the selected profile
        #[arg(long)]
        all: bool,
        /// Repair only recoverable pkg-owned transaction state before reporting
        #[arg(long)]
        repair: bool,
    },

    /// Expose an installed package's desktop, icon and MIME resources in the user's data dirs.
    Integrate {
        /// Installed package name
        package: String,
        /// Show the actions without changing host files
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove only host integration links owned by an installed package.
    Deintegrate {
        /// Installed package name
        package: String,
        /// Show the actions without changing host files
        #[arg(long)]
        dry_run: bool,
    },

    /// Restore a retained profile generation (defaults to the previous one)
    Rollback {
        /// Generation identifier under profiles/<profile>/generations
        generation: Option<String>,
    },

    /// Execute an activated command through its recorded per-command runtime.
    Run {
        /// Command name exposed by the selected profile.
        command: String,
        /// Arguments passed verbatim to the command.
        #[arg(trailing_var_arg = true)]
        args: Vec<OsString>,
    },

    /// Capture a legacy profile activation as an explicitly unverified generation.
    Migrate,

    /// List locally installed packages in the profile
    List,

    /// Synchronize local package catalog from configured remote repositories
    Sync,

    /// Resolve and apply newer versions from repository snapshots (alias of `pkg upgrade`)
    Update {
        /// Upgrade only one installed package
        name: Option<String>,
        /// Show the update plan without downloading or changing state
        #[arg(long)]
        dry_run: bool,
        /// Automatically accept upgrade confirmations
        #[arg(short = 'y', long = "yes")]
        yes: bool,
        /// Maximum concurrent artifact acquisitions (1-16)
        #[arg(long, default_value_t = 4, value_parser = parse_jobs)]
        jobs: usize,
    },

    /// Search for a package in the remote catalog
    Search {
        /// Package name to search for
        query: String,

        /// Filter by package format (deb, rpm, alpm)
        #[arg(long)]
        format: Option<String>,

        /// Filter by repository ID (e.g. fedora-41, arch-extra)
        #[arg(long)]
        repo: Option<String>,
    },

    /// Manage remote repositories
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },

    /// Show detailed metadata and state for a package or artifact
    Info {
        /// Package name or path to a local package artifact
        target: String,
    },

    /// Find installed packages that expose a command in the selected profile
    QueryCommand { command: String },

    /// Create, list, or drop isolated task profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },

    /// Resolve and optionally apply newer versions from repository snapshots
    Upgrade {
        /// Upgrade only one installed package
        name: Option<String>,
        /// Show the upgrade plan without downloading or changing state
        #[arg(long)]
        dry_run: bool,
        /// Retained for a stable non-interactive command surface
        #[arg(short = 'y', long = "yes")]
        yes: bool,
        /// Maximum concurrent artifact acquisitions (1-16)
        #[arg(long, default_value_t = 4, value_parser = parse_jobs)]
        jobs: usize,
    },

    /// Run the native JSON-RPC MCP server over standard input/output
    Mcp,
}

#[derive(Subcommand, Debug)]
enum ProfileCommands {
    /// Create an empty profile workspace
    Create { name: String },
    /// Remove a profile and its activations when all content is pkg-owned
    Drop { name: String },
    /// List profiles known to pkg
    List,
}

#[derive(Subcommand, Debug)]
enum RepoCommands {
    /// Add a new remote repository
    Add {
        /// Unique identifier for the repository (e.g., ubuntu-noble, fedora-41, arch-extra)
        id: String,
        /// Repository format ecosystem (deb, rpm, alpm)
        #[arg(long)]
        format: Option<String>,
        /// Repository base URL (e.g., http://archive.ubuntu.com/ubuntu)
        url: Option<String>,
        /// Distribution suite (e.g., noble, 41, extra)
        distribution: Option<String>,
        /// Components to fetch (e.g., main universe)
        #[arg(trailing_var_arg = true)]
        components: Vec<String>,
        /// Repository priority (higher values preferred during resolution)
        #[arg(long)]
        priority: Option<u32>,
    },
    /// List configured repositories
    List {
        /// List all official repositories available from the remote curated registry
        #[arg(long, short)]
        remote: bool,
    },
    /// Synchronize repository metadata from configured remote repositories
    #[command(alias = "update")]
    Sync,
}

fn init_tracing(verbose: bool) {
    let default_level = if verbose { "debug" } else { "warn,pgp=error" };
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .try_init();
}

fn emit_json<T: Serialize>(value: &T) -> Result<()> {
    JSON_EMITTED.store(true, Ordering::Relaxed);
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_jobs(value: &str) -> std::result::Result<usize, String> {
    let jobs = value
        .parse::<usize>()
        .map_err(|_| "must be an integer between 1 and 16".to_string())?;
    if (1..=16).contains(&jobs) {
        Ok(jobs)
    } else {
        Err("must be between 1 and 16".to_string())
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let layout = if let Some(ref dir) = cli.data_dir {
        StoreLayout::new(dir)
    } else {
        StoreLayout::default_rootless()
    };

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            let mut cmd = Cli::command();
            cmd.print_help()?;
            println!();
            return Ok(());
        }
    };

    // Keep one implementation for both historical spellings.  `pkg update`
    // updates installed packages; repository metadata is handled by
    // `pkg repo sync` (with `pkg sync` and `pkg repo update` retained as
    // compatibility spellings).
    let command = match command {
        Commands::Update {
            name,
            dry_run,
            yes,
            jobs,
        } => Commands::Upgrade {
            name,
            dry_run,
            yes,
            jobs,
        },
        command => command,
    };

    let plan_only = matches!(
        &command,
        Commands::Install { dry_run: true, .. }
            | Commands::Remove { dry_run: true, .. }
            | Commands::Gc { dry_run: true }
            | Commands::Doctor { repair: false, .. }
            | Commands::Integrate { dry_run: true, .. }
            | Commands::Deintegrate { dry_run: true, .. }
            | Commands::Upgrade { dry_run: true, .. }
    );
    let engine = if plan_only {
        Engine::open_for_dry_run(layout)?
    } else {
        Engine::open(layout)?
    };
    let json_mode = cli.json;
    let non_interactive = cli.non_interactive;

    match command {
        Commands::Install {
            targets,
            dry_run,
            yes,
            ignore_missing_libs,
            interactive,
            no_deps,
        } => {
            let config_path = engine.layout().base_dir().join("repositories.toml");
            let config = if config_path.exists() {
                pkg_core::repository::RepositoriesConfig::load_from_file(&config_path).ok()
            } else {
                None
            };

            let mut queue: Vec<TargetItem> = Vec::new();
            let mut skipped_count = 0;
            let mut failed_count = 0;
            let mut installed_count = 0;
            let mut json_plans = Vec::new();
            let mut json_errors = Vec::new();

            // Phase 1: Planning / Resolution Queue
            for path in &targets {
                let (target_item, pkg_name) = if path.exists() {
                    let format = match pkg_core::format::detect_format(path) {
                        Ok(f) => f,
                        Err(e) => {
                            eprintln!("Error detecting format of '{}': {}", path.display(), e);
                            failed_count += 1;
                            continue;
                        }
                    };
                    let adapter = pkg_core::format::get_adapter(format);
                    let meta = match adapter.parse_metadata(path) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("Error reading metadata from '{}': {}", path.display(), e);
                            failed_count += 1;
                            continue;
                        }
                    };
                    let name = meta.name.to_string();
                    let version = meta.version.to_string();
                    let format_str = meta.format.to_string();
                    (
                        TargetItem::Local {
                            path: path.clone(),
                            name: name.clone(),
                            version,
                            format: format_str,
                        },
                        name,
                    )
                } else {
                    let spec = path.to_string_lossy().to_string();
                    if !json_mode {
                        println!("Resolving package target '{}'...", spec);
                    }

                    let resolution = if interactive && !spec.contains('/') {
                        let candidates = match engine.db().find_remote_candidates(&spec) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("Error resolving package target '{}': {}", spec, e);
                                failed_count += 1;
                                continue;
                            }
                        };
                        if candidates.is_empty() {
                            RemoteResolution::NotFound
                        } else if candidates.len() == 1 {
                            RemoteResolution::Exact(candidates.into_iter().next().unwrap())
                        } else {
                            RemoteResolution::Ambiguous(candidates)
                        }
                    } else {
                        match engine.resolve_remote_package(&spec, config.as_ref()) {
                            Ok(res) => res,
                            Err(e) => {
                                eprintln!("Error resolving package target '{}': {}", spec, e);
                                failed_count += 1;
                                continue;
                            }
                        }
                    };

                    let remote_pkg = match resolution {
                        RemoteResolution::NotFound => {
                            eprintln!(
                                "Error: Package target '{}' not found in local paths or active repository snapshots.",
                                spec
                            );
                            failed_count += 1;
                            continue;
                        }
                        RemoteResolution::Exact(pkg) => pkg,
                        RemoteResolution::Ambiguous(candidates) => {
                            if json_mode || non_interactive {
                                json_errors.push(serde_json::json!({
                                    "code": "AMBIGUOUS_CANDIDATES",
                                    "target": spec,
                                    "candidates": candidates,
                                }));
                                failed_count += 1;
                                continue;
                            }
                            println!("\nMultiple candidates match '{}':", spec);
                            for (i, cand) in candidates.iter().enumerate() {
                                println!(
                                    "  {}) {} {} [{}] (from repository '{}')",
                                    i + 1,
                                    cand.name,
                                    cand.version,
                                    cand.format,
                                    cand.repository_id
                                );
                            }

                            use std::io::IsTerminal;
                            if std::io::stdin().is_terminal() {
                                use std::io::Write;
                                print!(
                                    "\nSelect candidate to install [1-{}] (or Enter / 's' to skip): ",
                                    candidates.len()
                                );
                                std::io::stdout().flush().ok();
                                let mut input = String::new();
                                std::io::stdin().read_line(&mut input)?;
                                let trimmed = input.trim();
                                if trimmed.is_empty()
                                    || trimmed.eq_ignore_ascii_case("s")
                                    || trimmed.eq_ignore_ascii_case("cancel")
                                {
                                    println!("Skipping '{}'.", spec);
                                    skipped_count += 1;
                                    continue;
                                }
                                match trimmed.parse::<usize>() {
                                    Ok(num) if num >= 1 && num <= candidates.len() => {
                                        candidates[num - 1].clone()
                                    }
                                    _ => {
                                        eprintln!(
                                            "Invalid selection '{}'. Skipping '{}'.",
                                            trimmed, spec
                                        );
                                        skipped_count += 1;
                                        continue;
                                    }
                                }
                            } else {
                                eprintln!(
                                    "Ambiguous package target '{}'. Please qualify directly by repository (e.g. '{}/{}').",
                                    spec, candidates[0].repository_id, candidates[0].name
                                );
                                failed_count += 1;
                                continue;
                            }
                        }
                    };
                    let name = remote_pkg.name.clone();
                    (TargetItem::Remote(remote_pkg), name)
                };

                // Point 2: Check if package is already installed
                let (item_ver, item_fmt) = match &target_item {
                    TargetItem::Local {
                        version, format, ..
                    } => (version.as_str(), format.as_str()),
                    TargetItem::Remote(r) => (r.version.as_str(), r.format.as_str()),
                };

                if let Ok(Some(existing)) = engine.get_installed_package(&cli.profile, &pkg_name) {
                    let existing_fmt = existing.format.to_string();
                    let existing_ver = existing.version.as_str();

                    if existing_fmt != item_fmt {
                        let prompt = format!(
                            "Package '{}' is already installed as [{}] ({}). Overwrite with [{}] ({})? [y/N]: ",
                            pkg_name, existing_fmt, existing_ver, item_fmt, item_ver
                        );
                        if !prompt_confirm(&prompt, false, yes || non_interactive || json_mode)? {
                            println!("Skipping '{}'.", pkg_name);
                            skipped_count += 1;
                            continue;
                        }
                    } else if existing_ver == item_ver {
                        let prompt = format!(
                            "Package '{}' ({}, [{}]) is already installed in profile '{}'. Reinstall? [y/N]: ",
                            pkg_name, existing_ver, existing_fmt, cli.profile
                        );
                        if !prompt_confirm(&prompt, false, yes || non_interactive || json_mode)? {
                            println!("Skipping '{}'.", pkg_name);
                            skipped_count += 1;
                            continue;
                        }
                    } else {
                        let prompt = format!(
                            "Package '{}' is already installed ({} [{}]). Replace with version {}? [Y/n]: ",
                            pkg_name, existing_ver, existing_fmt, item_ver
                        );
                        if !prompt_confirm(&prompt, true, yes || non_interactive || json_mode)? {
                            println!("Skipping '{}'.", pkg_name);
                            skipped_count += 1;
                            continue;
                        }
                    }
                }

                queue.push(target_item);
            }

            // Phase 2: Execution Queue
            for item in queue {
                let (preferred_repo, preferred_format) = match &item {
                    TargetItem::Local { format, .. } => (None, Some(format.clone())),
                    TargetItem::Remote(p) => {
                        (Some(p.repository_id.clone()), Some(p.format.clone()))
                    }
                };

                let (artifact_path, pkg_name) = match item {
                    TargetItem::Local { path, name, .. } => (path, name),
                    TargetItem::Remote(remote_pkg) => {
                        let name = remote_pkg.name.clone();
                        if !json_mode {
                            println!(
                                "Found {} {} [{}] in {}",
                                remote_pkg.name,
                                remote_pkg.version,
                                remote_pkg.format,
                                remote_pkg.repository_id
                            );
                        }
                        if dry_run {
                            if !json_mode {
                                println!(
                                    "Would download {} from {}",
                                    remote_pkg.name, remote_pkg.url
                                );
                            }
                            let cache_path =
                                engine.layout().artifact_cache_path(&remote_pkg.digest);
                            if cache_path.exists() {
                                let valid_cache =
                                    std::fs::symlink_metadata(&cache_path).ok().is_some_and(
                                        |metadata| {
                                            metadata.is_file() && !metadata.file_type().is_symlink()
                                        },
                                    ) && std::fs::metadata(&cache_path).ok().is_some_and(
                                        |metadata| metadata.len() == remote_pkg.size_bytes,
                                    ) && pkg_core::domain::package::ArtifactDigest::from_file(
                                        &cache_path,
                                    )
                                    .ok()
                                    .is_some_and(|digest| {
                                        digest.hex().eq_ignore_ascii_case(&remote_pkg.digest)
                                    });
                                if !valid_cache {
                                    eprintln!(
                                        "Dry-run cannot use cached artifact for '{}' because its digest or size does not match the repository snapshot.",
                                        remote_pkg.name
                                    );
                                    failed_count += 1;
                                    continue;
                                }
                                (cache_path, name)
                            } else {
                                eprintln!(
                                    "Dry-run cannot produce a verified plan for '{}' because the artifact is not cached.",
                                    remote_pkg.name
                                );
                                failed_count += 1;
                                continue;
                            }
                        } else {
                            if !json_mode {
                                println!("Downloading {}...", remote_pkg.name);
                            }
                            match download_with_progress(&engine, &remote_pkg, json_mode).await {
                                Ok(p) => (p, name),
                                Err(e) => {
                                    eprintln!("Error downloading '{}': {}", remote_pkg.name, e);
                                    failed_count += 1;
                                    continue;
                                }
                            }
                        }
                    }
                };

                let allow_missing = ignore_missing_libs;

                // Resolve declared package dependencies before the main plan.
                // The planner will validate the resulting installed snapshot
                // again, so a failed dependency can never be reported as a
                // successful install.
                if !dry_run && !no_deps {
                    if let Err(e) = install_declared_dependencies(
                        &engine,
                        &artifact_path,
                        &cli.profile,
                        preferred_repo.as_deref(),
                        preferred_format.as_deref(),
                        config.as_ref(),
                        yes || non_interactive || json_mode,
                        json_mode,
                        &[],
                        1,
                    )
                    .await
                    {
                        eprintln!(
                            "Failed to resolve declared dependencies for '{}': {}",
                            pkg_name, e
                        );
                        failed_count += 1;
                        continue;
                    }
                }

                // Point 3: Preflight inspection for missing shared libraries
                if !dry_run {
                    let preflight =
                        match engine.preflight_check_with_profile(&artifact_path, &cli.profile) {
                            Ok(report) => report,
                            Err(e) => {
                                eprintln!("Failed to inspect '{}': {}", pkg_name, e);
                                failed_count += 1;
                                continue;
                            }
                        };
                    if !preflight.missing_libraries.is_empty() {
                        if !json_mode {
                            println!(
                                "\n⚠️  Package '{}' requires missing host libraries:\n  {}",
                                preflight.package.name,
                                preflight.missing_libraries.join(", ")
                            );
                        }

                        let mut detected_deps = Vec::new();
                        if !no_deps {
                            for dep in &preflight.package.dependencies {
                                let dep_name = dep.name.as_str();
                                if preflight
                                    .missing_libraries
                                    .iter()
                                    .any(|m| matches_missing_library(dep_name, m))
                                {
                                    if let Ok(RemoteResolution::Exact(p)) = engine
                                        .resolve_dependency_package(
                                            dep_name,
                                            preferred_repo.as_deref(),
                                            preferred_format.as_deref(),
                                            config.as_ref(),
                                        )
                                    {
                                        if !detected_deps.iter().any(
                                            |d: &pkg_core::domain::package::RemotePackage| {
                                                d.name == p.name
                                            },
                                        ) {
                                            detected_deps.push(p);
                                        }
                                    }
                                }
                            }
                        }

                        if !detected_deps.is_empty() {
                            if !json_mode {
                                println!("The following dependency packages can be installed:");
                            }
                            for d in &detected_deps {
                                if !json_mode {
                                    println!(
                                        "  - {} {} [{}] (from {})",
                                        d.name, d.version, d.format, d.repository_id
                                    );
                                }
                            }
                            let install_deps = prompt_confirm(
                                "Install missing dependencies automatically? [Y/n]: ",
                                true,
                                yes || non_interactive || json_mode,
                            )?;
                            if install_deps {
                                for d in &detected_deps {
                                    if !json_mode {
                                        println!("Downloading dependency {}...", d.name);
                                    }
                                    match download_with_progress(&engine, d, json_mode).await {
                                        Ok(dep_path) => {
                                            match engine.install_with_options(
                                                &dep_path,
                                                &cli.profile,
                                                false,
                                                InstallOptions {
                                                    allow_missing_libraries: false,
                                                    skip_dependencies: false,
                                                },
                                            ) {
                                                Ok(_) => {
                                                    if !json_mode {
                                                        println!(
                                                            "  ✓ Installed dependency {} {}",
                                                            d.name, d.version
                                                        );
                                                    }
                                                }
                                                Err(e) => {
                                                    eprintln!(
                                                        "Failed to install dependency '{}': {}",
                                                        d.name, e
                                                    );
                                                    failed_count += 1;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "Failed to install dependency '{}': {}",
                                                d.name, e
                                            );
                                            failed_count += 1;
                                        }
                                    }
                                }
                            }
                        }

                        // Re-evaluate missing libraries after dependency installation
                        let remaining_missing = match engine
                            .preflight_check_with_profile(&artifact_path, &cli.profile)
                        {
                            Ok(recheck) => recheck.missing_libraries,
                            Err(e) => {
                                eprintln!(
                                    "Failed to recheck dependencies for '{}': {}",
                                    preflight.package.name, e
                                );
                                failed_count += 1;
                                continue;
                            }
                        };

                        if remaining_missing.is_empty() {
                            if !json_mode {
                                println!("  ✓ All library dependencies successfully satisfied.");
                            }
                        } else if allow_missing {
                            if !json_mode {
                                println!(
                                    "  ⚠️  Proceeding with explicitly unresolved libraries: {}",
                                    remaining_missing.join(", ")
                                );
                            }
                        } else {
                            if !json_mode {
                                println!(
                                    "\n⚠️  The following host libraries are still missing:\n  {}",
                                    remaining_missing.join(", ")
                                );
                            }
                            eprintln!(
                                "Skipping '{}' due to missing host libraries.",
                                preflight.package.name
                            );
                            failed_count += 1;
                            continue;
                        }
                    }
                }

                let install_result = engine.install_with_options(
                    &artifact_path,
                    &cli.profile,
                    dry_run,
                    InstallOptions {
                        allow_missing_libraries: allow_missing,
                        skip_dependencies: no_deps,
                    },
                );

                match install_result {
                    Ok(plan) => {
                        if json_mode {
                            json_plans.push(plan.clone());
                        } else if dry_run {
                            println!("Install Plan (dry-run):");
                            println!(
                                "  Package:      {} {}",
                                plan.package.name, plan.package.version
                            );
                            println!("  Architecture: {}", plan.package.architecture);
                            println!("  Format:       {}", plan.package.format);
                            println!("  Digest:       {}", plan.package.digest);
                            println!("  Store target: {}", plan.target_store_dir.display());
                            if !plan.binaries.is_empty() {
                                println!("  Binaries to activate:");
                                for bin in &plan.binaries {
                                    println!(
                                        "    - {} -> {}",
                                        bin.command,
                                        bin.profile_symlink_path.display()
                                    );
                                }
                            }
                            if !plan.resolved_dependencies.is_empty() {
                                println!("  Dependency closure:");
                                for dependency in &plan.resolved_dependencies {
                                    println!(
                                        "    - {} {} [{}]",
                                        dependency.name, dependency.version, dependency.format
                                    );
                                }
                            }
                            if !plan.ignored_scripts.is_empty() {
                                println!("  Ignored maintainer scripts (policy default-deny):");
                                for script in &plan.ignored_scripts {
                                    println!("    - {script}");
                                }
                            }
                            if !plan.adaptations.is_empty() {
                                println!("  Adaptations:");
                                for adaptation in &plan.adaptations {
                                    println!(
                                        "    - {} v{} ({} output(s))",
                                        adaptation.recipe_id,
                                        adaptation.recipe_version,
                                        adaptation.outputs.len()
                                    );
                                }
                            }
                            if !plan.runtimes.is_empty() {
                                println!("  Runtime manifests:");
                                for runtime in &plan.runtimes {
                                    println!(
                                        "    - {} ({})",
                                        runtime.runtime_id, runtime.execution.command
                                    );
                                }
                            } else if !plan.binaries.is_empty() {
                                println!(
                                    "  Runtime evidence: deferred until a verified staging run"
                                );
                            }
                        } else {
                            println!("Installed {} {}", plan.package.name, plan.package.version);
                            if !plan.missing_libraries.is_empty() {
                                println!(
                                    "  ⚠️  WARNING: Executable(s) installed with missing libraries: {}",
                                    plan.missing_libraries.join(", ")
                                );
                            }
                            if !plan.binaries.is_empty() {
                                println!("Activated binaries in profile '{}':", cli.profile);
                                for bin in &plan.binaries {
                                    println!("  - {}", bin.command);
                                }
                            }
                            if !plan.ignored_scripts.is_empty() {
                                println!("Ignored maintainer scripts:");
                                for script in &plan.ignored_scripts {
                                    println!("  - {script} (default-deny)");
                                }
                            }
                            if !plan.resolved_dependencies.is_empty() {
                                println!("Resolved dependency closure:");
                                for dependency in &plan.resolved_dependencies {
                                    println!(
                                        "  - {} {} [{}]",
                                        dependency.name, dependency.version, dependency.format
                                    );
                                }
                            }
                            if !plan.adaptations.is_empty() {
                                println!("Applied adaptations:");
                                for adaptation in &plan.adaptations {
                                    println!(
                                        "  - {} v{} ({} output(s))",
                                        adaptation.recipe_id,
                                        adaptation.recipe_version,
                                        adaptation.outputs.len()
                                    );
                                }
                            }
                            if !plan.runtimes.is_empty() {
                                println!("Per-command runtimes:");
                                for runtime in &plan.runtimes {
                                    println!(
                                        "  - {} ({}, {})",
                                        runtime.runtime_id,
                                        runtime.execution.command,
                                        if runtime.verified {
                                            "verified"
                                        } else {
                                            "unverified"
                                        }
                                    );
                                }
                            }
                        }
                        installed_count += 1;
                    }
                    Err(e) => {
                        eprintln!("Error installing '{}': {}", pkg_name, e);
                        failed_count += 1;
                    }
                }
            }

            if json_mode {
                emit_json(&serde_json::json!({
                    "status": if failed_count == 0 { "success" } else { "error" },
                    "profile": cli.profile,
                    "plans": json_plans,
                    "installed": installed_count,
                    "skipped": skipped_count,
                    "failed": failed_count,
                    "errors": json_errors,
                }))?;
            } else if targets.len() > 1 {
                println!(
                    "\nSummary: {} installed, {} skipped, {} failed.",
                    installed_count, skipped_count, failed_count
                );
            }

            if failed_count > 0 {
                return Err(anyhow::anyhow!(
                    "Installation failed for requested target(s)."
                ));
            }
        }
        Commands::Remove { name, dry_run } => {
            let plan = engine.remove(&name, &cli.profile, dry_run)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "operation": "remove",
                    "dry_run": dry_run,
                    "profile": cli.profile,
                    "plan": plan,
                }))?;
            } else if dry_run {
                println!("Remove Plan (dry-run):");
                println!("  Package:  {} {}", plan.package_name, plan.version);
                println!("  Store ID: {}", plan.store_id);
                println!("  Store path: {}", plan.store_path.display());
                if !plan.binaries_to_remove.is_empty() {
                    println!("  Binaries to unlink:");
                    for bin in &plan.binaries_to_remove {
                        println!("    - {}", bin.display());
                    }
                }
            } else {
                println!(
                    "Removed {} {} from profile '{}'",
                    plan.package_name, plan.version, cli.profile
                );
            }
        }
        Commands::Gc { dry_run } => {
            let report = engine.gc(dry_run)?;
            if json_mode {
                emit_json(&report)?;
            } else if report.candidates.is_empty() && report.runtime_manifests.is_empty() {
                println!("No unreachable pkg-owned store objects found.");
            } else if dry_run {
                println!("GC plan (dry-run):");
                for candidate in &report.candidates {
                    println!(
                        "  - {} {} [{}] -> {}",
                        candidate.package_name,
                        candidate.version,
                        candidate.store_id,
                        candidate.store_path.display()
                    );
                }
                println!("{} object(s) would be collected.", report.candidates.len());
                if !report.runtime_manifests.is_empty() {
                    println!(
                        "{} orphan runtime manifest(s) would be collected.",
                        report.runtime_manifests.len()
                    );
                }
            } else {
                println!(
                    "Collected {} unreachable store object(s).",
                    report.candidates.len()
                );
                if !report.runtime_manifests.is_empty() {
                    println!(
                        "Collected {} orphan runtime manifest(s).",
                        report.runtime_manifests.len()
                    );
                }
            }
            if !report.retained.is_empty() {
                println!(
                    "Retained {} object(s) with references or uncertain state.",
                    report.retained.len()
                );
            }
        }
        Commands::Doctor { all, repair } => {
            let selected = if all {
                None
            } else {
                Some(cli.profile.as_str())
            };
            let report = if repair {
                engine.doctor_repair(selected)?
            } else {
                engine.doctor(selected)?
            };
            if json_mode {
                emit_json(&report)?;
            } else {
                for finding in &report.findings {
                    let path = finding
                        .path
                        .as_ref()
                        .map(|path| format!(" ({})", path.display()))
                        .unwrap_or_default();
                    println!(
                        "[{:?}] {}: {}{}",
                        finding.level, finding.code, finding.message, path
                    );
                }
            }
            if report.exit_code() != 0 {
                return Err(anyhow::anyhow!(
                    "doctor encontrou inconsistências (exit code {})",
                    report.exit_code()
                ));
            }
        }
        Commands::Integrate { package, dry_run } => {
            let plan = engine.integrate(&cli.profile, &package, dry_run)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "operation": if dry_run { "integrate_plan" } else { "integrate" },
                    "plan": plan,
                }))?;
            } else {
                print_integration_plan(&plan, dry_run, "integration");
            }
        }
        Commands::Deintegrate { package, dry_run } => {
            let plan = engine.deintegrate(&cli.profile, &package, dry_run)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "operation": if dry_run { "deintegrate_plan" } else { "deintegrate" },
                    "plan": plan,
                }))?;
            } else {
                print_integration_plan(&plan, dry_run, "deintegration");
            }
        }
        Commands::Rollback { generation } => {
            let selected = engine.rollback(&cli.profile, generation.as_deref())?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "operation": "rollback",
                    "profile": cli.profile,
                    "generation": selected,
                }))?;
            } else {
                println!(
                    "Rolled profile '{}' back to generation '{}'.",
                    cli.profile, selected
                );
            }
        }
        Commands::Run { command, args } => {
            if json_mode {
                let output = engine.run_command_capture(&cli.profile, &command, &args)?;
                let success = output.status.success();
                emit_json(&serde_json::json!({
                    "status": if success { "success" } else { "error" },
                    "profile": cli.profile,
                    "command": command,
                    "exit_code": output.status.code(),
                    "stdout": String::from_utf8_lossy(&output.stdout),
                    "stderr": String::from_utf8_lossy(&output.stderr),
                }))?;
                if !success {
                    return Err(anyhow::anyhow!("command '{command}' exited unsuccessfully"));
                }
                return Ok(());
            }
            #[cfg(unix)]
            {
                engine.exec_command(&cli.profile, &command, &args)?;
            }
            #[cfg(not(unix))]
            {
                let status = engine.run_command(&cli.profile, &command, &args)?;
                if !status.success() {
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
        }
        Commands::Migrate => {
            let generation = engine.migrate_legacy_profile(&cli.profile)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "operation": "migrate",
                    "profile": cli.profile,
                    "generation": generation,
                    "verified": false,
                }))?;
            } else {
                println!(
                    "Captured legacy profile '{}' as unverified generation '{}'.",
                    cli.profile, generation
                );
            }
        }
        Commands::List => {
            let packages = engine.list(&cli.profile)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "profile": cli.profile,
                    "packages": packages,
                }))?;
            } else if packages.is_empty() {
                println!("No packages installed in profile '{}'.", cli.profile);
            } else {
                println!(
                    "{:<20} {:<15} {:<10} {:<8} {:<8} STORE_ID",
                    "NAME", "VERSION", "STATUS", "FORMAT", "ACTIVE"
                );
                let status = "installed";
                for pkg in packages {
                    let active_str = if pkg.active { "yes" } else { "no" };
                    let format_str = pkg.format.to_string();
                    println!(
                        "{:<20} {:<15} {:<10} {:<8} {:<8} {}",
                        pkg.name.as_str(),
                        pkg.version.as_str(),
                        status,
                        format_str,
                        active_str,
                        pkg.store_id
                    );
                }
            }
        }
        Commands::Sync => {
            sync_repositories(&engine, json_mode).await?;
        }
        Commands::Mcp => {
            run_mcp(&engine, &cli.profile).await?;
        }
        Commands::Repo { command } => {
            let config_path = engine.layout().base_dir().join("repositories.toml");
            match command {
                RepoCommands::Sync => {
                    sync_repositories(&engine, json_mode).await?;
                }
                RepoCommands::List { remote } => {
                    if remote {
                        let registry =
                            pkg_core::repository::CuratedRegistry::fetch_remote_or_fallback().await;
                        let installed_ids: std::collections::HashSet<String> = if config_path
                            .exists()
                        {
                            pkg_core::repository::RepositoriesConfig::load_from_file(&config_path)
                                .map(|cfg| cfg.repositories.into_iter().map(|r| r.id).collect())
                                .unwrap_or_default()
                        } else {
                            std::collections::HashSet::new()
                        };

                        if json_mode {
                            let repos_json: Vec<_> = registry
                                .repositories
                                .iter()
                                .map(|r| {
                                    serde_json::json!({
                                        "id": r.id,
                                        "name": r.name,
                                        "distro": r.distro,
                                        "format": r.format,
                                        "url": r.url,
                                        "distribution": r.distribution,
                                        "components": r.components,
                                        "priority": r.priority,
                                        "installed": installed_ids.contains(&r.id),
                                        "description": r.description,
                                    })
                                })
                                .collect();
                            emit_json(&serde_json::json!({
                                "status": "success",
                                "schema_version": registry.schema_version,
                                "repositories": repos_json,
                            }))?;
                        } else {
                            println!(
                                "{:<18} {:<8} {:<6} {:<12} DESCRIPTION",
                                "ID", "DISTRO", "FORMAT", "STATUS"
                            );
                            for r in &registry.repositories {
                                let status = if installed_ids.contains(&r.id) {
                                    "installed"
                                } else {
                                    "available"
                                };
                                let desc = r.description.as_deref().unwrap_or("-");
                                println!(
                                    "{:<18} {:<8} {:<6} {:<12} {}",
                                    r.id, r.distro, r.format, status, desc
                                );
                            }
                            println!("\nTip: Run `pkg repo add <id>` to enable an official repository.");
                        }
                        return Ok(());
                    }

                    if !config_path.exists() {
                        if json_mode {
                            emit_json(
                                &serde_json::json!({"status": "success", "repositories": []}),
                            )?;
                        } else {
                            println!("No repositories configured.");
                            println!("Run `pkg sync` or `pkg repo list --remote` to get started.");
                        }
                        return Ok(());
                    }
                    let config =
                        pkg_core::repository::RepositoriesConfig::load_from_file(&config_path)?;
                    if json_mode {
                        emit_json(&serde_json::json!({
                            "status": "success",
                            "repositories": config.repositories,
                        }))?;
                    } else {
                        println!(
                            "{:<20} {:<8} {:<10} {:<35} {:<15} COMPONENTS",
                            "ID", "FORMAT", "PRIORITY", "URL", "DISTRIBUTION"
                        );
                        for repo in config.repositories {
                            let prio_str = repo
                                .priority
                                .map(|p| p.to_string())
                                .unwrap_or_else(|| "-".to_string());
                            println!(
                                "{:<20} {:<8} {:<10} {:<35} {:<15} {}",
                                repo.id,
                                repo.format,
                                prio_str,
                                repo.url,
                                repo.distribution,
                                repo.components.join(", ")
                            );
                        }
                    }
                }
                RepoCommands::Add {
                    id,
                    format,
                    url,
                    distribution,
                    components,
                    priority,
                } => {
                    let mut config = if config_path.exists() {
                        pkg_core::repository::RepositoriesConfig::load_from_file(&config_path)?
                    } else {
                        pkg_core::repository::RepositoriesConfig {
                            repositories: vec![],
                        }
                    };

                    if config.repositories.iter().any(|r| r.id == id) {
                        return Err(anyhow::anyhow!(
                            "Repository with ID '{}' already exists in {}",
                            id,
                            config_path.display()
                        ));
                    }

                    let repo_to_add = match url {
                        Some(url_str) => {
                            let dist = distribution.ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Distribution suite is required when adding a custom repository"
                                )
                            })?;
                            pkg_core::repository::RepositoryConfig {
                                id: id.clone(),
                                format: format.unwrap_or_else(|| "deb".to_string()),
                                url: url_str,
                                distribution: dist,
                                components,
                                public_key_path: None,
                                priority,
                            }
                        }
                        None => {
                            let registry =
                                pkg_core::repository::CuratedRegistry::fetch_remote_or_fallback()
                                    .await;
                            let curated = registry.find_by_id(&id).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "Repository '{}' is not in the curated registry.\nUse: pkg repo add <id> <url> <distribution> [components...]\nOr check available repositories with: pkg repo list --remote",
                                    id
                                )
                            })?;

                            pkg_core::repository::RepositoryConfig {
                                id: curated.id.clone(),
                                format: format.unwrap_or_else(|| curated.format.clone()),
                                url: curated.url.clone(),
                                distribution: distribution
                                    .unwrap_or_else(|| curated.distribution.clone()),
                                components: if components.is_empty() {
                                    curated.components.clone()
                                } else {
                                    components
                                },
                                public_key_path: curated.public_key_path.clone(),
                                priority: priority.or(curated.priority),
                            }
                        }
                    };

                    config.repositories.push(repo_to_add.clone());

                    let toml_string = toml::to_string_pretty(&config)?;
                    std::fs::write(&config_path, toml_string)?;
                    if json_mode {
                        emit_json(&serde_json::json!({
                            "status": "success",
                            "operation": "repo_add",
                            "repository": repo_to_add,
                        }))?;
                    } else {
                        println!(
                            "Successfully added repository '{}' ({})",
                            repo_to_add.id, repo_to_add.url
                        );
                        println!("Run `pkg sync` or `pkg repo sync` to synchronize packages.");
                    }
                }
            }
        }
        Commands::Search {
            query,
            format,
            repo,
        } => {
            let results = engine.search_filtered(&query, format.as_deref(), repo.as_deref())?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "query": query,
                    "results": results,
                }))?;
            } else if results.is_empty() {
                println!(
                    "No packages matching '{}' found in active snapshots.",
                    query
                );
            } else {
                println!(
                    "{:<25} {:<20} {:<10} {:<8} {:<25} SIZE",
                    "NAME", "VERSION", "ARCH", "FORMAT", "REPOSITORY"
                );
                for pkg in &results {
                    println!(
                        "{:<25} {:<20} {:<10} {:<8} {:<25} {} bytes",
                        pkg.name,
                        pkg.version,
                        pkg.architecture,
                        pkg.format,
                        pkg.repository_id,
                        pkg.size_bytes
                    );
                }
                println!("\nTotal: {} package(s) found.", results.len());
            }
        }
        Commands::Info { target } => {
            let info = engine.info(&target, &cli.profile)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "profile": cli.profile,
                    "info": info,
                }))?;
            } else {
                match info {
                    PackageInfo::LocalArtifact(pkg) => {
                        println!("Identity:");
                        println!("  Name:         {}", pkg.name);
                        println!("  Version:      {}", pkg.version);
                        println!("  Architecture: {}", pkg.architecture);
                        println!("  Format:       {}", pkg.format);
                        if let Some(ref desc) = pkg.description {
                            println!("  Description:  {desc}");
                        }
                        println!("\nProvenance:");
                        println!("  Digest:       {}", pkg.digest);
                        println!("  Artifact size: {} bytes", pkg.size_bytes);
                        if let Some(size) = pkg.installed_size {
                            println!("  Installed size: {size} bytes");
                        }
                        if !pkg.provides.is_empty() {
                            println!("\nProvides:");
                            for cap in &pkg.provides {
                                println!("  - {cap}");
                            }
                        }
                        if !pkg.scripts.is_empty() {
                            println!("\nMaintainer Scripts (policy default-deny):");
                            for s in &pkg.scripts {
                                println!("  - {}", s.name);
                            }
                        }
                    }
                    PackageInfo::Installed(pkg) => {
                        println!("Identity:");
                        println!("  Name:         {}", pkg.name);
                        println!("  Version:      {}", pkg.version);
                        println!("  Architecture: {}", pkg.architecture);
                        println!("  Format:       {}", pkg.format);
                        println!("\nProvenance:");
                        println!("  Digest:       {}", pkg.digest);
                        println!("\nLocal State:");
                        println!("  Store ID:     {}", pkg.store_id);
                        println!("  Store path:   {}", pkg.store_path.display());
                        println!("  Profile:      {}", pkg.profile);
                        println!("  Installed at: {}", pkg.installed_at);
                        println!("  Active:       {}", if pkg.active { "yes" } else { "no" });
                        if !pkg.binaries.is_empty() {
                            println!("\nActivated Binaries:");
                            for bin in &pkg.binaries {
                                println!("  - {bin}");
                            }
                        }
                    }
                }
            }
        }
        Commands::QueryCommand { command } => {
            let packages = engine.query_command(&cli.profile, &command)?;
            if json_mode {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "profile": cli.profile,
                    "command": command,
                    "packages": packages,
                }))?;
            } else if packages.is_empty() {
                println!("No installed package exposes '{command}'.");
            } else {
                for package in packages {
                    println!("{} {}", package.name, package.version);
                }
            }
        }
        Commands::Upgrade {
            name,
            dry_run,
            yes: _yes,
            jobs,
        } => {
            let upgrades = engine.plan_upgrade(&cli.profile, name.as_deref())?;
            if json_mode && dry_run {
                emit_json(&serde_json::json!({
                    "status": "success",
                    "operation": "upgrade",
                    "dry_run": true,
                    "profile": cli.profile,
                    "upgrades": upgrades,
                }))?;
            } else if dry_run {
                if upgrades.is_empty() {
                    println!("No upgrades available for profile '{}'.", cli.profile);
                } else {
                    println!("Upgrade plan (dry-run):");
                    for upgrade in &upgrades {
                        println!(
                            "  - {} {} -> {} [{}] from {}",
                            upgrade.installed.name,
                            upgrade.installed.version,
                            upgrade.candidate.version,
                            upgrade.candidate.format,
                            upgrade.candidate.repository_id
                        );
                    }
                }
            } else if upgrades.is_empty() {
                if json_mode {
                    emit_json(&serde_json::json!({
                        "status": "success",
                        "operation": "upgrade",
                        "profile": cli.profile,
                        "upgrades": [],
                    }))?;
                } else {
                    println!("No upgrades available for profile '{}'.", cli.profile);
                }
            } else {
                let original_generation = engine
                    .db()
                    .active_generation(&cli.profile)?
                    .map(|generation| generation.generation_id);
                let config_path = engine.layout().base_dir().join("repositories.toml");
                let config = config_path
                    .is_file()
                    .then(|| pkg_core::repository::RepositoriesConfig::load_from_file(&config_path))
                    .transpose()?;
                let mut applied = Vec::new();
                let mut error = None;
                let replacement_names: Vec<_> = upgrades
                    .iter()
                    .map(|upgrade| upgrade.installed.name.clone())
                    .collect();
                let upgrade_artifacts = match acquire_remote_artifacts(
                    engine.layout(),
                    upgrades.iter().map(|upgrade| upgrade.candidate.clone()),
                    jobs,
                    json_mode,
                )
                .await
                {
                    Ok(artifacts) => artifacts,
                    Err(err) => {
                        let error = format!("failed to acquire upgrade artifacts: {err}");
                        if json_mode {
                            emit_json(&serde_json::json!({
                                "status": "error",
                                "operation": "upgrade",
                                "profile": cli.profile,
                                "applied": applied,
                                "error": error,
                                "rolled_back_to": original_generation,
                            }))?;
                        }
                        return Err(anyhow::anyhow!(error));
                    }
                };
                for upgrade in &upgrades {
                    match upgrade_artifacts.get(&upgrade.candidate.digest.to_ascii_lowercase()) {
                        Some(acquired) => {
                            let artifact = &acquired.path;
                            if let Err(err) = install_declared_dependencies(
                                &engine,
                                artifact,
                                &cli.profile,
                                Some(&upgrade.candidate.repository_id),
                                Some(&upgrade.candidate.format),
                                config.as_ref(),
                                true,
                                json_mode,
                                &replacement_names,
                                jobs,
                            )
                            .await
                            {
                                error = Some(format!(
                                    "failed to resolve dependencies for upgrade '{}': {err}",
                                    upgrade.candidate.name
                                ));
                                break;
                            }
                            match engine.install_with_options_replacing(
                                artifact,
                                &cli.profile,
                                false,
                                InstallOptions::default(),
                                &replacement_names,
                            ) {
                                Ok(plan) => applied.push(plan),
                                Err(err) => {
                                    error = Some(format!(
                                        "failed to install upgrade '{}': {err}",
                                        upgrade.candidate.name
                                    ));
                                    break;
                                }
                            }
                        }
                        None => {
                            error = Some(format!(
                                "acquisition returned no artifact for upgrade '{}' ({})",
                                upgrade.candidate.name, upgrade.candidate.digest
                            ));
                            break;
                        }
                    }
                }
                if let Some(error) = error {
                    if let Some(generation) = original_generation.as_deref() {
                        let _ = engine.rollback(&cli.profile, Some(generation));
                    }
                    if json_mode {
                        emit_json(&serde_json::json!({
                            "status": "error",
                            "operation": "upgrade",
                            "profile": cli.profile,
                            "applied": applied,
                            "error": error,
                            "rolled_back_to": original_generation,
                        }))?;
                    }
                    return Err(anyhow::anyhow!(error));
                }
                if json_mode {
                    emit_json(&serde_json::json!({
                        "status": "success",
                        "operation": "upgrade",
                        "profile": cli.profile,
                        "applied": applied,
                    }))?;
                } else {
                    for plan in applied {
                        println!(
                            "Upgraded {} to {}.",
                            plan.package.name, plan.package.version
                        );
                    }
                }
            }
        }
        Commands::Profile { command } => match command {
            ProfileCommands::Create { name } => {
                engine.create_profile(&name)?;
                if json_mode {
                    emit_json(
                        &serde_json::json!({"status": "success", "operation": "profile_create", "profile": name}),
                    )?;
                } else {
                    println!("Created profile '{name}'.");
                }
            }
            ProfileCommands::Drop { name } => {
                engine.drop_profile(&name)?;
                if json_mode {
                    emit_json(
                        &serde_json::json!({"status": "success", "operation": "profile_drop", "profile": name}),
                    )?;
                } else {
                    println!("Dropped profile '{name}'.");
                }
            }
            ProfileCommands::List => {
                let profiles = engine.list_profiles()?;
                if json_mode {
                    emit_json(&serde_json::json!({"status": "success", "profiles": profiles}))?;
                } else if profiles.is_empty() {
                    println!("No profiles found.");
                } else {
                    for profile in profiles {
                        println!("{profile}");
                    }
                }
            }
        },
        Commands::Update { .. } => {
            unreachable!("pkg update is normalized to pkg upgrade before dispatch")
        }
    }

    Ok(())
}

fn exit_code(error: &anyhow::Error) -> i32 {
    if let Some(error) = error.downcast_ref::<pkg_core::Error>() {
        return match error {
            pkg_core::Error::PackageNotFound(_) => 10,
            pkg_core::Error::ActivationConflict { .. } => 20,
            pkg_core::Error::ResolutionFailed(_) => 21,
            pkg_core::Error::ArchitectureMismatch { .. } | pkg_core::Error::IncompatibleHost(_) => {
                30
            }
            pkg_core::Error::LockError(_) => 40,
            _ => 1,
        };
    }
    1
}

#[tokio::main]
async fn main() {
    let json_requested = std::env::args().any(|arg| arg == "--json");
    if let Err(error) = run().await {
        if json_requested && !JSON_EMITTED.load(Ordering::Relaxed) {
            let code = exit_code(&error);
            let _ = emit_json(&serde_json::json!({
                "status": "error",
                "code": code,
                "error": error.to_string(),
            }));
        }
        eprintln!("Error: {error}");
        std::process::exit(exit_code(&error));
    }
}

enum TargetItem {
    Local {
        path: PathBuf,
        name: String,
        version: String,
        format: String,
    },
    Remote(pkg_core::domain::package::RemotePackage),
}

fn prompt_confirm(prompt: &str, default_yes: bool, assume_yes: bool) -> Result<bool> {
    use std::io::{IsTerminal, Write};

    if assume_yes {
        return Ok(true);
    }

    if !std::io::stdin().is_terminal() {
        return Ok(default_yes);
    }

    print!("{}", prompt);
    std::io::stdout().flush().ok();

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();

    if trimmed.is_empty() {
        Ok(default_yes)
    } else if trimmed == "y" || trimmed == "yes" {
        Ok(true)
    } else if trimmed == "n" || trimmed == "no" {
        Ok(false)
    } else {
        Ok(default_yes)
    }
}

fn print_integration_plan(
    plan: &pkg_core::domain::integration::IntegrationPlan,
    dry_run: bool,
    operation: &str,
) {
    if dry_run {
        println!("{} plan for '{}' (dry-run):", operation, plan.package_name);
    } else {
        println!("{} applied for '{}':", operation, plan.package_name);
    }
    if plan.actions.is_empty() {
        println!("  No supported user-space integration resources found.");
    }
    for action in &plan.actions {
        println!(
            "  - {}: {} -> {}",
            action.kind.as_str(),
            action.source_path.display(),
            action.target_path.display()
        );
    }
    for conflict in &plan.conflicts {
        println!(
            "  ! conflict {} at {}: {}",
            conflict.kind.as_str(),
            conflict.target_path.display(),
            conflict.reason
        );
    }
}

async fn download_with_progress(
    engine: &Engine,
    remote_pkg: &pkg_core::domain::package::RemotePackage,
    json_mode: bool,
) -> Result<PathBuf> {
    if json_mode {
        return Ok(engine
            .download_remote_with_progress(remote_pkg, |_, _| {})
            .await?);
    }
    use indicatif::{ProgressBar, ProgressStyle};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let pb = ProgressBar::new(remote_pkg.size_bytes);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    );

    let pb_clone = pb.clone();
    let was_cached = Arc::new(AtomicBool::new(true));
    let was_cached_clone = Arc::clone(&was_cached);

    let path = engine
        .download_remote_with_progress(remote_pkg, move |downloaded, total| {
            was_cached_clone.store(false, Ordering::Relaxed);
            if let Some(total) = total {
                pb_clone.set_length(total);
            }
            pb_clone.set_position(downloaded);
        })
        .await?;

    if was_cached.load(Ordering::Relaxed) {
        pb.finish_and_clear();
        println!("  ✓ Using cached artifact: {}", remote_pkg.digest);
    } else {
        pb.finish_with_message("Download complete");
    }

    Ok(path)
}

/// Fetches one digest-addressed artifact per unique catalog digest. The
/// coordinator owns output and task cancellation; individual tasks only touch
/// immutable cache paths and never retain an `Engine` or a database handle.
async fn acquire_remote_artifacts<I>(
    layout: &StoreLayout,
    packages: I,
    jobs: usize,
    json_mode: bool,
) -> Result<BTreeMap<String, ArtifactAcquisitionResult>>
where
    I: IntoIterator<Item = pkg_core::domain::package::RemotePackage>,
{
    let mut unique: BTreeMap<String, pkg_core::domain::package::RemotePackage> = BTreeMap::new();
    for package in packages {
        let key = package.digest.to_ascii_lowercase();
        if let Some(existing) = unique.get(&key) {
            if existing.size_bytes != package.size_bytes {
                anyhow::bail!(
                    "catalog candidates share digest '{}' but declare different sizes",
                    package.digest
                );
            }
            continue;
        }
        unique.insert(key, package);
    }

    let semaphore = Arc::new(Semaphore::new(jobs));
    let mut tasks: JoinSet<Result<ArtifactAcquisitionResult>> = JoinSet::new();
    for package in unique.into_values() {
        let label = format!("{} {}", package.name, package.version);
        let spec = ArtifactAcquisitionSpec::new(layout, package)?;
        let semaphore = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|_| anyhow::anyhow!("artifact acquisition coordinator was cancelled"))?;
            pkg_core::acquire_artifact(spec, |_, _| {})
                .await
                .map_err(|error| anyhow::anyhow!("artifact '{label}': {error}"))
        });
    }

    let mut acquired = BTreeMap::new();
    while let Some(task) = tasks.join_next().await {
        match task {
            Ok(Ok(result)) => {
                acquired.insert(result.package.digest.to_ascii_lowercase(), result);
            }
            Ok(Err(error)) => {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err(error);
            }
            Err(error) => {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err(anyhow::anyhow!("artifact acquisition task failed: {error}"));
            }
        }
    }

    if !json_mode {
        for result in acquired.values() {
            match result.source {
                ArtifactAcquisitionSource::Cache => {
                    println!("  ✓ Using cached artifact: {}", result.package.digest);
                }
                ArtifactAcquisitionSource::Download => {
                    println!("  ✓ Downloaded artifact: {}", result.package.digest);
                }
            }
        }
    }
    Ok(acquired)
}

fn matches_missing_library(dep_name: &str, missing_lib: &str) -> bool {
    let m_base = missing_lib.split('.').next().unwrap_or(missing_lib);
    let dep_clean = dep_name.trim_end_matches(|c: char| c.is_ascii_digit());

    if dep_name == m_base || dep_clean == m_base {
        return true;
    }
    if !dep_clean.is_empty() && (m_base.starts_with(dep_clean) || dep_clean.starts_with(m_base)) {
        return true;
    }
    if missing_lib.contains(dep_name) || dep_name.contains(m_base) {
        return true;
    }
    false
}

/// Acquires and installs the package constraints that are represented by
/// repository packages. Interpreter names are handled by the reviewed host
/// interpreter adapter and remain available to the planner as host evidence.
#[allow(clippy::too_many_arguments)]
async fn install_declared_dependencies(
    engine: &Engine,
    root_artifact: &std::path::Path,
    profile: &str,
    _preferred_repo: Option<&str>,
    _preferred_format: Option<&str>,
    _config: Option<&pkg_core::repository::RepositoriesConfig>,
    assume_yes: bool,
    json_mode: bool,
    replaced_packages: &[pkg_core::domain::package::PackageName],
    jobs: usize,
) -> Result<()> {
    // Repository snapshots are enough to choose an artifact, but some older
    // snapshots lack complete transitive constraint metadata. Re-plan every
    // downloaded artifact from its authenticated control metadata before
    // installation. This produces a dependency-first queue even when the
    // catalog only described the root dependency edge.
    let mut visits = vec![DependencyVisit::Expand {
        artifact: root_artifact.to_path_buf(),
        selected_dependency: None,
    }];
    let mut visiting = HashSet::new();
    let mut scheduled = HashSet::new();
    let mut install_queue = Vec::new();

    while let Some(visit) = visits.pop() {
        match visit {
            DependencyVisit::Expand {
                artifact,
                selected_dependency,
            } => {
                let plan =
                    engine.plan_install_with_replacements(&artifact, profile, replaced_packages)?;
                let identity = dependency_identity(&plan.package);

                if scheduled.contains(&identity) || visiting.contains(&identity) {
                    continue;
                }

                let already_satisfied = selected_dependency.is_some()
                    && !replaced_packages.contains(&plan.package.name)
                    && engine
                        .get_installed_package(profile, plan.package.name.as_str())?
                        .is_some_and(|installed| {
                            installed.version == plan.package.version
                                && installed.format == plan.package.format
                        });
                if already_satisfied {
                    scheduled.insert(identity);
                    continue;
                }

                visiting.insert(identity.clone());
                visits.push(DependencyVisit::Finish {
                    identity,
                    artifact,
                    selected_dependency,
                });

                // Resolve this artifact's direct dependency frontier before
                // fetching it. Every item in the frontier is independent of
                // the others, so it can share the bounded acquisition pool.
                // The DFS stack remains deterministic and applies the
                // resulting dependency closure serially afterwards.
                let mut frontier = Vec::new();
                for dependency in plan.resolved_dependencies {
                    let identity = dependency_identity(&dependency);
                    if scheduled.contains(&identity) || visiting.contains(&identity) {
                        continue;
                    }
                    let candidate = resolved_remote_candidate(engine, &dependency)?;
                    if !prompt_confirm(
                        &format!(
                            "Install dependency '{}' automatically? [Y/n]: ",
                            candidate.name
                        ),
                        true,
                        assume_yes,
                    )? {
                        anyhow::bail!("dependency '{}' was declined", candidate.name);
                    }
                    frontier.push((dependency, candidate));
                }

                let artifacts = acquire_remote_artifacts(
                    engine.layout(),
                    frontier.iter().map(|(_, candidate)| candidate.clone()),
                    jobs,
                    json_mode,
                )
                .await?;

                // A stack is LIFO, so reverse the resolver's deterministic
                // dependency order. Each dependency is then expanded only
                // after the metadata of its own artifact is authenticated.
                for (dependency, candidate) in frontier.into_iter().rev() {
                    let artifact = artifacts
                        .get(&candidate.digest.to_ascii_lowercase())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "acquisition returned no artifact for dependency '{}' ({})",
                                candidate.name,
                                candidate.digest
                            )
                        })?
                        .path
                        .clone();
                    visits.push(DependencyVisit::Expand {
                        artifact,
                        selected_dependency: Some(dependency),
                    });
                }
            }
            DependencyVisit::Finish {
                identity,
                artifact,
                selected_dependency,
            } => {
                visiting.remove(&identity);
                if scheduled.insert(identity)
                    && let Some(dependency) = selected_dependency
                {
                    install_queue.push((dependency.name.to_string(), artifact));
                }
            }
        }
    }

    for (name, artifact) in install_queue {
        engine
            .install_with_options_replacing(
                &artifact,
                profile,
                false,
                InstallOptions::default(),
                replaced_packages,
            )
            .map_err(|e| anyhow::anyhow!("dependency '{}': {}", name, e))?;
    }
    Ok(())
}

enum DependencyVisit {
    Expand {
        artifact: PathBuf,
        selected_dependency: Option<NormalizedPackage>,
    },
    Finish {
        identity: String,
        artifact: PathBuf,
        selected_dependency: Option<NormalizedPackage>,
    },
}

fn dependency_identity(package: &NormalizedPackage) -> String {
    format!(
        "{}|{}|{}|{}",
        package.name,
        package.version,
        package.format,
        package.digest.hex()
    )
}

fn resolved_remote_candidate(
    engine: &Engine,
    dependency: &NormalizedPackage,
) -> Result<pkg_core::domain::package::RemotePackage> {
    let mut candidates = engine
        .db()
        .find_remote_candidates(dependency.name.as_str())?;
    candidates.retain(|candidate| {
        candidate.version == dependency.version.as_str()
            && candidate.format == dependency.format.to_string()
            && (candidate.digest.is_empty()
                || candidate
                    .digest
                    .eq_ignore_ascii_case(dependency.digest.hex())
                || candidate
                    .digest
                    .eq_ignore_ascii_case(&dependency.digest.to_string()))
    });
    match candidates.as_slice() {
        [candidate] => Ok(candidate.clone()),
        [] => anyhow::bail!(
            "resolved dependency '{}' {} is absent from the active catalog",
            dependency.name,
            dependency.version
        ),
        many => anyhow::bail!(
            "resolved dependency '{}' is ambiguous ({} matching artifacts)",
            dependency.name,
            many.len()
        ),
    }
}

async fn sync_repositories(engine: &Engine, json_mode: bool) -> Result<()> {
    let config_path = engine.layout().base_dir().join("repositories.toml");
    if !config_path.exists() {
        let (distro_id, _) = pkg_core::host::HostFacts::detect_distro();
        let distro_name = distro_id.as_deref().unwrap_or("generic");
        if !json_mode {
            println!(
                "No repositories.toml found at {}. Auto-detected host platform [{}]. Initializing native repositories...",
                config_path.display(),
                distro_name
            );
        }
        let default_config = pkg_core::repository::RepositoriesConfig::default_for_host();
        let toml_string = toml::to_string_pretty(&default_config)
            .map_err(|e| anyhow::anyhow!("Failed to format default repositories.toml: {e}"))?;
        std::fs::write(&config_path, toml_string)?;
    }
    if !json_mode {
        println!("Reading config from {}...", config_path.display());
    }
    let config = pkg_core::repository::RepositoriesConfig::load_from_file(&config_path)?;
    if !json_mode {
        println!("Updating {} repositories...", config.repositories.len());
    }

    let spinner = if json_mode {
        None
    } else {
        let spinner = indicatif::ProgressBar::new_spinner();
        spinner.set_style(
            indicatif::ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                .template("{spinner:.green} {msg}")
                .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
        );
        spinner.set_message("Synchronizing repository indexes and verifying signatures...");
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(spinner)
    };

    let total = engine.update(&config).await?;
    if let Some(spinner) = spinner {
        spinner.finish_and_clear();
    }

    if json_mode {
        emit_json(&serde_json::json!({
            "status": "success",
            "operation": "sync",
            "repositories": config.repositories.len(),
            "packages": total,
        }))?;
    } else {
        println!(
            "Successfully updated snapshots. {} remote packages available.",
            total
        );
    }
    Ok(())
}

async fn run_mcp(engine: &Engine, default_profile: &str) -> Result<()> {
    use std::io::BufRead;

    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: serde_json::Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": serde_json::Value::Null,
                        "error": {"code": -32700, "message": error.to_string()},
                    })
                );
                continue;
            }
        };
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let method = request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if method == "notifications/initialized" {
            continue;
        }
        let response =
            match mcp_dispatch(engine, default_profile, method, request.get("params")).await {
                Ok(result) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32000, "message": error.to_string()},
                }),
            };
        println!("{}", serde_json::to_string(&response)?);
    }
    Ok(())
}

async fn mcp_dispatch(
    engine: &Engine,
    default_profile: &str,
    method: &str,
    params: Option<&serde_json::Value>,
) -> Result<serde_json::Value> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "pkg", "version": pkg_core::version()},
        })),
        "tools/list" => Ok(serde_json::json!({
            "tools": [
                mcp_tool("pkg.search", "Search repository snapshots", serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}, "format": {"type": "string"}, "repo": {"type": "string"}},
                    "required": ["query"],
                })),
                mcp_tool("pkg.list", "List installed packages", serde_json::json!({
                    "type": "object", "properties": {"profile": {"type": "string"}},
                })),
                mcp_tool("pkg.query_command", "Find installed providers of a command", serde_json::json!({
                    "type": "object", "properties": {"command": {"type": "string"}, "profile": {"type": "string"}},
                    "required": ["command"],
                })),
                mcp_tool("pkg.upgrade_plan", "Plan available upgrades", serde_json::json!({
                    "type": "object", "properties": {"profile": {"type": "string"}, "package": {"type": "string"}},
                })),
                mcp_tool("pkg.info", "Inspect a package or local artifact", serde_json::json!({
                    "type": "object", "properties": {"target": {"type": "string"}, "profile": {"type": "string"}},
                    "required": ["target"],
                })),
                mcp_tool("pkg.doctor", "Inspect pkg-owned consistency", serde_json::json!({
                    "type": "object", "properties": {"profile": {"type": "string"}},
                })),
                mcp_tool("pkg.profile_create", "Create an isolated profile", serde_json::json!({
                    "type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"],
                })),
                mcp_tool("pkg.profile_drop", "Drop an isolated profile", serde_json::json!({
                    "type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"],
                })),
                mcp_tool("pkg.install", "Install a local artifact or repository target", serde_json::json!({
                    "type": "object",
                    "properties": {
                        "target": {"type": "string"},
                        "profile": {"type": "string"},
                        "allow_missing_libs": {"type": "boolean"},
                    },
                    "required": ["target"],
                })),
                mcp_tool("pkg.remove", "Remove an installed package", serde_json::json!({
                    "type": "object",
                    "properties": {"package": {"type": "string"}, "profile": {"type": "string"}},
                    "required": ["package"],
                })),
            ],
        })),
        "tools/call" => {
            let params = params.ok_or_else(|| anyhow::anyhow!("tools/call requires params"))?;
            let name = params
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("tools/call requires a tool name"))?;
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let result = mcp_call_tool(engine, default_profile, name, &arguments).await?;
            Ok(serde_json::json!({
                "content": [{"type": "text", "text": serde_json::to_string(&result)?}],
                "structuredContent": result,
            }))
        }
        _ => Err(anyhow::anyhow!("unsupported MCP method '{method}'")),
    }
}

fn mcp_tool(name: &str, description: &str, input_schema: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

async fn mcp_call_tool(
    engine: &Engine,
    default_profile: &str,
    name: &str,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value> {
    let profile = arguments
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(default_profile);
    match name {
        "pkg.search" => {
            let query = arguments
                .get("query")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("query is required"))?;
            Ok(serde_json::to_value(engine.search_filtered(
                query,
                arguments.get("format").and_then(serde_json::Value::as_str),
                arguments.get("repo").and_then(serde_json::Value::as_str),
            )?)?)
        }
        "pkg.list" => Ok(serde_json::to_value(engine.list(profile)?)?),
        "pkg.query_command" => {
            let command = arguments
                .get("command")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("command is required"))?;
            Ok(serde_json::to_value(
                engine.query_command(profile, command)?,
            )?)
        }
        "pkg.upgrade_plan" => Ok(serde_json::to_value(engine.plan_upgrade(
            profile,
            arguments.get("package").and_then(serde_json::Value::as_str),
        )?)?),
        "pkg.info" => {
            let target = arguments
                .get("target")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("target is required"))?;
            Ok(serde_json::to_value(engine.info(target, profile)?)?)
        }
        "pkg.doctor" => Ok(serde_json::to_value(engine.doctor(Some(profile))?)?),
        "pkg.profile_create" | "pkg.create_profile" => {
            let name = arguments
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("name is required"))?;
            engine.create_profile(name)?;
            Ok(serde_json::json!({"status": "success", "profile": name}))
        }
        "pkg.profile_drop" | "pkg.drop_profile" => {
            let name = arguments
                .get("name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("name is required"))?;
            engine.drop_profile(name)?;
            Ok(serde_json::json!({"status": "success", "profile": name}))
        }
        "pkg.remove" => {
            let package = arguments
                .get("package")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("package is required"))?;
            Ok(serde_json::to_value(
                engine.remove(package, profile, false)?,
            )?)
        }
        "pkg.install" => {
            let target = arguments
                .get("target")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("target is required"))?;
            let allow_missing = arguments
                .get("allow_missing_libs")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let no_deps = arguments
                .get("no_deps")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let config_path = engine.layout().base_dir().join("repositories.toml");
            let config = config_path
                .is_file()
                .then(|| pkg_core::repository::RepositoriesConfig::load_from_file(&config_path))
                .transpose()?;
            let artifact = if std::path::Path::new(target).is_file() {
                PathBuf::from(target)
            } else {
                let resolution = engine.resolve_remote_package(target, config.as_ref())?;
                let remote = match resolution {
                    RemoteResolution::Exact(remote) => remote,
                    RemoteResolution::NotFound => {
                        return Err(anyhow::anyhow!("package target '{target}' was not found"));
                    }
                    RemoteResolution::Ambiguous(_) => {
                        return Err(anyhow::anyhow!("package target '{target}' is ambiguous"));
                    }
                };
                let artifact = download_with_progress(engine, &remote, true).await?;
                if !no_deps {
                    install_declared_dependencies(
                        engine,
                        &artifact,
                        profile,
                        Some(&remote.repository_id),
                        Some(&remote.format),
                        config.as_ref(),
                        true,
                        true,
                        &[],
                        1,
                    )
                    .await?;
                }
                artifact
            };
            if !no_deps && std::path::Path::new(target).is_file() {
                install_declared_dependencies(
                    engine,
                    &artifact,
                    profile,
                    None,
                    None,
                    config.as_ref(),
                    true,
                    true,
                    &[],
                    1,
                )
                .await?;
            }
            Ok(serde_json::to_value(engine.install_with_options(
                &artifact,
                profile,
                false,
                InstallOptions {
                    allow_missing_libraries: allow_missing,
                    skip_dependencies: no_deps,
                },
            )?)?)
        }
        _ => Err(anyhow::anyhow!("unsupported MCP tool '{name}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct SlowServer {
        url: String,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for SlowServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl SlowServer {
        async fn start(
            payloads: BTreeMap<String, Vec<u8>>,
            active: Arc<AtomicUsize>,
            peak: Arc<AtomicUsize>,
        ) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let payloads = Arc::new(payloads);
            let task = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let payloads = Arc::clone(&payloads);
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&peak);
                    tokio::spawn(async move {
                        let mut request = Vec::new();
                        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                            let mut buffer = [0; 1024];
                            let read = socket.read(&mut buffer).await.unwrap();
                            if read == 0 {
                                return;
                            }
                            request.extend_from_slice(&buffer[..read]);
                        }
                        let path = String::from_utf8_lossy(&request)
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("/")
                            .to_string();
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
                        let payload = payloads.get(&path).unwrap();
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        socket.write_all(header.as_bytes()).await.unwrap();
                        socket.write_all(payload).await.unwrap();
                        let _ = socket.shutdown().await;
                        active.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            });
            Self { url, task }
        }
    }

    #[tokio::test]
    async fn acquisition_pool_honors_jobs_and_deduplicates_digests() {
        let mut payloads = BTreeMap::new();
        let mut packages = Vec::new();
        for index in 0..3u8 {
            let path = format!("/artifact-{index}");
            let payload = vec![index; 16 * 1024];
            let digest = format!("{:x}", Sha256::digest(&payload));
            payloads.insert(path.clone(), payload.clone());
            packages.push(pkg_core::domain::package::RemotePackage {
                repository_id: "test".into(),
                name: format!("pkg-{index}"),
                version: "1".into(),
                architecture: "x86_64".into(),
                format: "deb".into(),
                digest,
                size_bytes: payload.len() as u64,
                url: path,
                constraints: vec![],
                provides: vec![],
                versioned_provides: vec![],
            });
        }

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let server = SlowServer::start(payloads, Arc::clone(&active), Arc::clone(&peak)).await;
        for package in &mut packages {
            package.url = format!("{}{}", server.url, package.url);
        }
        // A second consumer of the first digest must reuse the same task.
        packages.push(packages[0].clone());

        let temp = tempfile::tempdir().unwrap();
        let acquired = acquire_remote_artifacts(
            &StoreLayout::new(temp.path().join("data")),
            packages,
            2,
            true,
        )
        .await
        .unwrap();

        assert_eq!(acquired.len(), 3);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn jobs_are_bounded() {
        assert_eq!(parse_jobs("1"), Ok(1));
        assert_eq!(parse_jobs("16"), Ok(16));
        assert!(parse_jobs("0").is_err());
        assert!(parse_jobs("17").is_err());
    }
}
