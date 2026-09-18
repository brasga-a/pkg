//! Gate M2 (Repository) Release Gate Verification Suite.
//!
//! Verifies:
//! - repository configuration loading and parsing;
//! - immutable local snapshots in SQLite state;
//! - digest-addressed artifact caching and integrity mismatch rejection (INV-017);
//! - bounded download limit enforcement (INV-005);
//! - GPG signature verification rejection of invalid/tampered InRelease signatures.

use pkg_core::domain::package::RemotePackage;
use pkg_core::error::Error;
use pkg_core::repository::RepositoriesConfig;
use pkg_core::transport::BoundedDownloader;
use pkg_core::{Engine, StoreLayout};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_repository_config_loading() {
    let temp = tempdir().unwrap();
    let config_path = temp.path().join("repositories.toml");
    fs::write(
        &config_path,
        r#"
[[repository]]
id = "test-repo"
url = "http://example.com/debian"
distribution = "stable"
components = ["main", "contrib"]
"#,
    )
    .unwrap();

    let config = RepositoriesConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config.repositories.len(), 1);
    assert_eq!(config.repositories[0].id, "test-repo");
    assert_eq!(config.repositories[0].distribution, "stable");
    assert_eq!(config.repositories[0].components, vec!["main", "contrib"]);
}

#[test]
fn test_repository_snapshot_atomic_commit_and_search() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path());
    let engine = Engine::open(layout).unwrap();

    let remote_pkg = RemotePackage {
        repository_id: "test-repo".to_string(),
        name: "test-tool".to_string(),
        version: "2.0.0".to_string(),
        architecture: "amd64".to_string(),
        format: "deb".to_string(),
        digest: "abcdef1234567890".to_string(),
        size_bytes: 1024,
        url: "http://example.com/pool/main/t/test-tool_2.0.0_amd64.deb".to_string(),
        constraints: Vec::new(),
        provides: Vec::new(),
        versioned_provides: Vec::new(),
    };

    // Commit snapshot into SQLite
    engine
        .db()
        .commit_repository_snapshot(
            "test-repo",
            "deb",
            "http://example.com/debian",
            "stable",
            std::slice::from_ref(&remote_pkg),
        )
        .unwrap();

    // Verify search finds package (including partial query) and get_remote_package finds exact match
    let found = engine.search("test").unwrap();
    assert_eq!(found.len(), 1);
    let pkg = &found[0];
    assert_eq!(pkg.name, "test-tool");
    assert_eq!(pkg.version, "2.0.0");
    assert_eq!(pkg.repository_id, "test-repo");

    let exact = engine.get_remote_package("test-tool").unwrap();
    assert!(exact.is_some());
    assert_eq!(exact.unwrap().name, "test-tool");

    // Search for non-existent package
    let not_found = engine.search("nonexistent-pkg").unwrap();
    assert!(not_found.is_empty());
}

#[tokio::test]
async fn test_artifact_cache_digest_mismatch_rejected() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path())).unwrap();
    let package = remote_package("http://127.0.0.1:1/unused", "0".repeat(64), 13);
    let cache = engine.layout().artifact_cache_path(&package.digest);
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, b"wrong content").unwrap();
    assert!(matches!(
        engine.download_remote(&package).await,
        Err(Error::SecurityViolation(_))
    ));
    assert!(!cache.exists());
}

fn remote_package(url: &str, digest: String, size: u64) -> RemotePackage {
    RemotePackage {
        repository_id: "test".into(),
        name: "test-tool".into(),
        version: "1.0".into(),
        architecture: "amd64".into(),
        format: "deb".into(),
        digest,
        size_bytes: size,
        url: url.into(),
        constraints: Vec::new(),
        provides: Vec::new(),
        versioned_provides: Vec::new(),
    }
}

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
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
async fn test_bounded_downloader_limits_enforced() {
    use pkg_core::transport::DownloadLimits;
    for announced in [Some(16), None] {
        let server = Server::start(move |_| (200, vec![1; 16], announced)).await;
        let downloader =
            BoundedDownloader::new(reqwest::Client::new(), DownloadLimits { max_bytes: 8 });
        let temp = tempdir().unwrap();
        let dest = temp.path().join("downloaded.bin");
        assert!(matches!(
            downloader.download_to_file(&server.url, &dest).await,
            Err(Error::LimitsExceeded(_))
        ));
        assert!(!dest.exists());
    }
}

