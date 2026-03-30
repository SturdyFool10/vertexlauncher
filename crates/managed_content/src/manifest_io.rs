use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::{
    CONTENT_MANIFEST_FILE_NAME, ContentInstallManifest, InstalledContentIdentity,
    MODPACK_STATE_FILE_NAME, ModpackInstallState,
};

#[track_caller]
fn fs_read_to_string(path: &Path) -> std::io::Result<String> {
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = std::fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_write(path: &Path, raw: &str) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    let result = std::fs::write(path, raw);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
fn fs_remove_file(path: &Path) -> std::io::Result<()> {
    tracing::debug!(target: "vertexlauncher/io", op = "remove_file", path = %path.display());
    let result = std::fs::remove_file(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err);
    }
    result
}

#[must_use]
pub fn content_manifest_path(instance_root: &Path) -> PathBuf {
    instance_root.join(CONTENT_MANIFEST_FILE_NAME)
}

#[must_use]
pub fn load_content_manifest(instance_root: &Path) -> ContentInstallManifest {
    let path = content_manifest_path(instance_root);
    let manifest = fs_read_to_string(path.as_path())
        .ok()
        .and_then(|raw| toml::from_str::<ContentInstallManifest>(&raw).ok())
        .unwrap_or_default();
    let mut normalized = manifest.clone();
    normalize_content_manifest(instance_root, &mut normalized);
    if normalized != manifest {
        let _ = save_content_manifest(instance_root, &normalized);
    }
    normalized
}

pub fn remove_content_manifest_entries_for_path(
    instance_root: &Path,
    content_path: &Path,
) -> Result<bool, String> {
    let mut manifest = load_content_manifest(instance_root);
    let normalized_target = content_path
        .strip_prefix(instance_root)
        .unwrap_or(content_path)
        .to_str()
        .map(normalize_content_path_key)
        .unwrap_or_default();
    if normalized_target.is_empty() {
        return Ok(false);
    }

    let previous_len = manifest.projects.len();
    manifest.projects.retain(|_, project| {
        normalize_content_path_key(project.file_path.as_str()) != normalized_target
    });
    if manifest.projects.len() == previous_len {
        return Ok(false);
    }

    save_content_manifest(instance_root, &manifest)?;
    Ok(true)
}

pub fn save_content_manifest(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
) -> Result<(), String> {
    let mut normalized = manifest.clone();
    normalize_content_manifest(instance_root, &mut normalized);
    let path = content_manifest_path(instance_root);
    if normalized.projects.is_empty() {
        if path.exists() {
            let _ = fs_remove_file(path.as_path());
        }
        return Ok(());
    }
    let raw = toml::to_string_pretty(&normalized)
        .map_err(|err| format!("failed to serialize content manifest: {err}"))?;
    fs_write(path.as_path(), raw.as_str())
        .map_err(|err| format!("failed to write content manifest {}: {err}", path.display()))
}

#[must_use]
pub fn modpack_install_state_path(instance_root: &Path) -> PathBuf {
    instance_root.join(MODPACK_STATE_FILE_NAME)
}

#[must_use]
pub fn load_modpack_install_state(instance_root: &Path) -> Option<ModpackInstallState> {
    let path = modpack_install_state_path(instance_root);
    let raw = fs_read_to_string(path.as_path()).ok()?;
    let mut state = toml::from_str::<ModpackInstallState>(raw.as_str()).ok()?;
    normalize_content_manifest(instance_root, &mut state.base_manifest);
    Some(state)
}

pub fn save_modpack_install_state(
    instance_root: &Path,
    state: &ModpackInstallState,
) -> Result<(), String> {
    let mut normalized = state.clone();
    normalize_content_manifest(instance_root, &mut normalized.base_manifest);
    let path = modpack_install_state_path(instance_root);
    if normalized.format.trim().is_empty() || normalized.version_id.trim().is_empty() {
        return remove_modpack_install_state(instance_root);
    }
    let raw = toml::to_string_pretty(&normalized)
        .map_err(|err| format!("failed to serialize modpack install state: {err}"))?;
    fs_write(path.as_path(), raw.as_str()).map_err(|err| {
        format!(
            "failed to write modpack install state {}: {err}",
            path.display()
        )
    })
}

