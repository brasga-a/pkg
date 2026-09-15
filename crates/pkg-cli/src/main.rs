//! CLI entry point for `pkg` package manager.

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use pkg_core::{Engine, InstallOptions, PackageInfo, RemoteResolution, StoreLayout};

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

    /// List locally installed packages in the profile
    List,

    /// Update local package catalog from configured remote repositories
    Update,

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
}

#[derive(Subcommand, Debug)]
enum RepoCommands {
    /// Add a new remote repository
    Add {
        /// Unique identifier for the repository (e.g., ubuntu-noble, fedora-41, arch-extra)
        id: String,
        /// Repository format ecosystem (deb, rpm, alpm)
        #[arg(long, default_value = "deb")]
        format: String,
        /// Repository base URL (e.g., http://archive.ubuntu.com/ubuntu)
        url: String,
        /// Distribution suite (e.g., noble, 41, extra)
        distribution: String,
        /// Components to fetch (e.g., main universe)
        components: Vec<String>,
        /// Repository priority (higher values preferred during resolution)
        #[arg(long)]
        priority: Option<u32>,
    },
    /// List configured repositories
    List,
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

    let engine = Engine::open(layout)?;

    match command {
        Commands::Install {
            targets,
            dry_run,
            yes,
            ignore_missing_libs,
            interactive,
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
                    println!("Resolving package target '{}'...", spec);

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
                        if !prompt_confirm(&prompt, false, yes)? {
                            println!("Skipping '{}'.", pkg_name);
                            skipped_count += 1;
                            continue;
                        }
                    } else if existing_ver == item_ver {
                        let prompt = format!(
                            "Package '{}' ({}, [{}]) is already installed in profile '{}'. Reinstall? [y/N]: ",
                            pkg_name, existing_ver, existing_fmt, cli.profile
                        );
                        if !prompt_confirm(&prompt, false, yes)? {
                            println!("Skipping '{}'.", pkg_name);
                            skipped_count += 1;
                            continue;
                        }
                    } else {
                        let prompt = format!(
                            "Package '{}' is already installed ({} [{}]). Replace with version {}? [Y/n]: ",
                            pkg_name, existing_ver, existing_fmt, item_ver
                        );
                        if !prompt_confirm(&prompt, true, yes)? {
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
                        println!(
                            "Found {} {} [{}] in {}",
                            remote_pkg.name,
                            remote_pkg.version,
                            remote_pkg.format,
                            remote_pkg.repository_id
                        );
                        if dry_run {
                            println!("Would download {} from {}", remote_pkg.name, remote_pkg.url);
                            let cache_path =
                                engine.layout().artifact_cache_path(&remote_pkg.digest);
                            if cache_path.exists() {
                                (cache_path, name)
                            } else {
                                println!(
                                    "Dry-run: remote package {} verified from {}.",
                                    remote_pkg.name, remote_pkg.repository_id
                                );
                                installed_count += 1;
                                continue;
                            }
                        } else {
                            println!("Downloading {}...", remote_pkg.name);
                            match download_with_progress(&engine, &remote_pkg).await {
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

                let mut allow_missing = ignore_missing_libs;

                // Point 3: Preflight inspection for missing shared libraries
                if !dry_run {
                    if let Ok(preflight) =
                        engine.preflight_check_with_profile(&artifact_path, &cli.profile)
                    {
                        if !preflight.missing_libraries.is_empty() {
                            println!(
                                "\n⚠️  Package '{}' requires missing host libraries:\n  {}",
                                preflight.package.name,
                                preflight.missing_libraries.join(", ")
                            );

                            let mut detected_deps = Vec::new();
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

                            if !detected_deps.is_empty() {
                                println!("The following dependency packages can be installed:");
                                for d in &detected_deps {
                                    println!(
                                        "  - {} {} [{}] (from {})",
                                        d.name, d.version, d.format, d.repository_id
                                    );
                                }
                                let install_deps = prompt_confirm(
                                    "Install missing dependencies automatically? [Y/n]: ",
                                    true,
                                    yes,
                                )?;
                                if install_deps {
                                    for d in &detected_deps {
                                        println!("Downloading dependency {}...", d.name);
                                        match download_with_progress(&engine, d).await {
                                            Ok(dep_path) => {
                                                let _ = engine.install_with_options(
                                                    &dep_path,
                                                    &cli.profile,
                                                    false,
                                                    InstallOptions {
                                                        allow_missing_libraries: true,
                                                    },
                                                );
                                                println!(
                                                    "  ✓ Installed dependency {} {}",
                                                    d.name, d.version
                                                );
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "Failed to install dependency '{}': {}",
                                                    d.name, e
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            // Re-evaluate missing libraries after dependency installation
                            let remaining_missing = if let Ok(recheck) =
                                engine.preflight_check_with_profile(&artifact_path, &cli.profile)
                            {
                                recheck.missing_libraries
                            } else {
                                preflight.missing_libraries
                            };

                            if remaining_missing.is_empty() {
                                println!("  ✓ All library dependencies successfully satisfied.");
                            } else if !allow_missing {
                                println!(
                                    "\n⚠️  The following host libraries are still missing:\n  {}",
                                    remaining_missing.join(", ")
                                );
                                let install_anyway = prompt_confirm(
                                    "Install package anyway without these remaining libraries? (Warning: the executable may fail at runtime) [y/N]: ",
                                    false,
                                    yes,
                                )?;
                                if install_anyway {
                                    allow_missing = true;
                                } else {
                                    eprintln!(
                                        "Skipping '{}' due to missing host libraries.",
                                        preflight.package.name
                                    );
                                    failed_count += 1;
                                    continue;
                                }
                            }
                        }
                    }
                }

                let install_result = engine.install_with_options(
                    &artifact_path,
                    &cli.profile,
                    dry_run,
                    InstallOptions {
                        allow_missing_libraries: allow_missing,
                    },
                );

                match install_result {
                    Ok(plan) => {
                        if dry_run {
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
                            if !plan.ignored_scripts.is_empty() {
                                println!("  Ignored maintainer scripts (policy default-deny):");
                                for script in &plan.ignored_scripts {
                                    println!("    - {script}");
                                }
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
                        }
                        installed_count += 1;
                    }
                    Err(e) => {
                        eprintln!("Error installing '{}': {}", pkg_name, e);
                        failed_count += 1;
                    }
                }
            }

            if targets.len() > 1 {
                println!(
                    "\nSummary: {} installed, {} skipped, {} failed.",
                    installed_count, skipped_count, failed_count
                );
            }

            if failed_count > 0 && installed_count == 0 {
                return Err(anyhow::anyhow!(
                    "Installation failed for requested target(s)."
                ));
            }
        }
        Commands::Remove { name, dry_run } => {
            let plan = engine.remove(&name, &cli.profile, dry_run)?;
            if dry_run {
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
        Commands::List => {
            let packages = engine.list(&cli.profile)?;
            if packages.is_empty() {
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
        Commands::Update => {
            let config_path = engine.layout().base_dir().join("repositories.toml");
            if !config_path.exists() {
                println!(
                    "No repositories.toml found at {}. Generating defaults...",
                    config_path.display()
                );
                std::fs::write(
                    &config_path,
                    r#"
[[repository]]
id = "ubuntu-noble"
format = "deb"
url = "http://archive.ubuntu.com/ubuntu"
distribution = "noble"
components = ["main", "universe", "restricted", "multiverse"]
priority = 10

[[repository]]
id = "debian-bookworm"
format = "deb"
url = "http://deb.debian.org/debian"
distribution = "bookworm"
components = ["main", "contrib", "non-free"]
priority = 10

[[repository]]
id = "fedora-41"
format = "rpm"
url = "https://archives.fedoraproject.org/pub/archive/fedora/linux/releases/41/Everything/x86_64/os"
distribution = "41"
components = []
priority = 20

[[repository]]
id = "arch-core"
format = "alpm"
url = "https://geo.mirror.pkgbuild.com"
distribution = "core"
components = []
priority = 30

[[repository]]
id = "arch-extra"
format = "alpm"
url = "https://geo.mirror.pkgbuild.com"
distribution = "extra"
components = []
priority = 30
"#,
                )?;
            }
            println!("Reading config from {}...", config_path.display());
            let config = pkg_core::repository::RepositoriesConfig::load_from_file(&config_path)?;
            println!("Updating {} repositories...", config.repositories.len());

            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_style(
                indicatif::ProgressStyle::default_spinner()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
                    .template("{spinner:.green} {msg}")
                    .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
            );
            spinner.set_message("Synchronizing repository indexes and verifying signatures...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(80));

            let total = engine.update(&config).await?;
            spinner.finish_and_clear();

            println!(
                "Successfully updated snapshots. {} remote packages available.",
                total
            );
        }
        Commands::Repo { command } => {
            let config_path = engine.layout().base_dir().join("repositories.toml");
            match command {
                RepoCommands::List => {
                    if !config_path.exists() {
                        println!("No repositories configured.");
                        return Ok(());
                    }
                    let config =
                        pkg_core::repository::RepositoriesConfig::load_from_file(&config_path)?;
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
                            "Repository with ID '{}' already exists",
                            id
                        ));
                    }

                    config
                        .repositories
                        .push(pkg_core::repository::RepositoryConfig {
                            id: id.clone(),
                            format,
                            url: url.clone(),
                            distribution,
                            components,
                            public_key_path: None,
                            priority,
                        });

                    let toml_string = toml::to_string_pretty(&config)?;
                    std::fs::write(&config_path, toml_string)?;
                    println!("Successfully added repository '{}' ({})", id, url);
                    println!("Run `pkg update` to sync the new repository.");
                }
            }
        }
        Commands::Search {
            query,
            format,
            repo,
        } => {
            let results = engine.search_filtered(&query, format.as_deref(), repo.as_deref())?;
            if results.is_empty() {
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

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
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

async fn download_with_progress(
    engine: &Engine,
    remote_pkg: &pkg_core::domain::package::RemotePackage,
) -> Result<PathBuf> {
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
