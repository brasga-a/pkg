//! Host and package evidence domain models for resolver compatibility checks (ADR-016, INV-009).
//!
//! Captures verified host facts, system libraries, dynamic linker paths, and capability tokens
//! needed to satisfy or reject normalized constraints without relying on nominal package-name aliases.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::capability::Capability;
use crate::domain::constraint::VersionConstraint;
use crate::domain::package::{Architecture, PackageVersion};
use crate::domain::version::VersionEcosystem;
use crate::host::HostFacts;

/// Standard host Linux library search paths for 64-bit systems.
pub const STANDARD_LIB_SEARCH_DIRS: &[&str] = &[
    "/lib64",
    "/usr/lib64",
    "/lib/x86_64-linux-gnu",
    "/usr/lib/x86_64-linux-gnu",
    "/lib",
    "/usr/lib",
];

/// Verified evidence of a capability provided by the host platform or an installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEvidence {
    /// Normalized capability identifier.
    pub capability: Capability,
    /// Explicit version of the capability, if provided.
    pub version: Option<PackageVersion>,
    /// Source origin of the evidence (e.g. `host:system`, `store:libc6_2.38-1`).
    pub provider_origin: String,
    /// ABI symbol versions exported (e.g. `["GLIBC_2.17", "GLIBC_2.34", "GLIBC_2.38"]`).
    pub symbols: Vec<String>,
}

/// Verified evidence of a native package installed on the host operating system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPackageEvidence {
    /// Nominal name of the native package.
    pub name: String,
    /// Host package version.
    pub version: PackageVersion,
    /// Host packaging ecosystem (e.g. `debian`, `alpm`, `rpm`).
    pub ecosystem: String,
    /// Explicit capabilities provided by the host package (e.g. features, virtual names, libraries).
    pub provides: Vec<Capability>,
}

/// Verified evidence about the host environment used for capability and ABI evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEvidence {
    /// Host CPU architecture.
    pub architecture: Architecture,
    /// Verified capabilities provided directly by the host operating system.
    pub provided_capabilities: HashMap<String, CapabilityEvidence>,
    /// Verified linker search paths on the host.
    pub library_search_paths: Vec<PathBuf>,
    /// Native packages discovered on the host system.
    pub host_packages: HashMap<String, HostPackageEvidence>,
}

impl HostEvidence {
    /// Creates a fluent builder for constructing test or customized host evidence.
    pub fn builder() -> HostEvidenceBuilder {
        HostEvidenceBuilder::default()
    }

    /// Automatically detects host evidence from the current running environment.
    pub fn detect(facts: &HostFacts) -> Self {
        let mut builder = Self::builder().architecture(facts.architecture.clone());

        // Probe standard library search paths
        for &dir in STANDARD_LIB_SEARCH_DIRS {
            let path = PathBuf::from(dir);
            if path.is_dir() {
                builder = builder.add_lib_path(path);
            }
        }

        // Broad dynamic discovery of host executables
        detect_executables(&mut builder);

        // Detect libc SONAME on host if present
        for &dir in STANDARD_LIB_SEARCH_DIRS {
            let libc = Path::new(dir).join("libc.so.6");
            if libc.exists() {
                let symbols = crate::host::elf::inspect_elf(&libc, None)
                    .ok()
                    .flatten()
                    .map(|inspection| inspection.defined_symbol_versions)
                    .unwrap_or_default();
                let inferred_version = symbols
                    .iter()
                    .filter_map(|symbol| symbol.strip_prefix("GLIBC_"))
                    .filter(|version| {
                        version.split('.').all(|part| {
                            !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
                        })
                    })
                    .max_by(|left, right| compare_numeric_version(left, right))
                    .map(str::to_string);
                let symbol_refs = symbols.iter().map(String::as_str).collect::<Vec<_>>();
                builder =
                    builder.add_library("libc.so.6", inferred_version.as_deref(), &symbol_refs);
                break;
            }
        }

        // Broad detection of standard desktop and system subsystems
        detect_subsystems(&mut builder);

        // Native host package manager inspection
        detect_pacman_local(&mut builder);
        detect_dpkg_status(&mut builder);

        builder.build()
    }

    /// Checks whether the host provides the requested capability, respecting version and symbols.
    pub fn provides_capability(&self, cap_str: &str) -> Option<&CapabilityEvidence> {
        self.provided_capabilities.get(cap_str)
    }

    /// Checks whether the host provides the requested feature/token.
    pub fn provides_feature(&self, name: &str) -> Option<&CapabilityEvidence> {
        self.provided_capabilities
            .get(&format!("feature:{name}"))
            .or_else(|| self.provided_capabilities.get(name))
    }

