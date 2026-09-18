//! Read-only consistency diagnostics for pkg-owned state (Gate M5).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::domain::contracts::ActivationGeneration;
use crate::error::Result;
use crate::state::StateDatabase;
use crate::store::StoreLayout;

/// Severity assigned to one consistency finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FindingLevel {
    Ok,
    Warn,
    Error,
    Recoverable,
    ManualActionRequired,
}

/// One diagnostic emitted by [`Doctor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorFinding {
    pub level: FindingLevel,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Complete read-only consistency report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub profiles: Vec<String>,
    pub findings: Vec<DoctorFinding>,
}

impl DoctorReport {
    /// Returns true when no error, recovery, or manual-action finding exists.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| matches!(finding.level, FindingLevel::Ok | FindingLevel::Warn))
    }

    /// Highest severity in the report, useful for a stable CLI exit decision.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        if self.findings.iter().any(|finding| {
            matches!(
                finding.level,
                FindingLevel::Error | FindingLevel::ManualActionRequired
            )
        }) {
            2
        } else if self
            .findings
            .iter()
            .any(|finding| finding.level == FindingLevel::Recoverable)
        {
            3
        } else {
            0
        }
    }
}

/// Performs conservative diagnostics without repairing or deleting anything.
#[derive(Debug)]
pub struct Doctor;

impl Doctor {
    /// Inspects all profiles when `profile` is `None`, or one named profile.
    pub fn inspect(
        layout: &StoreLayout,
        db: &StateDatabase,
        profile: Option<&str>,
    ) -> Result<DoctorReport> {
        let mut findings = Vec::new();
        let profiles = collect_profiles(layout, profile)?;

        for transaction in db.list_incomplete_transactions()? {
            findings.push(DoctorFinding {
                level: FindingLevel::Recoverable,
                code: "INCOMPLETE_TRANSACTION".into(),
                message: format!(
                    "transaction '{}' remains in phase '{}' and requires startup recovery",
                    transaction.id, transaction.phase
                ),
                path: Some(layout.staging_dir(&transaction.id)),
            });
        }

        for object in db.list_store_objects()? {
            let expected = layout.store_object_dir(&object.store_id);
            if object.store_path != expected {
                findings.push(DoctorFinding {
                    level: FindingLevel::ManualActionRequired,
                    code: "STORE_PATH_MISMATCH".into(),
                    message: format!(
                        "store object '{}' points to {}, expected {}",
                        object.store_id,
                        object.store_path.display(),
                        expected.display()
                    ),
                    path: Some(object.store_path),
                });
                continue;
            }
            if let Err(error) = layout.validate_store_path(&expected) {
                findings.push(DoctorFinding {
                    level: FindingLevel::ManualActionRequired,
                    code: "STORE_PATH_INVALID".into(),
                    message: error.to_string(),
                    path: Some(expected),
                });
            } else if !expected.is_dir() {
                findings.push(DoctorFinding {
                    level: FindingLevel::Error,
                    code: "STORE_OBJECT_MISSING".into(),
                    message: format!("recorded store object '{}' is missing", object.store_id),
                    path: Some(expected),
                });
            }
        }

        if let Err(error) = db.runtime_store_references(&layout.runtimes_dir(), &layout.store_dir())
        {
            findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "RUNTIME_REFERENCE_INVALID".into(),
                message: error.to_string(),
                path: Some(layout.runtimes_dir()),
            });
        }

        for profile_name in &profiles {
            inspect_profile(layout, db, profile_name, &mut findings)?;
        }

        match crate::gc::GarbageCollector::scan(layout, db) {
            Ok(gc) if !gc.candidates.is_empty() || !gc.runtime_manifests.is_empty() => {
                findings.push(DoctorFinding {
                level: FindingLevel::Warn,
                code: "UNREACHABLE_OBJECTS".into(),
                message: format!(
                    "{} pkg-owned store object(s) and {} runtime manifest(s) are unreachable and can be reviewed with `pkg gc --dry-run`",
                    gc.candidates.len(),
                    gc.runtime_manifests.len()
                ),
                path: Some(layout.store_dir()),
                })
            }
            Ok(_) => {}
            Err(error) => findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "GC_SCAN_FAILED".into(),
                message: error.to_string(),
                path: Some(layout.store_dir()),
            }),
        }

        if findings.is_empty() {
            findings.push(DoctorFinding {
                level: FindingLevel::Ok,
                code: "HEALTHY".into(),
                message: "pkg state is internally consistent for the selected scope".into(),
                path: None,
            });
        }

        Ok(DoctorReport { profiles, findings })
    }
}

fn collect_profiles(layout: &StoreLayout, selected: Option<&str>) -> Result<Vec<String>> {
    if let Some(profile) = selected {
        StoreLayout::validate_profile(profile)?;
        return Ok(vec![profile.to_string()]);
    }
    let mut profiles = BTreeSet::new();
    let entries = match fs::read_dir(layout.profiles_root()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let profile = entry.file_name().to_string_lossy().into_owned();
            StoreLayout::validate_profile(&profile)?;
            profiles.insert(profile);
        }
    }
    Ok(profiles.into_iter().collect())
}

