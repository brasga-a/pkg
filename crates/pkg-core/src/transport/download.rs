use futures::StreamExt;
use reqwest::Client;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};

/// Limits applied to bounding HTTP downloads to prevent exhaustion attacks.
#[derive(Debug, Clone)]
pub struct DownloadLimits {
    pub max_bytes: u64,
}

impl Default for DownloadLimits {
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024 * 1024, // 5 GB default cap for packages
        }
    }
}

/// A downloader that strictly enforces size limits during streaming to prevent resource exhaustion.
#[derive(Debug, Clone)]
pub struct BoundedDownloader {
    client: Client,
    limits: DownloadLimits,
}

impl BoundedDownloader {
    /// Creates a new BoundedDownloader with the specified client and limits.
    pub fn new(client: Client, limits: DownloadLimits) -> Self {
        Self { client, limits }
    }

    /// Creates a new BoundedDownloader with default client configurations and limits.
    pub fn try_default() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(300)) // 5 minute total timeout
            .build()
            .map_err(|e| Error::Network(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self {
            client,
            limits: DownloadLimits::default(),
        })
    }

    /// Downloads the resource at `url` to the specified `destination` file.
    ///
    /// Streams the response directly to disk. If the server advertises a Content-Length
    /// exceeding `max_bytes`, or if the streamed body exceeds `max_bytes`, the download
    /// is aborted with an error.
    pub async fn download_to_file(&self, url: &str, destination: &Path) -> Result<()> {
        self.download_to_file_with_progress(url, destination, |_, _| {})
            .await
    }

    /// Downloads the resource at `url` to the specified `destination` file, invoking
    /// `on_progress` with `(downloaded_bytes, total_bytes)` as chunks arrive.
    pub async fn download_to_file_with_progress<F>(
        &self,
        url: &str,
        destination: &Path,
        mut on_progress: F,
    ) -> Result<()>
    where
        F: FnMut(u64, Option<u64>) + Send + Sync,
    {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Network(format!("Failed to send request to {url}: {e}")))?;

        let response = response
            .error_for_status()
            .map_err(|e| Error::Network(format!("HTTP error downloading {url}: {e}")))?;

        let total_size = response.content_length();

        // Pre-flight check on advertised size
        if let Some(content_length) = total_size {
            if content_length > self.limits.max_bytes {
                return Err(Error::LimitsExceeded(format!(
                    "Advertised download size ({} bytes) exceeds limit ({} bytes) for {}",
                    content_length, self.limits.max_bytes, url
                )));
            }
        }

        // Ensure parent directory exists
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "Failed to create download directory {}: {}",
                    parent.display(),
                    e
                )))
            })?;
        }

        let mut file = File::create(destination).await.map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "Failed to create download file {}: {}",
                destination.display(),
                e
            )))
        })?;

        let mut downloaded_bytes = 0u64;
        let mut stream = response.bytes_stream();

        on_progress(0, total_size);

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| Error::Network(format!("Error reading chunk from {url}: {e}")))?;

            downloaded_bytes += chunk.len() as u64;
            if downloaded_bytes > self.limits.max_bytes {
                // Remove the partial file
                drop(file);
                let _ = tokio::fs::remove_file(destination).await;
                return Err(Error::LimitsExceeded(format!(
                    "Download stream exceeded limit ({} bytes) for {}",
                    self.limits.max_bytes, url
                )));
            }

            file.write_all(&chunk).await.map_err(|e| {
                Error::Io(std::io::Error::other(format!(
                    "Failed to write chunk to {}: {}",
                    destination.display(),
                    e
                )))
            })?;

            on_progress(downloaded_bytes, total_size);
        }

        file.flush().await.map_err(|e| {
            Error::Io(std::io::Error::other(format!(
                "Failed to flush file {}: {}",
                destination.display(),
                e
            )))
        })?;

        Ok(())
    }
}
