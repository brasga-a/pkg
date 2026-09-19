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

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use crate::domain::capability::Capability;
use crate::domain::constraint::{CapabilityConstraint, Constraint, VersionConstraint};
use crate::domain::package::{NormalizedPackage, PackageName};
use crate::domain::version::VersionEcosystem;
use crate::host::elf::inspect_elf;
use crate::resolver::evidence::{
    CapabilityEvidence, HostEvidence, canonical_commands_for_package,
    canonical_package_equivalents, canonical_sonames_for_package,
};
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
    replaced_packages: HashSet<PackageName>,
}

impl Resolver {
    /// Creates a new resolver initialized with verified host evidence.
    pub fn new(host: HostEvidence) -> Self {
        Self {
            host,
            installed_packages: HashMap::new(),
            repository_packages: HashMap::new(),
            replaced_packages: HashSet::new(),
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

    /// Marks installed package names that will be replaced by this
    /// transaction. Their old records must not satisfy dependencies or
    /// create conflicts while the replacement closure is planned.
    pub fn with_replaced_packages<I>(mut self, packages: I) -> Self
    where
        I: IntoIterator<Item = PackageName>,
    {
        self.replaced_packages.extend(packages);
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
                    evidence.version.as_ref().is_some_and(|host_ver| {
                        cap.version
                            .matches(host_ver.as_str(), VersionEcosystem::Debian)
                    })
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

        // 2. Check installed packages in store. HashMap iteration order is not
        // stable, so sort candidates before selecting one. This keeps a
        // capability resolution reproducible across processes and hosts.
        let mut installed_candidates: Vec<&NormalizedPackage> = self
            .installed_packages
            .iter()
            .filter(|(name, _)| !self.replaced_packages.contains(*name))
            .map(|(_, package)| package)
            .collect();
        installed_candidates.sort_by(|a, b| package_candidate_cmp(a, b));
        for pkg in installed_candidates {
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

        // 4. Search repository candidates for a provider. Flattening the map
        // before sorting avoids depending on HashMap bucket order when multiple
        // repositories provide the same capability.
        let mut provider_candidates: Vec<&NormalizedPackage> = self
            .repository_packages
            .values()
            .flat_map(|candidates| candidates.iter())
            .collect();
        provider_candidates.sort_by(|a, b| package_candidate_cmp(a, b));
        for candidate in provider_candidates {
            if !candidate.architecture.matches_host(&self.host.architecture) {
                continue;
            }
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

        // Interpreter adapters may intentionally use a host interpreter. This
        // is capability evidence, not a general package-name equivalence: only
        // the reviewed interpreter names and unconstrained requirements qualify.
        if *version_constraint == VersionConstraint::Any
            && matches!(
                name.as_str(),
                "python" | "python3" | "perl" | "ruby" | "node"
            )
        {
            let interpreter_commands: &[&str] = match name.as_str() {
                "python" | "python3" => &["python", "python3"],
                other => &[other],
            };
            if interpreter_commands.iter().any(|command| {
                self.host
                    .provides_capability(&format!("bin:{command}"))
                    .is_some()
            }) {
                installed_satisfied.push(format!("host:interpreter:{name}"));
                resolved_names.insert(name.clone());
                return Ok(());
            }
        }

        // Check host evidence for virtual feature capability (e.g. gsettings-backend, dbus-session-bus)
        if let Some(evidence) = self.host.provides_feature(name.as_str()) {
            let version_ok = match version_constraint {
                VersionConstraint::Any => true,
                VersionConstraint::Relational(_, _) => {
                    evidence.version.as_ref().is_some_and(|host_ver| {
                        version_constraint.matches(host_ver.as_str(), VersionEcosystem::Debian)
                    })
                }
            };
            if version_ok {
                host_satisfied.push(evidence.clone());
                resolved_names.insert(name.clone());
                return Ok(());
            }
        }

        // Check host evidence for command/tool packages (e.g. xdg-utils, xz-utils)
        if let Some(commands) = canonical_commands_for_package(name.as_str()) {
            if commands.iter().any(|cmd| {
                self.host
                    .provides_capability(&format!("bin:{cmd}"))
                    .is_some()
            }) {
                host_satisfied.push(CapabilityEvidence {
                    capability: Capability::Feature(name.to_string()),
                    version: None,
                    provider_origin: format!("host:tool:{name}"),
                    symbols: Vec::new(),
                });
                resolved_names.insert(name.clone());
                return Ok(());
            }
        }

        // Check host native packages matching ecosystem
        if let Some(host_pkg) =
            self.host
                .provides_package(name.as_str(), version_constraint, ecosystem)
        {
            host_satisfied.push(CapabilityEvidence {
                capability: Capability::Feature(name.to_string()),
                version: Some(host_pkg.version.clone()),
                provider_origin: format!("host:pkg:{}", host_pkg.name),
                symbols: Vec::new(),
            });
            resolved_names.insert(name.clone());
            return Ok(());
        }

        // Check host evidence for shared library SONAMEs (ADR-016, INV-009)
        let candidate_sonames = canonical_sonames_for_package(name.as_str());
        for soname in &candidate_sonames {
            if self.host.has_soname(soname) {
                // If version-constrained, verify against host capability or equivalent host packages
                let version_ok = match version_constraint {
                    VersionConstraint::Any => true,
                    VersionConstraint::Relational(_, _) => {
                        let host_cap = self.host.provides_capability(&format!("lib:{soname}"));
                        if let Some(host_ver) = host_cap.and_then(|c| c.version.as_ref()) {
                            version_constraint.matches(host_ver.as_str(), VersionEcosystem::Debian)
                        } else {
                            // Check equivalent host packages for version evidence
                            let equiv_names = canonical_package_equivalents(name.as_str());
                            equiv_names.iter().any(|equiv| {
                                self.host.host_packages.get(*equiv).is_some_and(|pkg| {
                                    let eco = match pkg.ecosystem.as_str() {
                                        "debian" | "ubuntu" => VersionEcosystem::Debian,
                                        "rpm" | "fedora" | "rhel" | "suse" | "centos" => {
                                            VersionEcosystem::Rpm
                                        }
                                        "alpm" | "arch" => VersionEcosystem::Alpm,
                                        _ => VersionEcosystem::Debian,
                                    };
                                    version_constraint.matches(pkg.version.as_str(), eco)
                                })
                            })
                        }
                    }
                };

                if version_ok {
                    let inferred_ver = self
                        .host
                        .provides_capability(&format!("lib:{soname}"))
                        .and_then(|c| c.version.clone());
                    host_satisfied.push(CapabilityEvidence {
                        capability: Capability::SharedLibrary(soname.clone()),
                        version: inferred_ver,
                        provider_origin: format!("host:lib:{name}"),
                        symbols: Vec::new(),
                    });
                    resolved_names.insert(name.clone());
                    return Ok(());
                }
            }
        }

        // Check host native packages matching equivalent names across ecosystems (ADR-016, INV-007)
        let equiv_names = canonical_package_equivalents(name.as_str());
        for equiv in equiv_names {
            if let Some(host_pkg) = self.host.host_packages.get(*equiv) {
                let eco = match host_pkg.ecosystem.as_str() {
                    "debian" | "ubuntu" => VersionEcosystem::Debian,
                    "rpm" | "fedora" | "rhel" | "suse" | "centos" => VersionEcosystem::Rpm,
                    "alpm" | "arch" => VersionEcosystem::Alpm,
                    _ => VersionEcosystem::Debian,
                };
                if version_constraint.matches(host_pkg.version.as_str(), eco) {
                    host_satisfied.push(CapabilityEvidence {
                        capability: Capability::Feature(name.to_string()),
                        version: Some(host_pkg.version.clone()),
                        provider_origin: format!("host:pkg:{}", host_pkg.name),
                        symbols: Vec::new(),
                    });
                    resolved_names.insert(name.clone());
                    return Ok(());
                }
            }
        }

        // Check installed packages (direct package match)
        if let Some(installed) = self.installed_packages.get(name)
            && !self.replaced_packages.contains(name)
        {
            let eco = match installed.format {
                crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
                crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
                crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
                _ => VersionEcosystem::Debian,
            };

            if version_constraint.matches(installed.version.as_str(), eco) {
                // Check ecosystem compatibility (INV-007)
                let installed_eco = package_ecosystem(installed.format);
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

        // Check installed packages (virtual capability provider)
        let cap_feature = CapabilityConstraint {
            identifier: format!("feature:{name}"),
            version: version_constraint.clone(),
            original_expression: original_expr.to_string(),
        };
        for installed in self.installed_packages.values() {
            if self.replaced_packages.contains(&installed.name) {
                continue;
            }
            if self.package_provides_capability(installed, &cap_feature) {
                installed_satisfied.push(format!("{}:{name}", installed.name));
                resolved_names.insert(name.clone());
                return Ok(());
            }
        }

        // Check already selected closure packages
        for pkg in closure.iter() {
            if self.package_provides_capability(pkg, &cap_feature) {
                resolved_names.insert(name.clone());
                return Ok(());
            }
        }

        // Search repository candidates (direct name match)
        let mut last_false_equivalence = None;
        let mut last_arch_mismatch = None;
        let mut last_version_mismatch = None;

        if let Some(candidates) = self.repository_packages.get(name) {
            let mut ordered_candidates: Vec<&NormalizedPackage> = candidates.iter().collect();
            ordered_candidates.sort_by(|a, b| package_candidate_cmp(a, b));
            for cand in ordered_candidates {
                if !cand.architecture.matches_host(&self.host.architecture) {
                    last_arch_mismatch = Some(RejectionReason::ArchitectureMismatch {
                        candidate: format!("{}-{}", cand.name, cand.version),
                        package_arch: cand.architecture.to_string(),
                        host_arch: self.host.architecture.to_string(),
                    });
                    continue;
                }
                let cand_eco = match cand.format {
                    crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
                    crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
                    crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
                    _ => VersionEcosystem::Debian,
                };

                let format_str = package_ecosystem(cand.format);
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
        }

        // Search repository candidates for a virtual package/capability provider (Provides: <name>)
        let mut virtual_candidates: Vec<&NormalizedPackage> = self
            .repository_packages
            .values()
            .flat_map(|candidates| candidates.iter())
            .filter(|cand| {
                if !cand.architecture.matches_host(&self.host.architecture) {
                    return false;
                }
                let format_str = package_ecosystem(cand.format);
                if !ecosystem.is_empty() && format_str != ecosystem {
                    return false;
                }
                self.package_provides_capability(cand, &cap_feature)
            })
            .collect();
        virtual_candidates.sort_by(|a, b| package_candidate_cmp(a, b));

        for cand in virtual_candidates {
            if resolved_names.contains(&cand.name) {
                resolved_names.insert(name.clone());
                return Ok(());
            }
            if in_progress.contains(&cand.name) {
                return Ok(());
            }

            in_progress.insert(cand.name.clone());
            let mut sub_chain = chain.clone();
            let mut sub_ok = true;
            for sub_c in &cand.constraints {
                if let Err(_e) = self.resolve_constraint(
                    cand,
                    sub_c,
                    closure,
                    host_satisfied,
                    installed_satisfied,
                    in_progress,
                    resolved_names,
                    &mut sub_chain,
                ) {
                    sub_ok = false;
                    break;
                }
            }
            in_progress.remove(&cand.name);

            if sub_ok {
                resolved_names.insert(cand.name.clone());
                resolved_names.insert(name.clone());
                closure.push(cand.clone());
                return Ok(());
            }
        }

        if let Some(reason) = last_false_equivalence {
            chain.add_step(&current_id, original_expr.to_string(), reason);
            return Err(ResolutionError {
                chain: chain.clone(),
            });
        }
        if let Some(reason) = last_arch_mismatch {
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
        let mut provided = pkg
            .provides
            .iter()
            .map(|capability| (capability, None))
            .collect::<Vec<_>>();
        provided.extend(
            pkg.versioned_provides
                .iter()
                .map(|entry| (&entry.capability, Some(entry.version.as_str()))),
        );
        for (p, provided_version) in provided {
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
                match (provided_version, &cap.version) {
                    // An unversioned Provides entry proves only the existence
                    // of the capability.  The package's own version is not a
                    // version of the virtual capability and must not satisfy
                    // a relational requirement accidentally.
                    (None, VersionConstraint::Any) => return true,
                    (Some(version), constraint) if constraint.matches(version, eco) => return true,
                    _ => {}
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
        let selected_package_names: HashSet<_> = all_packages
            .iter()
            .map(|package| package.name.clone())
            .collect();

        for pkg in &all_packages {
            for constraint in &pkg.constraints {
                if let Constraint::Conflict {
                    target,
                    version,
                    original_expression,
                } = constraint
                {
                    // Check if any other package in closure or installed matches target
                    for other in &all_packages {
                        // A package's own metadata may mention its name (for
                        // example a generated conflict expression).  That is
                        // not a mutual conflict with another selected
                        // package; only compare against a distinct node in
                        // the planned closure.
                        if std::ptr::eq(*other, *pkg) {
                            continue;
                        }
                        if Self::conflict_matches(pkg, target, version, other) {
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

                    for inst_pkg in self.installed_packages.values() {
                        // Replaced records and any installed package selected
                        // in this closure belong to the old generation. They
                        // must not conflict with their incoming replacement.
                        // This matters for a targeted upgrade whose required
                        // dependency is itself upgraded in the same closure.
                        if self.replaced_packages.contains(&inst_pkg.name)
                            || selected_package_names.contains(&inst_pkg.name)
                        {
                            continue;
                        }
                        if Self::conflict_matches(pkg, target, version, inst_pkg) {
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

    fn conflict_matches(
        conflicting_package: &NormalizedPackage,
        target: &str,
        version: &VersionConstraint,
        other: &NormalizedPackage,
    ) -> bool {
        let package_name_matches = other.name.as_str() == target;
        let feature_matches = other.provides.iter().any(|provided| match provided {
            Capability::Feature(feature) => feature == target,
            _ => false,
        });
        if !package_name_matches && !feature_matches {
            return false;
        }

        let ecosystem = match conflicting_package.format {
            crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
            crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
            crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
            crate::domain::package::PackageFormat::Tarball => VersionEcosystem::Debian,
        };

        if package_name_matches {
            return version.matches(other.version.as_str(), ecosystem);
        }
        if *version == VersionConstraint::Any {
            return true;
        }

        // A versioned conflict against a virtual capability requires explicit
        // provider-version evidence. The package's own version is not a
        // version of an unrelated virtual capability.
        other.versioned_provides.iter().any(|provided| {
            matches!(&provided.capability, Capability::Feature(feature) if feature == target)
                && version.matches(provided.version.as_str(), ecosystem)
        })
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
                }) || p.versioned_provides.iter().any(|entry| {
                    matches!(&entry.capability, Capability::SharedLibrary(s) if s == &missing)
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

/// Maps the normalized format name to the dependency ecosystem vocabulary
/// retained by each adapter. Debian's source metadata uses `debian`, while the
/// artifact enum displays the shorter `deb` label.
fn package_ecosystem(format: crate::domain::package::PackageFormat) -> String {
    match format {
        crate::domain::package::PackageFormat::Deb => "debian".to_string(),
        crate::domain::package::PackageFormat::Rpm => "rpm".to_string(),
        crate::domain::package::PackageFormat::Alpm => "alpm".to_string(),
        crate::domain::package::PackageFormat::Tarball => "tarball".to_string(),
    }
}

/// Provides a total, reproducible ordering for package candidates.
///
/// Versions are compared with their native ecosystem comparator when the two
/// candidates use the same format. Candidates from different formats do not
/// have a meaningful cross-ecosystem version ordering, so the format and
/// digest tie-breakers keep selection deterministic without coercing versions
/// into SemVer (INV-008).
fn package_candidate_cmp(a: &NormalizedPackage, b: &NormalizedPackage) -> Ordering {
    let name_order = a.name.cmp(&b.name);
    if name_order != Ordering::Equal {
        return name_order;
    }

    if a.format == b.format {
        let ecosystem = version_ecosystem(a.format);
        let version_order = crate::domain::version::compare_versions(
            b.version.as_str(),
            a.version.as_str(),
            ecosystem,
        );
        if version_order != Ordering::Equal {
            return version_order;
        }
    } else {
        let format_order = package_format_rank(a.format).cmp(&package_format_rank(b.format));
        if format_order != Ordering::Equal {
            return format_order;
        }
    }

    // Keep the remaining ordering independent of repository insertion order.
    let version_order = b.version.cmp(&a.version);
    if version_order != Ordering::Equal {
        return version_order;
    }
    a.digest.to_string().cmp(&b.digest.to_string())
}

fn package_format_rank(format: crate::domain::package::PackageFormat) -> u8 {
    match format {
        crate::domain::package::PackageFormat::Deb => 0,
        crate::domain::package::PackageFormat::Rpm => 1,
        crate::domain::package::PackageFormat::Alpm => 2,
        crate::domain::package::PackageFormat::Tarball => 3,
    }
}

fn version_ecosystem(format: crate::domain::package::PackageFormat) -> VersionEcosystem {
    match format {
        crate::domain::package::PackageFormat::Deb => VersionEcosystem::Debian,
        crate::domain::package::PackageFormat::Rpm => VersionEcosystem::Rpm,
        crate::domain::package::PackageFormat::Alpm => VersionEcosystem::Alpm,
        crate::domain::package::PackageFormat::Tarball => VersionEcosystem::Debian,
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
            versioned_provides: Vec::new(),
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
    fn test_resolve_debian_package_satisfied_via_host_soname() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", Some("2.38"), &["GLIBC_2.38"])
            .build();

        let resolver = Resolver::new(host);

        let pkg = make_pkg(
            "deb-tool",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("libc6").unwrap(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.34"),
                ),
                ecosystem: "debian".to_string(),
                original_expression: "libc6 (>= 2.34)".to_string(),
            }],
            vec![],
        );

        let plan = resolver.resolve(&pkg).unwrap();
        assert_eq!(plan.host_satisfied_capabilities.len(), 1);
        assert_eq!(plan.packages_to_install.len(), 0);
        assert_eq!(
            plan.host_satisfied_capabilities[0].capability,
            Capability::SharedLibrary("libc.so.6".to_string())
        );
    }

    #[test]
    fn test_resolve_debian_package_satisfied_via_host_equivalent_package() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_host_package("alsa-lib", "1.2.11-1", "alpm", vec![])
            .build();

        let resolver = Resolver::new(host);

        let pkg = make_pkg(
            "deb-media-player",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("libasound2").unwrap(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("1.0.17"),
                ),
                ecosystem: "debian".to_string(),
                original_expression: "libasound2 (>= 1.0.17)".to_string(),
            }],
            vec![],
        );

        let plan = resolver.resolve(&pkg).unwrap();
        assert_eq!(plan.packages_to_install.len(), 0);
        assert_eq!(plan.host_satisfied_capabilities.len(), 1);
    }

    #[test]
    fn test_resolve_debian_package_satisfied_via_host_command() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_executable("xdg-open")
            .build();

        let resolver = Resolver::new(host);

        let pkg = make_pkg(
            "deb-desktop-app",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("xdg-utils").unwrap(),
                version: VersionConstraint::Any,
                ecosystem: "debian".to_string(),
                original_expression: "xdg-utils".to_string(),
            }],
            vec![],
        );

        let plan = resolver.resolve(&pkg).unwrap();
        assert_eq!(plan.packages_to_install.len(), 0);
        assert_eq!(plan.host_satisfied_capabilities.len(), 1);
    }

    #[test]
    fn test_resolve_debian_package_incompatible_version_rejected() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", Some("2.35"), &["GLIBC_2.35"])
            .build();

        let resolver = Resolver::new(host);

        let pkg = make_pkg(
            "deb-tool-future",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("libc6").unwrap(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.99"),
                ),
                ecosystem: "debian".to_string(),
                original_expression: "libc6 (>= 2.99)".to_string(),
            }],
            vec![],
        );

        assert!(resolver.resolve(&pkg).is_err());
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
                version: VersionConstraint::Any,
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

    #[test]
    fn versioned_conflict_only_rejects_matching_versions() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        let installed = make_pkg(
            "libvirt-clients",
            "12.0.0-1",
            PackageFormat::Deb,
            vec![],
            vec![],
        );
        let target = make_pkg(
            "libvirt-common",
            "12.0.0-1",
            PackageFormat::Deb,
            vec![Constraint::Conflict {
                target: "libvirt-clients".into(),
                version: VersionConstraint::Relational(
                    VersionOp::Less,
                    PackageVersion::new("10.6.0-2~"),
                ),
                original_expression: "libvirt-clients (<< 10.6.0-2~)".into(),
            }],
            vec![],
        );

        assert!(
            Resolver::new(host)
                .with_installed_packages(vec![installed])
                .resolve(&target)
                .is_ok()
        );
    }

    #[test]
    fn upgrade_root_replaces_its_old_conflicting_generation() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        let installed = make_pkg("root", "1.0", PackageFormat::Deb, vec![], vec![]);
        let dependency = make_pkg(
            "dependency",
            "2.0",
            PackageFormat::Deb,
            vec![Constraint::Conflict {
                target: "root".into(),
                version: VersionConstraint::Relational(VersionOp::Less, PackageVersion::new("2.0")),
                original_expression: "Breaks: root (<< 2.0)".into(),
            }],
            vec![],
        );
        let target = make_pkg(
            "root",
            "2.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("dependency").unwrap(),
                version: VersionConstraint::Any,
                ecosystem: "debian".into(),
                original_expression: "dependency".into(),
            }],
            vec![],
        );

        let result = Resolver::new(host)
            .with_installed_packages(vec![installed])
            .with_repository_packages(vec![dependency])
            .resolve(&target);
        assert!(result.is_ok());
    }

    #[test]
    fn upgrade_dependency_replaces_its_old_conflicting_generation() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        let installed_client = make_pkg("client", "1.0", PackageFormat::Deb, vec![], vec![]);
        let provider = make_pkg(
            "provider",
            "2.0",
            PackageFormat::Deb,
            vec![Constraint::Conflict {
                target: "client".into(),
                version: VersionConstraint::Relational(VersionOp::Less, PackageVersion::new("2.0")),
                original_expression: "Breaks: client (<< 2.0)".into(),
            }],
            vec![],
        );
        let incoming_client = make_pkg("client", "2.0", PackageFormat::Deb, vec![], vec![]);
        let target = make_pkg(
            "root",
            "2.0",
            PackageFormat::Deb,
            vec![
                Constraint::Package {
                    name: PackageName::new("provider").unwrap(),
                    version: VersionConstraint::Any,
                    ecosystem: "debian".into(),
                    original_expression: "provider".into(),
                },
                Constraint::Package {
                    name: PackageName::new("client").unwrap(),
                    version: VersionConstraint::Relational(
                        VersionOp::GreaterEqual,
                        PackageVersion::new("2.0"),
                    ),
                    ecosystem: "debian".into(),
                    original_expression: "client (>= 2.0)".into(),
                },
            ],
            vec![],
        );

        let result = Resolver::new(host)
            .with_installed_packages(vec![installed_client])
            .with_repository_packages(vec![provider, incoming_client])
            .resolve(&target);
        assert!(result.is_ok());
    }

    #[test]
    fn package_does_not_conflict_with_itself() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        let target = make_pkg(
            "self-conflicting",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Conflict {
                target: "self-conflicting".into(),
                version: VersionConstraint::Any,
                original_expression: "Conflicts: self-conflicting".into(),
            }],
            vec![],
        );

        assert!(Resolver::new(host).resolve(&target).is_ok());
    }

    #[test]
    fn versioned_provides_use_capability_version_and_selection_is_stable() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        let old = make_pkg("libfeature", "1.0", PackageFormat::Deb, vec![], vec![]);
        let mut old = old;
        old.versioned_provides = vec![crate::domain::package::VersionedCapability {
            capability: Capability::Feature("virtual-feature".into()),
            version: PackageVersion::new("2.0"),
        }];
        let mut new = make_pkg("libfeature", "2.0", PackageFormat::Deb, vec![], vec![]);
        new.versioned_provides = vec![crate::domain::package::VersionedCapability {
            capability: Capability::Feature("virtual-feature".into()),
            version: PackageVersion::new("3.0"),
        }];
        let target = make_pkg(
            "consumer",
            "1.0",
            PackageFormat::Deb,
            vec![Constraint::Capability(CapabilityConstraint {
                identifier: "feature:virtual-feature".into(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.5"),
                ),
                original_expression: "virtual-feature (>= 2.5)".into(),
            })],
            vec![],
        );

        let plan = Resolver::new(host)
            .with_repository_packages(vec![new, old])
            .resolve(&target);
        let plan = plan.expect("versioned capability should be satisfied");
        assert_eq!(plan.packages_to_install[0].version.as_str(), "2.0");
    }

    #[test]
    fn unversioned_capability_does_not_satisfy_versioned_requirement() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        let provider = make_pkg(
            "libfeature",
            "99.0",
            PackageFormat::Deb,
            vec![],
            vec![Capability::Feature("virtual-feature".into())],
        );
        let target = make_pkg(
            "consumer",
            "1.0",
            PackageFormat::Deb,
            vec![Constraint::Capability(CapabilityConstraint {
                identifier: "feature:virtual-feature".into(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.0"),
                ),
                original_expression: "virtual-feature (>= 2.0)".into(),
            })],
            vec![],
        );

        let error = Resolver::new(host)
            .with_repository_packages(vec![provider])
            .resolve(&target)
            .expect_err("unversioned Provides must not imply a capability version");
        assert!(error.to_string().contains("No provider found"), "{error}");
    }

    #[test]
    fn host_capability_without_version_does_not_satisfy_versioned_requirement() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", None, &["GLIBC_2.38"])
            .build();
        let target = make_pkg(
            "consumer",
            "1.0",
            PackageFormat::Deb,
            vec![Constraint::Capability(CapabilityConstraint {
                identifier: "lib:libc.so.6".into(),
                version: VersionConstraint::Relational(
                    VersionOp::GreaterEqual,
                    PackageVersion::new("2.0"),
                ),
                original_expression: "libc6 (>= 2.0)".into(),
            })],
            vec![],
        );

        let error = Resolver::new(host)
            .resolve(&target)
            .expect_err("host evidence without a version is incomplete");
        assert!(error.to_string().contains("does not satisfy"), "{error}");
    }

