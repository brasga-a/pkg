mod common;

use common::deb_builder::DebPackageBuilder;
use std::fs;
use tempfile::tempdir;

use pkg_core::activation::Activator;
use pkg_core::domain::package::RemotePackage;
use pkg_core::engine::{Engine, InstallOptions, RemoteResolution};
use pkg_core::repository::{RepositoriesConfig, RepositoryConfig};
use pkg_core::store::StoreLayout;

#[tokio::test]
async fn test_contextual_dependency_resolution_priorities() {
    let tmp = tempdir().unwrap();
    let layout = StoreLayout::new(tmp.path());
    let engine = Engine::open(layout).unwrap();

    // Configure repositories with Arch having highest priority (30) vs Ubuntu/Debian (10)
    let config = RepositoriesConfig {
        repositories: vec![
            RepositoryConfig {
                id: "arch-core".into(),
                format: "alpm".into(),
                url: "https://geo.mirror.pkgbuild.com/core/os/x86_64".into(),
                distribution: "core".into(),
                components: vec![],
                public_key_path: None,
                priority: Some(30),
            },
            RepositoryConfig {
                id: "ubuntu-noble".into(),
                format: "deb".into(),
                url: "http://archive.ubuntu.com/ubuntu".into(),
                distribution: "noble".into(),
                components: vec!["main".into()],
                public_key_path: None,
                priority: Some(10),
            },
            RepositoryConfig {
                id: "debian-bookworm".into(),
                format: "deb".into(),
                url: "http://deb.debian.org/debian".into(),
                distribution: "bookworm".into(),
                components: vec!["main".into()],
                public_key_path: None,
                priority: Some(10),
            },
        ],
    };

    let fake_packages = [
        RemotePackage {
            repository_id: "arch-core".into(),
            name: "libxml2".into(),
            version: "2.15.4-1".into(),
            architecture: "x86_64".into(),
            format: "alpm".into(),
            digest: "a".repeat(64),
            size_bytes: 1000,
            url: "https://example.com/arch/libxml2".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        },
        RemotePackage {
            repository_id: "ubuntu-noble".into(),
            name: "libxml2".into(),
            version: "2.9.14+dfsg-1.3ubuntu3".into(),
            architecture: "amd64".into(),
            format: "deb".into(),
            digest: "b".repeat(64),
            size_bytes: 1000,
            url: "http://example.com/ubuntu/libxml2".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        },
        RemotePackage {
            repository_id: "debian-bookworm".into(),
            name: "libxml2".into(),
            version: "2.9.14+dfsg-1.3~deb12u6".into(),
            architecture: "amd64".into(),
            format: "deb".into(),
            digest: "c".repeat(64),
            size_bytes: 1000,
            url: "http://example.com/debian/libxml2".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        },
        RemotePackage {
            repository_id: "ubuntu-noble".into(),
            name: "libvirt0".into(),
            version: "10.0.0-2ubuntu8".into(),
            architecture: "amd64".into(),
            format: "deb".into(),
            digest: "d".repeat(64),
            size_bytes: 2000,
            url: "http://example.com/ubuntu/libvirt0".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        },
        RemotePackage {
            repository_id: "debian-bookworm".into(),
            name: "libvirt0".into(),
            version: "9.0.0-4+deb12u2".into(),
            architecture: "amd64".into(),
            format: "deb".into(),
            digest: "e".repeat(64),
            size_bytes: 2000,
            url: "http://example.com/debian/libvirt0".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        },
        RemotePackage {
            repository_id: "arch-core".into(),
            name: "arch-only-dep".into(),
            version: "1.0-1".into(),
            architecture: "x86_64".into(),
            format: "alpm".into(),
            digest: "f".repeat(64),
            size_bytes: 500,
            url: "https://example.com/arch/arch-only-dep".into(),
            constraints: Vec::new(),
            provides: Vec::new(),
            versioned_provides: Vec::new(),
        },
    ];

    // Seed packages into state db
    for r in &config.repositories {
        let repo_pkgs: Vec<_> = fake_packages
            .iter()
            .filter(|p| p.repository_id == r.id)
            .cloned()
            .collect();
        engine
            .db()
            .commit_repository_snapshot(&r.id, &r.format, &r.url, &r.distribution, &repo_pkgs)
            .unwrap();
    }

    // 1. Contextual resolution for libxml2 with parent repo = ubuntu-noble [deb]:
    // Must select ubuntu-noble [deb] even though arch-core has higher priority (30 vs 10).
    let res = engine
        .resolve_dependency_package("libxml2", Some("ubuntu-noble"), Some("deb"), Some(&config))
        .unwrap();
    match res {
        RemoteResolution::Exact(p) => {
            assert_eq!(p.repository_id, "ubuntu-noble");
            assert_eq!(p.format, "deb");
            assert_eq!(p.version, "2.9.14+dfsg-1.3ubuntu3");
        }
        other => panic!("Expected exact ubuntu-noble resolution, got {:?}", other),
    }

    // 2. Ambiguity resolution for libvirt0 (exists in both ubuntu-noble and debian-bookworm with equal priority):
    // Must pick ubuntu-noble because preferred_repo is ubuntu-noble!
    let res = engine
        .resolve_dependency_package("libvirt0", Some("ubuntu-noble"), Some("deb"), Some(&config))
        .unwrap();
    match res {
        RemoteResolution::Exact(p) => {
            assert_eq!(p.repository_id, "ubuntu-noble");
            assert_eq!(p.name, "libvirt0");
        }
        other => panic!("Expected exact ubuntu-noble resolution, got {:?}", other),
    }

    // 3. Prevent cross-format mixing ("não fazermos salada de frutas"):
    // An alpm package must NOT be accepted when preferred_format is deb.
    let res = engine
        .resolve_dependency_package(
            "arch-only-dep",
            Some("ubuntu-noble"),
            Some("deb"),
            Some(&config),
        )
        .unwrap();
    assert_eq!(res, RemoteResolution::NotFound);
}