fn inspect_profile(
    layout: &StoreLayout,
    db: &StateDatabase,
    profile: &str,
    findings: &mut Vec<DoctorFinding>,
) -> Result<()> {
    let profile_dir = layout.profile_dir(profile);
    if !profile_dir.exists() {
        if db.active_generation(profile)?.is_some() || !db.list_packages(profile)?.is_empty() {
            findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "PROFILE_MISSING".into(),
                message: format!("profile '{profile}' has database state but no directory"),
                path: Some(profile_dir),
            });
        }
        return Ok(());
    }

    let active = db.active_generation(profile)?;
    let current = layout.profile_current_path(profile);
    match (active.as_ref(), fs::read_link(&current)) {
        (Some(active), Ok(pointer)) => {
            if pointer.file_name().and_then(|name| name.to_str())
                != Some(active.generation_id.as_str())
            {
                findings.push(DoctorFinding {
                    level: FindingLevel::Error,
                    code: "ACTIVE_GENERATION_DIVERGENCE".into(),
                    message: format!(
                        "profile '{profile}' current pointer does not match database generation '{}'",
                        active.generation_id
                    ),
                    path: Some(current.clone()),
                });
            }
            inspect_generation(layout, profile, &active.generation_id, findings)?;
        }
        (Some(active), Err(_)) => findings.push(DoctorFinding {
            level: FindingLevel::Error,
            code: "CURRENT_POINTER_MISSING".into(),
            message: format!(
                "profile '{profile}' has active generation '{}' but no current pointer",
                active.generation_id
            ),
            path: Some(current.clone()),
        }),
        (None, Ok(_)) => findings.push(DoctorFinding {
            level: FindingLevel::ManualActionRequired,
            code: "ORPHAN_CURRENT_POINTER".into(),
            message: format!("profile '{profile}' has a current pointer without database state"),
            path: Some(current.clone()),
        }),
        (None, Err(_)) => {}
    }

    let bin = layout.profile_bin_link(profile);
    if let Ok(metadata) = fs::symlink_metadata(&bin) {
        if metadata.file_type().is_symlink() {
            match fs::read_dir(&bin) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry?;
                        let entry_path = entry.path();
                        let entry_metadata = fs::symlink_metadata(&entry_path)?;
                        if entry_metadata.file_type().is_symlink() {
                            let target = fs::read_link(&entry_path)?;
                            let resolved = if target.is_absolute() {
                                target
                            } else {
                                entry_path.parent().unwrap_or(&bin).join(target)
                            };
                            if !resolved.exists() {
                                findings.push(DoctorFinding {
                                    level: FindingLevel::Error,
                                    code: "DANGLING_ACTIVATION".into(),
                                    message: format!(
                                        "profile command link is dangling: {}",
                                        entry_path.display()
                                    ),
                                    path: Some(entry_path),
                                });
                            }
                        } else if entry_metadata.is_file() {
                            findings.push(DoctorFinding {
                                level: FindingLevel::ManualActionRequired,
                                code: "UNMANAGED_PROFILE_COMMAND".into(),
                                message: format!(
                                    "profile bin contains a regular file: {}",
                                    entry_path.display()
                                ),
                                path: Some(entry_path),
                            });
                        }
                    }
                }
                Err(error) => findings.push(DoctorFinding {
                    level: FindingLevel::Error,
                    code: "PROFILE_BIN_UNREADABLE".into(),
                    message: error.to_string(),
                    path: Some(bin.clone()),
                }),
            }
        } else if metadata.is_dir() {
            findings.push(DoctorFinding {
                level: FindingLevel::Warn,
                code: "LEGACY_PROFILE_BIN".into(),
                message: format!("profile '{profile}' still exposes a legacy bin directory"),
                path: Some(bin),
            });
        }
    }

    for integration in db.list_integrations(profile, None)? {
        if crate::domain::integration::IntegrationKind::parse(&integration.kind).is_err() {
            findings.push(DoctorFinding {
                level: FindingLevel::ManualActionRequired,
                code: "INTEGRATION_KIND_INVALID".into(),
                message: format!(
                    "integration record has unknown kind '{}': {}",
                    integration.kind,
                    integration.target_path.display()
                ),
                path: Some(integration.target_path.clone()),
            });
            continue;
        }
        if !integration.source_path.is_file() {
            findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "INTEGRATION_SOURCE_MISSING".into(),
                message: format!(
                    "integration source is missing: {}",
                    integration.source_path.display()
                ),
                path: Some(integration.source_path.clone()),
            });
        }
        match fs::symlink_metadata(&integration.target_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                match fs::read_link(&integration.target_path) {
                    Ok(target) => {
                        let resolved = if target.is_absolute() {
                            target
                        } else {
                            integration
                                .target_path
                                .parent()
                                .unwrap_or_else(|| Path::new("/"))
                                .join(target)
                        };
                        match fs::canonicalize(resolved) {
                            Ok(resolved) if resolved == integration.source_path => {}
                            Ok(resolved) => findings.push(DoctorFinding {
                                level: FindingLevel::ManualActionRequired,
                                code: "INTEGRATION_TARGET_REPLACED".into(),
                                message: format!(
                                    "integration target points to {}, expected {}",
                                    resolved.display(),
                                    integration.source_path.display()
                                ),
                                path: Some(integration.target_path.clone()),
                            }),
                            Err(error) => findings.push(DoctorFinding {
                                level: FindingLevel::Error,
                                code: "INTEGRATION_TARGET_DANGLING".into(),
                                message: error.to_string(),
                                path: Some(integration.target_path.clone()),
                            }),
                        }
                    }
                    Err(error) => findings.push(DoctorFinding {
                        level: FindingLevel::Error,
                        code: "INTEGRATION_TARGET_UNREADABLE".into(),
                        message: error.to_string(),
                        path: Some(integration.target_path.clone()),
                    }),
                }
            }
            Ok(_) => findings.push(DoctorFinding {
                level: FindingLevel::ManualActionRequired,
                code: "INTEGRATION_TARGET_REPLACED".into(),
                message: "integration target is no longer a symlink".into(),
                path: Some(integration.target_path.clone()),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                findings.push(DoctorFinding {
                    level: FindingLevel::Error,
                    code: "INTEGRATION_TARGET_MISSING".into(),
                    message: "recorded integration target is missing".into(),
                    path: Some(integration.target_path.clone()),
                })
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

fn inspect_generation(
    layout: &StoreLayout,
    profile: &str,
    generation_id: &str,
    findings: &mut Vec<DoctorFinding>,
) -> Result<()> {
    StoreLayout::validate_component(generation_id, "generation id")?;
    let generation_dir = layout.profile_generations_dir(profile).join(generation_id);
    let manifest_path = generation_dir.join("manifest.json");
    let metadata = match fs::symlink_metadata(&manifest_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "GENERATION_MANIFEST_MISSING".into(),
                message: format!("generation '{generation_id}' has no manifest"),
                path: Some(manifest_path),
            });
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        findings.push(DoctorFinding {
            level: FindingLevel::Error,
            code: "GENERATION_MANIFEST_INVALID".into(),
            message: "generation manifest is not a regular file".into(),
            path: Some(manifest_path),
        });
        return Ok(());
    }
    let generation: ActivationGeneration = match serde_json::from_slice(&fs::read(&manifest_path)?)
    {
        Ok(generation) => generation,
        Err(error) => {
            findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "GENERATION_MANIFEST_MALFORMED".into(),
                message: error.to_string(),
                path: Some(manifest_path),
            });
            return Ok(());
        }
    };
    if generation.profile != profile || generation.generation_id != generation_id {
        findings.push(DoctorFinding {
            level: FindingLevel::Error,
            code: "GENERATION_IDENTITY_MISMATCH".into(),
            message: "generation manifest identity does not match its path".into(),
            path: Some(manifest_path),
        });
    }
    for runtime_id in generation.runtimes {
        let runtime_path = layout.runtime_dir(&runtime_id).join("manifest.json");
        if !runtime_path.is_file() {
            findings.push(DoctorFinding {
                level: FindingLevel::Error,
                code: "RUNTIME_MANIFEST_MISSING".into(),
                message: format!("runtime manifest '{runtime_id}' is missing"),
                path: Some(runtime_path),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn empty_layout_reports_healthy_state() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();
        let report = Doctor::inspect(&layout, &db, None).unwrap();
        assert!(report.is_healthy());
        assert_eq!(report.findings[0].code, "HEALTHY");
    }

    #[test]
    fn incomplete_transaction_is_recoverable_finding() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();
        db.record_transaction_start("tx-doctor", "install", "Staging", "tool", None, None)
            .unwrap();
        let report = Doctor::inspect(&layout, &db, None).unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "INCOMPLETE_TRANSACTION")
        );
        assert_eq!(report.exit_code(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn broken_profile_bin_is_reported_without_aborting_diagnostics() {
        let temp = tempdir().unwrap();
        let layout = StoreLayout::new(temp.path().join("pkg"));
        layout.ensure_dirs().unwrap();
        let profile = layout.profile_dir("broken");
        fs::create_dir_all(&profile).unwrap();
        std::os::unix::fs::symlink("missing-generation/bin", profile.join("bin")).unwrap();
        let db = StateDatabase::open(&layout.db_path()).unwrap();
        let report = Doctor::inspect(&layout, &db, Some("broken")).unwrap();
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "PROFILE_BIN_UNREADABLE")
        );
    }
}
