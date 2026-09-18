//! Runtime adaptation, launcher generation, and scoped process environment (ADR-010, INV-006).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::domain::capability::Capability;
use crate::domain::contracts::{
    ExecutionPlan, LaunchStrategy, ProviderEvidence, RuntimeManifest, digest_bytes,
    digest_serialized,
};
use crate::domain::plan::BinaryActivation;
use crate::error::Result;
use crate::host::elf::{inspect_elf, inspect_elf_with_extra_paths};

const NATIVE_RUNNER_ELF_MAGIC: &[u8; 4] = b"\x7fELF";

/// Returns the platform-specific static bootstrap produced by `build.rs`.
/// An empty result means the current build target has no supported static
/// runner and dynamic ELF commands must remain unavailable.
#[must_use]
pub fn native_runner_bytes() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/pkg-native-runner"))
}

/// Stable identity for the exact runner bytes embedded in this build.
#[must_use]
pub fn native_runner_version() -> String {
    format!(
        "native-static-v1:{}",
        digest_serialized(&native_runner_bytes().to_vec())
    )
}

/// Writes a command-local static runner and its structured sidecar config.
pub fn write_native_runner(
    launcher: &Path,
    target_script_rel_path: &Path,
    runtime_lib_dir: &Path,
) -> Result<()> {
    let bytes = native_runner_bytes();
    if !bytes.starts_with(NATIVE_RUNNER_ELF_MAGIC) {
        return Err(crate::error::Error::IncompatibleHost(
            "static native runner is unavailable for this target".into(),
        ));
    }
    let target = target_script_rel_path.to_string_lossy();
    let library_path = runtime_lib_dir.to_string_lossy();
    if target_script_rel_path.is_absolute()
        || target_script_rel_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || target.contains(['\n', '\r', '='])
        || library_path.contains(['\n', '\r', '='])
    {
        return Err(crate::error::Error::SecurityViolation(
            "runner paths cannot contain newline or '='".into(),
        ));
    }
    fs::write(launcher, bytes)?;
    let mut permissions = fs::metadata(launcher)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(launcher, permissions)?;
    let config = launcher.with_extension("conf");
    fs::write(
        config,
        format!("target={target}\nlibrary_path={library_path}\n"),
    )?;
    Ok(())
}

/// Scoped environment variable mutation for a command launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedEnvVar {
    pub name: String,
    /// Store-relative paths to prepend (e.g. `usr/lib/python3.14/site-packages`).
    pub paths: Vec<PathBuf>,
}

/// Plan for generating a runtime launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLauncherPlan {
    /// Command name (e.g. `reflector`).
    pub command: String,
    /// Original script path relative to store root (e.g. `usr/bin/reflector`).
    pub target_script_rel_path: PathBuf,
    /// Absolute path to the resolved interpreter (e.g. `/usr/bin/python3`).
    pub interpreter: String,
    /// Arguments encoded in the original shebang (for example `-Es`).
    pub interpreter_args: Vec<String>,
    /// Scoped environment variables to inject.
    pub env_vars: Vec<ScopedEnvVar>,
}

