//! Normalized Constraint Intermediate Representation (ADR-009).
//!
//! Decouples source package metadata formats (Debian, RPM, ALPM) from dependency
//! resolution engines. Preserves original source expressions alongside normalized
//! constraints for diagnostics and human-readable explanation chains (INV-020).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

use crate::domain::package::{PackageName, PackageVersion};
use crate::domain::version::{VersionEcosystem, compare_versions};

/// Comparison operator for version constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VersionOp {
    Exact,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
}

impl fmt::Display for VersionOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact => write!(f, "="),
            Self::Greater => write!(f, ">"),
            Self::GreaterEqual => write!(f, ">="),
            Self::Less => write!(f, "<"),
            Self::LessEqual => write!(f, "<="),
        }
    }
}

/// Normalized version constraint requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VersionConstraint {
    /// Any version satisfies this requirement.
    Any,
    /// Must satisfy the specified relational operator against the target version.
    Relational(VersionOp, PackageVersion),
}

impl VersionConstraint {
    /// Evaluates if the candidate version string satisfies this constraint according to ecosystem rules.
    pub fn matches(&self, candidate: &str, ecosystem: VersionEcosystem) -> bool {
        match self {
            Self::Any => true,
            Self::Relational(op, target) => {
                let ord = compare_versions(candidate, target.as_str(), ecosystem);
                match op {
                    VersionOp::Exact => ord == Ordering::Equal,
                    VersionOp::Greater => ord == Ordering::Greater,
                    VersionOp::GreaterEqual => ord == Ordering::Greater || ord == Ordering::Equal,
                    VersionOp::Less => ord == Ordering::Less,
                    VersionOp::LessEqual => ord == Ordering::Less || ord == Ordering::Equal,
                }
            }
        }
    }
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "*"),
            Self::Relational(op, ver) => write!(f, "{op} {ver}"),
        }
    }
}

/// A capability requirement constraint (e.g. executable binary, SONAME, virtual feature).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityConstraint {
    /// Capability identifier (e.g., `bin:rg`, `lib:libc.so.6`, `feature:web-browser`).
    pub identifier: String,
    /// Version constraint on the capability, if specified.
    pub version: VersionConstraint,
    /// Original raw dependency expression from source package (INV-008).
    pub original_expression: String,
}

impl fmt::Display for CapabilityConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.version == VersionConstraint::Any {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{} ({})", self.identifier, self.version)
        }
    }
}

/// Normalized dependency/capability constraint IR (ADR-009).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Constraint {
    /// All nested constraints must be satisfied (AND).
    AllOf(Vec<Constraint>),
    /// At least one nested constraint must be satisfied (OR / alternative providers).
    AnyOf(Vec<Constraint>),
    /// Capability-based dependency (ADR-016).
    Capability(CapabilityConstraint),
    /// Direct package dependency (within source ecosystem).
    Package {
        name: PackageName,
        version: VersionConstraint,
        ecosystem: String,
        original_expression: String,
    },
    /// A capability or package that conflicts with this package.
    Conflict {
        target: String,
        original_expression: String,
    },
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllOf(items) => {
                let s: Vec<_> = items.iter().map(|i| i.to_string()).collect();
                write!(f, "AllOf({})", s.join(", "))
            }
            Self::AnyOf(items) => {
                let s: Vec<_> = items.iter().map(|i| i.to_string()).collect();
                write!(f, "AnyOf({})", s.join(" | "))
            }
            Self::Capability(cap) => write!(f, "{cap}"),
            Self::Package { name, version, .. } => {
                if *version == VersionConstraint::Any {
                    write!(f, "pkg:{name}")
                } else {
                    write!(f, "pkg:{name} ({version})")
                }
            }
            Self::Conflict { target, .. } => write!(f, "conflict:{target}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constraint_matching() {
        let deb_req =
            VersionConstraint::Relational(VersionOp::GreaterEqual, PackageVersion::new("1.0~rc1"));
        assert!(deb_req.matches("1.0", VersionEcosystem::Debian));
        assert!(!deb_req.matches("0.9", VersionEcosystem::Debian));

        let rpm_req =
            VersionConstraint::Relational(VersionOp::Greater, PackageVersion::new("1.0^20240101"));
        assert!(rpm_req.matches("1.0.1", VersionEcosystem::Rpm));
        assert!(!rpm_req.matches("1.0", VersionEcosystem::Rpm));
    }

    #[test]
    fn test_constraint_display() {
        let c = Constraint::AllOf(vec![
            Constraint::Capability(CapabilityConstraint {
                identifier: "bin:rg".to_string(),
                version: VersionConstraint::Any,
                original_expression: "ripgrep".to_string(),
            }),
            Constraint::Capability(CapabilityConstraint {
                identifier: "lib:libc.so.6".to_string(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.34"),
                ),
                original_expression: "libc.so.6(GLIBC_2.34)(64bit)".to_string(),
            }),
        ]);

        let str_rep = c.to_string();
        assert!(str_rep.contains("bin:rg"));
        assert!(str_rep.contains("lib:libc.so.6 (>= 2.34)"));
    }
}
