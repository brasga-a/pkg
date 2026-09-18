//! Human-readable explanation chains for dependency resolution failures (ADR-009, INV-020).
//!
//! Provides structured diagnostic trees explaining precisely why a package closure
//! could not be satisfied, distinguishing between missing providers, version incompatibility,
//! cross-ecosystem false equivalences (INV-007), ELF/SONAME ABI mismatches (INV-009),
//! and declared package conflicts.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Reason why a dependency requirement or candidate was rejected during resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectionReason {
    /// No package or host environment provides the requested capability or package.
    MissingProvider { requirement: String },
    /// A candidate provider was located, but its version does not satisfy the constraint.
    IncompatibleVersion {
        candidate: String,
        found_version: String,
        required_constraint: String,
        ecosystem: String,
    },
    /// Nominal package-name equality across distros or packages does not imply capability satisfaction (INV-007).
    FalseEquivalence {
        candidate: String,
        candidate_ecosystem: String,
        reason: String,
    },
    /// ELF binary inspection found a missing dynamic library SONAME or ABI symbol mismatch (INV-009).
    ElfAbiMismatch {
        candidate: String,
        missing_soname: String,
        details: String,
    },
    /// Declared mutual package conflict or incompatibility.
    Conflict {
        conflicting_package: String,
        conflict_target: String,
    },
    /// Architecture incompatible with host platform.
    ArchitectureMismatch {
        candidate: String,
        package_arch: String,
        host_arch: String,
    },
    /// In an `AnyOf` alternative branch, all alternatives failed.
    AlternativesFailed(Vec<ExplanationStep>),
}

impl fmt::Display for RejectionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProvider { requirement } => {
                write!(f, "no provider found for requirement '{requirement}'")
            }
            Self::IncompatibleVersion {
                candidate,
                found_version,
                required_constraint,
                ecosystem,
            } => {
                write!(
                    f,
                    "candidate '{candidate}' version '{found_version}' ({ecosystem}) does not satisfy '{required_constraint}'"
                )
            }
            Self::FalseEquivalence {
                candidate,
                candidate_ecosystem,
                reason,
            } => {
                write!(
                    f,
                    "nominal match '{candidate}' ({candidate_ecosystem}) rejected: {reason} (INV-007)"
                )
            }
            Self::ElfAbiMismatch {
                candidate,
                missing_soname,
                details,
            } => {
                write!(
                    f,
                    "ABI mismatch in '{candidate}': missing SONAME '{missing_soname}' ({details}) (INV-009)"
                )
            }
            Self::Conflict {
                conflicting_package,
                conflict_target,
            } => {
                write!(
                    f,
                    "package '{conflicting_package}' conflicts with '{conflict_target}'"
                )
            }
            Self::ArchitectureMismatch {
                candidate,
                package_arch,
                host_arch,
            } => {
                write!(
                    f,
                    "architecture mismatch for '{candidate}': requires '{package_arch}', host is '{host_arch}'"
                )
            }
            Self::AlternativesFailed(alts) => {
                write!(f, "all {} alternatives failed", alts.len())
            }
        }
    }
}

/// A step in an explanation chain recording why a specific constraint failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplanationStep {
    /// Package or constraint that demanded this requirement.
    pub required_by: String,
    /// The constraint requirement that could not be satisfied.
    pub constraint: String,
    /// The specific cause of rejection.
    pub failure_reason: RejectionReason,
}

/// A complete, human-readable causality chain for a resolution failure (INV-020).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplanationChain {
    /// The root package or goal being resolved.
    pub root_target: String,
    /// Ordered steps from root demand down to the root-cause failure.
    pub steps: Vec<ExplanationStep>,
}

impl ExplanationChain {
    /// Creates a new explanation chain for the specified root target.
    pub fn new(root_target: impl Into<String>) -> Self {
        Self {
            root_target: root_target.into(),
            steps: Vec::new(),
        }
    }