    /// Checks whether the host provides the requested native package matching ecosystem and version constraint.
    pub fn provides_package(
        &self,
        name: &str,
        version: &VersionConstraint,
        target_ecosystem: &str,
    ) -> Option<&HostPackageEvidence> {
        if let Some(pkg) = self.host_packages.get(name) {
            if target_ecosystem.is_empty() || target_ecosystem.eq_ignore_ascii_case(&pkg.ecosystem)
            {
                let eco = match pkg.ecosystem.as_str() {
                    "debian" | "ubuntu" => VersionEcosystem::Debian,
                    "rpm" | "fedora" | "rhel" | "suse" | "centos" => VersionEcosystem::Rpm,
                    "alpm" | "arch" => VersionEcosystem::Alpm,
                    _ => VersionEcosystem::Debian,
                };
                if version.matches(pkg.version.as_str(), eco) {
                    return Some(pkg);
                }
            }
        }
        None
    }

    /// Checks if a dynamic library SONAME exists in any verified host library search path.
    pub fn has_soname(&self, soname: &str) -> bool {
        let lib_key = format!("lib:{soname}");
        if self.provided_capabilities.contains_key(&lib_key) {
            return true;
        }
        for dir in &self.library_search_paths {
            let candidate = dir.join(soname);
            if host_library_matches_architecture(&candidate, &self.architecture) {
                return true;
            }
        }
        false
    }
}

fn host_library_matches_architecture(path: &Path, architecture: &Architecture) -> bool {
    let Some(inspection) = crate::host::elf::inspect_elf(path, None).ok().flatten() else {
        return false;
    };
    let abi = crate::host::elf::expected_elf_abi(architecture);
    abi.is_none_or(|(machine, class_bits)| {
        inspection.machine == machine && inspection.class_bits == class_bits
    }) && inspection.little_endian
}

fn compare_numeric_version(left: &str, right: &str) -> std::cmp::Ordering {
    let mut left_parts = left.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    let mut right_parts = right
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    loop {
        match (left_parts.next(), right_parts.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(value)) => {
                if value == 0 {
                    continue;
                }
                return std::cmp::Ordering::Less;
            }
            (Some(value), None) => {
                if value == 0 {
                    continue;
                }
                return std::cmp::Ordering::Greater;
            }
            (Some(left), Some(right)) => match left.cmp(&right) {
                std::cmp::Ordering::Equal => {}
                ordering => return ordering,
            },
        }
    }
}

/// Fluent builder for constructing `HostEvidence` fixtures.
#[derive(Debug, Default)]
pub struct HostEvidenceBuilder {
    architecture: Option<Architecture>,
    provided_capabilities: HashMap<String, CapabilityEvidence>,
    library_search_paths: Vec<PathBuf>,
    host_packages: HashMap<String, HostPackageEvidence>,
}

impl HostEvidenceBuilder {
    /// Sets the host architecture.
    pub fn architecture(mut self, arch: Architecture) -> Self {
        self.architecture = Some(arch);
        self
    }

