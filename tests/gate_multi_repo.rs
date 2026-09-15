//! Gate tests for multi-ecosystem remote repositories (RPM-MD, Arch ALPM, and Debian),
//! catalog synchronization, format filtering, target qualification, and disambiguation.

mod common;

use common::{AlpmPackageBuilder, RpmPackageBuilder};
use flate2::Compression;
use flate2::write::GzEncoder;
use pkg_core::domain::package::ArtifactDigest;
use pkg_core::engine::RemoteResolution;
use pkg_core::repository::{RepositoriesConfig, RepositoryConfig};
use pkg_core::{Engine, StoreLayout};
use sha2::{Digest, Sha256};
use std::fs;
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

fn create_alpm_db(pkgname: &str, ver: &str, sha: &str, size: u64, filename: &str) -> Vec<u8> {
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

fn create_rpm_primary(
    pkgname: &str,
    ver: &str,
    rel: &str,
    sha: &str,
    size: u64,
    href: &str,
) -> (Vec<u8>, String) {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata xmlns="http://linux.duke.edu/metadata/common" packages="1">
  <package type="rpm">
    <name>{pkgname}</name>
    <arch>x86_64</arch>
    <version epoch="0" ver="{ver}" rel="{rel}"/>
    <checksum type="sha256" pkgid="YES">{sha}</checksum>
    <size package="{size}"/>
    <location href="{href}"/>
  </package>
</metadata>"#
    );

    let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
    gz_encoder.write_all(xml.as_bytes()).unwrap();
    let gz_bytes = gz_encoder.finish().unwrap();

    let digest = format!("{:x}", Sha256::digest(&gz_bytes));
    (gz_bytes, digest)
}

#[tokio::test]
async fn test_multi_repo_online_sync_and_search() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("store"))).unwrap();

    // 1. Build test mock repository contents
    let alpm_db_bytes = create_alpm_db(
        "review-tool",
        "8.10.1-1",
        "0000111122223333444455556666777788889999aaaabbbbccccddddeeeeffff",
        350000,
        "review-tool-8.10.1-1-x86_64.pkg.tar.zst",
    );

    let (rpm_primary_gz, rpm_primary_sha) = create_rpm_primary(
        "review-tool",
        "8.9.1",
        "1.fc41",
        "aaaabbbbccccddddeeeeffff0000111122223333444455556666777788889999",
        410000,
        "Packages/r/review-tool-8.9.1-1.fc41.x86_64.rpm",
    );

    let repomd_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<repomd xmlns="http://linux.duke.edu/metadata/repo">
  <data type="primary">
    <checksum type="sha256">{rpm_primary_sha}</checksum>
    <location href="repodata/primary.xml.gz"/>
    <size>{}</size>
  </data>
