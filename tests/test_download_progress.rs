//! Tests for progress reporting in transport download and engine remote download.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use pkg_core::domain::package::RemotePackage;
use pkg_core::transport::BoundedDownloader;
use pkg_core::{Engine, StoreLayout};

struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start(
        handler: impl Fn(&str) -> (u16, Vec<u8>, Option<usize>) + Send + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, body, length) = handler(path);
                let mut headers = format!("HTTP/1.1 {status} Test\r\nConnection: close\r\n");
                if let Some(length) = length {
                    headers.push_str(&format!("Content-Length: {length}\r\n"));
                }
                headers.push_str("\r\n");
                let _ = socket.write_all(headers.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            }
        });
        Self { url, task }
    }
}

#[tokio::test]
async fn test_download_to_file_with_progress_callback() {
    let payload = vec![42u8; 1024 * 64]; // 64 KB
    let payload_len = payload.len();
    let payload_clone = payload.clone();

    let server = Server::start(move |_| (200, payload_clone.clone(), Some(payload_len))).await;

    let temp = tempdir().unwrap();
    let dest = temp.path().join("downloaded.bin");

    let downloader = BoundedDownloader::try_default().unwrap();

    let last_downloaded = Arc::new(AtomicU64::new(0));
    let last_downloaded_clone = Arc::clone(&last_downloaded);
    let total_observed = Arc::new(AtomicU64::new(0));
    let total_observed_clone = Arc::clone(&total_observed);

    downloader
        .download_to_file_with_progress(
            &format!("{}/file.bin", server.url),
            &dest,
            move |curr, total| {
                last_downloaded_clone.store(curr, Ordering::Relaxed);
                if let Some(t) = total {
                    total_observed_clone.store(t, Ordering::Relaxed);
                }
            },
        )
        .await
        .unwrap();

    assert_eq!(total_observed.load(Ordering::Relaxed), payload_len as u64);
    assert_eq!(last_downloaded.load(Ordering::Relaxed), payload_len as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
}

#[tokio::test]
async fn test_engine_download_remote_with_progress() {
    use sha2::{Digest, Sha256};

    let payload = b"Hello from remote package payload with progress tracking!";
    let payload_len = payload.len() as u64;
    let digest = format!("{:x}", Sha256::digest(payload));

    let payload_vec = payload.to_vec();
    let server = Server::start(move |_| (200, payload_vec.clone(), Some(payload_vec.len()))).await;

    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();

    let remote_pkg = RemotePackage {
        repository_id: "test-repo".into(),
        name: "test-progress-pkg".into(),
        version: "1.0.0".into(),
        architecture: "amd64".into(),
        format: "deb".into(),
        digest: digest.clone(),
        size_bytes: payload_len,
        url: format!("{}/test.deb", server.url),
    };

    let last_bytes = Arc::new(AtomicU64::new(0));
    let last_bytes_clone = Arc::clone(&last_bytes);

    let cache_path = engine
        .download_remote_with_progress(&remote_pkg, move |curr, _| {
            last_bytes_clone.store(curr, Ordering::Relaxed);
        })
        .await
        .unwrap();

    assert!(cache_path.exists());
    assert_eq!(std::fs::read(&cache_path).unwrap(), payload);
    assert_eq!(last_bytes.load(Ordering::Relaxed), payload_len);
}
