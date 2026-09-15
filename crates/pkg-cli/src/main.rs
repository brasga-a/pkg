//! CLI entry point for `pkg` package manager.

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use pkg_core::{Engine, PackageInfo, RemoteResolution, StoreLayout};

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
        Commands::Install { targets, dry_run } => {
            let config_path = engine.layout().base_dir().join("repositories.toml");
            let config = if config_path.exists() {
                pkg_core::repository::RepositoriesConfig::load_from_file(&config_path).ok()
            } else {
                None
            };

            for path in targets {
                let artifact_path = if path.exists() {
                    path
                } else {
                    let name = path.to_string_lossy();
                    println!("Resolving package target '{}'...", name);

                    let resolution = engine.resolve_remote_package(&name, config.as_ref())?;
                    let remote_pkg = match resolution {
                        RemoteResolution::NotFound => {
                            return Err(anyhow::anyhow!(
                                "Package target '{}' not found in local paths or active repository snapshots.",
                                name
                            ));
                        }
                        RemoteResolution::Exact(pkg) => pkg,
                        RemoteResolution::Ambiguous(candidates) => {
                            println!("\nMultiple candidates match '{}':", name);
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
                                    "\nSelect candidate to install [1-{}] (or Enter to cancel): ",
                                    candidates.len()
                                );
                                std::io::stdout().flush().ok();
                                let mut input = String::new();
                                std::io::stdin().read_line(&mut input)?;
                                let trimmed = input.trim();
                                if trimmed.is_empty() {
                                    println!("Installation cancelled.");
                                    return Ok(());
                                }
                                match trimmed.parse::<usize>() {
                                    Ok(num) if num >= 1 && num <= candidates.len() => {
                                        candidates[num - 1].clone()
                                    }
                                    _ => {
                                        return Err(anyhow::anyhow!(
                                            "Invalid selection '{}'. Installation cancelled.",
                                            trimmed
                                        ));
                                    }
                                }
                            } else {
                                return Err(anyhow::anyhow!(
                                    "Ambiguous package target '{}'. Please qualify directly by repository (e.g. '{}/{}').",
                                    name,
                                    candidates[0].repository_id,
                                    candidates[0].name
                                ));
                            }
                        }
                    };

                    println!(
                        "Found {} {} [{}] in {}",
                        remote_pkg.name,
                        remote_pkg.version,
                        remote_pkg.format,
                        remote_pkg.repository_id
                    );
                    if dry_run {
                        println!("Would download {} from {}", remote_pkg.name, remote_pkg.url);
                        let cache_path = engine.layout().artifact_cache_path(&remote_pkg.digest);
                        if cache_path.exists() {
                            cache_path
                        } else {
                            println!(
                                "Dry-run: remote package {} verified from {}.",
                                remote_pkg.name, remote_pkg.repository_id
                            );
                            continue;
                        }
                    } else {
                        println!("Downloading {}...", remote_pkg.name);
                        engine.download_remote(&remote_pkg).await?
                    }
                };

                let plan = engine.install(&artifact_path, &cli.profile, dry_run)?;
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
            let total = engine.update(&config).await?;
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