</repomd>"#,
        rpm_primary_gz.len()
    );

    // Debian InRelease & Packages fixture (pre-signed fixture)
    let inrelease_fixture = include_str!("fixtures/repository/InRelease");
    let deb_packages_gz = include_bytes!("fixtures/repository/Packages.gz").to_vec();
    let deb_packages_xz = include_bytes!("fixtures/repository/Packages.xz").to_vec();

    let alpm_db_copy = alpm_db_bytes.clone();
    let rpm_primary_copy = rpm_primary_gz.clone();
    let repomd_xml_copy = repomd_xml.into_bytes();

    let server = MockServer::start(move |path| {
        if path.ends_with("/extra.db") {
            (200, alpm_db_copy.clone())
        } else if path.ends_with("/repodata/repomd.xml") {
            (200, repomd_xml_copy.clone())
        } else if path.ends_with("/repodata/primary.xml.gz") {
            (200, rpm_primary_copy.clone())
        } else if path.ends_with("/InRelease") {
            (200, inrelease_fixture.as_bytes().to_vec())
        } else if path.ends_with("/Packages.xz") {
            (200, deb_packages_xz.clone())
        } else if path.ends_with("/Packages.gz") {
            (200, deb_packages_gz.clone())
        } else {
            (404, b"Not found".to_vec())
        }
    })
    .await;

    // Debian signing key from fixture
    let deb_key = temp.path().join("archive.asc");
    fs::write(&deb_key, include_str!("fixtures/repository/test-key.asc")).unwrap();

    let config = RepositoriesConfig {
        repositories: vec![
            RepositoryConfig {
                id: "fedora-41".to_string(),
                format: "rpm".to_string(),
                url: format!("{}/fedora", server.url),
                distribution: "41".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(20),
            },
            RepositoryConfig {
                id: "arch-extra".to_string(),
                format: "alpm".to_string(),
                url: format!("{}/arch", server.url),
                distribution: "extra".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(30),
            },
            RepositoryConfig {
                id: "ubuntu-noble".to_string(),
                format: "deb".to_string(),
                url: format!("{}/deb", server.url),
                distribution: "testsuite".to_string(),
                components: vec!["main".to_string()],
                public_key_path: Some(deb_key),
                priority: Some(10),
            },
        ],
    };

    // 2. Perform multi-ecosystem repository update
    let total = engine
        .update(&config)
        .await
        .expect("Multi-repository update must succeed");
    assert!(
        total >= 3,
        "Expected at least 3 packages synchronized across distros"
    );

    // 3. Search for 'review-tool' and verify all formats are present
    let all_tools = engine.search("review-tool").unwrap();
    assert_eq!(
        all_tools.len(),
        3,
        "Should find review-tool in all 3 ecosystems"
    );

    let formats: Vec<_> = all_tools.iter().map(|p| p.format.as_str()).collect();
    assert!(formats.contains(&"rpm"), "Must contain rpm format");
    assert!(formats.contains(&"alpm"), "Must contain alpm format");
    assert!(formats.contains(&"deb"), "Must contain deb format");

    // 4. Test filtering by format
    let rpm_only = engine
        .search_filtered("review-tool", Some("rpm"), None)
        .unwrap();
    assert_eq!(rpm_only.len(), 1);
    assert_eq!(rpm_only[0].repository_id, "fedora-41");
    assert_eq!(rpm_only[0].format, "rpm");

    let alpm_only = engine
        .search_filtered("review-tool", Some("alpm"), None)
        .unwrap();
    assert_eq!(alpm_only.len(), 1);
    assert_eq!(alpm_only[0].repository_id, "arch-extra");
    assert_eq!(alpm_only[0].format, "alpm");

    let deb_only = engine
        .search_filtered("review-tool", Some("deb"), None)
        .unwrap();
    assert_eq!(deb_only.len(), 1);
    assert_eq!(deb_only[0].repository_id, "ubuntu-noble");
    assert_eq!(deb_only[0].format, "deb");

    // 5. Test filtering by repository ID
    let repo_filter = engine
        .search_filtered("review-tool", None, Some("fedora-41"))
        .unwrap();
    assert_eq!(repo_filter.len(), 1);
    assert_eq!(repo_filter[0].repository_id, "fedora-41");
}

