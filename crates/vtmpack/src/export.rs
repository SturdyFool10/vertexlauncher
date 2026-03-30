use std::{
    collections::HashSet,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use managed_content::{
    CONTENT_MANIFEST_FILE_NAME, ContentInstallManifest, ManagedContentSource, load_content_manifest,
};

use crate::constants::VTMPACK_MANIFEST_VERSION;
use crate::{
    VtmpackDownloadableEntry, VtmpackExportOptions, VtmpackExportProgress, VtmpackExportStats,
    VtmpackInstanceMetadata, VtmpackManifest, VtmpackProviderMode,
};

pub fn sanitize_managed_manifest_for_export(
    manifest: &ContentInstallManifest,
    options: &VtmpackExportOptions,
) -> ContentInstallManifest {
    match options.provider_mode {
        VtmpackProviderMode::IncludeCurseForge => manifest.clone(),
        VtmpackProviderMode::ExcludeCurseForge => {
            let mut sanitized = ContentInstallManifest::default();
            for (key, project) in &manifest.projects {
                if project.selected_source == Some(ManagedContentSource::CurseForge) {
                    continue;
                }

                let mut project = project.clone();
                project.curseforge_project_id = None;
                sanitized.projects.insert(key.clone(), project);
            }
            sanitized
        }
    }
}

pub fn export_instance_as_vtmpack(
    instance: &VtmpackInstanceMetadata,
    instance_root: &Path,
    output_path: &Path,
    options: &VtmpackExportOptions,
) -> Result<VtmpackExportStats, String> {
    export_instance_as_vtmpack_with_progress(instance, instance_root, output_path, options, |_| {})
}

pub fn export_instance_as_vtmpack_with_progress<F>(
    instance: &VtmpackInstanceMetadata,
    instance_root: &Path,
    output_path: &Path,
    options: &VtmpackExportOptions,
    mut progress: F,
) -> Result<VtmpackExportStats, String>
where
    F: FnMut(VtmpackExportProgress),
{
    progress(progress_update("Reading managed content manifest...", 0, 1));
    let managed_manifest = load_content_manifest(instance_root);
    let sanitized_managed_manifest =
        sanitize_managed_manifest_for_export(&managed_manifest, options);

    let downloadable_entries = sanitized_managed_manifest
        .projects
        .iter()
        .filter_map(|(project_key, project)| {
            let normalized_path = normalize_pack_path(project.file_path.as_str());
            if normalized_path.is_empty() {
                return None;
            }
            Some(VtmpackDownloadableEntry {
                project_key: if project.project_key.trim().is_empty() {
                    project_key.clone()
                } else {
                    project.project_key.clone()
                },
                name: project.name.clone(),
                file_path: normalized_path,
                modrinth_project_id: project.modrinth_project_id.clone(),
                curseforge_project_id: project.curseforge_project_id,
                selected_source: project
                    .selected_source
                    .map(|source| source.label().to_owned()),
                selected_version_id: project.selected_version_id.clone(),
                selected_version_name: project.selected_version_name.clone(),
            })
        })
        .collect::<Vec<_>>();

    let downloadable_paths = downloadable_entries
        .iter()
        .map(|entry| normalize_pack_path(entry.file_path.as_str()))
        .collect::<HashSet<_>>();

    let selected_root_entries = options
        .included_root_entries
        .iter()
        .filter_map(|(entry, included)| included.then_some(entry.as_str()))
        .collect::<HashSet<_>>();

    progress(progress_update("Scanning exportable files...", 1, 1));

    let mods_dir = instance_root.join("mods");
    let mut bundled_mod_files = Vec::<PathBuf>::new();
    if selected_root_entries.contains("mods") && mods_dir.exists() {
        let entries = fs::read_dir(mods_dir.as_path()).map_err(|err| {
            format!(
                "failed to read mods directory {}: {err}",
                mods_dir.display()
            )
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(instance_root)
                .unwrap_or(path.as_path())
                .to_path_buf();
            let normalized = normalize_pack_path(relative.display().to_string().as_str());
            if !downloadable_paths.contains(normalized.as_str()) {
                bundled_mod_files.push(path);
            }
        }
    }

    let configs_dir = instance_root.join("config");
    let mut config_files = Vec::<PathBuf>::new();
    if selected_root_entries.contains("config") && configs_dir.exists() {
        collect_regular_files_recursive(configs_dir.as_path(), &mut config_files).map_err(
            |err| {
                format!(
                    "failed to collect config files under {}: {err}",
                    configs_dir.display()
                )
            },
        )?;
    }

    let bundled_mod_paths = bundled_mod_files
        .iter()
        .map(|path| {
            let relative_from_mods = path
                .strip_prefix(mods_dir.as_path())
                .unwrap_or(path.as_path());
            normalize_pack_path(
                Path::new("bundled_mods")
                    .join(relative_from_mods)
                    .display()
                    .to_string()
                    .as_str(),
            )
        })
        .collect::<Vec<_>>();
    let config_paths = config_files
        .iter()
        .map(|path| {
            let relative_from_configs = path
                .strip_prefix(configs_dir.as_path())
                .unwrap_or(path.as_path());
            normalize_pack_path(
                Path::new("configs")
                    .join(relative_from_configs)
                    .display()
                    .to_string()
                    .as_str(),
            )
        })
        .collect::<Vec<_>>();
    let mut additional_files = Vec::<PathBuf>::new();
    for entry in selected_root_entries {
        if matches!(entry, "mods" | "config") {
            continue;
        }
        let root_entry_path = instance_root.join(entry);
        if !root_entry_path.exists() {
            continue;
        }
        if root_entry_path.is_dir() {
            collect_regular_files_recursive(root_entry_path.as_path(), &mut additional_files)
                .map_err(|err| {
                    format!(
                        "failed to collect files under {}: {err}",
                        root_entry_path.display()
                    )
                })?;
        } else if root_entry_path.is_file() {
            additional_files.push(root_entry_path);
        }
    }
    additional_files.sort();
    additional_files.dedup();
    let additional_paths = additional_files
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(instance_root).unwrap_or(path.as_path());
            normalize_pack_path(relative.display().to_string().as_str())
        })
        .collect::<Vec<_>>();

    let pack_manifest = VtmpackManifest {
        format: "vtmpack".to_owned(),
        version: VTMPACK_MANIFEST_VERSION,
        exported_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0),
        instance: instance.clone(),
        downloadable_content: downloadable_entries,
        bundled_mods: bundled_mod_paths,
        configs: config_paths,
        additional_paths,
    };

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %parent.display(), error = %err, context = "create vtmpack export directory");
            format!(
                "failed to create export directory {}: {err}",
                parent.display()
            )
        })?;
    }

    let output_file = fs::File::create(output_path)
        .map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "file_create", path = %output_path.display(), error = %err, context = "create vtmpack archive");
            format!("failed to create {}: {err}", output_path.display())
        })?;
    let encoder = xz2::write::XzEncoder::new(output_file, 9);
    let mut archive = tar::Builder::new(encoder);
    let total_steps = 3 + bundled_mod_files.len() + config_files.len() + additional_files.len();
    let mut completed_steps = 0usize;

    let manifest_bytes = toml::to_string_pretty(&pack_manifest)
        .map_err(|err| format!("failed to serialize vtmpack manifest: {err}"))?
        .into_bytes();
    progress(progress_update(
        "Writing pack manifest...",
        completed_steps,
        total_steps,
    ));
    append_bytes_to_archive(&mut archive, "manifest.toml", manifest_bytes.as_slice())?;
    completed_steps += 1;

    if !sanitized_managed_manifest.projects.is_empty() {
        let raw = toml::to_string_pretty(&sanitized_managed_manifest)
            .map_err(|err| format!("failed to serialize export content manifest: {err}"))?;
        progress(progress_update(
            "Writing content metadata...",
            completed_steps,
            total_steps,
        ));
        append_bytes_to_archive(
            &mut archive,
            "metadata/vertex-content-manifest.toml",
            raw.as_bytes(),
        )?;
    } else {
        progress(progress_update(
            "Skipping empty content metadata...",
            completed_steps,
            total_steps,
        ));
    }
    completed_steps += 1;

    for file in bundled_mod_files {
        let relative = file
            .strip_prefix(mods_dir.as_path())
            .unwrap_or(file.as_path());
        let target = Path::new("bundled_mods").join(relative);
        progress(progress_update(
            &format!("Bundling mod {}", target.display()),
            completed_steps,
            total_steps,
        ));
        archive
            .append_path_with_name(file.as_path(), target.as_path())
            .map_err(|err| format!("failed to append bundled mod {}: {err}", file.display()))?;
        completed_steps += 1;
    }

    for file in config_files {
        let relative = file
            .strip_prefix(configs_dir.as_path())
            .unwrap_or(file.as_path());
        let target = Path::new("configs").join(relative);
        progress(progress_update(
            &format!("Bundling config {}", target.display()),
            completed_steps,
            total_steps,
        ));
        archive
            .append_path_with_name(file.as_path(), target.as_path())
            .map_err(|err| format!("failed to append config file {}: {err}", file.display()))?;
        completed_steps += 1;
    }

    for file in additional_files {
        let relative = file.strip_prefix(instance_root).unwrap_or(file.as_path());
        let target = Path::new("root_entries").join(relative);
        progress(progress_update(
            &format!("Bundling {}", target.display()),
            completed_steps,
            total_steps,
        ));
        archive
            .append_path_with_name(file.as_path(), target.as_path())
            .map_err(|err| format!("failed to append extra file {}: {err}", file.display()))?;
        completed_steps += 1;
    }

    progress(progress_update(
        "Finalizing archive...",
        completed_steps,
        total_steps,
    ));
    archive
        .finish()
        .map_err(|err| format!("failed to finalize archive: {err}"))?;
    let encoder = archive
        .into_inner()
        .map_err(|err| format!("failed to flush archive stream: {err}"))?;
    encoder
        .finish()
        .map_err(|err| format!("failed to finalize xz stream: {err}"))?;
    completed_steps += 1;
    progress(progress_update(
        "Export complete.",
        completed_steps,
        total_steps,
    ));

    Ok(VtmpackExportStats {
        bundled_mod_files: pack_manifest.bundled_mods.len(),
        config_files: pack_manifest.configs.len(),
        additional_files: pack_manifest.additional_paths.len(),
    })
}

