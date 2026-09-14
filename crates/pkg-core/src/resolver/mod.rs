//! Normalized capability and evidence-based dependency resolver (ADR-009, ADR-016).
//!
//! Evaluates package constraints against host evidence (linker paths, system SONAMEs,
//! kernel/libc minimums), installed store packages, and repository candidates.
//!
//! Strict invariants enforced:
//! - **INV-007:** Package-name equality across distros never proves satisfaction.
//! - **INV-008:** Native version ordering is preserved per ecosystem without SemVer coercion.
//! - **INV-009:** ELF/SONAME/ABI evidence is required; nominal package matches lacking evidence are invalidated.
//! - **INV-020:** Resolver failures return structured, human-readable explanation chains.

pub mod evidence;
pub mod explanation;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::domain::capability::Capability;
use crate::domain::constraint::{CapabilityConstraint, Constraint, VersionConstraint};
use crate::domain::package::{NormalizedPackage, PackageName};
use crate::domain::version::VersionEcosystem;
use crate::host::elf::inspect_elf;
use crate::resolver::evidence::{CapabilityEvidence, HostEvidence};
use crate::resolver::explanation::{ExplanationChain, RejectionReason};

/// Represents an unsatisfied resolution error containing an explanation chain (INV-020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionError {
    /// Diagnostic explanation chain describing the failure path and root causes.
    pub chain: ExplanationChain,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.chain)
    }
}

impl std::error::Error for ResolutionError {}

/// The output of a successful dependency and capability resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionPlan {
    /// The root package targeted for installation.
    pub root: NormalizedPackage,
    /// Additional packages required in the closure, ordered in install sequence.
    pub packages_to_install: Vec<NormalizedPackage>,
    /// Capabilities satisfied directly by verified host evidence.
    pub host_satisfied_capabilities: Vec<CapabilityEvidence>,
    /// Capabilities satisfied by existing packages in the store.
    pub installed_satisfied_capabilities: Vec<String>,
}

/// Capability and evidence-based dependency solver.
#[derive(Debug, Clone)]
pub struct Resolver {
    host: HostEvidence,
    installed_packages: HashMap<PackageName, NormalizedPackage>,
    repository_packages: HashMap<PackageName, Vec<NormalizedPackage>>,
}

impl Resolver {
    /// Creates a new resolver initialized with verified host evidence.
    pub fn new(host: HostEvidence) -> Self {
        Self {
            host,
            installed_packages: HashMap::new(),
            repository_packages: HashMap::new(),
        }
    }

    /// Registers installed packages in the local store.
    pub fn with_installed_packages(mut self, packages: Vec<NormalizedPackage>) -> Self {
        for pkg in packages {
            self.installed_packages.insert(pkg.name.clone(), pkg);
        }
        self
    }

    /// Registers candidate packages available across configured repositories.
    pub fn with_repository_packages(mut self, packages: Vec<NormalizedPackage>) -> Self {
        for pkg in packages {
            self.repository_packages
                .entry(pkg.name.clone())
                .or_default()
                .push(pkg);
        }
        self
    }

    /// Accessor for host evidence.
    pub fn host(&self) -> &HostEvidence {
        &self.host
    }