#[tokio::test]
async fn test_target_resolution_and_disambiguation() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("store"))).unwrap();

    // Populate database directly with candidates across 3 repos
    let fedora_pkg = pkg_core::domain::package::RemotePackage {
        repository_id: "fedora-41".to_string(),
        name: "curl".to_string(),
        version: "8.9.1-1.fc41".to_string(),
        architecture: "x86_64".to_string(),
        format: "rpm".to_string(),
        digest: "a".repeat(64),
        size_bytes: 400000,
        url: "http://example.com/curl.rpm".to_string(),
    };

    let arch_pkg = pkg_core::domain::package::RemotePackage {
        repository_id: "arch-extra".to_string(),
        name: "curl".to_string(),
        version: "8.10.1-1".to_string(),
        architecture: "x86_64".to_string(),
        format: "alpm".to_string(),
        digest: "b".repeat(64),
        size_bytes: 350000,
        url: "http://example.com/curl.pkg.tar.zst".to_string(),
    };

    let deb_pkg = pkg_core::domain::package::RemotePackage {
        repository_id: "ubuntu-noble".to_string(),
        name: "curl".to_string(),
        version: "8.5.0-2".to_string(),
        architecture: "amd64".to_string(),
        format: "deb".to_string(),
        digest: "c".repeat(64),
        size_bytes: 250000,
        url: "http://example.com/curl.deb".to_string(),
    };

    engine
        .db()
        .commit_repository_snapshot("fedora-41", "rpm", "url", "41", &[fedora_pkg])
        .unwrap();
    engine
        .db()
        .commit_repository_snapshot("arch-extra", "alpm", "url", "extra", &[arch_pkg])
        .unwrap();
    engine
        .db()
        .commit_repository_snapshot("ubuntu-noble", "deb", "url", "noble", &[deb_pkg])
        .unwrap();

    // 1. Qualified by repository: fedora-41/curl
    let res = engine
        .resolve_remote_package("fedora-41/curl", None)
        .unwrap();
    match res {
        RemoteResolution::Exact(pkg) => {
            assert_eq!(pkg.repository_id, "fedora-41");
            assert_eq!(pkg.format, "rpm");
        }
        other => panic!("Expected Exact match, got {other:?}"),
    }

    // 2. Qualified by format: curl:alpm
    let res = engine.resolve_remote_package("curl:alpm", None).unwrap();
    match res {
        RemoteResolution::Exact(pkg) => {
            assert_eq!(pkg.repository_id, "arch-extra");
            assert_eq!(pkg.format, "alpm");
        }
        other => panic!("Expected Exact match, got {other:?}"),
    }

    // 3. Qualified by version: curl@8.5.0-2
    let res = engine.resolve_remote_package("curl@8.5.0-2", None).unwrap();
    match res {
        RemoteResolution::Exact(pkg) => {
            assert_eq!(pkg.repository_id, "ubuntu-noble");
            assert_eq!(pkg.version, "8.5.0-2");
        }
        other => panic!("Expected Exact match, got {other:?}"),
    }

    // 4. Unqualified without priority -> Ambiguous (3 candidates)
    let res = engine.resolve_remote_package("curl", None).unwrap();
    match res {
        RemoteResolution::Ambiguous(cands) => {
            assert_eq!(cands.len(), 3);
        }
        other => panic!("Expected Ambiguous resolution, got {other:?}"),
    }

    // 5. Unqualified with priority -> Arch has highest priority (30)
    let config = RepositoriesConfig {
        repositories: vec![
            RepositoryConfig {
                id: "fedora-41".to_string(),
                format: "rpm".to_string(),
                url: "http://f.org".to_string(),
                distribution: "41".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(20),
            },
            RepositoryConfig {
                id: "arch-extra".to_string(),
                format: "alpm".to_string(),
                url: "http://a.org".to_string(),
                distribution: "extra".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(30),
            },
            RepositoryConfig {
                id: "ubuntu-noble".to_string(),
                format: "deb".to_string(),
                url: "http://u.org".to_string(),
                distribution: "noble".to_string(),
                components: vec![],
                public_key_path: None,
                priority: Some(10),
            },
        ],
    };

    let res = engine
        .resolve_remote_package("curl", Some(&config))
        .unwrap();
    match res {
        RemoteResolution::Exact(pkg) => {
            assert_eq!(pkg.repository_id, "arch-extra");
            assert_eq!(pkg.format, "alpm");
        }
        other => panic!("Expected Exact resolution via priority, got {other:?}"),
    }

    // 6. Unknown package -> NotFound
    let res = engine.resolve_remote_package("nonexistent", None).unwrap();
    assert_eq!(res, RemoteResolution::NotFound);
}

