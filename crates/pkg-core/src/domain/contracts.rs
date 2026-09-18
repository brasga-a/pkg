//! Versioned, serializable contracts for verified installation and execution.
//!
//! These values are deliberately separate from the vendor package metadata.  A
//! package digest authenticates the input artifact; the realization and runtime
//! manifests describe what `pkg` actually selected and generated.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::capability::Capability;
use super::package::{Architecture, ArtifactDigest, PackageFormat};

/// Schema version for on-disk contract documents.
pub const CONTRACT_SCHEMA_VERSION: u32 = 1;

/// How the input artifact was authenticated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustEvidence {
    /// Artifact came from a verified repository snapshot.
    Repository {
        repository_id: String,
        snapshot_id: String,
        metadata_verified: bool,
    },
    /// Local input.  A digest is still recorded, but it is not a repository
    /// signature and must remain distinguishable in results.
    LocalUnsigned,
}

/// Evidence about the original package artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactEvidence {
    pub schema_version: u32,
    pub source: String,
    pub format: PackageFormat,
    pub architecture: Architecture,
    pub size_bytes: u64,
    pub digest: ArtifactDigest,
    pub catalog_snapshot: Option<String>,
    pub trust: TrustEvidence,
}

impl ArtifactEvidence {
    #[must_use]
    pub fn local(source: impl Into<String>, package: &super::package::NormalizedPackage) -> Self {
        Self {
            schema_version: CONTRACT_SCHEMA_VERSION,
            source: source.into(),
            format: package.format,
            architecture: package.architecture.clone(),
            size_bytes: package.size_bytes,
            digest: package.digest.clone(),
            catalog_snapshot: None,
            trust: TrustEvidence::LocalUnsigned,
        }
    }
}

/// A normalized payload entry used by realization manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub relative_path: PathBuf,
    pub kind: String,
    pub mode: u32,
    pub size_bytes: u64,
    pub digest: Option<String>,
    pub symlink_target: Option<PathBuf>,
    pub generated_by: Option<String>,
}

/// Bounded static evidence collected for an ELF payload entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElfManifestEvidence {
    pub relative_path: PathBuf,
    pub interpreter: Option<String>,
    pub machine: u16,
    pub class_bits: u8,
    pub little_endian: bool,
    pub needed_libraries: Vec<String>,
    pub soname: Option<String>,
    pub symbol_versions: BTreeMap<String, Vec<String>>,
    pub defined_symbol_versions: Vec<String>,
}

/// Deterministic description of the realized package tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadManifest {
    pub schema_version: u32,
    pub entries: Vec<ManifestEntry>,
    pub inspection_complete: bool,
    pub tree_digest: String,
    /// Static ELF records captured while the payload was staged.
    #[serde(default)]
    pub elf_evidence: Vec<ElfManifestEvidence>,
}

impl PayloadManifest {
    /// Builds a canonical manifest by inspecting a realized tree.  The caller
    /// chooses whether an entry was generated; no executable or lifecycle
    /// script is run during inspection.
    pub fn from_tree(root: &std::path::Path) -> std::io::Result<Self> {
        let mut entries = Vec::new();
        collect_entries(root, root, &mut entries)?;
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let tree_digest = digest_serialized(&entries);
        Ok(Self {
            schema_version: CONTRACT_SCHEMA_VERSION,
            entries,
            inspection_complete: true,
            tree_digest,
            elf_evidence: Vec::new(),
        })
    }

    #[must_use]
    pub fn incomplete() -> Self {
        Self {
            schema_version: CONTRACT_SCHEMA_VERSION,
            entries: Vec::new(),
            inspection_complete: false,
            tree_digest: String::new(),
            elf_evidence: Vec::new(),
        }
    }

    /// Verifies the realized filesystem tree against this manifest while
    /// ignoring provenance annotations (`generated_by`) that describe how an
    /// entry was produced rather than its bytes or topology.
    pub fn matches_realized_tree(&self, root: &std::path::Path) -> std::io::Result<bool> {
        let actual = Self::from_tree(root)?;
        let mut expected_entries = self.entries.clone();
        for entry in &mut expected_entries {
            entry.generated_by = None;
        }
        expected_entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(expected_entries == actual.entries)
    }
}

/// A reviewed, explicit transformation recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptationPlan {
    pub recipe_id: String,
    pub recipe_version: String,
    pub preconditions: Vec<String>,
    pub inputs: Vec<PathBuf>,
    pub outputs: Vec<PathBuf>,
    pub postconditions: Vec<String>,
}

/// Evidence connecting a consumer requirement to one exact provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEvidence {
    pub requirement: String,
    pub provider_origin: String,
    pub provider_path: PathBuf,
    pub capability: Capability,
    pub soname: Option<String>,
    pub symbol_versions: Vec<String>,
    pub architecture: Architecture,
    /// Fingerprint of the exact provider bytes selected for this runtime.
    /// Older manifests may omit it; new verified manifests always record it.
    #[serde(default)]
    pub digest: Option<String>,
    pub reason: String,
}

/// Launch strategy admitted by the runtime verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchStrategy {
    Direct,
    NativeRunner,
    ExplicitHostLoader,
    InterpreterAdapter,
}