/// Builds frozen command and runtime contracts from the prepared payload.
///
/// This function only reads the staging tree.  It deliberately records the
/// exact provider path selected by the static ELF inspector instead of
/// exporting a profile-wide search directory.
pub fn build_execution_contracts(
    staging_dir: &std::path::Path,
    target_store_dir: &std::path::Path,
    binaries: &[BinaryActivation],
    extra_search_dirs: &[PathBuf],
    module_dirs: &[PathBuf],
) -> Result<(Vec<ExecutionPlan>, Vec<RuntimeManifest>)> {
    let host = crate::host::HostFacts::detect();
    let mut executions = Vec::new();
    let mut runtimes = Vec::new();

    for binary in binaries {
        let executable = binary.relative_store_path.clone();
        let full_path = staging_dir.join(&executable);
        let script = script_interpreter(&full_path);
        let interpreter = script.as_ref().map(|(path, _)| path.clone());
        let interpreter_args = script
            .as_ref()
            .map(|(_, args)| args.clone())
            .unwrap_or_default();
        if let Some(interpreter_path) = &interpreter {
            let interpreter_name = interpreter_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !supported_interpreter_family(&interpreter_name) {
                return Err(crate::error::Error::IncompatibleHost(format!(
                    "interpreter family '{}' has no verified adapter",
                    interpreter_name
                )));
            }
            let available = if interpreter_path.is_absolute() {
                interpreter_path.is_file()
            } else {
                resolve_general_interpreter(
                    interpreter_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default(),
                )
                .is_some()
            };
            if !available {
                return Err(crate::error::Error::IncompatibleHost(format!(
                    "script interpreter is unavailable: {}",
                    interpreter_path.display()
                )));
            }
        }
        let mut providers = Vec::new();
        let mut closure = vec![executable.to_string_lossy().into_owned()];
        let mut library_view: BTreeMap<String, PathBuf> = BTreeMap::new();
        let mut unresolved_libraries = Vec::new();

        let elf_inspection =
            inspect_elf_with_extra_paths(&full_path, Some(staging_dir), extra_search_dirs)?;
        if let Some(inspection) = &elf_inspection {
            // The kernel resolves PT_INTERP as an absolute host path.  A
            // loader found only inside the package staging tree cannot be
            // reached after promotion, and this implementation deliberately
            // does not bundle or invoke foreign loaders yet.
            if let Some(interpreter) = &inspection.interpreter
                && !Path::new(interpreter).is_file()
            {
                return Err(crate::error::Error::IncompatibleHost(format!(
                    "ELF interpreter is available only inside the package and has no supported host-loader adapter: {interpreter}"
                )));
            }
            for needed in inspection
                .needed_libraries
                .iter()
                .filter(|needed| needed.contains('/'))
            {
                if inspection
                    .resolved_paths
                    .get(needed)
                    .is_some_and(|path| path.starts_with(staging_dir))
                {
                    return Err(crate::error::Error::IncompatibleHost(format!(
                        "pathname dependency '{}' resolves inside the isolated store and cannot be relocated",
                        needed
                    )));
                }
            }
            unresolved_libraries.extend(inspection.missing_libraries.iter().cloned());
            let mut queue = inspection
                .needed_libraries
                .iter()
                .filter_map(|needed| {
                    inspection
                        .resolved_paths
                        .get(needed)
                        .cloned()
                        .map(|path| (needed.clone(), path, inspection.symbol_versions.clone()))
                })
                .collect::<std::collections::VecDeque<_>>();
            let mut visited = BTreeSet::new();
            while let Some((needed, path, symbol_versions)) = queue.pop_front() {
                if !visited.insert(path.clone()) {
                    continue;
                }
                let provider_metadata = inspect_elf_with_extra_paths(
                    &path,
                    path.starts_with(staging_dir).then_some(staging_dir),
                    extra_search_dirs,
                )?;
                if let Some(required) = symbol_versions.get(&needed)
                    && !required.is_empty()
                {
                    let exported = provider_metadata
                        .as_ref()
                        .map(|metadata| &metadata.defined_symbol_versions)
                        .ok_or_else(|| {
                            crate::error::Error::IncompatibleHost(format!(
                                "provider for {needed} is not an ELF object; symbol-version evidence is unavailable"
                            ))
                        })?;
                    if !required.iter().all(|version| exported.contains(version)) {
                        return Err(crate::error::Error::IncompatibleHost(format!(
                            "provider for {needed} does not export required symbol versions: {}",
                            required.join(", ")
                        )));
                    }
                }
                let origin = if path.starts_with(staging_dir) {
                    format!("store:{}", target_store_dir.display())
                } else {
                    "host:system".to_string()
                };
                let recorded_path = path
                    .strip_prefix(staging_dir)
                    .map(|relative| target_store_dir.join(relative))
                    .unwrap_or_else(|_| path.clone());
                let evidence = ProviderEvidence {
                    requirement: needed.clone(),
                    provider_origin: origin,
                    provider_path: recorded_path.clone(),
                    capability: Capability::SharedLibrary(needed.clone()),
                    soname: provider_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.soname.clone())
                        .or_else(|| Some(needed.clone())),
                    symbol_versions: symbol_versions.get(&needed).cloned().unwrap_or_default(),
                    architecture: host.architecture.clone(),
                    digest: fs::read(&path).ok().map(|bytes| digest_bytes(&bytes)),
                    reason: "static ELF DT_NEEDED resolution".to_string(),
                };
                if !needed.contains('/') {
                    if let Some(existing) = library_view.get(&needed)
                        && existing != &path
                    {
                        return Err(crate::error::Error::IncompatibleHost(format!(
                            "incompatible providers selected for the same library identity '{}': {} and {}",
                            needed,
                            existing.display(),
                            path.display()
                        )));
                    }
                    library_view.insert(needed.clone(), path.clone());
                }
                closure.push(recorded_path.to_string_lossy().into_owned());
                providers.push(evidence);

                // Resolve dependencies of each selected provider as well.  A
                // command-local view is valid only when the whole transitive
                // closure is represented, not merely the direct DT_NEEDED
                // names of the executable.
                if let Some(metadata) = provider_metadata {
                    unresolved_libraries.extend(metadata.missing_libraries.iter().cloned());
                    for child in &metadata.needed_libraries {
                        if let Some(child_path) = metadata.resolved_paths.get(child) {
                            queue.push_back((
                                child.clone(),
                                child_path.clone(),
                                metadata.symbol_versions.clone(),
                            ));
                        }
                    }
                }
            }
        }

        // Native Python extensions are part of the command's runtime closure,
        // even though the entrypoint itself is a script.  Inspect each module
        // statically and add its selected providers to the same command-local
        // view; an unresolved extension dependency keeps the manifest
        // explicitly unverified.
        for module_dir in module_dirs {
            let module_root = staging_dir.join(module_dir);
            for module_path in collect_files(&module_root)? {
                if module_path.extension().and_then(|ext| ext.to_str()) != Some("so") {
                    continue;
                }
                let Some(module_inspection) = inspect_elf_with_extra_paths(
                    &module_path,
                    Some(staging_dir),
                    extra_search_dirs,
                )?
                else {
                    return Err(crate::error::Error::MalformedArchive(format!(
                        "native extension is not an ELF object: {}",
                        module_path.display()
                    )));
                };
                unresolved_libraries.extend(module_inspection.missing_libraries.iter().cloned());
                for needed in &module_inspection.needed_libraries {
                    let Some(path) = module_inspection.resolved_paths.get(needed).cloned() else {
                        continue;
                    };
                    let recorded_path = path
                        .strip_prefix(staging_dir)
                        .map(|relative| target_store_dir.join(relative))
                        .unwrap_or_else(|_| path.clone());
                    if !needed.contains('/') {
                        if let Some(existing) = library_view.get(needed)
                            && existing != &path
                        {
                            return Err(crate::error::Error::IncompatibleHost(format!(
                                "incompatible providers selected for the same library identity '{}': {} and {}",
                                needed,
                                existing.display(),
                                path.display()
                            )));
                        }
                        library_view.insert(needed.clone(), path.clone());
                    }
                    closure.push(recorded_path.to_string_lossy().into_owned());
                    let provider_metadata = inspect_elf_with_extra_paths(
                        &path,
                        path.starts_with(staging_dir).then_some(staging_dir),
                        extra_search_dirs,
                    )?;
                    providers.push(ProviderEvidence {
                        requirement: needed.clone(),
                        provider_origin: if path.starts_with(staging_dir) {
                            format!("store:{}", target_store_dir.display())
                        } else {
                            "host:system".into()
                        },
                        provider_path: recorded_path,
                        capability: Capability::SharedLibrary(needed.clone()),
                        soname: provider_metadata
                            .as_ref()
                            .and_then(|metadata| metadata.soname.clone())
                            .or_else(|| Some(needed.clone())),
                        symbol_versions: module_inspection
                            .symbol_versions
                            .get(needed)
                            .cloned()
                            .unwrap_or_default(),
                        architecture: host.architecture.clone(),
                        digest: fs::read(&path).ok().map(|bytes| digest_bytes(&bytes)),
                        reason: "static ELF Python extension resolution".into(),
                    });
                }
            }
        }

        let strategy = if interpreter.is_some() {
            LaunchStrategy::InterpreterAdapter
        } else if elf_inspection.as_ref().is_some_and(|inspection| {
            inspection.interpreter.is_none() && inspection.needed_libraries.is_empty()
        }) {
            LaunchStrategy::Direct
        } else if elf_inspection.is_some() {
            LaunchStrategy::NativeRunner
        } else {
            LaunchStrategy::Direct
        };
        let mut environment = BTreeMap::new();
        // The runtime ID is a logical reference.  Absolute paths are recorded
        // in the manifest but are never interpolated into a shell command.
        environment.insert("PKG_PROFILE_RUNTIME".into(), "managed".into());
        let execution = ExecutionPlan {
            command: binary.command.clone(),
            executable,
            payload_digest: fs::read(&full_path).ok().map(|bytes| digest_bytes(&bytes)),
            interpreter,
            interpreter_args,
            argv: Vec::new(),
            environment,
            strategy,
            providers,
            closure,
            adaptations: Vec::new(),
            optional_omissions: unresolved_libraries
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|library| format!("unresolved-library:{library}"))
                .collect(),
        };
        let mut host_facts = crate::host::HostFacts::release_metadata();
        host_facts.extend(crate::host::HostFacts::loader_metadata());
        host_facts.insert("os".into(), host.os.clone());
        host_facts.insert("architecture".into(), host.architecture.to_string());
        host_facts.insert("distro".into(), host.distro_id.clone().unwrap_or_default());
        if let Some(id_like) = &host.distro_id_like {
            host_facts.insert("distro_like".into(), id_like.clone());
        }
        let mut runtime = RuntimeManifest {
            schema_version: crate::domain::contracts::CONTRACT_SCHEMA_VERSION,
            verified: execution.optional_omissions.is_empty(),
            runtime_id: String::new(),
            execution: execution.clone(),
            library_view,
            module_roots: module_dirs.to_vec(),
            runner_version: native_runner_version(),
            runner_digest: None,
            runner_target: None,
            host_facts,
            references: vec![target_store_dir.to_string_lossy().into_owned()],
        };
        runtime.runtime_id = runtime_identity(&runtime);
        executions.push(execution);
        runtimes.push(runtime);
    }
    Ok((executions, runtimes))
}