#[tokio::test]
async fn cache_is_published_only_after_complete_verified_download() {
    use pkg_core::format::deb::DebAdapter;
    let temp = tempdir().unwrap();
    let payload = temp.path().join("payload");
    fs::write(&payload, b"payload").unwrap();
    let digest = DebAdapter::compute_digest(&payload)
        .unwrap()
        .hex()
        .to_string();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let truncated = Server::start(|_| (200, b"pay".to_vec(), Some(7))).await;
    let mut package = remote_package(&truncated.url, digest, 7);
    let cache = engine.layout().artifact_cache_path(&package.digest);
    assert!(engine.download_remote(&package).await.is_err());
    assert!(!cache.exists());
    assert_eq!(fs::read_dir(cache.parent().unwrap()).unwrap().count(), 0);
    let incorrect = Server::start(|_| (200, b"corrupt".to_vec(), Some(7))).await;
    package.url = incorrect.url.clone();
    assert!(matches!(
        engine.download_remote(&package).await,
        Err(Error::SecurityViolation(_))
    ));
    assert!(!cache.exists());
    assert_eq!(fs::read_dir(cache.parent().unwrap()).unwrap().count(), 0);
    let complete = Server::start(|_| (200, b"payload".to_vec(), Some(7))).await;
    package.url = complete.url.clone();
    assert_eq!(engine.download_remote(&package).await.unwrap(), cache);
    assert_eq!(fs::read(&cache).unwrap(), b"payload");
    drop(complete);
    // No network is available now; the verified cache remains usable.
    assert_eq!(engine.download_remote(&package).await.unwrap(), cache);
    package.size_bytes = 8;
    assert!(matches!(
        engine.download_remote(&package).await,
        Err(Error::SecurityViolation(_))
    ));
}

#[tokio::test]
async fn invalid_digest_cannot_address_paths_outside_cache() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
    let victim = temp.path().join("keep");
    fs::write(&victim, b"user data").unwrap();
    for digest in [
        victim.to_string_lossy().into_owned(),
        "../../keep".into(),
        String::new(),
    ] {
        let package = remote_package("http://127.0.0.1:1/unused", digest, 9);
        assert!(matches!(
            engine.download_remote(&package).await,
            Err(Error::SecurityViolation(_))
        ));
    }
    assert_eq!(fs::read(victim).unwrap(), b"user data");
}

#[tokio::test]
async fn test_strict_gpg_keyring_required() {
    let server = Server::start(|_| (200, b"unsigned release".to_vec(), Some(16))).await;
    let temp = tempdir().unwrap();
    let res = pkg_core::repository::deb::update_debian_repository(
        &server.url,
        "testsuite",
        &["main".into()],
        None,
        &temp.path().join("keyrings"),
    )
    .await;
    assert!(
        matches!(res, Err(Error::SecurityViolation(ref msg)) if msg.contains("No trusted GPG keyring found")),
        "{res:?}"
    );
}

async fn repository_server(gzip_only: bool, tampered: bool, bad_signature: bool) -> Server {
    Server::start(move |path| {
        let body = if path.ends_with("/InRelease") {
            let text = include_str!("fixtures/repository/InRelease");
            if bad_signature {
                text.replace("Suite: testsuite", "Suite: modified")
                    .into_bytes()
            } else {
                text.as_bytes().to_vec()
            }
        } else if path.ends_with(".xz") && !gzip_only {
            if tampered {
                include_bytes!("fixtures/repository/Packages-tampered.xz").to_vec()
            } else {
                include_bytes!("fixtures/repository/Packages.xz").to_vec()
            }
        } else if path.ends_with(".gz") {
            if tampered {
                include_bytes!("fixtures/repository/Packages-tampered.gz").to_vec()
            } else {
                include_bytes!("fixtures/repository/Packages.gz").to_vec()
            }
        } else {
            return (404, vec![], Some(0));
        };
        let len = body.len();
        (200, body, Some(len))
    })
    .await
}

#[tokio::test]
async fn authenticated_xz_and_gzip_indices_are_accepted() {
    for gzip_only in [false, true] {
        let server = repository_server(gzip_only, false, false).await;
        let temp = tempdir().unwrap();
        let key = temp.path().join("key.asc");
        fs::write(&key, include_bytes!("fixtures/repository/test-key.asc")).unwrap();
        let packages = pkg_core::repository::deb::update_debian_repository(
            &server.url,
            "testsuite",
            &["main".into()],
            Some(&key),
            temp.path(),
        )
        .await
        .unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "review-tool");
    }
}

#[tokio::test]
async fn tampered_indices_and_signatures_preserve_previous_snapshot() {
    use pkg_core::repository::RepositoryConfig;
    for (gzip_only, tampered, bad_signature) in [
        (false, true, false),
        (true, true, false),
        (false, false, true),
    ] {
        let server = repository_server(gzip_only, tampered, bad_signature).await;
        let temp = tempdir().unwrap();
        let key = temp.path().join("key.asc");
        fs::write(&key, include_bytes!("fixtures/repository/test-key.asc")).unwrap();
        let engine = Engine::open(StoreLayout::new(temp.path().join("data"))).unwrap();
        engine
            .db()
            .commit_repository_snapshot(
                "test",
                "deb",
                &server.url,
                "testsuite",
                &[remote_package("unused", "0".repeat(64), 0)],
            )
            .unwrap();
        let config = RepositoriesConfig {
            repositories: vec![RepositoryConfig {
                id: "test".into(),
                format: "deb".into(),
                url: server.url.clone(),
                distribution: "testsuite".into(),
                components: vec!["main".into()],
                public_key_path: Some(key),
                priority: None,
            }],
        };
        assert!(engine.update(&config).await.is_err());
        assert!(engine.get_remote_package("test-tool").unwrap().is_some());
        assert!(engine.get_remote_package("review-tool").unwrap().is_none());
    }
}
