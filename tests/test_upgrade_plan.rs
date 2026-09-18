use pkg_core::domain::package::{
    Architecture, ArtifactDigest, PackageFormat, PackageName, PackageVersion, RemotePackage,
};
use pkg_core::state::NewStoreObject;
use pkg_core::{Engine, StoreLayout};
use tempfile::tempdir;

#[test]
fn upgrade_plan_selects_only_newer_host_compatible_candidates() {
    let temp = tempdir().unwrap();
    let layout = StoreLayout::new(temp.path().join("pkg"));
    let engine = Engine::open(layout.clone()).unwrap();
    let name = PackageName::new("tool").unwrap();
    let version = PackageVersion::new("1.0-1");
    let digest =
        ArtifactDigest::sha256("1111111111111111111111111111111111111111111111111111111111111111");
    let store_id = "111111111111-tool-1.0-1";
    let store_path = layout.store_object_dir(store_id);
    std::fs::create_dir_all(&store_path).unwrap();
    engine
        .db()
        .record_store_object(&NewStoreObject {
            store_id,
            name: &name,
            version: &version,
            architecture: &Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: &digest,
            store_path: &store_path,
            files: &[],
        })
        .unwrap();
    engine
        .db()
        .record_package("default", &name, &version, store_id)
        .unwrap();

    let candidates = vec![
        RemotePackage {
            repository_id: "ubuntu".into(),
            name: "tool".into(),
            version: "0.9-1".into(),
            architecture: "x86_64".into(),
            format: "deb".into(),
            digest: "a".into(),
            size_bytes: 1,
            url: "https://example.invalid/tool-old.deb".into(),
            constraints: vec![],
            provides: vec![],
            versioned_provides: vec![],
        },
        RemotePackage {
            repository_id: "ubuntu".into(),
            name: "tool".into(),
            version: "1.1-1".into(),
            architecture: "x86_64".into(),
            format: "deb".into(),
            digest: "b".into(),
            size_bytes: 1,
            url: "https://example.invalid/tool-new.deb".into(),
            constraints: vec![],
            provides: vec![],
            versioned_provides: vec![],
        },
        RemotePackage {
            repository_id: "ubuntu".into(),
            name: "tool".into(),
            version: "2.0-1".into(),
            architecture: "aarch64".into(),
            format: "deb".into(),
            digest: "c".into(),
            size_bytes: 1,
            url: "https://example.invalid/tool-arm.deb".into(),
            constraints: vec![],
            provides: vec![],
            versioned_provides: vec![],
        },
    ];
    engine
        .db()
        .commit_repository_snapshot(
            "ubuntu",
            "deb",
            "https://example.invalid",
            "stable",
            &candidates,
        )
        .unwrap();

    let upgrades = engine.plan_upgrade("default", None).unwrap();
    assert_eq!(upgrades.len(), 1);
    assert_eq!(upgrades[0].candidate.version, "1.1-1");
}