    /// Adds a library search path.
    pub fn add_lib_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.library_search_paths.push(path.into());
        self
    }

    /// In-place registration of a shared library capability.
    pub fn add_library_in_place(&mut self, soname: &str, version: Option<&str>, symbols: &[&str]) {
        let key = format!("lib:{soname}");
        let evidence = CapabilityEvidence {
            capability: Capability::SharedLibrary(soname.to_string()),
            version: version.map(PackageVersion::new),
            provider_origin: "host:system".to_string(),
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
        };
        self.provided_capabilities.insert(key, evidence);
    }

    /// Registers a shared library capability with optional version and exported symbol versions.
    pub fn add_library(mut self, soname: &str, version: Option<&str>, symbols: &[&str]) -> Self {
        self.add_library_in_place(soname, version, symbols);
        self
    }

    /// In-place registration of an executable binary capability.
    pub fn add_executable_in_place(&mut self, command: &str) {
        let key = format!("bin:{command}");
        let evidence = CapabilityEvidence {
            capability: Capability::Executable(command.to_string()),
            version: None,
            provider_origin: "host:system".to_string(),
            symbols: Vec::new(),
        };
        self.provided_capabilities.insert(key, evidence);
    }

    /// Registers an executable binary capability.
    pub fn add_executable(mut self, command: &str) -> Self {
        self.add_executable_in_place(command);
        self
    }

    /// In-place registration of a feature capability.
    pub fn add_feature_in_place(&mut self, feature: &str) {
        let key = format!("feature:{feature}");
        let evidence = CapabilityEvidence {
            capability: Capability::Feature(feature.to_string()),
            version: None,
            provider_origin: "host:system".to_string(),
            symbols: Vec::new(),
        };
        self.provided_capabilities.insert(key, evidence);
    }

    /// Registers a generic or virtual feature capability.
    pub fn add_feature(mut self, feature: &str) -> Self {
        self.add_feature_in_place(feature);
        self
    }

    /// In-place registration of a versioned feature capability.
    pub fn add_feature_with_version_in_place(&mut self, feature: &str, version: Option<&str>) {
        let key = format!("feature:{feature}");
        let evidence = CapabilityEvidence {
            capability: Capability::Feature(feature.to_string()),
            version: version.map(PackageVersion::new),
            provider_origin: "host:system".to_string(),
            symbols: Vec::new(),
        };
        self.provided_capabilities.insert(key, evidence);
    }

    /// Registers a versioned feature capability.
    pub fn add_feature_with_version(mut self, feature: &str, version: Option<&str>) -> Self {
        self.add_feature_with_version_in_place(feature, version);
        self
    }

    /// In-place registration of native package evidence.
    pub fn add_host_package_in_place(
        &mut self,
        name: &str,
        version: &str,
        ecosystem: &str,
        provides: Vec<Capability>,
    ) {
        let evidence = HostPackageEvidence {
            name: name.to_string(),
            version: PackageVersion::new(version),
            ecosystem: ecosystem.to_string(),
            provides,
        };
        self.host_packages.insert(name.to_string(), evidence);
    }

    /// Registers native package evidence.
    pub fn add_host_package(
        mut self,
        name: &str,
        version: &str,
        ecosystem: &str,
        provides: Vec<Capability>,
    ) -> Self {
        self.add_host_package_in_place(name, version, ecosystem, provides);
        self
    }

    /// Builds the configured `HostEvidence`.
    pub fn build(self) -> HostEvidence {
        HostEvidence {
            architecture: self.architecture.unwrap_or(Architecture::X86_64),
            provided_capabilities: self.provided_capabilities,
            library_search_paths: self.library_search_paths,
            host_packages: self.host_packages,
        }
    }
}

/// Dynamically discovers executables in standard system binary directories.
fn detect_executables(builder: &mut HostEvidenceBuilder) {
    for bin_dir in &["/bin", "/usr/bin"] {
        let path = Path::new(bin_dir);
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_file() || file_type.is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        builder.add_executable_in_place(name);
                    }
                }
            }
        }
    }
}

/// Detects standard Linux desktop and system subsystems.
fn detect_subsystems(builder: &mut HostEvidenceBuilder) {
    // 1. D-Bus & IPC
    let has_dbus = Path::new("/usr/bin/dbus-daemon").exists()
        || Path::new("/usr/bin/dbus-send").exists()
        || Path::new("/usr/share/dbus-1").exists()
        || Path::new("/run/dbus/system_bus_socket").exists()
        || std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
    if has_dbus {
        builder.add_feature_in_place("dbus");
        builder.add_feature_in_place("dbus-session-bus");
        builder.add_feature_in_place("default-dbus-session-bus");
        builder.add_feature_in_place("dbus-user-session");
        builder.add_feature_in_place("dbus-system-bus");
    }

    // 2. GSettings & DConf desktop configuration
    let has_gsettings = Path::new("/usr/share/glib-2.0/schemas").exists()
        || Path::new("/usr/bin/dconf").exists()
        || Path::new("/usr/lib/dconf").exists()
        || Path::new("/usr/lib64/dconf").exists()
        || Path::new("/usr/lib/x86_64-linux-gnu/dconf").exists();
    if has_gsettings {
        builder.add_feature_in_place("gsettings-backend");
        builder.add_feature_in_place("dconf-gsettings-backend");
        builder.add_feature_in_place("dconf-service");
        builder.add_feature_in_place("gsettings-desktop-schemas");
    }

    // 3. Display & Windowing
    let has_x11 = Path::new("/usr/share/X11").exists()
        || Path::new("/usr/bin/Xorg").exists()
        || Path::new("/tmp/.X11-unix").exists()
        || std::env::var_os("DISPLAY").is_some();
    if has_x11 {
        builder.add_feature_in_place("x11-common");
        builder.add_feature_in_place("x11");
        builder.add_feature_in_place("xserver-xorg");
    }

    let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || Path::new("/usr/bin/wayland-scanner").exists()
        || Path::new("/usr/share/wayland").exists();
    if has_wayland {
        builder.add_feature_in_place("wayland");
        builder.add_feature_in_place("wayland-client");
    }

    // 4. Audio Subsystems
    if Path::new("/usr/bin/pipewire").exists() {
        builder.add_feature_in_place("pipewire");
        builder.add_feature_in_place("pipewire-audio");
        builder.add_feature_in_place("pipewire-session-manager");
    }
    if Path::new("/usr/bin/pulseaudio").exists() {
        builder.add_feature_in_place("pulseaudio");
        builder.add_feature_in_place("pulse");
    }
    if Path::new("/usr/share/alsa").exists()
        || Path::new("/etc/asound.conf").exists()
        || Path::new("/proc/asound").exists()
    {
        builder.add_feature_in_place("alsa");
        builder.add_feature_in_place("alsa-utils");
        builder.add_feature_in_place("alsa-lib");
    }

    // 5. PKI & CA Certificates
    if Path::new("/etc/ssl/certs/ca-certificates.crt").exists()
        || Path::new("/etc/pki/tls/certs/ca-bundle.crt").exists()
        || Path::new("/etc/ssl/certs").exists()
    {
        builder.add_feature_in_place("ca-certificates");
    }

    // 6. Desktop Integration & MIME & Fonts
    if Path::new("/usr/share/mime").exists() {
        builder.add_feature_in_place("shared-mime-info");
    }
    if Path::new("/usr/share/applications").exists() {
        builder.add_feature_in_place("desktop-file-utils");
    }
    if Path::new("/usr/share/icons/hicolor").exists() {
        builder.add_feature_in_place("hicolor-icon-theme");
    }
    if Path::new("/usr/bin/xdg-open").exists() {
        builder.add_feature_in_place("xdg-utils");
    }
    if Path::new("/usr/share/fonts").exists() || Path::new("/etc/fonts").exists() {
        builder.add_feature_in_place("fontconfig");
    }
}