pub fn remove_modpack_install_state(instance_root: &Path) -> Result<(), String> {
    let path = modpack_install_state_path(instance_root);
    if path.exists() {
        fs_remove_file(path.as_path()).map_err(|err| {
            format!(
                "failed to remove modpack install state {}: {err}",
                path.display()
            )
        })?;
    }
    Ok(())
}

#[must_use]
pub fn load_managed_content_identities(
    instance_root: &Path,
) -> HashMap<String, InstalledContentIdentity> {
    let manifest = load_content_manifest(instance_root);
    manifest
        .projects
        .into_values()
        .filter_map(|project| {
            let source = project.selected_source?;
            Some((
                normalize_content_path_key(project.file_path.as_str()),
                InstalledContentIdentity {
                    name: project.name,
                    file_path: project.file_path,
                    pack_managed: project.pack_managed,
                    source: source.into(),
                    modrinth_project_id: project.modrinth_project_id,
                    curseforge_project_id: project.curseforge_project_id,
                    selected_version_id: project.selected_version_id.unwrap_or_default(),
                },
            ))
        })
        .collect()
}

pub fn normalize_content_manifest(instance_root: &Path, manifest: &mut ContentInstallManifest) {
    let mut missing_keys = Vec::new();
    for (key, value) in &mut manifest.projects {
        if let Some(resolved_path) =
            resolve_content_manifest_path(instance_root, value.file_path.as_str())
        {
            value.file_path = resolved_path;
        } else {
            missing_keys.push(key.clone());
        }
    }
    for key in missing_keys {
        manifest.projects.remove(key.as_str());
    }

    let project_keys: std::collections::HashSet<String> =
        manifest.projects.keys().cloned().collect();
    for (key, value) in &mut manifest.projects {
        value.project_key = key.clone();
        value.file_path = normalize_content_path(value.file_path.as_str());
        value
            .direct_dependencies
            .retain(|dependency| dependency != key && project_keys.contains(dependency));
        value.direct_dependencies.sort();
        value.direct_dependencies.dedup();
        if value.selected_version_id.is_none() {
            value.selected_version_id = Some(String::new());
        }
        if value.selected_version_name.is_none() {
            value.selected_version_name = Some(String::new());
        }
    }
}

fn resolve_content_manifest_path(instance_root: &Path, value: &str) -> Option<String> {
    let normalized = normalize_content_path(value);
    if normalized.is_empty() {
        return None;
    }

    let exact_path = instance_root.join(normalized.as_str());
    if exact_path.exists() {
        return Some(normalized);
    }

    let mut current = instance_root.to_path_buf();
    let mut resolved_components = Vec::new();
    for component in Path::new(normalized.as_str()).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                let exact_child = current.join(part.as_ref());
                let resolved_part = if exact_child.exists() {
                    part.into_owned()
                } else {
                    std::fs::read_dir(current.as_path())
                        .ok()?
                        .filter_map(Result::ok)
                        .find_map(|entry| {
                            let file_name = entry.file_name();
                            let file_name = file_name.to_string_lossy();
                            file_name
                                .eq_ignore_ascii_case(part.as_ref())
                                .then(|| file_name.into_owned())
                        })?
                };
                current.push(resolved_part.as_str());
                resolved_components.push(resolved_part);
            }
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
        }
    }

    current.exists().then(|| resolved_components.join("/"))
}

fn normalize_content_path(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
}