    /// Resolves the full dependency closure for `target`.
    ///
    /// Returns `Ok(ResolutionPlan)` if satisfied, or `Err(ResolutionError)` with a complete
    /// human-readable explanation chain (INV-020).
    pub fn resolve(&self, target: &NormalizedPackage) -> Result<ResolutionPlan, ResolutionError> {
        let mut chain = ExplanationChain::new(format!("{}-{}", target.name, target.version));

        // 1. Architecture check
        if !target.architecture.matches_host(&self.host.architecture) {
            chain.add_step(
                format!("{}-{}", target.name, target.version),
                format!("arch:{}", target.architecture),
                RejectionReason::ArchitectureMismatch {
                    candidate: target.name.to_string(),
                    package_arch: target.architecture.to_string(),
                    host_arch: self.host.architecture.to_string(),
                },
            );
            return Err(ResolutionError { chain });
        }

        let mut closure_packages = Vec::new();
        let mut host_satisfied = Vec::new();
        let mut installed_satisfied = Vec::new();
        let mut in_progress = HashSet::new();
        let mut resolved_names = HashSet::new();

        in_progress.insert(target.name.clone());
        resolved_names.insert(target.name.clone());

        for constraint in &target.constraints {
            self.resolve_constraint(
                target,
                constraint,
                &mut closure_packages,
                &mut host_satisfied,
                &mut installed_satisfied,
                &mut in_progress,
                &mut resolved_names,
                &mut chain,
            )?;
        }

        // Validate mutual conflicts
        self.check_conflicts(target, &closure_packages, &mut chain)?;

        Ok(ResolutionPlan {
            root: target.clone(),
            packages_to_install: closure_packages,
            host_satisfied_capabilities: host_satisfied,
            installed_satisfied_capabilities: installed_satisfied,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_constraint(
        &self,
        current_pkg: &NormalizedPackage,
        constraint: &Constraint,
        closure: &mut Vec<NormalizedPackage>,
        host_satisfied: &mut Vec<CapabilityEvidence>,
        installed_satisfied: &mut Vec<String>,
        in_progress: &mut HashSet<PackageName>,
        resolved_names: &mut HashSet<PackageName>,
        chain: &mut ExplanationChain,
    ) -> Result<(), ResolutionError> {
        let current_id = format!("{}-{}", current_pkg.name, current_pkg.version);

        match constraint {
            Constraint::AllOf(items) => {
                for item in items {
                    self.resolve_constraint(
                        current_pkg,
                        item,
                        closure,
                        host_satisfied,
                        installed_satisfied,
                        in_progress,
                        resolved_names,
                        chain,
                    )?;
                }
                Ok(())
            }
            Constraint::AnyOf(alternatives) => {
                let mut alt_failures = Vec::new();
                for alt in alternatives {
                    let mut temp_closure = closure.clone();
                    let mut temp_host = host_satisfied.clone();
                    let mut temp_installed = installed_satisfied.clone();
                    let mut temp_in_progress = in_progress.clone();
                    let mut temp_resolved = resolved_names.clone();
                    let mut temp_chain = ExplanationChain::new(&current_id);

                    if self
                        .resolve_constraint(
                            current_pkg,
                            alt,
                            &mut temp_closure,
                            &mut temp_host,
                            &mut temp_installed,
                            &mut temp_in_progress,
                            &mut temp_resolved,
                            &mut temp_chain,
                        )
                        .is_ok()
                    {
                        *closure = temp_closure;
                        *host_satisfied = temp_host;
                        *installed_satisfied = temp_installed;
                        *in_progress = temp_in_progress;
                        *resolved_names = temp_resolved;
                        return Ok(());
                    } else if let Some(last_step) = temp_chain.steps.pop() {
                        alt_failures.push(last_step);
                    }
                }

                chain.add_step(
                    &current_id,
                    format!("{constraint}"),
                    RejectionReason::AlternativesFailed(alt_failures),
                );
                Err(ResolutionError {
                    chain: chain.clone(),
                })
            }
            Constraint::Capability(cap) => self.resolve_capability(
                current_pkg,
                cap,
                closure,
                host_satisfied,
                installed_satisfied,
                in_progress,
                resolved_names,
                chain,
            ),
            Constraint::Package {
                name,
                version,
                ecosystem,
                original_expression,
            } => self.resolve_package(
                current_pkg,
                name,
                version,
                ecosystem,
                original_expression,
                closure,
                host_satisfied,
                installed_satisfied,
                in_progress,
                resolved_names,
                chain,
            ),
            Constraint::Conflict { .. } => {
                // Conflicts are verified during whole-closure conflict verification phase
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_capability(
        &self,
        current_pkg: &NormalizedPackage,
        cap: &CapabilityConstraint,
        closure: &mut Vec<NormalizedPackage>,
        host_satisfied: &mut Vec<CapabilityEvidence>,
        installed_satisfied: &mut Vec<String>,
        in_progress: &mut HashSet<PackageName>,
        resolved_names: &mut HashSet<PackageName>,
        chain: &mut ExplanationChain,
    ) -> Result<(), ResolutionError> {
        let current_id = format!("{}-{}", current_pkg.name, current_pkg.version);

        // 1. Check host evidence (ADR-016)
        if let Some(evidence) = self.host.provides_capability(&cap.identifier) {
            let version_ok = match &cap.version {
                VersionConstraint::Any => true,
                VersionConstraint::Relational(_, _) => {
                    if let Some(host_ver) = &evidence.version {
                        // Host version matched using candidate host version
                        cap.version
                            .matches(host_ver.as_str(), VersionEcosystem::Debian)
                    } else {
                        // Host has unversioned library; if exact symbol requirements exist, check them
                        true
                    }
                }
            };

            if version_ok {
                host_satisfied.push(evidence.clone());
                return Ok(());
            }

            // Host provided the capability but failed the version requirement
            let found_ver = evidence
                .version
                .as_ref()
                .map(|v| v.as_str())
                .unwrap_or("unversioned");
            chain.add_step(
                &current_id,
                cap.to_string(),
                RejectionReason::IncompatibleVersion {
                    candidate: "host:system".to_string(),
                    found_version: found_ver.to_string(),
                    required_constraint: cap.version.to_string(),
                    ecosystem: "host".to_string(),
                },
            );
            return Err(ResolutionError {
                chain: chain.clone(),
            });
        }

        // If library, check if host has the SONAME file in linker paths
        if let Some(soname) = cap.identifier.strip_prefix("lib:") {
            if self.host.has_soname(soname) && cap.version == VersionConstraint::Any {
                host_satisfied.push(CapabilityEvidence {
                    capability: Capability::SharedLibrary(soname.to_string()),
                    version: None,
                    provider_origin: "host:system".to_string(),
                    symbols: Vec::new(),
                });
                return Ok(());
            }
        }

        // 2. Check installed packages in store
        for pkg in self.installed_packages.values() {
            if self.package_provides_capability(pkg, cap) {
                installed_satisfied.push(format!("{}:{cap}", pkg.name));
                return Ok(());
            }
        }

        // 3. Check already selected closure packages
        for pkg in closure.iter() {
            if self.package_provides_capability(pkg, cap) {
                return Ok(());
            }
        }

        // 4. Search repository candidates for a provider
        for candidates in self.repository_packages.values() {
            for candidate in candidates {
                if self.package_provides_capability(candidate, cap) {
                    if resolved_names.contains(&candidate.name) {
                        return Ok(());
                    }
                    if in_progress.contains(&candidate.name) {
                        // Cycle detected
                        return Ok(());
                    }

                    in_progress.insert(candidate.name.clone());
                    for sub_c in &candidate.constraints {
                        self.resolve_constraint(
                            candidate,
                            sub_c,
                            closure,
                            host_satisfied,
                            installed_satisfied,
                            in_progress,
                            resolved_names,
                            chain,
                        )?;
                    }
                    in_progress.remove(&candidate.name);
                    resolved_names.insert(candidate.name.clone());
                    closure.push(candidate.clone());
                    return Ok(());
                }
            }
        }

        // 5. Invariant check: Reject false package-name equivalence (INV-007, INV-009)
        // If an available package shares a nominal name with the capability (e.g. `glibc` or `libc6`
        // when looking for `lib:libc.so.6`), verify if it was rejected due to missing capability / ABI.
        let target_stem = cap
            .identifier
            .split(':')
            .nth(1)
            .unwrap_or(&cap.identifier)
            .split('.')
            .next()
            .unwrap_or(&cap.identifier);

        for (pkg_name, candidates) in &self.repository_packages {
            if (pkg_name.as_str().contains(target_stem) || target_stem.contains(pkg_name.as_str()))
                && let Some(cand) = candidates.first()
            {
                // Check if it has version mismatch or lacks capability evidence
                chain.add_step(
                    &current_id,
                    cap.to_string(),
                    RejectionReason::FalseEquivalence {
                        candidate: format!("{}-{}", cand.name, cand.version),
                        candidate_ecosystem: format!("{}", cand.format),
                        reason: format!(
                            "package '{}' does not verify capability '{}' (INV-007)",
                            cand.name, cap.identifier
                        ),
                    },
                );
                return Err(ResolutionError {
                    chain: chain.clone(),
                });
            }
        }

        chain.add_step(
            &current_id,
            cap.to_string(),
            RejectionReason::MissingProvider {
                requirement: cap.to_string(),
            },
        );
        Err(ResolutionError {
            chain: chain.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_package(
        &self,
        current_pkg: &NormalizedPackage,
        name: &PackageName,
        version_constraint: &VersionConstraint,
        ecosystem: &str,
        original_expr: &str,
        closure: &mut Vec<NormalizedPackage>,
        host_satisfied: &mut Vec<CapabilityEvidence>,
        installed_satisfied: &mut Vec<String>,
        in_progress: &mut HashSet<PackageName>,
        resolved_names: &mut HashSet<PackageName>,
        chain: &mut ExplanationChain,
    ) -> Result<(), ResolutionError> {
        let current_id = format!("{}-{}", current_pkg.name, current_pkg.version);

        // Check if already in closure or resolved
        if resolved_names.contains(name) {
            return Ok(());
        }

        // Check installed packages
        if let Some(installed) = self.installed_packages.get(name) {
            let eco = match installed.format {
                crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
                crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
                crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
                _ => VersionEcosystem::Debian,
            };

            if version_constraint.matches(installed.version.as_str(), eco) {
                // Check ecosystem compatibility (INV-007)
                let installed_eco = format!("{}", installed.format);
                if !ecosystem.is_empty() && installed_eco != ecosystem {
                    chain.add_step(
                        &current_id,
                        original_expr.to_string(),
                        RejectionReason::FalseEquivalence {
                            candidate: format!("{}-{}", installed.name, installed.version),
                            candidate_ecosystem: installed_eco.clone(),
                            reason: format!(
                                "cannot satisfy '{}' ({ecosystem}) with installed package from ecosystem '{installed_eco}' without capability evidence (INV-007)",
                                name
                            ),
                        },
                    );
                    return Err(ResolutionError {
                        chain: chain.clone(),
                    });
                }

                installed_satisfied.push(format!("{}-{}", installed.name, installed.version));
                resolved_names.insert(name.clone());
                return Ok(());
            }
        }

        // Search repository candidates
        let mut last_false_equivalence = None;
        let mut last_version_mismatch = None;

        if let Some(candidates) = self.repository_packages.get(name) {
            for cand in candidates {
                let cand_eco = match cand.format {
                    crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
                    crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
                    crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
                    _ => VersionEcosystem::Debian,
                };

                let format_str = format!("{}", cand.format);
                if !ecosystem.is_empty() && format_str != ecosystem {
                    last_false_equivalence = Some(RejectionReason::FalseEquivalence {
                        candidate: format!("{}-{}", cand.name, cand.version),
                        candidate_ecosystem: format_str,
                        reason: "nominal match across distributions rejected without capability evidence (INV-007)".to_string(),
                    });
                    continue;
                }

                if !version_constraint.matches(cand.version.as_str(), cand_eco) {
                    last_version_mismatch = Some(RejectionReason::IncompatibleVersion {
                        candidate: cand.name.to_string(),
                        found_version: cand.version.to_string(),
                        required_constraint: version_constraint.to_string(),
                        ecosystem: format_str,
                    });
                    continue;
                }

                if in_progress.contains(&cand.name) {
                    return Ok(());
                }

                in_progress.insert(cand.name.clone());
                for sub_c in &cand.constraints {
                    self.resolve_constraint(
                        cand,
                        sub_c,
                        closure,
                        host_satisfied,
                        installed_satisfied,
                        in_progress,
                        resolved_names,
                        chain,
                    )?;
                }
                in_progress.remove(&cand.name);
                resolved_names.insert(cand.name.clone());
                closure.push(cand.clone());
                return Ok(());
            }

            if let Some(reason) = last_false_equivalence {
                chain.add_step(&current_id, original_expr.to_string(), reason);
                return Err(ResolutionError {
                    chain: chain.clone(),
                });
            }
            if let Some(reason) = last_version_mismatch {
                chain.add_step(&current_id, original_expr.to_string(), reason);
                return Err(ResolutionError {
                    chain: chain.clone(),
                });
            }
        }

        chain.add_step(
            &current_id,
            original_expr.to_string(),
            RejectionReason::MissingProvider {
                requirement: format!("package '{name}' ({version_constraint})"),
            },
        );
        Err(ResolutionError {
            chain: chain.clone(),
        })
    }

    fn package_provides_capability(
        &self,
        pkg: &NormalizedPackage,
        cap: &CapabilityConstraint,
    ) -> bool {
        for p in &pkg.provides {
            let matches_ident = match p {
                Capability::SharedLibrary(so) => {
                    cap.identifier == format!("lib:{so}") || cap.identifier == *so
                }
                Capability::Executable(cmd) => {
                    cap.identifier == format!("bin:{cmd}") || cap.identifier == *cmd
                }
                Capability::Feature(feat) => {
                    cap.identifier == format!("feature:{feat}") || cap.identifier == *feat
                }
            };

            if matches_ident {
                let eco = match pkg.format {
                    crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
                    crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
                    crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
                    _ => VersionEcosystem::Debian,
                };
                if cap.version.matches(pkg.version.as_str(), eco) {
                    return true;
                }
            }
        }
        false
    }

    fn check_conflicts(
        &self,
        root: &NormalizedPackage,
        closure: &[NormalizedPackage],
        chain: &mut ExplanationChain,
    ) -> Result<(), ResolutionError> {
        let mut all_packages = vec![root];
        all_packages.extend(closure.iter());

        for pkg in &all_packages {
            for constraint in &pkg.constraints {
                if let Constraint::Conflict {
                    target,
                    original_expression,
                } = constraint
                {
                    // Check if any other package in closure or installed matches target
                    for other in &all_packages {
                        if other.name.as_str() == target.as_str()
                            || other.provides.iter().any(|p| match p {
                                Capability::Feature(f) => f == target,
                                _ => false,
                            })
                        {
                            chain.add_step(
                                format!("{}-{}", pkg.name, pkg.version),
                                original_expression.clone(),
                                RejectionReason::Conflict {
                                    conflicting_package: pkg.name.to_string(),
                                    conflict_target: target.clone(),
                                },
                            );
                            return Err(ResolutionError {
                                chain: chain.clone(),
                            });
                        }
                    }

                    for (inst_name, inst_pkg) in &self.installed_packages {
                        if inst_name.as_str() == target.as_str()
                            || inst_pkg.provides.iter().any(|p| match p {
                                Capability::Feature(f) => f == target,
                                _ => false,
                            })
                        {
                            chain.add_step(
                                format!("{}-{}", pkg.name, pkg.version),
                                original_expression.clone(),
                                RejectionReason::Conflict {
                                    conflicting_package: pkg.name.to_string(),
                                    conflict_target: target.clone(),
                                },
                            );
                            return Err(ResolutionError {
                                chain: chain.clone(),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Verifies static ELF binary compatibility against host evidence and package closure (INV-009).
    ///
    /// Validates `DT_NEEDED` dynamic libraries using binary inspection rather than `ldd`.
    /// Rejects any binary whose required libraries are absent from host standard paths
    /// and package closure.
    pub fn validate_elf_evidence(
        &self,
        elf_path: &Path,
        closure: &[NormalizedPackage],
        staging_root: Option<&Path>,
    ) -> Result<(), ResolutionError> {
        let inspection = match inspect_elf(elf_path, staging_root) {
            Ok(Some(i)) => i,
            Ok(None) => return Ok(()), // Not an ELF binary
            Err(e) => {
                let mut chain = ExplanationChain::new(elf_path.display().to_string());
                chain.add_step(
                    elf_path.display().to_string(),
                    "elf:binary-inspection",
                    RejectionReason::ElfAbiMismatch {
                        candidate: elf_path.display().to_string(),
                        missing_soname: "unparseable".to_string(),
                        details: e.to_string(),
                    },
                );
                return Err(ResolutionError { chain });
            }
        };

        for missing in inspection.missing_libraries {
            // Check if any package in closure provides this missing SONAME
            let provided_in_closure = closure.iter().any(|p| {
                p.provides.iter().any(|c| match c {
                    Capability::SharedLibrary(s) => s == &missing,
                    _ => false,
                })
            });

            if !provided_in_closure && !self.host.has_soname(&missing) {
                let mut chain = ExplanationChain::new(elf_path.display().to_string());
                chain.add_step(
                    elf_path.display().to_string(),
                    format!("DT_NEEDED:{missing}"),
                    RejectionReason::ElfAbiMismatch {
                        candidate: elf_path.display().to_string(),
                        missing_soname: missing.clone(),
                        details: format!(
                            "SONAME '{missing}' not found in host linker paths or package closure (INV-009)"
                        ),
                    },
                );
                return Err(ResolutionError { chain });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::constraint::VersionOp;
    use crate::domain::package::{Architecture, ArtifactDigest, PackageFormat, PackageVersion};

    fn make_pkg(
        name: &str,
        version: &str,
        format: PackageFormat,
        constraints: Vec<Constraint>,
        provides: Vec<Capability>,
    ) -> NormalizedPackage {
        NormalizedPackage {
            name: PackageName::new(name).unwrap(),
            version: PackageVersion::new(version),
            architecture: Architecture::X86_64,
            format,
            digest: ArtifactDigest::sha256("00".repeat(32)),
            size_bytes: 1024,
            description: None,
            dependencies: Vec::new(),
            constraints,
            provides,
            scripts: Vec::new(),
            entries: Vec::new(),
            installed_size: None,
        }
    }

    #[test]
    fn test_resolve_satisfied_by_host() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", Some("2.38"), &["GLIBC_2.38"])
            .build();

        let resolver = Resolver::new(host);

        let pkg = make_pkg(
            "my-tool",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Capability(CapabilityConstraint {
                identifier: "lib:libc.so.6".to_string(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.34"),
                ),
                original_expression: "libc6 (>= 2.34)".to_string(),
            })],
            vec![],
        );

        let plan = resolver.resolve(&pkg).unwrap();
        assert_eq!(plan.host_satisfied_capabilities.len(), 1);
        assert_eq!(plan.packages_to_install.len(), 0);
    }

    #[test]
    fn test_resolve_rejects_false_equivalence() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();

        // Repository has glibc (RPM), but target demands Debian package
        let glibc_rpm = make_pkg(
            "glibc",
            "2.38-1.fc40",
            PackageFormat::Rpm,
            vec![],
            vec![Capability::SharedLibrary("libc.so.6".to_string())],
        );

        let resolver = Resolver::new(host).with_repository_packages(vec![glibc_rpm]);

        // Target Debian package demands debian package "glibc"
        let pkg = make_pkg(
            "deb-tool",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("glibc").unwrap(),
                version: VersionConstraint::Any,
                ecosystem: "debian".to_string(),
                original_expression: "glibc".to_string(),
            }],
            vec![],
        );

        let err = resolver.resolve(&pkg).unwrap_err();
        assert!(
            err.chain
                .has_rejection(|r| matches!(r, RejectionReason::FalseEquivalence { .. }))
        );
        assert!(err.to_string().contains("INV-007"));
    }

    #[test]
    fn test_resolve_incompatible_version_explanation() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", Some("2.34"), &[])
            .build();

        let resolver = Resolver::new(host);

        let pkg = make_pkg(
            "fedora-tool",
            "1.0.0",
            PackageFormat::Rpm,
            vec![Constraint::Capability(CapabilityConstraint {
                identifier: "lib:libc.so.6".to_string(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.38"),
                ),
                original_expression: "libc.so.6(GLIBC_2.38)".to_string(),
            })],
            vec![],
        );

        let err = resolver.resolve(&pkg).unwrap_err();
        assert!(
            err.chain
                .has_rejection(|r| matches!(r, RejectionReason::IncompatibleVersion { .. }))
        );
        assert!(err.to_string().contains("does not satisfy '>= 2.38'"));
    }

    #[test]
    fn test_resolve_mutual_conflict() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();

        let installed = make_pkg(
            "ripgrep-legacy",
            "12.0.0",
            PackageFormat::Deb,
            vec![],
            vec![],
        );

        let resolver = Resolver::new(host).with_installed_packages(vec![installed]);

        let target = make_pkg(
            "ripgrep",
            "14.0.0",
            PackageFormat::Deb,
            vec![Constraint::Conflict {
                target: "ripgrep-legacy".to_string(),
                original_expression: "Conflicts: ripgrep-legacy".to_string(),
            }],
            vec![],
        );

        let err = resolver.resolve(&target).unwrap_err();
        assert!(
            err.chain
                .has_rejection(|r| matches!(r, RejectionReason::Conflict { .. }))
        );
    }
}