/// Interpreter families whose shebang contract is stable when launched with
/// an absolute host executable and package-local script path.  Native module
/// discovery remains adapter-specific (currently Python); admitting a family
/// here only covers script argument/environment semantics.
fn supported_interpreter_family(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "dash" | "r" | "rscript")
        || name.starts_with("python")
        || name.starts_with("perl")
        || name.starts_with("ruby")
        || name.starts_with("node")
}

/// Computes a runtime identity from the complete manifest contract while
/// excluding the identity field itself.  Runner/version/host and view changes
/// therefore cannot silently reuse an older runtime object.
pub fn runtime_identity(manifest: &RuntimeManifest) -> String {
    let mut identity = manifest.clone();
    identity.runtime_id.clear();
    // Staging paths and realized runtime-view symlinks are transaction-local
    // materialization details.  Identity uses the provider paths recorded in
    // the execution evidence so promotion cannot change the runtime ID.
    identity.library_view = identity
        .execution
        .providers
        .iter()
        .map(|provider| (provider.requirement.clone(), provider.provider_path.clone()))
        .collect();
    digest_serialized(&identity)
}

/// Executes a frozen command contract with only the runtime environment it
/// declares.  This is the native runner boundary: callers do not need to
/// export a profile-wide `LD_LIBRARY_PATH`, and ambient loader controls are
/// removed before a payload process is started.
pub fn execute_command(
    runtime: &RuntimeManifest,
    store_root: &Path,
    args: &[std::ffi::OsString],
) -> Result<std::process::ExitStatus> {
    Ok(prepare_command(runtime, store_root, args)?.status()?)
}