/// Inspects pacman local database on Arch Linux hosts.
fn detect_pacman_local(builder: &mut HostEvidenceBuilder) {
    detect_pacman_local_at(Path::new("/var/lib/pacman/local"), builder);
}

fn detect_pacman_local_at(pacman_dir: &Path, builder: &mut HostEvidenceBuilder) {
    let Ok(entries) = std::fs::read_dir(pacman_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let desc_path = entry.path().join("desc");
        if !desc_path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&desc_path) else {
            continue;
        };

        let mut current_section = "";
        let mut pkg_name = None;
        let mut pkg_version = None;
        let mut provides = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('%') && line.ends_with('%') {
                current_section = line;
                continue;
            }
            if line.is_empty() {
                continue;
            }
            match current_section {
                "%NAME%" if pkg_name.is_none() => {
                    pkg_name = Some(line.to_string());
                }
                "%VERSION%" if pkg_version.is_none() => {
                    pkg_version = Some(line.to_string());
                }
                "%PROVIDES%" => {
                    let prov = line.split('=').next().unwrap_or(line).trim();
                    if !prov.is_empty() {
                        provides.push(prov.to_string());
                    }
                }
                _ => {}
            }
        }

        if let (Some(name), Some(version)) = (pkg_name, pkg_version) {
            let mut provides_caps = Vec::new();
            for prov in provides {
                if prov.contains(".so") {
                    builder.add_library_in_place(&prov, None, &[]);
                    provides_caps.push(Capability::SharedLibrary(prov));
                } else {
                    builder.add_feature_in_place(&prov);
                    provides_caps.push(Capability::Feature(prov));
                }
            }
            builder.add_host_package_in_place(&name, &version, "alpm", provides_caps);
        }
    }
}

/// Inspects dpkg status file on Debian and Ubuntu hosts.
fn detect_dpkg_status(builder: &mut HostEvidenceBuilder) {
    detect_dpkg_status_at(Path::new("/var/lib/dpkg/status"), builder);
}

