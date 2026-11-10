use vertex_i18n::tr;
use super::*;

#[allow(dead_code)]
pub(crate) fn pack_managed_path_keys(
    live_manifest: &ContentInstallManifest,
    base_manifest: &ContentInstallManifest,
) -> std::collections::HashSet<String> {
    live_manifest
        .projects
        .values()
        .filter(|project| project.pack_managed)
        .map(|project| managed_content::normalize_content_path_key(project.file_path.as_path()))
        .chain(base_manifest.projects.values().map(|project| {
            managed_content::normalize_content_path_key(project.file_path.as_path())
        }))
        .collect()
}

#[allow(dead_code)]
pub(crate) fn preserve_non_pack_managed_content(
    existing_root: &Path,
    temp_root: &Path,
    pack_managed_paths: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for folder in ["mods", "resourcepacks", "shaderpacks", "datapacks"] {
        let current_dir = existing_root.join(folder);
        let Ok(entries) = fs::read_dir(current_dir.as_path()) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(|err| tr!("failed to read directory entry: {err}", err = err).into_owned())?;
            let source_path = entry.path();
            let relative_path = source_path
                .strip_prefix(existing_root)
                .unwrap_or(source_path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let relative_key =
                managed_content::normalize_content_path_key(Path::new(relative_path.as_str()));
            if pack_managed_paths.contains(relative_key.as_str()) {
                continue;
            }
            let destination = temp_root.join(relative_path.as_str());
            copy_path_recursive(source_path.as_path(), destination.as_path())?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn preserve_instance_user_state(
    existing_root: &Path,
    temp_root: &Path,
) -> Result<(), String> {
    let saves_root = existing_root.join("saves");
    if saves_root.exists() {
        copy_path_recursive(saves_root.as_path(), temp_root.join("saves").as_path())?;
    }
    let servers_dat = existing_root.join("servers.dat");
    if servers_dat.exists() {
        copy_path_recursive(
            servers_dat.as_path(),
            temp_root.join("servers.dat").as_path(),
        )?;
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn copy_path_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    if source.is_dir() {
        fs_create_dir_all_logged(destination)
            .map_err(|err| tr!("failed to create {destination}: {err}", destination = destination.display(), err = err).into_owned())?;
        let entries = fs_read_dir_logged(source)
            .map_err(|err| tr!("failed to read {source}: {err}", source = source.display(), err = err).into_owned())?;
        for entry in entries {
            let entry = entry.map_err(|err| tr!("failed to read directory entry: {err}", err = err).into_owned())?;
            copy_path_recursive(
                entry.path().as_path(),
                destination.join(entry.file_name()).as_path(),
            )?;
        }
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs_create_dir_all_logged(parent)
            .map_err(|err| tr!("failed to create {parent}: {err}", parent = parent.display(), err = err).into_owned())?;
    }
    fs_copy_logged(source, destination).map_err(|err| {
        tr!("failed to copy {source} to {destination}: {err}", source = source.display(), destination = destination.display(), err = err).into_owned()
    })?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn swap_instance_root(existing_root: &Path, temp_root: &Path) -> Result<(), String> {
    let backup_root = existing_root.with_extension("modpack-update-backup");
    if backup_root.exists() {
        fs_remove_dir_all_logged(backup_root.as_path()).map_err(|err| {
            tr!("failed to remove stale backup {backup_root}: {err}", backup_root = backup_root.display(), err = err).into_owned()
        })?;
    }
    fs_rename_logged(existing_root, backup_root.as_path()).map_err(|err| {
        tr!("failed to stage old instance root {existing_root}: {err}", existing_root = existing_root.display(), err = err).into_owned()
    })?;
    if let Err(err) = fs_rename_logged(temp_root, existing_root) {
        let _ = fs_rename_logged(backup_root.as_path(), existing_root);
        return Err(tr!("failed to activate updated instance root {existing_root}: {err}", existing_root = existing_root.display(), err = err).into_owned());
    }
    fs_remove_dir_all_logged(backup_root.as_path()).map_err(|err| {
        tr!("failed to remove update backup {backup_root}: {err}", backup_root = backup_root.display(), err = err).into_owned()
    })?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn unique_temp_instance_root(installations_root: &Path, instance_id: &str) -> PathBuf {
    for attempt in 0..1024_u32 {
        let candidate =
            installations_root.join(format!(".vertex-modpack-update-{instance_id}-{attempt}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    installations_root.join(format!(".vertex-modpack-update-{instance_id}-overflow"))
}