    /// Appends a failure step to the chain.
    pub fn add_step(
        &mut self,
        required_by: impl Into<String>,
        constraint: impl Into<String>,
        failure_reason: RejectionReason,
    ) {
        self.steps.push(ExplanationStep {
            required_by: required_by.into(),
            constraint: constraint.into(),
            failure_reason,
        });
    }

    /// Tests whether any step in the chain matches a predicate on `RejectionReason`.
    pub fn has_rejection(&self, predicate: impl Fn(&RejectionReason) -> bool) -> bool {
        self.steps.iter().any(|s| match &s.failure_reason {
            RejectionReason::AlternativesFailed(alts) => {
                predicate(&s.failure_reason)
                    || alts.iter().any(|alt| predicate(&alt.failure_reason))
            }
            other => predicate(other),
        })
    }
}

impl fmt::Display for ExplanationChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Resolution failed for '{}':", self.root_target)?;
        for (i, step) in self.steps.iter().enumerate() {
            let indent = "  ".repeat(i + 1);
            writeln!(
                f,
                "{indent}--> required by '{}': constraint '{}'",
                step.required_by, step.constraint
            )?;
            match &step.failure_reason {
                RejectionReason::MissingProvider { requirement } => {
                    writeln!(
                        f,
                        "{indent}    x No provider found for requirement '{requirement}' in repositories or host"
                    )?;
                }
                RejectionReason::IncompatibleVersion {
                    candidate,
                    found_version,
                    required_constraint,
                    ecosystem,
                } => {
                    writeln!(
                        f,
                        "{indent}    x Candidate '{candidate}' ({found_version}, {ecosystem}) does not satisfy '{required_constraint}'"
                    )?;
                }
                RejectionReason::FalseEquivalence {
                    candidate,
                    candidate_ecosystem,
                    reason,
                } => {
                    writeln!(
                        f,
                        "{indent}    x Nominal candidate '{candidate}' ({candidate_ecosystem}) rejected: {reason} (INV-007)"
                    )?;
                }
                RejectionReason::ElfAbiMismatch {
                    candidate,
                    missing_soname,
                    details,
                } => {
                    writeln!(
                        f,
                        "{indent}    x ABI verification failed for '{candidate}': missing SONAME '{missing_soname}' ({details}) (INV-009)"
                    )?;
                }
                RejectionReason::Conflict {
                    conflicting_package,
                    conflict_target,
                } => {
                    writeln!(
                        f,
                        "{indent}    x Package '{conflicting_package}' conflicts with '{conflict_target}'"
                    )?;
                }
                RejectionReason::ArchitectureMismatch {
                    candidate,
                    package_arch,
                    host_arch,
                } => {
                    writeln!(
                        f,
                        "{indent}    x Architecture mismatch for '{candidate}': requires '{package_arch}', host is '{host_arch}'"
                    )?;
                }
                RejectionReason::AlternativesFailed(alts) => {
                    writeln!(f, "{indent}    x All alternative providers failed:")?;
                    for alt in alts {
                        writeln!(
                            f,
                            "{indent}      - option '{}': {}",
                            alt.constraint, alt.failure_reason
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explanation_chain_display_and_query() {
        let mut chain = ExplanationChain::new("app-1.0");
        chain.add_step(
            "app-1.0",
            "lib:libc.so.6 (>= 2.38)",
            RejectionReason::FalseEquivalence {
                candidate: "libc6".to_string(),
                candidate_ecosystem: "debian".to_string(),
                reason: "package version 2.34 does not satisfy required capability version 2.38"
                    .to_string(),
            },
        );

        let disp = chain.to_string();
        assert!(disp.contains("Resolution failed for 'app-1.0'"));
        assert!(disp.contains("INV-007"));
        assert!(chain.has_rejection(|r| matches!(r, RejectionReason::FalseEquivalence { .. })));
    }
}