/// Executes a frozen command contract while capturing its streams for a
/// machine-readable caller.  This path is opt-in; the normal CLI path keeps
/// inherited streams and exact signal semantics.
pub fn execute_command_capture(
    runtime: &RuntimeManifest,
    store_root: &Path,
    args: &[std::ffi::OsString],
) -> Result<std::process::Output> {
    Ok(prepare_command(runtime, store_root, args)?.output()?)
}

/// Replaces the caller with the runtime process on Unix.  Using `exec` for the
/// CLI entry point preserves signals and the exact exit status instead of
/// translating a child signal into an arbitrary parent exit code.
#[cfg(unix)]
pub fn exec_command(
    runtime: &RuntimeManifest,
    store_root: &Path,
    args: &[std::ffi::OsString],
) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let error = prepare_command(runtime, store_root, args)?.exec();
    Err(error.into())
}

fn prepare_command(
    runtime: &RuntimeManifest,
    store_root: &Path,
    args: &[std::ffi::OsString],
) -> Result<std::process::Command> {
    let executable = store_root.join(&runtime.execution.executable);
    if !executable.is_file() {
        return Err(crate::error::Error::IncompatibleHost(format!(
            "Runtime executable is unavailable: {}",
            executable.display()
        )));
    }
    if !runtime.verified {
        return Err(crate::error::Error::IncompatibleHost(format!(
            "runtime {} is unverified: unresolved requirements were retained",
            runtime.runtime_id
        )));
    }

    if let Some(expected_digest) = &runtime.execution.payload_digest {
        let target = if matches!(runtime.execution.strategy, LaunchStrategy::NativeRunner) {
            runtime
                .runner_target
                .as_ref()
                .map(|path| store_root.join(path))
                .unwrap_or_else(|| executable.clone())
        } else {
            executable.clone()
        };
        let actual_digest = digest_bytes(&fs::read(&target)?);
        if &actual_digest != expected_digest {
            return Err(crate::error::Error::TransactionRecoveryRequired(
                "realized payload executable was modified".into(),
            ));
        }
    }

    if matches!(runtime.execution.strategy, LaunchStrategy::NativeRunner) {
        let Some(expected_digest) = runtime.runner_digest.as_deref() else {
            return Err(crate::error::Error::TransactionRecoveryRequired(
                "native runtime has no bootstrap digest".into(),
            ));
        };
        let actual_digest = digest_bytes(&fs::read(&executable)?);
        if actual_digest != expected_digest {
            return Err(crate::error::Error::TransactionRecoveryRequired(
                "native runner bootstrap was modified".into(),
            ));
        }
        let Some(target) = runtime.runner_target.as_ref() else {
            return Err(crate::error::Error::TransactionRecoveryRequired(
                "native runtime has no runner target".into(),
            ));
        };
        let config_path = executable.with_extension("conf");
        let expected_config = format!(
            "target={}\nlibrary_path={}\n",
            target.to_string_lossy(),
            store_root
                .parent()
                .and_then(Path::parent)
                .unwrap_or(store_root)
                .join("runtimes")
                .join(runtime.runtime_id.trim_start_matches("sha256:"))
                .join("lib")
                .to_string_lossy()
        );
        if fs::read_to_string(&config_path).ok().as_deref() != Some(expected_config.as_str()) {
            return Err(crate::error::Error::TransactionRecoveryRequired(
                "native runner configuration was modified".into(),
            ));
        }
    }

    // Revalidate the exact provider evidence at launch.  This catches a
    // changed host library or a modified store object before the loader sees
    // it, while keeping the command-local view deterministic.
    for provider in &runtime.execution.providers {
        let Some(view_path) = runtime.library_view.get(&provider.requirement) else {
            continue;
        };
        let Some(metadata) = inspect_elf(view_path, None)? else {
            return Err(crate::error::Error::TransactionRecoveryRequired(format!(
                "runtime provider is no longer an ELF object: {}",
                view_path.display()
            )));
        };
        let Some((expected_machine, expected_class_bits)) =
            crate::host::elf::expected_elf_abi(&provider.architecture)
        else {
            return Err(crate::error::Error::IncompatibleHost(format!(
                "runtime provider architecture is not supported for ELF validation: {}",
                provider.architecture
            )));
        };
        if metadata.machine != expected_machine
            || metadata.class_bits != expected_class_bits
            || !metadata.little_endian
        {
            return Err(crate::error::Error::IncompatibleHost(format!(
                "runtime provider ABI changed: {}",
                view_path.display()
            )));
        }
        if let Some(expected_soname) = &provider.soname
            && metadata
                .soname
                .as_deref()
                .is_some_and(|actual| actual != expected_soname)
        {
            return Err(crate::error::Error::IncompatibleHost(format!(
                "runtime provider SONAME changed: {}",
                view_path.display()
            )));
        }
        if let Some(expected_digest) = &provider.digest {
            let actual_digest = digest_bytes(&fs::read(view_path)?);
            if &actual_digest != expected_digest {
                return Err(crate::error::Error::TransactionRecoveryRequired(format!(
                    "runtime provider bytes changed: {}",
                    view_path.display()
                )));
            }
        }
        if !provider
            .symbol_versions
            .iter()
            .all(|version| metadata.defined_symbol_versions.contains(version))
        {
            return Err(crate::error::Error::IncompatibleHost(format!(
                "runtime provider symbol versions changed: {}",
                view_path.display()
            )));
        }
    }

    let mut command = if let Some(interpreter) = &runtime.execution.interpreter {
        let mut command = std::process::Command::new(interpreter);
        command.args(&runtime.execution.interpreter_args);
        command.arg(&executable);
        command
    } else {
        std::process::Command::new(&executable)
    };
    command.args(args);

    // Loader-related variables are process controls, not package inputs.  A
    // caller can still provide ordinary application configuration through the
    // command's declared environment map.
    for name in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "LD_AUDIT",
        "LD_DEBUG",
        "LD_DEBUG_OUTPUT",
        "LD_ORIGIN_PATH",
        "LD_PROFILE",
        "LD_USE_LOAD_BIAS",
    ] {
        command.env_remove(name);
    }
    for (name, value) in &runtime.execution.environment {
        command.env(name, value);
    }
    if !runtime.library_view.is_empty() {
        let mut dirs = BTreeSet::new();
        for provider in runtime.library_view.values() {
            if !provider.exists() {
                return Err(crate::error::Error::TransactionRecoveryRequired(format!(
                    "runtime provider disappeared before launch: {}",
                    provider.display()
                )));
            }
            if let Some(parent) = provider.parent() {
                dirs.insert(parent.to_path_buf());
            }
        }
        let joined = std::env::join_paths(dirs).map_err(|e| {
            crate::error::Error::Internal(format!("invalid runtime library view: {e}"))
        })?;
        command.env("LD_LIBRARY_PATH", joined);
    }
    Ok(command)
}

