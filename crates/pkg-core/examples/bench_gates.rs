//! Reproducible release-gate benchmark driver.
//!
//! Each subcommand performs one representative operation in a fresh process.
//! The shell harness wraps these commands with hyperfine and records the host
//! and toolchain alongside the measured values.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pkg_core::domain::package::PackageFormat;
use pkg_core::format::{ExtractionLimits, detect_format, get_adapter};
use pkg_core::host::HostFacts;
use pkg_core::repository::deb::parse_packages_index;
use pkg_core::resolver::evidence::HostEvidence;
use pkg_core::{Engine, Resolver, StoreLayout};

fn main() -> pkg_core::Result<()> {
    let mut args = env::args_os().skip(1);
    let category = args.next().ok_or_else(|| {
        pkg_core::Error::Internal(
            "usage: bench_gates <repository|solve|extraction|activation|recovery> [artifact]"
                .into(),
        )
    })?;
    let category = category.to_string_lossy();
    let artifact = args.next().map(PathBuf::from);

    match category.as_ref() {
        "repository" => benchmark_repository(),
        "solve" => benchmark_solve(artifact.as_deref().ok_or_else(missing_artifact)?),
        "extraction" => benchmark_extraction(artifact.as_deref().ok_or_else(missing_artifact)?),
        "activation" => benchmark_activation(artifact.as_deref().ok_or_else(missing_artifact)?),
        "recovery" => benchmark_recovery(),
        other => Err(pkg_core::Error::Internal(format!(
            "unknown benchmark category: {other}"
        ))),
    }
}

fn missing_artifact() -> pkg_core::Error {
    pkg_core::Error::Internal("this benchmark category requires a package artifact".into())
}

fn benchmark_repository() -> pkg_core::Result<()> {
    let mut index = String::new();
    for number in 0..500 {
        index.push_str(&format!(
            "Package: fixture-{number}\nVersion: 1.0.{number}\nArchitecture: amd64\nFilename: pool/f/fixture-{number}.deb\nSize: 634\nSHA256: {:064x}\n\n",
            number + 1
        ));
    }
    let packages = parse_packages_index(&index, "https://example.invalid/debian", "stable");
    assert_eq!(packages.len(), 500);
    black_box(packages);
    Ok(())
}

fn load_package(path: &Path) -> pkg_core::Result<pkg_core::domain::package::NormalizedPackage> {
    let format = detect_format(path)?;
    get_adapter(format).parse_metadata(path)
}

fn benchmark_solve(path: &Path) -> pkg_core::Result<()> {
    let package = load_package(path)?;
    let host = HostEvidence::detect(&HostFacts::detect());
    let plan = Resolver::new(host)
        .resolve(&package)
        .map_err(|error| pkg_core::Error::ResolutionFailed(Box::new(error.chain)))?;
    black_box(plan);
    Ok(())
}

fn benchmark_extraction(path: &Path) -> pkg_core::Result<()> {
    let format = detect_format(path)?;
    assert!(matches!(
        format,
        PackageFormat::Deb | PackageFormat::Rpm | PackageFormat::Alpm
    ));
    let adapter = get_adapter(format);
    let destination = tempfile::tempdir()?;
    let report = adapter.extract_payload(path, destination.path(), &ExtractionLimits::default())?;
    black_box(report);
    Ok(())
}

fn unique_temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    env::temp_dir().join(format!("pkg-bench-{label}-{}-{nonce}", std::process::id()))
}

fn benchmark_activation(path: &Path) -> pkg_core::Result<()> {
    let root = unique_temp_root("activation");
    let layout = StoreLayout::new(&root);
    let engine = Engine::open(layout)?;
    let result = engine.install(path, "default", false);
    drop(engine);
    let _ = fs::remove_dir_all(root);
    result.map(|plan| {
        black_box(plan);
    })
}

fn benchmark_recovery() -> pkg_core::Result<()> {
    let root = unique_temp_root("recovery");
    let layout = StoreLayout::new(&root);
    layout.ensure_dirs()?;
    let db = pkg_core::state::StateDatabase::open(&layout.db_path())?;
    db.record_transaction_start(
        "bench-recovery",
        "install",
        "Staging",
        "fixture",
        None,
        None,
    )?;
    drop(db);
    let engine = Engine::open(layout)?;
    black_box(engine.db().list_incomplete_transactions()?);
    drop(engine);
    let _ = fs::remove_dir_all(root);
    Ok(())
}
