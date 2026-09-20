//! Integration tests for Curated Repository Registry, Distro Auto-Detection,
//! CLI repository management commands, and Resilient Parallel Synchronization.

use assert_cmd::Command;
use flate2::Compression;
use flate2::write::GzEncoder;
use pkg_core::engine::Engine;
use pkg_core::host::HostFacts;
use pkg_core::repository::{CuratedRegistry, RepositoriesConfig, RepositoryConfig};
use pkg_core::store::StoreLayout;
use std::io::Write;
use std::sync::Arc;
use tar::Builder;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct MockServer {
    url: String,
    _task: tokio::task::JoinHandle<()>,
}

impl MockServer {
    async fn start<F>(handler: F) -> Self
    where
        F: Fn(&str) -> (u16, Vec<u8>) + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handler = Arc::new(handler);

        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => break,
                };
                let handler = Arc::clone(&handler);

                tokio::spawn(async move {
                    let mut request = Vec::new();
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let mut buf = [0; 4096];
                        let n = socket.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&buf[..n]);
                    }
                    let req_str = String::from_utf8_lossy(&request);
                    let path = req_str.split_whitespace().nth(1).unwrap_or("/");
                    let (status, body) = handler(path);

                    let headers = format!(
                        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(headers.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });

        Self { url, _task: task }
    }
}

fn create_mock_alpm_db(pkgname: &str, ver: &str, sha: &str, size: u64, filename: &str) -> Vec<u8> {
    let mut tar_builder = Builder::new(Vec::new());
    let desc_content = format!(
        "%FILENAME%\n{filename}\n\n%NAME%\n{pkgname}\n\n%VERSION%\n{ver}\n\n%CSIZE%\n{size}\n\n%SHA256SUM%\n{sha}\n\n%ARCH%\nx86_64\n"
    );

    let mut header = tar::Header::new_gnu();
    header.set_path(format!("{pkgname}-{ver}/desc")).unwrap();
    header.set_size(desc_content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar_builder
        .append(&header, desc_content.as_bytes())
        .unwrap();

    let tar_bytes = tar_builder.into_inner().unwrap();
    let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
    gz_encoder.write_all(&tar_bytes).unwrap();
    gz_encoder.finish().unwrap()
}

#[test]
fn test_curated_registry_fallback_structure() {
    let registry = CuratedRegistry::fallback();
    assert_eq!(registry.schema_version, "1.0");
    assert!(!registry.repositories.is_empty());

    let expected_ids = [
        "arch-core",
        "arch-extra",
        "arch-multilib",
        "ubuntu-noble",
        "ubuntu-resolute",
        "ubuntu-jammy",
        "ubuntu-focal",
        "debian-bookworm",
        "debian-trixie",
        "debian-sid",
        "debian-bullseye",
        "fedora-41",
        "fedora-42",
        "fedora-40",
        "opensuse-tumbleweed",
        "opensuse-leap-15-6",
        "alpine-v3.20",
        "alpine-edge",
        "almalinux-9-baseos",
        "almalinux-9-appstream",
        "rocky-9-baseos",
        "rocky-9-appstream",
    ];

    for id in &expected_ids {
        assert!(
            registry.find_by_id(id).is_some(),
            "Expected repository '{id}' to exist in curated registry"
        );
    }
}

#[test]
fn test_curated_registry_distro_detection_heuristics() {
    let registry = CuratedRegistry::fallback();

    // Arch Linux
    let arch_repos = registry.default_for_distro("arch", None, None);
    let arch_ids: Vec<&str> = arch_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(arch_ids, vec!["arch-core", "arch-extra"]);

    // Arch derivative (Manjaro)
    let manjaro_repos = registry.default_for_distro("manjaro", None, None);
    let manjaro_ids: Vec<&str> = manjaro_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(manjaro_ids, vec!["arch-core", "arch-extra"]);

    // Ubuntu Noble 24.04
    let ubuntu_noble = registry.default_for_distro("ubuntu", Some("noble"), Some("24.04"));
    let u_noble_ids: Vec<&str> = ubuntu_noble.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(u_noble_ids, vec!["ubuntu-noble"]);

    // Ubuntu Resolute 26.04
    let ubuntu_resolute = registry.default_for_distro("ubuntu", Some("resolute"), Some("26.04"));
    let u_res_ids: Vec<&str> = ubuntu_resolute.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(u_res_ids, vec!["ubuntu-resolute"]);

    // Debian Bookworm
    let debian_repos = registry.default_for_distro("debian", Some("bookworm"), Some("12"));
    let deb_ids: Vec<&str> = debian_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(deb_ids, vec!["debian-bookworm"]);

    // Fedora 41
    let fedora_repos = registry.default_for_distro("fedora", None, Some("41"));
    let fed_ids: Vec<&str> = fedora_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(fed_ids, vec!["fedora-41"]);

    // openSUSE Tumbleweed
    let suse_repos = registry.default_for_distro("opensuse-tumbleweed", None, None);
    let suse_ids: Vec<&str> = suse_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(suse_ids, vec!["opensuse-tumbleweed"]);

    // Alpine v3.20
    let alpine_repos = registry.default_for_distro("alpine", None, Some("3.20"));
    let alp_ids: Vec<&str> = alpine_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(alp_ids, vec!["alpine-v3.20"]);

    // AlmaLinux 9
    let alma_repos = registry.default_for_distro("almalinux", None, Some("9"));
    let alma_ids: Vec<&str> = alma_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        alma_ids,
        vec!["almalinux-9-baseos", "almalinux-9-appstream"]
    );

    // Unknown distro fallback
    let unknown_repos = registry.default_for_distro("unknown_distro", None, None);
    let unk_ids: Vec<&str> = unknown_repos.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(unk_ids, vec!["arch-core", "arch-extra"]);
}