fn script_interpreter(path: &std::path::Path) -> Option<(PathBuf, Vec<String>)> {
    let bytes = fs::read(path).ok()?;
    let line_end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .unwrap_or(bytes.len());
    let shebang = bytes.get(..line_end)?.strip_prefix(b"#!")?;
    let line = String::from_utf8_lossy(shebang);
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    if first.ends_with("/env") {
        let next = parts.next()?;
        if next == "-S" {
            let interpreter = parts.next().map(PathBuf::from)?;
            Some((
                interpreter,
                parts
                    .map(PathBuf::from)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            ))
        } else {
            Some((PathBuf::from(next), parts.map(str::to_string).collect()))
        }
    } else {
        Some((PathBuf::from(first), parts.map(str::to_string).collect()))
    }
}

/// Inspects extracted binaries and generates scoped launchers where needed.
pub fn prepare_launchers(
    staging_dir: &Path,
    binaries: &mut [BinaryActivation],
    extracted_files: &mut Vec<PathBuf>,
) -> Result<()> {
    prepare_launchers_with_plans(staging_dir, binaries, extracted_files).map(|_| ())
}

/// Same launcher preparation as [`prepare_launchers`], while returning the
/// exact interpreter contracts used to generate each launcher.  Install uses
/// these plans to persist the adapter evidence instead of trying to infer it
/// from the generated `/bin/sh` wrapper after the fact.
pub fn prepare_launchers_with_plans(
    staging_dir: &Path,
    binaries: &mut [BinaryActivation],
    extracted_files: &mut Vec<PathBuf>,
) -> Result<BTreeMap<String, ScriptLauncherPlan>> {
    // 1. Collect all python site-packages / dist-packages relative directory paths
    let python_module_dirs = detect_python_module_dirs(extracted_files);
    let mut plans = BTreeMap::new();

    // 2. For each binary, check if runtime adaptation is required
    for binary in binaries.iter_mut() {
        let script_full_path = staging_dir.join(&binary.relative_store_path);
        if let Some(plan) = inspect_script(
            &binary.command,
            &binary.relative_store_path,
            &script_full_path,
            &python_module_dirs,
        ) {
            let resolved_interpreter = Path::new(&plan.interpreter);
            if !resolved_interpreter.is_file() {
                return Err(crate::error::Error::IncompatibleHost(format!(
                    "script interpreter is unavailable: {}",
                    plan.interpreter
                )));
            }
            if let Some(requested) = script_interpreter(&script_full_path)
                .and_then(|(path, _)| path.file_name().map(|name| name.to_owned()))
                .and_then(|name| name.to_str().map(str::to_ascii_lowercase))
                .filter(|name| name.starts_with("python"))
                && !python_interpreter_matches_request(&requested, resolved_interpreter)
            {
                return Err(crate::error::Error::IncompatibleHost(format!(
                    "requested Python interpreter '{}' is unavailable; selected {}",
                    requested, plan.interpreter
                )));
            }
            let launcher_dir = staging_dir.join(".pkg-launcher");
            fs::create_dir_all(&launcher_dir)?;
            let launcher_rel_path = PathBuf::from(".pkg-launcher").join(&binary.command);
            let launcher_file = staging_dir.join(&launcher_rel_path);

            let content = generate_launcher_script(&plan);
            fs::write(&launcher_file, content)?;

            let mut perms = fs::metadata(&launcher_file)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&launcher_file, perms)?;

            binary.relative_store_path = launcher_rel_path.clone();
            if !extracted_files.contains(&launcher_rel_path) {
                extracted_files.push(launcher_rel_path);
            }
            plans.insert(binary.command.clone(), plan);
        }
    }

    Ok(plans)
}

