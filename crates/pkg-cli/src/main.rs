//! CLI entry point for `pkg` package manager.

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use pkg_core::{Engine, PackageInfo, StoreLayout};

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
    /// Install a package from a local artifact (.deb)
    Install {
        /// Path to the local package artifact
        path: PathBuf,

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

    /// Show detailed metadata and state for a package or artifact
    Info {
        /// Package name or path to a local package artifact
        target: String,
    },
}

fn init_tracing(verbose: bool) {
    let default_level = if verbose { "debug" } else { "warn" };
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .try_init();
}

fn run() -> Result<()> {
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
        Commands::Install { path, dry_run } => {
            let plan = engine.install(&path, &cli.profile, dry_run)?;
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

fn main() -> Result<()> {
    run()
}