#[test]
fn test_host_facts_default_repositories_config() {
    let config = HostFacts::default_repositories_config();
    assert!(
        !config.repositories.is_empty(),
        "HostFacts should produce at least one default repository"
    );
    for repo in &config.repositories {
        assert!(!repo.id.is_empty());
        assert!(!repo.url.is_empty());
        assert!(!repo.format.is_empty());
    }
}

#[test]
fn test_cli_repo_list_remote() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().to_str().unwrap();

    let mut cmd = Command::cargo_bin("pkg").unwrap();
    let assert = cmd
        .args(["--data-dir", data_dir, "repo", "remote"])
        .assert();
    assert
        .success()
        .stdout(predicates::str::contains("arch-core"))
        .stdout(predicates::str::contains("ubuntu-noble"))
        .stdout(predicates::str::contains("fedora-41"));
}

#[test]
fn test_cli_repo_list_remote_json() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().to_str().unwrap();

    let mut cmd = Command::cargo_bin("pkg").unwrap();
    let assert = cmd
        .args(["--data-dir", data_dir, "--json", "repo", "remote"])
        .assert();
    let output = assert.success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("valid json output");
    assert_eq!(parsed["status"], "success");
    assert_eq!(parsed["schema_version"], "1.0");
    let repos = parsed["repositories"].as_array().expect("array of repos");
    assert!(repos.len() >= 8);
}

#[test]
fn test_cli_repo_add_curated_id() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().to_str().unwrap();

    // 1. Add arch-multilib via curated ID
    let mut cmd = Command::cargo_bin("pkg").unwrap();
    let assert = cmd
        .args(["--data-dir", data_dir, "repo", "add", "arch-multilib"])
        .assert();
    assert.success().stdout(predicates::str::contains(
        "Successfully added repository 'arch-multilib'",
    ));

    // 2. Verify repositories.toml was created and contains arch-multilib
    let config_path = temp.path().join("repositories.toml");
    assert!(config_path.exists());
    let config = RepositoriesConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config.repositories.len(), 1);
    assert_eq!(config.repositories[0].id, "arch-multilib");
    assert_eq!(config.repositories[0].format, "alpm");
    assert_eq!(config.repositories[0].distribution, "multilib");

    // 3. Adding again should fail with duplicate error
    let mut dup_cmd = Command::cargo_bin("pkg").unwrap();
    dup_cmd
        .args(["--data-dir", data_dir, "repo", "add", "arch-multilib"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));

    // 4. Adding non-existent curated ID should fail informatively
    let mut invalid_cmd = Command::cargo_bin("pkg").unwrap();
    invalid_cmd
        .args([
            "--data-dir",
            data_dir,
            "repo",
            "add",
            "non-existent-distro-repo",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not in the curated registry"));

    // 5. Remove arch-multilib
    let mut remove_cmd = Command::cargo_bin("pkg").unwrap();
    remove_cmd
        .args(["--data-dir", data_dir, "repo", "remove", "arch-multilib"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Successfully removed repository 'arch-multilib'",
        ));

    let config_after = RepositoriesConfig::load_from_file(&config_path).unwrap();
    assert_eq!(config_after.repositories.len(), 0);
}

#[tokio::test]
async fn test_resilient_parallel_sync() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("store"))).unwrap();

    let alpm_db_bytes = create_mock_alpm_db(
        "resilient-pkg",
        "1.0.0-1",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1024,
        "resilient-pkg-1.0.0-1-x86_64.pkg.tar.zst",
    );

    let server = MockServer::start(move |path| {
        if path == "/core/os/x86_64/core.db" {
            (200, alpm_db_bytes.clone())
        } else {
            (404, b"Not Found".to_vec())
        }
    })
    .await;

    // Config with 1 working repo and 1 broken repo (network error)
    let config = RepositoriesConfig {
        repositories: vec![
            RepositoryConfig {
                id: "working-arch".to_string(),
                format: "alpm".to_string(),
                url: server.url.clone(),
                distribution: "core".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(100),
            },
            RepositoryConfig {
                id: "broken-repo".to_string(),
                format: "alpm".to_string(),
                // Use invalid port where nothing is listening
                url: "http://127.0.0.1:1".to_string(),
                distribution: "core".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(50),
            },
        ],
    };

    // Parallel update should NOT fail the entire batch: working-arch must be saved
    let total = engine
        .update(&config)
        .await
        .expect("Resilient sync should succeed when at least one repo succeeds");
    assert_eq!(total, 1);

    // Search verifies working repo was committed into SQLite
    let results = engine.search("resilient-pkg").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "resilient-pkg");
    assert_eq!(results[0].repository_id, "working-arch");

    // Now test fail-closed behavior: when ALL repos fail, engine.update must return Err
    let all_broken_config = RepositoriesConfig {
        repositories: vec![RepositoryConfig {
            id: "broken-1".to_string(),
            format: "alpm".to_string(),
            url: "http://127.0.0.1:1".to_string(),
            distribution: "core".to_string(),
            components: vec![],
            public_key_path: None,
            priority: None,
        }],
    };

    let fail_result = engine.update(&all_broken_config).await;
    assert!(
        fail_result.is_err(),
        "Engine update must fail closed when all repositories fail"
    );
}