#[must_use]
pub fn normalize_content_path_key(value: &str) -> String {
    normalize_content_path(value).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentInstallManifest, InstalledContentProject, ManagedContentSource};

    fn temp_test_root(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vertexlauncher-managed-content-test-{test_name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn normalize_content_manifest_preserves_actual_file_case() {
        let temp_root = temp_test_root("normalize-case");
        let mods_dir = temp_root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let jar_path = mods_dir.join("Sodium-Fabric.jar");
        std::fs::write(jar_path.as_path(), b"test").expect("write jar");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::sodium".to_owned(),
            InstalledContentProject {
                project_key: String::new(),
                name: "Sodium".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: "mods/sodium-fabric.jar".to_owned(),
                modrinth_project_id: Some("AANobbMI".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("abc123".to_owned()),
                selected_version_name: Some("1.0.0".to_owned()),
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );

        normalize_content_manifest(temp_root.as_path(), &mut manifest);

        let project = manifest
            .projects
            .get("mod::sodium")
            .expect("project should remain present");
        assert_eq!(project.file_path, "mods/Sodium-Fabric.jar");

        let _ = std::fs::remove_file(jar_path.as_path());
        let _ = std::fs::remove_dir_all(temp_root.as_path());
    }

    #[test]
    fn load_content_manifest_persists_normalized_entries() {
        let temp_root = temp_test_root("persist-normalized");
        let mods_dir = temp_root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let jar_path = mods_dir.join("Sodium-Fabric.jar");
        std::fs::write(jar_path.as_path(), b"test").expect("write jar");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::sodium".to_owned(),
            InstalledContentProject {
                project_key: String::new(),
                name: "Sodium".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: "mods/sodium-fabric.jar".to_owned(),
                modrinth_project_id: Some("AANobbMI".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("abc123".to_owned()),
                selected_version_name: Some("1.0.0".to_owned()),
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );
        manifest.projects.insert(
            "mod::missing".to_owned(),
            InstalledContentProject {
                project_key: String::new(),
                name: "Missing".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: "mods/missing.jar".to_owned(),
                modrinth_project_id: Some("missing".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("def456".to_owned()),
                selected_version_name: Some("2.0.0".to_owned()),
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );
        let raw = toml::to_string_pretty(&manifest).expect("serialize raw manifest");
        std::fs::write(content_manifest_path(temp_root.as_path()), raw).expect("write manifest");

        let loaded = load_content_manifest(temp_root.as_path());
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(
            loaded
                .projects
                .get("mod::sodium")
                .expect("normalized project should remain")
                .file_path,
            "mods/Sodium-Fabric.jar"
        );

        let persisted = std::fs::read_to_string(content_manifest_path(temp_root.as_path()))
            .expect("read normalized manifest");
        let persisted_manifest =
            toml::from_str::<ContentInstallManifest>(persisted.as_str()).expect("parse manifest");
        assert_eq!(persisted_manifest.projects.len(), 1);
        assert_eq!(
            persisted_manifest
                .projects
                .get("mod::sodium")
                .expect("normalized project should persist")
                .file_path,
            "mods/Sodium-Fabric.jar"
        );

        let _ = std::fs::remove_dir_all(temp_root.as_path());
    }

    #[test]
    fn remove_content_manifest_entries_for_path_updates_saved_dependencies() {
        let temp_root = temp_test_root("remove-path");
        let mods_dir = temp_root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let root_jar_path = mods_dir.join("Example.jar");
        let dep_jar_path = mods_dir.join("Core.jar");
        std::fs::write(root_jar_path.as_path(), b"root").expect("write root jar");
        std::fs::write(dep_jar_path.as_path(), b"dep").expect("write dep jar");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::example".to_owned(),
            InstalledContentProject {
                project_key: "mod::example".to_owned(),
                name: "Example".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: "mods/Example.jar".to_owned(),
                modrinth_project_id: Some("example".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("root".to_owned()),
                selected_version_name: Some("1.0.0".to_owned()),
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: vec!["mod::core".to_owned()],
            },
        );
        manifest.projects.insert(
            "mod::core".to_owned(),
            InstalledContentProject {
                project_key: "mod::core".to_owned(),
                name: "Core".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: "mods/Core.jar".to_owned(),
                modrinth_project_id: Some("core".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("dep".to_owned()),
                selected_version_name: Some("1.0.0".to_owned()),
                pack_managed: false,
                explicitly_installed: false,
                direct_dependencies: Vec::new(),
            },
        );
        std::fs::write(
            content_manifest_path(temp_root.as_path()),
            toml::to_string_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let removed =
            remove_content_manifest_entries_for_path(temp_root.as_path(), dep_jar_path.as_path())
                .expect("remove manifest entry");
        assert!(removed, "expected manifest entry removal");

        let persisted = load_content_manifest(temp_root.as_path());
        assert!(
            !persisted.projects.contains_key("mod::core"),
            "removed project should no longer exist"
        );
        assert!(
            persisted
                .projects
                .get("mod::example")
                .expect("root project should remain")
                .direct_dependencies
                .is_empty(),
            "dependency list should be pruned when an entry is removed"
        );

        let _ = std::fs::remove_dir_all(temp_root.as_path());
    }
}