/// One command's frozen execution contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub command: String,
    pub executable: PathBuf,
    /// Digest of the realized executable or script before launch.
    #[serde(default)]
    pub payload_digest: Option<String>,
    pub interpreter: Option<PathBuf>,
    /// Arguments declared in the script's shebang, before the script path.
    /// Keeping them separate from user arguments preserves the original
    /// interpreter contract for `env -S` and flags such as Python `-Es`.
    #[serde(default)]
    pub interpreter_args: Vec<String>,
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub strategy: LaunchStrategy,
    pub providers: Vec<ProviderEvidence>,
    pub closure: Vec<String>,
    pub adaptations: Vec<AdaptationPlan>,
    pub optional_omissions: Vec<String>,
}

/// Runtime-local manifest referenced by one or more command entrypoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    pub schema_version: u32,
    /// False when an experimental install retained unresolved requirements.
    /// New runtimes must never infer verification from a missing field.
    #[serde(default)]
    pub verified: bool,
    pub runtime_id: String,
    pub execution: ExecutionPlan,
    pub library_view: BTreeMap<String, PathBuf>,
    pub module_roots: Vec<PathBuf>,
    pub runner_version: String,
    /// Digest of the static bootstrap bytes used by a native runner.
    #[serde(default)]
    pub runner_digest: Option<String>,
    /// Original payload target represented by a native runner sidecar.
    #[serde(default)]
    pub runner_target: Option<PathBuf>,
    pub host_facts: BTreeMap<String, String>,
    pub references: Vec<String>,
}

/// Complete set of commands exposed by a profile generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationGeneration {
    pub schema_version: u32,
    pub generation_id: String,
    pub profile: String,
    pub previous_generation: Option<String>,
    pub commands: BTreeMap<String, PathBuf>,
    pub runtimes: Vec<String>,
    pub owned_paths: Vec<PathBuf>,
    /// Legacy snapshots are intentionally false until every command has a
    /// verified runtime manifest.  New publications set this to true.
    #[serde(default)]
    pub verified: bool,
}

/// Durable transaction receipt used by recovery and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub schema_version: u32,
    pub transaction_id: String,
    pub operation: String,
    pub plan_identity: String,
    pub previous_generation: Option<String>,
    pub new_generation: Option<String>,
    pub created_objects: Vec<String>,
    pub reused_objects: Vec<String>,
    pub expected_digests: BTreeMap<String, String>,
    pub phase: String,
}

/// Computes a stable SHA-256 identity for a serializable contract value.
pub fn digest_serialized<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("contract serialization must be infallible");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Computes a SHA-256 digest over exact file bytes.
#[must_use]
pub fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn collect_entries(
    root: &std::path::Path,
    current: &std::path::Path,
    entries: &mut Vec<ManifestEntry>,
) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for item in std::fs::read_dir(current)? {
        let item = item?;
        let path = item.path();
        let relative_path = path
            .strip_prefix(root)
            .expect("tree entry must be under root")
            .to_path_buf();
        let metadata = std::fs::symlink_metadata(&path)?;
        let mode = metadata.permissions().mode() & 0o7777;
        if metadata.file_type().is_symlink() {
            entries.push(ManifestEntry {
                relative_path,
                kind: "symlink".into(),
                mode,
                size_bytes: 0,
                digest: None,
                symlink_target: Some(std::fs::read_link(&path)?),
                generated_by: None,
            });
        } else if metadata.is_dir() {
            entries.push(ManifestEntry {
                relative_path: relative_path.clone(),
                kind: "directory".into(),
                mode,
                size_bytes: 0,
                digest: None,
                symlink_target: None,
                generated_by: None,
            });
            collect_entries(root, &path, entries)?;
        } else if metadata.is_file() {
            let digest = ArtifactDigest::from_file(&path)
                .map(|d| d.to_string())
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            entries.push(ManifestEntry {
                relative_path,
                kind: "file".into(),
                mode,
                size_bytes: metadata.len(),
                digest: Some(digest),
                symlink_target: None,
                generated_by: None,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_digest_is_deterministic() {
        let value = BTreeMap::from([
            (String::from("b"), String::from("2")),
            (String::from("a"), String::from("1")),
        ]);
        assert_eq!(digest_serialized(&value), digest_serialized(&value));
        assert!(digest_serialized(&value).starts_with("sha256:"));
    }

    #[test]
    fn manifest_preserves_literal_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("file"), b"ok").unwrap();
        std::os::unix::fs::symlink("file", temp.path().join("link")).unwrap();
        let manifest = PayloadManifest::from_tree(temp.path()).unwrap();
        let link = manifest
            .entries
            .iter()
            .find(|e| e.relative_path == *std::path::Path::new("link"))
            .unwrap();
        assert_eq!(
            link.symlink_target.as_deref(),
            Some(std::path::Path::new("file"))
        );
    }

    #[test]
    fn manifest_verifies_realized_tree_without_provenance_annotations() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tool"), b"ok").unwrap();
        let mut manifest = PayloadManifest::from_tree(temp.path()).unwrap();
        manifest.entries[0].generated_by = Some("pkg.test.recipe".into());
        assert!(manifest.matches_realized_tree(temp.path()).unwrap());
        std::fs::write(temp.path().join("tool"), b"changed").unwrap();
        assert!(!manifest.matches_realized_tree(temp.path()).unwrap());
    }
}