/// Keeps a versioned Python shebang tied to the same interpreter minor.  A
/// generic `python3` request may use any host Python 3 executable, while
/// `python3.12` must not silently run under `python3.11`.
fn python_interpreter_matches_request(requested: &str, resolved: &Path) -> bool {
    let Some(resolved_name) = resolved.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let requested_suffix = requested.strip_prefix("python").unwrap_or_default();
    if requested_suffix.contains('.')
        && requested_suffix
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
    {
        return resolved_name == requested;
    }
    if requested_suffix.starts_with('3') {
        return resolved_name.starts_with("python3");
    }
    resolved_name.starts_with("python")
}

/// Detects any Python `site-packages` or `dist-packages` directory paths in the payload.
#[must_use]
pub fn detect_python_module_dirs(extracted_files: &[PathBuf]) -> Vec<PathBuf> {
    let mut set = BTreeSet::new();
    for file in extracted_files {
        let mut curr = file.as_path();
        while let Some(parent) = curr.parent() {
            if let Some(file_name) = parent.file_name() {
                if file_name == "site-packages" || file_name == "dist-packages" {
                    set.insert(parent.to_path_buf());
                    break;
                }
            }
            curr = parent;
        }
    }
    set.into_iter().collect()
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(crate::error::Error::SecurityViolation(format!(
            "runtime module root is a symlink: {}",
            root.display()
        )));
    }
    if metadata.is_file() {
        files.push(root.to_path_buf());
        return Ok(files);
    }
    if !metadata.is_dir() {
        return Ok(files);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let entry_metadata = fs::symlink_metadata(&path)?;
        if entry_metadata.file_type().is_symlink() {
            continue;
        }
        if entry_metadata.is_dir() {
            files.extend(collect_files(&path)?);
        } else if entry_metadata.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

/// Inspects a file to determine if it requires a runtime launcher.
#[must_use]
pub fn inspect_script(
    command: &str,
    relative_store_path: &Path,
    full_path: &Path,
    python_module_dirs: &[PathBuf],
) -> Option<ScriptLauncherPlan> {
    // Only inspect regular files or symlinks resolving to regular files
    let Ok(bytes) = fs::read(full_path) else {
        return None;
    };
    if bytes.len() < 2 || bytes[0] != b'#' || bytes[1] != b'!' {
        return None;
    }

    // Read first line as shebang
    let newline_idx = bytes
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(bytes.len());
    let first_line = String::from_utf8_lossy(&bytes[..newline_idx]);
    let shebang = first_line.trim_start_matches("#!").trim();
    if shebang.is_empty() {
        return None;
    }

    let parts: Vec<&str> = shebang.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // Determine interpreter
    let (raw_interp, is_env, interpreter_args) = if parts[0].ends_with("/env") && parts.len() > 1 {
        if parts.get(1) == Some(&"-S") && parts.len() > 2 {
            (
                parts[2],
                true,
                parts[3..].iter().map(|part| (*part).to_string()).collect(),
            )
        } else {
            (
                parts[1],
                true,
                parts[2..].iter().map(|part| (*part).to_string()).collect(),
            )
        }
    } else {
        (
            parts[0],
            false,
            parts[1..].iter().map(|part| (*part).to_string()).collect(),
        )
    };

    let interp_file_name = Path::new(raw_interp)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Case 1: Python script
    if interp_file_name.starts_with("python") {
        let host_interp_exists = !is_env && Path::new(raw_interp).is_file();
        let needs_interpreter_normalization = !host_interp_exists;
        let needs_pythonpath = !python_module_dirs.is_empty();

        if needs_interpreter_normalization || needs_pythonpath {
            let resolved_interp = resolve_python_interpreter(raw_interp);
            let mut env_vars = Vec::new();
            if needs_pythonpath {
                env_vars.push(ScopedEnvVar {
                    name: "PYTHONPATH".to_string(),
                    paths: python_module_dirs.to_vec(),
                });
            }
            return Some(ScriptLauncherPlan {
                command: command.to_string(),
                target_script_rel_path: relative_store_path.to_path_buf(),
                interpreter: resolved_interp,
                interpreter_args,
                env_vars,
            });
        }
    }

    // Case 2: interpreters selected by `/usr/bin/env` always get a launcher
    // so the contract records an absolute host interpreter and preserves
    // shebang arguments.  This covers the reviewed generic families (shell,
    // Perl, Ruby, R and Node) without silently relying on PATH at runtime.
    if is_env {
        if let Some(resolved) = resolve_general_interpreter(interp_file_name) {
            return Some(ScriptLauncherPlan {
                command: command.to_string(),
                target_script_rel_path: relative_store_path.to_path_buf(),
                interpreter: resolved,
                interpreter_args,
                env_vars: Vec::new(),
            });
        }
    // Case 3: an absolute interpreter path that is no longer present can be
    // normalized only when the host has a reviewed executable with this name.
    } else if !Path::new(raw_interp).is_file()
        && let Some(resolved) = resolve_general_interpreter(interp_file_name)
    {
        return Some(ScriptLauncherPlan {
            command: command.to_string(),
            target_script_rel_path: relative_store_path.to_path_buf(),
            interpreter: resolved,
            interpreter_args,
            env_vars: Vec::new(),
        });
    }

    None
}

/// Resolves an available Python 3 interpreter on the host system.
#[must_use]
pub fn resolve_python_interpreter(requested: &str) -> String {
    // If requested path exists directly, use it
    if Path::new(requested).is_file() {
        return requested.to_string();
    }

    // Prefer python3 on host
    let candidates = ["/usr/bin/python3", "/bin/python3", "/usr/local/bin/python3"];
    for cand in candidates {
        if Path::new(cand).is_file() {
            return cand.to_string();
        }
    }

    // Check PATH
    if let Some(path_os) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_os) {
            let p = dir.join("python3");
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }

    // Fallback
    "/usr/bin/python3".to_string()
}