    #[test]
    fn test_virtual_package_resolved_via_repository_provides() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .build();
        // Repository has dconf-gsettings-backend which provides "gsettings-backend"
        let dconf = make_pkg(
            "dconf-gsettings-backend",
            "0.40.0-4",
            PackageFormat::Deb,
            vec![],
            vec![Capability::Feature("gsettings-backend".to_string())],
        );
        // Target app depends on virtual package "gsettings-backend"
        let app = make_pkg(
            "desktop-app",
            "1.0.0",
            PackageFormat::Deb,
            vec![Constraint::Package {
                name: PackageName::new("gsettings-backend").unwrap(),
                version: VersionConstraint::Any,
                ecosystem: "debian".into(),
                original_expression: "gsettings-backend".into(),
            }],
            vec![],
        );

        let plan = Resolver::new(host)
            .with_repository_packages(vec![dconf])
            .resolve(&app)
            .expect("virtual package requirement should be satisfied by repository package with Provides");

        assert_eq!(plan.packages_to_install.len(), 1);
        assert_eq!(
            plan.packages_to_install[0].name.as_str(),
            "dconf-gsettings-backend"
        );
    }

    #[test]
    fn test_virtual_package_satisfied_via_host_feature_evidence() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_feature("gsettings-backend")
            .add_feature("default-dbus-session-bus")
            .build();

        // Target app depends on virtual package "gsettings-backend" and "default-dbus-session-bus"
        let app = make_pkg(
            "desktop-app",
            "1.0.0",
            PackageFormat::Deb,
            vec![
                Constraint::Package {
                    name: PackageName::new("gsettings-backend").unwrap(),
                    version: VersionConstraint::Any,
                    ecosystem: "debian".into(),
                    original_expression: "gsettings-backend".into(),
                },
                Constraint::Package {
                    name: PackageName::new("default-dbus-session-bus").unwrap(),
                    version: VersionConstraint::Any,
                    ecosystem: "debian".into(),
                    original_expression: "default-dbus-session-bus".into(),
                },
            ],
            vec![],
        );

        let plan = Resolver::new(host)
            .resolve(&app)
            .expect("virtual package requirements should be satisfied by host feature evidence");

        assert!(plan.packages_to_install.is_empty());
        assert_eq!(plan.host_satisfied_capabilities.len(), 2);
    }
}