#[tokio::test]
async fn test_remote_download_and_install_rpm_and_alpm() {
    let temp = tempdir().unwrap();
    let engine = Engine::open(StoreLayout::new(temp.path().join("store"))).unwrap();

    // Build real synthetic RPM and ALPM artifacts
    let rpm_path = temp.path().join("hello-tool-1.0.0-1.fc40.x86_64.rpm");
    RpmPackageBuilder::new("hello-tool")
        .version("1.0.0")
        .release("1.fc40")
        .file("usr/bin/hello-tool", b"#!/bin/sh\necho hello rpm\n", 0o755)
        .write_to(&rpm_path)
        .unwrap();

    let alpm_path = temp.path().join("alpm-tool-2.0.0-1-x86_64.pkg.tar.zst");
    AlpmPackageBuilder::new("alpm-tool")
        .version("2.0.0-1")
        .file("usr/bin/alpm-tool", b"#!/bin/sh\necho hello alpm\n", 0o755)
        .write_to(&alpm_path)
        .unwrap();

    let rpm_bytes = fs::read(&rpm_path).unwrap();
    let alpm_bytes = fs::read(&alpm_path).unwrap();

    let rpm_digest = ArtifactDigest::from_file(&rpm_path).unwrap();
    let alpm_digest = ArtifactDigest::from_file(&alpm_path).unwrap();

    let rpm_b = rpm_bytes.clone();
    let alpm_b = alpm_bytes.clone();

    let server = MockServer::start(move |path| {
        if path.ends_with("/hello-tool.rpm") {
            (200, rpm_b.clone())
        } else if path.ends_with("/alpm-tool.pkg.tar.zst") {
            (200, alpm_b.clone())
        } else {
            (404, b"Not found".to_vec())
        }
    })
    .await;

    // 1. Download & Install Remote RPM
    let remote_rpm = pkg_core::domain::package::RemotePackage {
        repository_id: "fedora-41".to_string(),
        name: "hello-tool".to_string(),
        version: "1.0.0-1.fc40".to_string(),
        architecture: "x86_64".to_string(),
        format: "rpm".to_string(),
        digest: rpm_digest.hex().to_string(),
        size_bytes: rpm_bytes.len() as u64,
        url: format!("{}/hello-tool.rpm", server.url),
    };

    let cached_rpm = engine
        .download_remote(&remote_rpm)
        .await
        .expect("RPM download must succeed");
    let rpm_plan = engine
        .install(&cached_rpm, "default", false)
        .expect("RPM install must succeed");
    assert_eq!(rpm_plan.package.name.as_str(), "hello-tool");
    assert_eq!(rpm_plan.package.format.to_string(), "rpm");

    // 2. Download & Install Remote ALPM
    let remote_alpm = pkg_core::domain::package::RemotePackage {
        repository_id: "arch-extra".to_string(),
        name: "alpm-tool".to_string(),
        version: "2.0.0-1".to_string(),
        architecture: "x86_64".to_string(),
        format: "alpm".to_string(),
        digest: alpm_digest.hex().to_string(),
        size_bytes: alpm_bytes.len() as u64,
        url: format!("{}/alpm-tool.pkg.tar.zst", server.url),
    };

    let cached_alpm = engine
        .download_remote(&remote_alpm)
        .await
        .expect("ALPM download must succeed");
    let alpm_plan = engine
        .install(&cached_alpm, "default", false)
        .expect("ALPM install must succeed");
    assert_eq!(alpm_plan.package.name.as_str(), "alpm-tool");
    assert_eq!(alpm_plan.package.format.to_string(), "alpm");

    // 3. Verify installed packages list
    let installed = engine.list("default").unwrap();
    assert_eq!(installed.len(), 2);
    let names: Vec<_> = installed.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"hello-tool"));
    assert!(names.contains(&"alpm-tool"));

    // 4. Verify activated binaries
    let profile_bin = engine.layout().profile_bin_dir("default");
    assert!(profile_bin.join("hello-tool").exists());
    assert!(profile_bin.join("alpm-tool").exists());
}