/// Resolves standard host interpreters (e.g. bash, sh).
#[must_use]
pub fn resolve_general_interpreter(name: &str) -> Option<String> {
    match name {
        "bash" => {
            let candidates = ["/usr/bin/bash", "/bin/bash"];
            candidates
                .into_iter()
                .find(|p| Path::new(p).is_file())
                .map(|s| s.to_string())
        }
        "sh" => {
            let candidates = ["/bin/sh", "/usr/bin/sh"];
            candidates
                .into_iter()
                .find(|p| Path::new(p).is_file())
                .map(|s| s.to_string())
        }
        name @ ("python" | "python3" | "perl" | "ruby" | "R" | "Rscript" | "node" | "nodejs") => {
            std::env::var_os("PATH").and_then(|path_os| {
                std::env::split_paths(&path_os)
                    .map(|dir| dir.join(name))
                    .find(|path| path.is_file())
                    .map(|path| path.to_string_lossy().into_owned())
            })
        }
        other => {
            if let Some(path_os) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&path_os) {
                    let p = dir.join(other);
                    if p.is_file() {
                        return Some(p.to_string_lossy().into_owned());
                    }
                }
            }
            None
        }
    }
}

/// Generates a POSIX-compliant symlink-safe shell launcher script.
#[must_use]
pub fn generate_launcher_script(plan: &ScriptLauncherPlan) -> String {
    let mut script = String::new();
    script.push_str("#!/bin/sh\n");
    script.push_str("# Generated by pkg runtime adapter (scoped launcher)\n");
    script.push_str("TARGET=\"$0\"\n");
    script.push_str("case \"$TARGET\" in\n");
    script.push_str("    */*) ;;\n");
    script.push_str("    *) TARGET=\"$(command -v -- \"$TARGET\")\" ;;\n");
    script.push_str("esac\n");
    script.push_str("while [ -L \"$TARGET\" ]; do\n");
    script.push_str("    DIR=\"$(CDPATH= cd -- \"$(dirname -- \"$TARGET\")\" && pwd)\"\n");
    script.push_str("    TARGET=\"$(readlink \"$TARGET\")\"\n");
    script.push_str("    case \"$TARGET\" in\n");
    script.push_str("        /*) ;;\n");
    script.push_str("        *) TARGET=\"$DIR/$TARGET\" ;;\n");
    script.push_str("    esac\n");
    script.push_str("done\n");
    script.push_str("LAUNCHER_DIR=\"$(CDPATH= cd -- \"$(dirname -- \"$TARGET\")\" && pwd)\"\n");
    script.push_str("BASE_DIR=\"$(CDPATH= cd -- \"$LAUNCHER_DIR/..\" && pwd)\"\n\n");

    for env_var in &plan.env_vars {
        if !env_var.paths.is_empty() {
            let expanded_paths: Vec<String> = env_var
                .paths
                .iter()
                .map(|p| {
                    format!(
                        "$BASE_DIR/{}",
                        shell_double_quote_fragment(&p.to_string_lossy())
                    )
                })
                .collect();
            let joined = expanded_paths.join(":");
            script.push_str(&format!(
                "export {}=\"{}${{{}:+:${}}}\"\n",
                env_var.name, joined, env_var.name, env_var.name
            ));
        }
    }

    script.push_str(&format!(
        "exec {}{} \"$BASE_DIR/{}\" \"$@\"\n",
        shell_single_quote(&plan.interpreter),
        plan.interpreter_args
            .iter()
            .map(|arg| format!(" {}", shell_single_quote(arg)))
            .collect::<String>(),
        shell_double_quote_fragment(&plan.target_script_rel_path.to_string_lossy())
    ));

    script
}

