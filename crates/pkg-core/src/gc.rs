//! Reachability-based garbage collection for pkg-owned store objects.
//!
//! GC is deliberately conservative: only objects recorded in the state database,
//! whose paths are direct children of the configured store and which have no
//! package, activation, or incomplete-transaction reference are candidates.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::Result;
use crate::state::{StateDatabase, StoreObjectRecord};
use crate::store::StoreLayout;

/// One store object that can be collected safely according to the current snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GcCandidate {
    /// Stable store identity.
    pub store_id: String,
    /// Package represented by the object.
    pub package_name: String,
    /// Version represented by the object.
    pub version: String,
    /// Validated path that would be removed.
    pub store_path: PathBuf,
}

/// Result of a GC scan or collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GcReport {
    /// Objects identified as unreachable.
    pub candidates: Vec<GcCandidate>,
    /// Objects retained because a reference was found or the state was uncertain.
    pub retained: Vec<String>,
    /// Runtime manifests with no retained generation reference.
    pub runtime_manifests: Vec<String>,
    /// Whether candidate directories were removed.
    pub collected: bool,
}

/// Performs conservative reachability analysis and optional collection.
#[derive(Debug)]
pub struct GarbageCollector;

impl GarbageCollector {
    /// Scans the state database without mutating the filesystem or database.
    pub fn scan(layout: &StoreLayout, db: &StateDatabase) -> Result<GcReport> {
        Self::run(layout, db, false)
    }

    /// Collects only objects that remain unreachable after a locked revalidation.
    pub fn collect(layout: &StoreLayout, db: &StateDatabase) -> Result<GcReport> {
        Self::run(layout, db, true)
    }

    fn run(layout: &StoreLayout, db: &StateDatabase, collect: bool) -> Result<GcReport> {
        let mut candidates = Vec::new();
        let mut retained = Vec::new();
        let orphan_runtimes = db.unreferenced_runtime_manifests()?;
        let runtime_references =
            db.runtime_store_references(&layout.runtimes_dir(), &layout.store_dir())?;

        for object in db.list_store_objects()? {
            let expected = layout.store_object_dir(&object.store_id);
            if db.store_is_referenced(&object.store_id)?
                || runtime_references.iter().any(|path| path == &expected)
            {
                retained.push(object.store_id);
                continue;
            }

            // The DB path must match the configured direct-child path exactly.
            // A mismatch is state corruption, not permission to delete either path.
            if object.store_path != expected {
                retained.push(object.store_id);
                continue;
            }
            layout.validate_store_path(&expected)?;
            if expected.exists() {
                candidates.push(Self::candidate(object));
            } else {
                // Keep the state row for doctor/reconciliation instead of silently
                // claiming that an absent object was collected.
                retained.push(Self::candidate_id(object));
            }
        }

        if collect {
            for candidate in &candidates {
                // Re-check references immediately before each deletion. The caller
                // holds the process writer lock, so another pkg writer cannot race us.
                if db.store_is_referenced(&candidate.store_id)?
                    || runtime_references
                        .iter()
                        .any(|path| path == &layout.store_object_dir(&candidate.store_id))
                {
                    retained.push(candidate.store_id.clone());
                    continue;
                }
                let path = layout.store_object_dir(&candidate.store_id);
                layout.validate_store_path(&path)?;
                if !path.exists() {
                    retained.push(candidate.store_id.clone());
                    continue;
                }
                fs::remove_dir_all(&path)?;
                db.remove_store_object_after_committed_check(&candidate.store_id)?;
            }
            for (runtime_id, manifest_path) in &orphan_runtimes {
                let expected = layout.runtime_dir(runtime_id).join("manifest.json");
                if *manifest_path != expected {
                    return Err(crate::error::Error::TransactionRecoveryRequired(format!(
                        "runtime manifest path mismatch: {}",
                        manifest_path.display()
                    )));
                }
                let runtime_dir = layout.runtime_dir(runtime_id);
                if let Ok(metadata) = fs::symlink_metadata(&runtime_dir) {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(crate::error::Error::SecurityViolation(format!(
                            "invalid runtime directory: {}",
                            runtime_dir.display()
                        )));
                    }
                    fs::remove_dir_all(&runtime_dir)?;
                }
                db.remove_runtime_manifest(runtime_id)?;
            }
            // Keep the candidate list as an audit of what was actually removed.
            // The `collected` flag tells callers that these paths no longer exist.
        }