pub fn sync_vtmpack_export_options(instance_root: &Path, options: &mut VtmpackExportOptions) {
    let available_entries = list_exportable_root_entries(instance_root);
    let available_set = available_entries.iter().cloned().collect::<HashSet<_>>();
    options
        .included_root_entries
        .retain(|entry, _| available_set.contains(entry));
    for entry in available_entries {
        options
            .included_root_entries
            .entry(entry.clone())
            .or_insert_with(|| default_vtmpack_root_entry_selected(&entry));
    }
}

pub fn list_exportable_root_entries(instance_root: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(instance_root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.is_empty() || name == CONTENT_MANIFEST_FILE_NAME {
                return None;
            }
            if !(path.is_dir() || path.is_file()) {
                return None;
            }
            Some(name)
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        (
            !default_vtmpack_root_entry_selected(entry),
            entry.to_ascii_lowercase(),
        )
    });
    entries
}

pub fn default_vtmpack_root_entry_selected(entry: &str) -> bool {
    matches!(entry, "mods" | "resourcepacks" | "shaderpacks" | "config")
}

fn collect_regular_files_recursive(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let entries = fs::read_dir(root)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_regular_files_recursive(path.as_path(), out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn append_bytes_to_archive(
    archive: &mut tar::Builder<xz2::write::XzEncoder<fs::File>>,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, path, Cursor::new(bytes))
        .map_err(|err| format!("failed to append {path} to archive: {err}"))
}

fn progress_update(
    message: &str,
    completed_steps: usize,
    total_steps: usize,
) -> VtmpackExportProgress {
    VtmpackExportProgress {
        message: message.to_owned(),
        completed_steps,
        total_steps,
    }
}

fn normalize_pack_path(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use managed_content::{InstalledContentProject, ManagedContentSource};

    use super::*;

    #[test]
    fn export_can_strip_curseforge_metadata() {
        let manifest = ContentInstallManifest {
            projects: BTreeMap::from([
                (
                    "mod::sodium".to_owned(),
                    InstalledContentProject {
                        name: "Sodium".to_owned(),
                        file_path: "mods/sodium.jar".to_owned(),
                        modrinth_project_id: Some("AANobbMI".to_owned()),
                        curseforge_project_id: Some(394468),
                        selected_source: Some(ManagedContentSource::Modrinth),
                        ..InstalledContentProject::default()
                    },
                ),
                (
                    "mod::embeddium".to_owned(),
                    InstalledContentProject {
                        name: "Embeddium".to_owned(),
                        file_path: "mods/embeddium.jar".to_owned(),
                        curseforge_project_id: Some(908741),
                        selected_source: Some(ManagedContentSource::CurseForge),
                        ..InstalledContentProject::default()
                    },
                ),
            ]),
        };

        let sanitized = sanitize_managed_manifest_for_export(
            &manifest,
            &VtmpackExportOptions {
                provider_mode: VtmpackProviderMode::ExcludeCurseForge,
                included_root_entries: BTreeMap::new(),
            },
        );

        assert!(sanitized.projects.contains_key("mod::sodium"));
        assert_eq!(
            sanitized
                .projects
                .get("mod::sodium")
                .and_then(|project| project.curseforge_project_id),
            None
        );
        assert!(!sanitized.projects.contains_key("mod::embeddium"));
    }
}