fn detect_dpkg_status_at(status_path: &Path, builder: &mut HostEvidenceBuilder) {
    let Ok(file) = std::fs::File::open(status_path) else {
        return;
    };
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(file);

    let mut pkg_name: Option<String> = None;
    let mut pkg_version: Option<String> = None;
    let mut is_installed = false;
    let mut provides_raw: Vec<String> = Vec::new();

    let mut commit_package =
        |name: Option<String>, version: Option<String>, installed: bool, provides: &[String]| {
            if installed && let (Some(name), Some(version)) = (name, version) {
                let mut provides_caps = Vec::new();
                for prov in provides {
                    let clean = prov.split('(').next().unwrap_or(prov).trim();
                    if clean.is_empty() {
                        continue;
                    }
                    if clean.contains(".so") {
                        builder.add_library_in_place(clean, None, &[]);
                        provides_caps.push(Capability::SharedLibrary(clean.to_string()));
                    } else {
                        builder.add_feature_in_place(clean);
                        provides_caps.push(Capability::Feature(clean.to_string()));
                    }
                }
                builder.add_host_package_in_place(&name, &version, "debian", provides_caps);
            }
        };

    for line in reader.lines().map_while(Result::ok) {
        if line.is_empty() {
            commit_package(
                pkg_name.take(),
                pkg_version.take(),
                is_installed,
                &provides_raw,
            );
            is_installed = false;
            provides_raw.clear();
            continue;
        }

        if let Some(rest) = line.strip_prefix("Package: ") {
            pkg_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Status: ") {
            if rest.contains("installed") && !rest.contains("not-installed") {
                is_installed = true;
            }
        } else if let Some(rest) = line.strip_prefix("Version: ") {
            pkg_version = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("Provides: ") {
            for item in rest.split(',') {
                provides_raw.push(item.trim().to_string());
            }
        }
    }
    commit_package(pkg_name, pkg_version, is_installed, &provides_raw);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_evidence_builder() {
        let host = HostEvidence::builder()
            .architecture(Architecture::X86_64)
            .add_library("libc.so.6", Some("2.38"), &["GLIBC_2.34", "GLIBC_2.38"])
            .add_executable("sh")
            .add_feature("gsettings-backend")
            .add_host_package(
                "dconf",
                "0.40.0",
                "alpm",
                vec![Capability::Feature("gsettings-backend".to_string())],
            )
            .build();

        assert!(host.has_soname("libc.so.6"));
        let cap = host.provides_capability("lib:libc.so.6").unwrap();
        assert_eq!(cap.version.as_ref().map(|v| v.as_str()), Some("2.38"));
        assert!(cap.symbols.contains(&"GLIBC_2.38".to_string()));

        assert!(host.provides_feature("gsettings-backend").is_some());
        assert!(
            host.provides_package("dconf", &VersionConstraint::Any, "alpm")
                .is_some()
        );
        // Cross-ecosystem nominal match without capability evidence rejected (INV-007)
        assert!(
            host.provides_package("dconf", &VersionConstraint::Any, "debian")
                .is_none()
        );
    }

    #[test]
    fn test_broad_host_evidence_detection() {
        let facts = HostFacts::detect();
        let host = HostEvidence::detect(&facts);

        // System binaries should be discovered
        assert!(host.provides_capability("bin:sh").is_some());

        // Basic subsystems should be detected if on standard Linux
        if Path::new("/etc/ssl/certs").exists() {
            assert!(host.provides_feature("ca-certificates").is_some());
        }
    }

    #[test]
    fn test_pacman_local_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let pkg_dir = temp.path().join("dconf-0.40.0-2");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let desc_content = "%NAME%\ndconf\n\n%VERSION%\n0.40.0-2\n\n%PROVIDES%\ngsettings-backend\ndconf-service\n";
        std::fs::write(pkg_dir.join("desc"), desc_content).unwrap();

        let mut builder = HostEvidence::builder();
        detect_pacman_local_at(temp.path(), &mut builder);
        let host = builder.build();

        assert!(host.provides_feature("gsettings-backend").is_some());
        assert!(host.provides_feature("dconf-service").is_some());
        assert!(
            host.provides_package("dconf", &VersionConstraint::Any, "alpm")
                .is_some()
        );
    }

    #[test]
    fn test_dpkg_status_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let status_path = temp.path().join("status");
        let status_content = "Package: dbus-user-session\nStatus: install ok installed\nVersion: 1.14.10-4ubuntu4\nProvides: default-dbus-session-bus, dbus-session-bus\n\nPackage: broken-pkg\nStatus: deinstall ok config-files\nVersion: 1.0\nProvides: should-not-be-included\n";
        std::fs::write(&status_path, status_content).unwrap();

        let mut builder = HostEvidence::builder();
        detect_dpkg_status_at(&status_path, &mut builder);
        let host = builder.build();

        assert!(host.provides_feature("default-dbus-session-bus").is_some());
        assert!(host.provides_feature("dbus-session-bus").is_some());
        assert!(host.provides_feature("should-not-be-included").is_none());
        assert!(
            host.provides_package("dbus-user-session", &VersionConstraint::Any, "debian")
                .is_some()
        );
    }
}
