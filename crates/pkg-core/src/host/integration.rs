//! Explicit, user-space desktop/icon/MIME integration.
//!
//! The module only plans and materializes typed actions. It never executes
//! package lifecycle scripts or writes system-wide locations.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::domain::integration::{
    IntegrationAction, IntegrationConflict, IntegrationKind, IntegrationPlan,
};
use crate::domain::package::ArtifactDigest;
use crate::error::{Error, Result};

/// A persisted ownership record needed to reverse one integration action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationOwnership {
    pub kind: IntegrationKind,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub source_digest: ArtifactDigest,
}

/// Returns the user data root used for host integration.
pub fn user_data_root() -> Result<PathBuf> {
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
        .ok_or_else(|| Error::IncompatibleHost("HOME/XDG_DATA_HOME is unavailable".into()))?;
    validate_user_root(&root)?;
    Ok(root)
}

/// Builds a read-only integration plan from an immutable store object.
pub fn plan(
    profile: &str,
    package_name: &str,
    store_id: &str,
    store_path: &Path,
) -> Result<IntegrationPlan> {
    let data_root = user_data_root()?;
    plan_with_root(profile, package_name, store_id, store_path, &data_root)
}

fn plan_with_root(
    profile: &str,
    package_name: &str,
    store_id: &str,
    store_path: &Path,
    data_root: &Path,
) -> Result<IntegrationPlan> {
    crate::store::StoreLayout::validate_component(profile, "profile")?;
    crate::store::StoreLayout::validate_component(package_name, "package")?;
    crate::store::StoreLayout::validate_component(store_id, "store id")?;
    let store_path = fs::canonicalize(store_path)?;
    if !store_path.is_dir() {
        return Err(Error::IncompatibleHost(format!(
            "store object is not a directory: {}",
            store_path.display()
        )));
    }
    let mut actions = Vec::new();
    discover(&store_path, &store_path, data_root, &mut actions)?;
    actions.sort_by(|a, b| a.target_path.cmp(&b.target_path));

    let mut conflicts = Vec::new();
    for action in &actions {
        match fs::symlink_metadata(&action.target_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&action.target_path)?;
                let resolved = resolve_link_target(&action.target_path, &target)?;
                if resolved != action.source_path {
                    conflicts.push(IntegrationConflict {
                        kind: action.kind,
                        target_path: action.target_path.clone(),
                        reason: format!(
                            "existing symlink points to {}, expected {}",
                            resolved.display(),
                            action.source_path.display()
                        ),
                    });
                }
            }
            Ok(_) => conflicts.push(IntegrationConflict {
                kind: action.kind,
                target_path: action.target_path.clone(),
                reason: "target is an existing regular or special file".into(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(IntegrationPlan {
        profile: profile.to_string(),
        package_name: package_name.to_string(),
        store_id: store_id.to_string(),
        actions,
        conflicts,
    })
}

/// Materializes a conflict-free plan as symlinks in the user's data root.
/// Returns only links newly created by this call so callers can roll back a
/// partially completed action list.
pub fn apply(plan: &IntegrationPlan) -> Result<Vec<IntegrationAction>> {
    if !plan.conflicts.is_empty() {
        return Err(Error::ActivationConflict {
            command: plan.conflicts[0].target_path.display().to_string(),
            existing_package: plan.conflicts[0].reason.clone(),
        });
    }
    let raw_data_root = user_data_root()?;
    apply_with_root(plan, &raw_data_root)
}

fn apply_with_root(plan: &IntegrationPlan, raw_data_root: &Path) -> Result<Vec<IntegrationAction>> {
    if !raw_data_root.exists() {
        fs::create_dir_all(raw_data_root)?;
    }
    let data_root = raw_data_root.canonicalize()?;
    let mut created = Vec::new();
    for action in &plan.actions {
        let newly_created = match apply_one(action, &data_root) {
            Ok(created) => created,
            Err(error) => {
                rollback_created(&created);
                return Err(error);
            }
        };
        if newly_created {
            created.push(action.clone());
        }
    }
    Ok(created)
}

/// Removes persisted integrations, failing closed if a user replaced one.
pub fn remove(ownership: &[IntegrationOwnership]) -> Result<Vec<PathBuf>> {
    validate_removal(ownership)?;
    let mut removed = Vec::new();
    for item in ownership {
        match fs::symlink_metadata(&item.target_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&item.target_path)?;
                let resolved = resolve_link_target(&item.target_path, &target)?;
                if resolved != item.source_path {
                    return Err(Error::ActivationConflict {
                        command: item.target_path.display().to_string(),
                        existing_package: "user-replaced integration target".into(),
                    });
                }
                fs::remove_file(&item.target_path)?;
                removed.push(item.target_path.clone());
            }
            Ok(_) => {
                return Err(Error::ActivationConflict {
                    command: item.target_path.display().to_string(),
                    existing_package: "integration target is no longer a symlink".into(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(removed)
}

/// Checks all ownership links without changing the host, so callers can do
/// this before a larger package transaction begins.
pub fn validate_removal(ownership: &[IntegrationOwnership]) -> Result<()> {
    for item in ownership {
        validate_integration_target(&item.target_path, item.kind)?;
        match fs::symlink_metadata(&item.target_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&item.target_path)?;
                let resolved = resolve_link_target(&item.target_path, &target)?;
                if resolved != item.source_path {
                    return Err(Error::ActivationConflict {
                        command: item.target_path.display().to_string(),
                        existing_package: "user-replaced integration target".into(),
                    });
                }
            }
            Ok(_) => {
                return Err(Error::ActivationConflict {
                    command: item.target_path.display().to_string(),
                    existing_package: "integration target is no longer a symlink".into(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn apply_one(action: &IntegrationAction, data_root: &Path) -> Result<bool> {
    validate_integration_target(&action.target_path, action.kind)?;
    let source = fs::canonicalize(&action.source_path)?;
    if !source.is_file() {
        return Err(Error::SecurityViolation(format!(
            "integration source is not a regular file: {}",
            source.display()
        )));
    }
    let parent = action
        .target_path
        .parent()
        .ok_or_else(|| Error::SecurityViolation("integration target has no parent".into()))?;
    ensure_directory(parent, data_root)?;
    match fs::symlink_metadata(&action.target_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(&action.target_path)?;
            let resolved = resolve_link_target(&action.target_path, &target)?;
            if resolved == action.source_path {
                return Ok(false);
            }
            return Err(Error::ActivationConflict {
                command: action.target_path.display().to_string(),
                existing_package: "integration target is owned by another source".into(),
            });
        }
        Ok(_) => {
            return Err(Error::ActivationConflict {
                command: action.target_path.display().to_string(),
                existing_package: "integration target is occupied".into(),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    std::os::unix::fs::symlink(source, &action.target_path)?;
    Ok(true)
}

fn discover(
    root: &Path,
    current: &Path,
    data_root: &Path,
    actions: &mut Vec<IntegrationAction>,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            discover(root, &path, data_root, actions)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Error::SecurityViolation("integration source escapes store".into()))?;
        let Some((kind, target_relative)) = integration_target(relative) else {
            continue;
        };
        validate_resource(&path, kind)?;
        let target = data_root.join(target_relative);
        validate_relative_target(&target, data_root)?;
        actions.push(IntegrationAction {
            kind,
            source_path: fs::canonicalize(&path)?,
            target_path: target,
            source_digest: ArtifactDigest::from_file(&path)?,
        });
    }
    Ok(())
}

fn validate_resource(path: &Path, kind: IntegrationKind) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > 4 * 1024 * 1024 {
        return Err(Error::LimitsExceeded(format!(
            "integration resource is too large: {}",
            path.display()
        )));
    }
    match kind {
        IntegrationKind::DesktopEntry => {
            let content = fs::read_to_string(path).map_err(|error| {
                Error::MalformedArchive(format!("desktop entry is not UTF-8: {error}"))
            })?;
            let mut section = false;
            let mut type_value = None;
            let mut name_value = None;
            let mut exec_value = None;
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    section = line == "[Desktop Entry]";
                    continue;
                }
                if !section || line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    match key.trim() {
                        "Type" => type_value = Some(value.trim()),
                        "Name" => name_value = Some(value.trim()),
                        "Exec" => exec_value = Some(value.trim()),
                        _ => {}
                    }
                }
            }
            if type_value.is_none_or(str::is_empty) || name_value.is_none_or(str::is_empty) {
                return Err(Error::MalformedArchive(format!(
                    "desktop entry lacks Type or Name: {}",
                    path.display()
                )));
            }
            if type_value == Some("Application") && exec_value.is_none_or(str::is_empty) {
                return Err(Error::MalformedArchive(format!(
                    "application desktop entry lacks Exec: {}",
                    path.display()
                )));
            }
        }
        IntegrationKind::MimePackage => {
            let content = fs::read_to_string(path).map_err(|error| {
                Error::MalformedArchive(format!("MIME package is not UTF-8: {error}"))
            })?;
            if content.contains("<!DOCTYPE") || content.contains("<!ENTITY") {
                return Err(Error::SecurityViolation(format!(
                    "MIME package cannot declare external entities: {}",
                    path.display()
                )));
            }
            if !content.contains("<mime-info") {
                return Err(Error::MalformedArchive(format!(
                    "MIME package lacks mime-info root: {}",
                    path.display()
                )));
            }
        }
        IntegrationKind::Icon => {}
    }
    Ok(())
}

fn integration_target(relative: &Path) -> Option<(IntegrationKind, PathBuf)> {
    let mut components = relative.components();
    if !matches!(components.next()?, Component::Normal(name) if name == "usr")
        || !matches!(components.next()?, Component::Normal(name) if name == "share")
    {
        return None;
    }
    let rest = components.collect::<PathBuf>();
    let first = rest.components().next()?;
    let kind = match first {
        Component::Normal(name) if name == "applications" => {
            if rest.extension().is_some_and(|ext| ext == "desktop") {
                IntegrationKind::DesktopEntry
            } else {
                return None;
            }
        }
        Component::Normal(name) if name == "icons" => IntegrationKind::Icon,
        Component::Normal(name) if name == "mime" => {
            if !rest.starts_with("mime/packages")
                || !rest.extension().is_some_and(|ext| ext == "xml")
            {
                return None;
            }
            IntegrationKind::MimePackage
        }
        _ => return None,
    };
    Some((kind, rest))
}

fn resolve_link_target(link: &Path, target: &Path) -> Result<PathBuf> {
    let candidate = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link.parent().unwrap_or_else(|| Path::new("/")).join(target)
    };
    Ok(fs::canonicalize(candidate)?)
}

fn ensure_directory(path: &Path, root: &Path) -> Result<()> {
    let root = fs::canonicalize(root)?;
    let relative = path.strip_prefix(&root).map_err(|_| {
        Error::SecurityViolation("integration directory escapes user data root".into())
    })?;
    let mut current = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(Error::SecurityViolation(
                "invalid integration directory".into(),
            ));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(Error::SecurityViolation(format!(
                    "integration parent is not a directory: {}",
                    current.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn validate_user_root(root: &Path) -> Result<()> {
    if !root.is_absolute() || root == Path::new("/") {
        return Err(Error::SecurityViolation(format!(
            "integration data root must be a non-root absolute path: {}",
            root.display()
        )));
    }
    if ["/etc", "/usr", "/var", "/bin", "/lib", "/sbin", "/opt"]
        .iter()
        .any(|prefix| root == Path::new(prefix) || root.starts_with(Path::new(prefix)))
    {
        return Err(Error::SecurityViolation(format!(
            "system path cannot be used for user integration: {}",
            root.display()
        )));
    }
    Ok(())
}

fn validate_relative_target(path: &Path, root: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| Error::SecurityViolation("integration target escapes data root".into()))?;
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(Error::SecurityViolation(
                "integration target has unsafe path".into(),
            ));
        }
    }
    Ok(())
}

fn validate_integration_target(path: &Path, kind: IntegrationKind) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error::SecurityViolation(format!(
            "integration target must be absolute: {}",
            path.display()
        )));
    }
    let marker = match kind {
        IntegrationKind::DesktopEntry => "applications",
        IntegrationKind::Icon => "icons",
        IntegrationKind::MimePackage => "mime",
    };
    let mut root = PathBuf::new();
    let mut found = false;
    for component in path.components() {
        if matches!(component, Component::Normal(name) if name == marker) {
            found = true;
            break;
        }
        root.push(component.as_os_str());
    }
    if !found {
        return Err(Error::SecurityViolation(format!(
            "integration target is outside its user data namespace: {}",
            path.display()
        )));
    }
    validate_user_root(&root)
}

fn rollback_created(created: &[IntegrationAction]) {
    for action in created.iter().rev() {
        if let Ok(target) = fs::read_link(&action.target_path) {
            if target == action.source_path {
                let _ = fs::remove_file(&action.target_path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn plans_and_reverses_desktop_and_icon_links() {
        let temp = tempdir().unwrap();
        let store = temp.path().join("store");
        fs::create_dir_all(store.join("usr/share/applications")).unwrap();
        fs::create_dir_all(store.join("usr/share/icons/hicolor/48x48/apps")).unwrap();
        fs::write(
            store.join("usr/share/applications/tool.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Tool\nExec=tool\n",
        )
        .unwrap();
        fs::write(
            store.join("usr/share/icons/hicolor/48x48/apps/tool.png"),
            b"png",
        )
        .unwrap();
        let data_root = temp.path().join("data");
        fs::create_dir_all(&data_root).unwrap();
        let plan = plan_with_root("p", "tool", "store-id", &store, &data_root).unwrap();
        assert_eq!(plan.actions.len(), 2);
        assert!(plan.conflicts.is_empty());
        let created = apply_with_root(&plan, &data_root).unwrap();
        assert_eq!(created.len(), 2);
        let ownership = plan
            .actions
            .iter()
            .map(|action| IntegrationOwnership {
                kind: action.kind,
                source_path: action.source_path.clone(),
                target_path: action.target_path.clone(),
                source_digest: action.source_digest.clone(),
            })
            .collect::<Vec<_>>();
        remove(&ownership).unwrap();
        assert!(!temp.path().join("data/applications/tool.desktop").exists());
    }

    #[test]
    fn rejects_invalid_desktop_and_mime_resources() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        fs::create_dir_all(&data_root).unwrap();

        let invalid_desktop = temp.path().join("invalid-desktop");
        fs::create_dir_all(invalid_desktop.join("usr/share/applications")).unwrap();
        fs::write(
            invalid_desktop.join("usr/share/applications/broken.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Broken\n",
        )
        .unwrap();
        let error =
            plan_with_root("p", "broken", "broken-id", &invalid_desktop, &data_root).unwrap_err();
        assert!(error.to_string().contains("lacks Exec"));

        let invalid_mime = temp.path().join("invalid-mime");
        fs::create_dir_all(invalid_mime.join("usr/share/mime/packages")).unwrap();
        fs::write(
            invalid_mime.join("usr/share/mime/packages/broken.xml"),
            b"<!DOCTYPE mime-info [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]>\n<mime-info/>",
        )
        .unwrap();
        let error = plan_with_root(
            "p",
            "broken-mime",
            "broken-mime-id",
            &invalid_mime,
            &data_root,
        )
        .unwrap_err();
        assert!(error.to_string().contains("external entities"));
    }

    #[test]
    fn reports_existing_user_target_as_conflict() {
        let temp = tempdir().unwrap();
        let store = temp.path().join("store");
        fs::create_dir_all(store.join("usr/share/applications")).unwrap();
        fs::write(
            store.join("usr/share/applications/tool.desktop"),
            b"[Desktop Entry]\nType=Application\nName=Tool\nExec=tool\n",
        )
        .unwrap();
        let data_root = temp.path().join("data");
        fs::create_dir_all(data_root.join("applications")).unwrap();
        fs::write(data_root.join("applications/tool.desktop"), b"user-owned").unwrap();

        let plan = plan_with_root("p", "tool", "store-id", &store, &data_root).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        assert!(plan.conflicts[0].reason.contains("existing regular"));
    }

    #[test]
    fn rejects_system_data_roots() {
        for root in [
            Path::new("/"),
            Path::new("/usr/share"),
            Path::new("/etc/pkg"),
        ] {
            assert!(validate_user_root(root).is_err(), "{}", root.display());
        }
        assert!(validate_user_root(Path::new("/tmp/pkg-user-data")).is_ok());
        assert!(
            validate_integration_target(
                Path::new("/usr/share/applications/tool.desktop"),
                IntegrationKind::DesktopEntry,
            )
            .is_err()
        );
    }

    #[test]
    fn packages_without_supported_desktop_resources_produce_no_actions() {
        let temp = tempdir().unwrap();
        let store = temp.path().join("store");
        fs::create_dir_all(store.join("usr/bin")).unwrap();
        fs::write(store.join("usr/bin/tool"), b"#!/bin/sh\n").unwrap();
        let data_root = temp.path().join("data");
        fs::create_dir_all(&data_root).unwrap();

        let plan = plan_with_root("p", "tool", "store-id", &store, &data_root).unwrap();
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn versioned_gui_corpus_covers_supported_actions_and_rejections() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("data");
        fs::create_dir_all(&data_root).unwrap();

        let valid_store = temp.path().join("valid-store");
        fs::create_dir_all(valid_store.join("usr/share/applications")).unwrap();
        fs::create_dir_all(valid_store.join("usr/share/icons/hicolor/48x48/apps")).unwrap();
        fs::create_dir_all(valid_store.join("usr/share/mime/packages")).unwrap();
        fs::write(
            valid_store.join("usr/share/applications/fixture.desktop"),
            include_bytes!("../../fixtures/gui/valid.desktop"),
        )
        .unwrap();
        fs::write(
            valid_store.join("usr/share/icons/hicolor/48x48/apps/fixture.png"),
            include_bytes!("../../fixtures/gui/valid-icon.png"),
        )
        .unwrap();
        fs::write(
            valid_store.join("usr/share/mime/packages/fixture.xml"),
            include_bytes!("../../fixtures/gui/valid-mime.xml"),
        )
        .unwrap();

        let plan = plan_with_root(
            "default",
            "fixture-gui",
            "fixture-store",
            &valid_store,
            &data_root,
        )
        .unwrap();
        assert_eq!(plan.actions.len(), 3);
        assert!(plan.conflicts.is_empty());
        let created = apply_with_root(&plan, &data_root).unwrap();
        assert_eq!(created.len(), 3);
        let ownership = plan
            .actions
            .iter()
            .map(|action| IntegrationOwnership {
                kind: action.kind,
                source_path: action.source_path.clone(),
                target_path: action.target_path.clone(),
                source_digest: action.source_digest.clone(),
            })
            .collect::<Vec<_>>();
        remove(&ownership).unwrap();

        let invalid_store = temp.path().join("invalid-store");
        fs::create_dir_all(invalid_store.join("usr/share/applications")).unwrap();
        fs::write(
            invalid_store.join("usr/share/applications/invalid.desktop"),
            include_bytes!("../../fixtures/gui/invalid.desktop"),
        )
        .unwrap();
        let error = plan_with_root(
            "default",
            "invalid-gui",
            "invalid-store",
            &invalid_store,
            &data_root,
        )
        .unwrap_err();
        assert!(error.to_string().contains("lacks Exec"));

        let empty_store = temp.path().join("empty-store");
        fs::create_dir_all(empty_store.join("usr/share/doc")).unwrap();
        fs::write(
            empty_store.join("usr/share/doc/README"),
            include_bytes!("../../fixtures/gui/no-integration"),
        )
        .unwrap();
        let empty = plan_with_root(
            "default",
            "no-integration",
            "empty-store",
            &empty_store,
            &data_root,
        )
        .unwrap();
        assert!(empty.actions.is_empty());
    }
}
