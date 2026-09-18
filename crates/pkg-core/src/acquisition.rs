//! Digest-addressed artifact acquisition that is independent from the stateful engine.
//!
//! This module deliberately does not open SQLite, acquire a profile lock, or mutate
//! an active generation.  That makes it safe for callers to run several acquisitions
//! concurrently while keeping profile publication serial.

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::package::{ArtifactDigest, RemotePackage};
use crate::error::{Error, Result};
use crate::store::StoreLayout;
use crate::transport::{BoundedDownloader, DownloadLimits};

/// All immutable inputs required to acquire one remote artifact.
#[derive(Debug, Clone)]
pub struct ArtifactAcquisitionSpec {
    /// Catalog entry whose artifact is being acquired.
    pub package: RemotePackage,
    /// Digest-addressed destination in the local artifact cache.
    pub cache_path: PathBuf,
    /// Streaming limits applied to the network transfer.
    pub limits: DownloadLimits,
}

impl ArtifactAcquisitionSpec {
    /// Builds a cache acquisition request from a store layout and catalog entry.
    pub fn new(layout: &StoreLayout, package: RemotePackage) -> Result<Self> {
        validate_digest(&package.digest)?;
        Ok(Self {
            cache_path: layout.artifact_cache_path(&package.digest),
            package,
            limits: DownloadLimits::default(),
        })
    }
}

/// How an acquired artifact became available locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactAcquisitionSource {
    /// A complete verified artifact was already present in the local cache.
    Cache,
    /// This task downloaded and verified the artifact before publishing it.
    Download,
}

/// Verified output of one immutable artifact acquisition.
#[derive(Debug, Clone)]
pub struct ArtifactAcquisitionResult {
    /// Catalog entry used to acquire the artifact.
    pub package: RemotePackage,
    /// Verified path in the digest-addressed cache.
    pub path: PathBuf,
    /// Whether this task used the cache or downloaded the artifact.
    pub source: ArtifactAcquisitionSource,
}

/// Acquires an artifact without accessing package state or mutating profiles.
///
/// The caller may invoke this from independent Tokio tasks.  File hashing and
/// cache publication run on Tokio's blocking pool; HTTP streaming stays async.
pub async fn acquire_artifact<F>(
    spec: ArtifactAcquisitionSpec,
    on_progress: F,
) -> Result<ArtifactAcquisitionResult>
where
    F: FnMut(u64, Option<u64>) + Send + Sync,
{
    let expected_digest = spec.package.digest.clone();
    let expected_size = spec.package.size_bytes;
    let cache_path = spec.cache_path.clone();

    if verify_existing_cache(cache_path.clone(), expected_digest.clone(), expected_size).await? {
        return Ok(ArtifactAcquisitionResult {
            package: spec.package,
            path: cache_path,
            source: ArtifactAcquisitionSource::Cache,
        });
    }

    let parent = cache_path.parent().ok_or_else(|| {
        Error::Internal(format!(
            "artifact cache path has no parent: {}",
            cache_path.display()
        ))
    })?;
    let parent = parent.to_path_buf();
    let temporary = run_blocking("creating artifact cache temporary", move || {
        fs::create_dir_all(&parent)?;
        tempfile::NamedTempFile::new_in(parent).map_err(Error::from)
    })
    .await?;

    let downloader = BoundedDownloader::new(
        reqwest::Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|error| Error::Network(format!("Failed to build HTTP client: {error}")))?,
        spec.limits,
    );
    downloader
        .download_to_file_with_progress(&spec.package.url, temporary.path(), on_progress)
        .await?;

    let temporary_path = temporary.path().to_path_buf();
    let (actual_digest, actual_size) = run_blocking("verifying downloaded artifact", move || {
        Ok((
            ArtifactDigest::from_file(&temporary_path)?,
            fs::metadata(&temporary_path)?.len(),
        ))
    })
    .await?;
    if !actual_digest.hex().eq_ignore_ascii_case(&expected_digest) || actual_size != expected_size {
        return Err(Error::SecurityViolation(format!(
            "Artifact size or digest mismatch: expected {}, got {}",
            expected_digest, actual_digest
        )));
    }

    let published_path = cache_path.clone();
    let expected_digest_for_publish = expected_digest.clone();
    let result_path = run_blocking("publishing verified artifact", move || {
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&published_path) {
            Ok(_) => Ok(published_path),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                // A competing task or process won the rename.  Its output is
                // acceptable only after a fresh complete verification.
                if verify_existing_cache_sync(
                    &published_path,
                    &expected_digest_for_publish,
                    expected_size,
                )? {
                    Ok(published_path)
                } else {
                    Err(Error::SecurityViolation(
                        "Concurrent artifact cache publication failed verification".into(),
                    ))
                }
            }
            Err(error) => Err(Error::Io(error.error)),
        }
    })
    .await?;

    Ok(ArtifactAcquisitionResult {
        package: spec.package,
        path: result_path,
        source: ArtifactAcquisitionSource::Download,
    })
}

async fn verify_existing_cache(
    cache_path: PathBuf,
    expected_digest: String,
    expected_size: u64,
) -> Result<bool> {
    run_blocking("verifying artifact cache entry", move || {
        verify_existing_cache_sync(&cache_path, &expected_digest, expected_size)
    })
    .await
}

fn verify_existing_cache_sync(
    cache_path: &Path,
    expected_digest: &str,
    expected_size: u64,
) -> Result<bool> {
    match fs::symlink_metadata(cache_path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::SecurityViolation(
                    "Invalid artifact cache entry".into(),
                ));
            }
            if metadata.len() == expected_size
                && ArtifactDigest::from_file(cache_path)?
                    .hex()
                    .eq_ignore_ascii_case(expected_digest)
            {
                return Ok(true);
            }
            fs::remove_file(cache_path)?;
            Err(Error::SecurityViolation(
                "Artifact cache size or digest mismatch".into(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::Io(error)),
    }
}

fn validate_digest(digest: &str) -> Result<()> {
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Error::SecurityViolation(
            "Invalid SHA256 digest in repository metadata".into(),
        ))
    }
}

async fn run_blocking<T, F>(operation: &'static str, operation_fn: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation_fn)
        .await
        .map_err(|error| Error::Internal(format!("{operation} task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_sha256_catalog_digest() {
        let layout = StoreLayout::new("target/acquisition-digest-test");
        let package = RemotePackage {
            repository_id: "test".into(),
            name: "demo".into(),
            version: "1".into(),
            architecture: "x86_64".into(),
            format: "deb".into(),
            digest: "bad".into(),
            size_bytes: 0,
            url: "https://example.invalid/demo.deb".into(),
            constraints: vec![],
            provides: vec![],
            versioned_provides: vec![],
        };
        assert!(ArtifactAcquisitionSpec::new(&layout, package).is_err());
    }
}