        Ok(GcReport {
            candidates,
            retained,
            runtime_manifests: orphan_runtimes
                .into_iter()
                .map(|(runtime_id, _)| runtime_id)
                .collect(),
            collected: collect,
        })
    }

    fn candidate(object: StoreObjectRecord) -> GcCandidate {
        GcCandidate {
            store_id: object.store_id,
            package_name: object.package_name,
            version: object.version,
            store_path: object.store_path,
        }
    }

    fn candidate_id(object: StoreObjectRecord) -> String {
        object.store_id
    }
}

impl std::fmt::Display for GcCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} ({})",
            self.package_name, self.version, self.store_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::package::{
        Architecture, ArtifactDigest, PackageFormat, PackageName, PackageVersion,
    };
    use crate::state::NewStoreObject;
    use tempfile::tempdir;

    #[test]
    fn scan_only_reports_unreferenced_known_object() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();
        let name = PackageName::new("orphan").unwrap();
        let version = PackageVersion::new("1");
        let digest = ArtifactDigest::sha256("abcdef0123456789");
        let path = layout.store_object_dir("abcdef012345-orphan-1");
        std::fs::create_dir_all(&path).unwrap();
        db.record_store_object(&NewStoreObject {
            store_id: "abcdef012345-orphan-1",
            name: &name,
            version: &version,
            architecture: &Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: &digest,
            store_path: &path,
            files: &[],
        })
        .unwrap();

        let report = GarbageCollector::scan(&layout, &db).unwrap();
        assert_eq!(report.candidates.len(), 1);
        assert!(!report.collected);
    }

    #[test]
    fn collect_removes_only_the_unreferenced_recorded_object() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();
        let name = PackageName::new("orphan").unwrap();
        let version = PackageVersion::new("1");
        let digest = ArtifactDigest::sha256("abcdef0123456789");
        let path = layout.store_object_dir("abcdef012345-orphan-1");
        std::fs::create_dir_all(&path).unwrap();
        db.record_store_object(&NewStoreObject {
            store_id: "abcdef012345-orphan-1",
            name: &name,
            version: &version,
            architecture: &Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: &digest,
            store_path: &path,
            files: &[],
        })
        .unwrap();
        let foreign = layout.store_dir().join("unregistered");
        std::fs::create_dir_all(&foreign).unwrap();

        let report = GarbageCollector::collect(&layout, &db).unwrap();
        assert_eq!(report.candidates.len(), 1);
        assert!(report.collected);
        assert!(!path.exists());
        assert!(foreign.exists());
        assert!(db.list_store_objects().unwrap().is_empty());
    }

    #[test]
    fn runtime_manifest_keeps_store_root_reachable() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();
        let name = PackageName::new("runtime-root").unwrap();
        let version = PackageVersion::new("1");
        let digest = ArtifactDigest::sha256("1111111111111111");
        let path = layout.store_object_dir("runtime-root-object");
        std::fs::create_dir_all(&path).unwrap();
        db.record_store_object(&NewStoreObject {
            store_id: "runtime-root-object",
            name: &name,
            version: &version,
            architecture: &Architecture::X86_64,
            format: PackageFormat::Deb,
            digest: &digest,
            store_path: &path,
            files: &[],
        })
        .unwrap();
        let manifest = crate::domain::contracts::RuntimeManifest {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            verified: true,
            runtime_id: "sha256:runtime-root".into(),
            execution: crate::domain::contracts::ExecutionPlan {
                command: "tool".into(),
                executable: "usr/bin/tool".into(),
                payload_digest: None,
                interpreter: None,
                interpreter_args: Vec::new(),
                argv: Vec::new(),
                environment: std::collections::BTreeMap::new(),
                strategy: crate::domain::contracts::LaunchStrategy::NativeRunner,
                providers: Vec::new(),
                closure: Vec::new(),
                adaptations: Vec::new(),
                optional_omissions: Vec::new(),
            },
            library_view: std::collections::BTreeMap::new(),
            module_roots: Vec::new(),
            runner_version: "test".into(),
            runner_digest: None,
            runner_target: None,
            host_facts: std::collections::BTreeMap::new(),
            references: vec![path.to_string_lossy().into_owned()],
        };
        let manifest_path = layout.write_runtime_manifest(&manifest).unwrap();
        db.record_runtime_manifest(
            &manifest.runtime_id,
            &manifest_path,
            &crate::domain::contracts::digest_serialized(&manifest),
        )
        .unwrap();
        let scan = GarbageCollector::scan(&layout, &db).unwrap();
        assert!(scan.candidates.is_empty());
        assert_eq!(scan.runtime_manifests, vec!["sha256:runtime-root"]);

        let mut tampered = manifest;
        tampered.references.clear();
        std::fs::write(&manifest_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert!(matches!(
            GarbageCollector::scan(&layout, &db),
            Err(crate::error::Error::TransactionRecoveryRequired(_))
        ));
    }
}