#[test]
fn test_profile_library_activation_and_deactivation() {
    let tmp = tempdir().unwrap();
    let layout = StoreLayout::new(tmp.path());
    let profile = "default";
    let profile_lib = layout.profile_lib_dir(profile);

    // Create a mock store directory containing shared libraries
    let store_dir = tmp.path().join("store/fake-libvirt0-10.0");
    let lib_dir = store_dir.join("usr/lib/x86_64-linux-gnu");
    fs::create_dir_all(&lib_dir).unwrap();

    let lib1 = lib_dir.join("libvirt.so.0");
    let lib2 = lib_dir.join("libvirt-lxc.so.0");
    fs::write(&lib1, b"mock elf library 1").unwrap();
    fs::write(&lib2, b"mock elf library 2").unwrap();

    // Activate libraries
    Activator::activate_libraries(&store_dir, &profile_lib).unwrap();

    assert!(profile_lib.join("libvirt.so.0").exists());
    assert!(profile_lib.join("libvirt-lxc.so.0").exists());
    assert!(profile_lib.join("libvirt.so.0").is_symlink());

    // Extra search paths include profile_lib
    let extra_paths = [profile_lib.clone()];
    assert!(extra_paths.iter().any(|d| d.join("libvirt.so.0").exists()));

    // Deactivate libraries
    Activator::deactivate_libraries(&store_dir, &profile_lib).unwrap();
    assert!(!profile_lib.join("libvirt.so.0").exists());
    assert!(!profile_lib.join("libvirt-lxc.so.0").exists());
}

fn elf_requiring(library: &str) -> Vec<u8> {
    let mut bytes = vec![0u8; 512];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    fn u16_at(b: &mut [u8], n: usize, v: u16) {
        b[n..n + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn u32_at(b: &mut [u8], n: usize, v: u32) {
        b[n..n + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn u64_at(b: &mut [u8], n: usize, v: u64) {
        b[n..n + 8].copy_from_slice(&v.to_le_bytes());
    }
    u16_at(&mut bytes, 16, 3);
    u16_at(&mut bytes, 18, 62);
    u32_at(&mut bytes, 20, 1);
    u64_at(&mut bytes, 32, 64);
    u16_at(&mut bytes, 52, 64);
    u16_at(&mut bytes, 54, 56);
    u16_at(&mut bytes, 56, 2);
    u32_at(&mut bytes, 64, 1);
    u32_at(&mut bytes, 68, 4);
    u64_at(&mut bytes, 96, 512);
    u64_at(&mut bytes, 104, 512);
    u32_at(&mut bytes, 120, 2);
    u32_at(&mut bytes, 124, 4);
    u64_at(&mut bytes, 128, 176);
    u64_at(&mut bytes, 136, 176);
    u64_at(&mut bytes, 152, 64);
    u64_at(&mut bytes, 160, 64);
    for (i, (tag, value)) in [(5, 256), (10, library.len() as u64 + 2), (1, 1), (0, 0)]
        .iter()
        .enumerate()
    {
        u64_at(&mut bytes, 176 + i * 16, *tag);
        u64_at(&mut bytes, 184 + i * 16, *value);
    }
    bytes[257..257 + library.len()].copy_from_slice(library.as_bytes());
    bytes
}

#[test]
fn test_preflight_and_install_resolves_store_libraries() {
    let tmp = tempdir().unwrap();
    let layout = StoreLayout::new(tmp.path());
    let engine = Engine::open(layout).unwrap();

    let custom_lib = "libtest-dep.so.1";
    let dep_artifact = tmp.path().join("test-dep.deb");
    DebPackageBuilder::new("test-dep")
        .file(
            "usr/lib/x86_64-linux-gnu/libtest-dep.so.1",
            b"dummy shared lib content",
            0o644,
        )
        .write_to(&dep_artifact)
        .unwrap();

    let app_artifact = tmp.path().join("test-app.deb");
    DebPackageBuilder::new("test-app")
        .file("usr/bin/test-app", elf_requiring(custom_lib), 0o755)
        .write_to(&app_artifact)
        .unwrap();

    // 1. Before installing test-dep, preflight reports libtest-dep.so.1 as missing
    let preflight = engine
        .preflight_check_with_profile(&app_artifact, "default")
        .unwrap();
    assert!(
        preflight
            .missing_libraries
            .contains(&custom_lib.to_string())
    );

    // 2. Install test-dep dependency package
    engine
        .install_with_options(&dep_artifact, "default", false, InstallOptions::default())
        .unwrap();

    // Verify lib is activated in profile/lib
    let profile_lib = engine.layout().profile_lib_dir("default");
    assert!(profile_lib.join(custom_lib).exists());

    // 3. Re-evaluate preflight for test-app: custom_lib should now be satisfied!
    let preflight_after = engine
        .preflight_check_with_profile(&app_artifact, "default")
        .unwrap();
    assert!(
        !preflight_after
            .missing_libraries
            .contains(&custom_lib.to_string())
    );
    assert!(preflight_after.missing_libraries.is_empty());

    // 4. Installing test-app now succeeds WITHOUT allow_missing_libraries!
    let plan = engine
        .install_with_options(
            &app_artifact,
            "default",
            false,
            InstallOptions {
                allow_missing_libraries: false,
            },
        )
        .unwrap();
    assert!(
        plan.host_libraries_verified
            .contains(&custom_lib.to_string())
    );
    assert!(plan.missing_libraries.is_empty());
}