fn shell_single_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn shell_double_quote_fragment(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_python_module_dirs() {
        let extracted = vec![
            PathBuf::from("usr/bin/reflector"),
            PathBuf::from("usr/lib/python3.14/site-packages/Reflector.py"),
            PathBuf::from("usr/lib/python3.14/site-packages/reflector/main.py"),
            PathBuf::from("etc/xdg/reflector/reflector.conf"),
        ];
        let dirs = detect_python_module_dirs(&extracted);
        assert_eq!(
            dirs,
            vec![PathBuf::from("usr/lib/python3.14/site-packages")]
        );
    }

    #[test]
    fn test_generate_launcher_script() {
        let plan = ScriptLauncherPlan {
            command: "reflector".to_string(),
            target_script_rel_path: PathBuf::from("usr/bin/reflector"),
            interpreter: "/usr/bin/python3".to_string(),
            interpreter_args: Vec::new(),
            env_vars: vec![ScopedEnvVar {
                name: "PYTHONPATH".to_string(),
                paths: vec![PathBuf::from("usr/lib/python3.14/site-packages")],
            }],
        };
        let script = generate_launcher_script(&plan);
        assert!(script.contains("export PYTHONPATH=\"$BASE_DIR/usr/lib/python3.14/site-packages${PYTHONPATH:+:$PYTHONPATH}\""));
        assert!(script.contains("exec /usr/bin/python3 \"$BASE_DIR/usr/bin/reflector\" \"$@\""));
        assert!(script.contains("LAUNCHER_DIR="));
    }
}
