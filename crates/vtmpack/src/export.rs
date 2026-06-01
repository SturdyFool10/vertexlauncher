use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use managed_content::{
    CONTENT_MANIFEST_FILE_NAME, ContentInstallManifest, InstalledContentProject,
    ManagedContentSource, content_manifest_path,
};

use crate::constants::VTMPACK_MANIFEST_VERSION;
use crate::{
    VtmpackCompressionMode, VtmpackDownloadableEntry, VtmpackExportOptions, VtmpackExportProgress,
    VtmpackExportStats, VtmpackInstanceMetadata, VtmpackManifest, VtmpackProviderMode,
};

const XZ_PRESET_STANDARD: u32 = 6;
const XZ_PRESET_EXTREME_FLAG: u32 = 1 << 31;
const XZ_PRESET_EXTREME: u32 = 9 | XZ_PRESET_EXTREME_FLAG;

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
    // Read the raw manifest without filesystem validation so that managed content
    // entries are preserved in downloadable_content even if their files are currently
    // missing on disk (e.g. not yet synced, or deleted by the user).
    let managed_manifest = {
        let path = content_manifest_path(instance_root);
        let manifest = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str::<ContentInstallManifest>(&raw).ok())
            .unwrap_or_default();
        manifest_with_disabled_mod_paths(instance_root, &manifest)
    };
    let selected_root_entries = options
        .included_root_entries
        .iter()
        .filter_map(|(entry, included)| included.then_some(entry.as_str()))
        .collect::<HashSet<_>>();

    progress(progress_update(
        "Checking mods against Modrinth hashes...",
        1,
        1,
    ));
    let rediscovered_manifest =
        rediscover_modrinth_mods(instance_root, &managed_manifest, &selected_root_entries);
    let sanitized_managed_manifest =
        sanitize_managed_manifest_for_export(&rediscovered_manifest, options);

    let downloadable_entries = sanitized_managed_manifest
        .projects
        .iter()
        .filter_map(|(project_key, project)| {
            let normalized_path = normalize_pack_path(project.file_path.as_path());
            if normalized_path.as_os_str().is_empty() {
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
                selected_file_sha1: project.selected_file_sha1.clone(),
                selected_file_sha512: project.selected_file_sha512.clone(),
            })
        })
        .collect::<Vec<_>>();

    let downloadable_entry_count = downloadable_entries.len();
    let downloadable_paths = downloadable_entries
        .iter()
        .map(|entry| normalize_pack_path(entry.file_path.as_path()))
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
            let normalized = normalize_pack_path(relative.as_path());
            if !downloadable_paths.contains(&normalized) {
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
            normalize_pack_path(Path::new("bundled_mods").join(relative_from_mods).as_path())
        })
        .collect::<Vec<_>>();
    let config_paths = config_files
        .iter()
        .map(|path| {
            let relative_from_configs = path
                .strip_prefix(configs_dir.as_path())
                .unwrap_or(path.as_path());
            normalize_pack_path(Path::new("configs").join(relative_from_configs).as_path())
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
            normalize_pack_path(relative)
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
    let total_steps = 3 + bundled_mod_files.len() + config_files.len() + additional_files.len();
    let mut completed_steps = 0usize;
    let bundled_mod_file_count = pack_manifest.bundled_mods.len();
    let config_file_count = pack_manifest.configs.len();
    let additional_file_count = pack_manifest.additional_paths.len();

    let encoder = xz2::write::XzEncoder::new(
        output_file,
        xz_preset_for_compression_mode(options.compression_mode),
    );
    let mut archive = tar::Builder::new(encoder);
    write_tar_payload(
        &mut archive,
        &pack_manifest,
        &sanitized_managed_manifest,
        bundled_mod_files,
        mods_dir.as_path(),
        config_files,
        configs_dir.as_path(),
        additional_files,
        instance_root,
        total_steps,
        &mut completed_steps,
        &mut progress,
    )?;
    progress(progress_update(
        "Finalizing XZ archive...",
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
        bundled_mod_files: bundled_mod_file_count,
        downloadable_mod_files: downloadable_entry_count,
        config_files: config_file_count,
        additional_files: additional_file_count,
    })
}

fn xz_preset_for_compression_mode(mode: VtmpackCompressionMode) -> u32 {
    match mode {
        VtmpackCompressionMode::Standard => XZ_PRESET_STANDARD,
        VtmpackCompressionMode::Extreme => XZ_PRESET_EXTREME,
    }
}

#[allow(clippy::too_many_arguments)]
fn write_tar_payload<W, F>(
    archive: &mut tar::Builder<W>,
    pack_manifest: &VtmpackManifest,
    sanitized_managed_manifest: &ContentInstallManifest,
    bundled_mod_files: Vec<PathBuf>,
    mods_dir: &Path,
    config_files: Vec<PathBuf>,
    configs_dir: &Path,
    additional_files: Vec<PathBuf>,
    instance_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut F,
) -> Result<(), String>
where
    W: Write,
    F: FnMut(VtmpackExportProgress),
{
    let manifest_bytes = toml::to_string_pretty(pack_manifest)
        .map_err(|err| format!("failed to serialize vtmpack manifest: {err}"))?
        .into_bytes();
    progress(progress_update(
        "Writing pack manifest...",
        *completed_steps,
        total_steps,
    ));
    append_bytes_to_archive(archive, "manifest.toml", manifest_bytes.as_slice())?;
    *completed_steps += 1;

    if !sanitized_managed_manifest.projects.is_empty() {
        let raw = toml::to_string_pretty(sanitized_managed_manifest)
            .map_err(|err| format!("failed to serialize export content manifest: {err}"))?;
        progress(progress_update(
            "Writing content metadata...",
            *completed_steps,
            total_steps,
        ));
        append_bytes_to_archive(
            archive,
            "metadata/vertex-content-manifest.toml",
            raw.as_bytes(),
        )?;
    } else {
        progress(progress_update(
            "Skipping empty content metadata...",
            *completed_steps,
            total_steps,
        ));
    }
    *completed_steps += 1;

    for file in bundled_mod_files {
        let relative = file.strip_prefix(mods_dir).unwrap_or(file.as_path());
        let target = Path::new("bundled_mods").join(relative);
        progress(progress_update(
            &format!("Bundling mod {}", target.display()),
            *completed_steps,
            total_steps,
        ));
        archive
            .append_path_with_name(file.as_path(), target.as_path())
            .map_err(|err| format!("failed to append bundled mod {}: {err}", file.display()))?;
        *completed_steps += 1;
    }

    for file in config_files {
        let relative = file.strip_prefix(configs_dir).unwrap_or(file.as_path());
        let target = Path::new("configs").join(relative);
        progress(progress_update(
            &format!("Bundling config {}", target.display()),
            *completed_steps,
            total_steps,
        ));
        archive
            .append_path_with_name(file.as_path(), target.as_path())
            .map_err(|err| format!("failed to append config file {}: {err}", file.display()))?;
        *completed_steps += 1;
    }

    for file in additional_files {
        let relative = file.strip_prefix(instance_root).unwrap_or(file.as_path());
        let target = Path::new("root_entries").join(relative);
        progress(progress_update(
            &format!("Bundling {}", target.display()),
            *completed_steps,
            total_steps,
        ));
        archive
            .append_path_with_name(file.as_path(), target.as_path())
            .map_err(|err| format!("failed to append extra file {}: {err}", file.display()))?;
        *completed_steps += 1;
    }

    Ok(())
}

pub(crate) fn manifest_with_disabled_mod_paths(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
) -> ContentInstallManifest {
    let mut manifest = manifest.clone();
    for project in manifest.projects.values_mut() {
        let normalized_path = normalize_pack_path(project.file_path.as_path());
        let path_text = normalized_path.to_string_lossy();
        if !path_text.starts_with("mods/") || path_text.ends_with(".DISABLED") {
            continue;
        }
        if instance_root.join(normalized_path.as_path()).exists() {
            continue;
        }
        let Some(file_name) = normalized_path.file_name() else {
            continue;
        };
        let disabled_path =
            normalized_path.with_file_name(format!("{}.DISABLED", file_name.to_string_lossy()));
        if instance_root.join(disabled_path.as_path()).is_file() {
            project.file_path = disabled_path;
        }
    }
    manifest
}

pub(crate) fn rediscover_modrinth_mods(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    selected_root_entries: &HashSet<&str>,
) -> ContentInstallManifest {
    if !selected_root_entries.contains("mods") {
        return manifest.clone();
    }

    let mods_dir = instance_root.join("mods");
    if !mods_dir.is_dir() {
        return manifest.clone();
    }

    let entries = match fs::read_dir(mods_dir.as_path()) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/vtmpack",
                path = %mods_dir.display(),
                error = %err,
                "failed to read mods directory for Modrinth hash rediscovery"
            );
            return manifest.clone();
        }
    };

    let known_modrinth_paths = manifest
        .projects
        .values()
        .filter(|project| project.selected_source == Some(ManagedContentSource::Modrinth))
        .filter(|project| {
            project
                .selected_version_id
                .as_deref()
                .is_some_and(|version| !version.trim().is_empty())
        })
        .map(|project| normalize_pack_path(project.file_path.as_path()))
        .collect::<HashSet<_>>();

    let mut mod_files = Vec::<DiscoveredModFile>::new();
    for entry in entries.flatten() {
        let absolute_path = entry.path();
        if !absolute_path.is_file() {
            continue;
        }
        let relative_path = absolute_path
            .strip_prefix(instance_root)
            .unwrap_or(absolute_path.as_path());
        let file_path = normalize_pack_path(relative_path);
        if file_path.as_os_str().is_empty() || known_modrinth_paths.contains(&file_path) {
            continue;
        }
        match modrinth::hash_file_sha1_and_sha512_hex(absolute_path.as_path()) {
            Ok((sha1, sha512)) => mod_files.push(DiscoveredModFile {
                absolute_path,
                file_path,
                sha1,
                sha512,
            }),
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/vtmpack",
                    path = %absolute_path.display(),
                    error = %err,
                    "failed to hash mod file for Modrinth rediscovery"
                );
            }
        }
    }

    if mod_files.is_empty() {
        return manifest.clone();
    }

    let client = modrinth::Client::default();
    let sha512_hashes = mod_files
        .iter()
        .map(|file| file.sha512.clone())
        .collect::<Vec<_>>();
    let version_matches = match client.get_versions_from_hashes(&sha512_hashes, "sha512") {
        Ok(matches) => matches,
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/vtmpack",
                error = %err,
                "Modrinth hash rediscovery failed"
            );
            HashMap::new()
        }
    };

    let project_ids = version_matches
        .values()
        .map(|version| version.project_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let projects_by_id = client
        .get_projects(&project_ids)
        .map(|projects| {
            projects
                .into_iter()
                .map(|project| (project.project_id.clone(), project))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_else(|err| {
            tracing::warn!(
                target: "vertexlauncher/vtmpack",
                error = %err,
                "failed to fetch Modrinth projects for rediscovered mods"
            );
            HashMap::new()
        });

    let mut projects = manifest.projects.clone();
    let mut path_keys = projects
        .iter()
        .map(|(key, project)| {
            (
                normalize_pack_path(project.file_path.as_path()),
                key.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    for mod_file in mod_files {
        let Some(version) = version_matches.get(mod_file.sha512.as_str()) else {
            continue;
        };
        if !version.files.iter().any(|file| {
            file.hashes
                .get("sha512")
                .is_some_and(|hash| hash.eq_ignore_ascii_case(mod_file.sha512.as_str()))
        }) {
            tracing::warn!(
                target: "vertexlauncher/vtmpack",
                path = %mod_file.absolute_path.display(),
                version_id = %version.id,
                "Modrinth hash lookup returned a version without the exact matched file hash; leaving file bundled"
            );
            continue;
        }

        let project_key = path_keys
            .get(&mod_file.file_path)
            .cloned()
            .unwrap_or_else(|| {
                unique_project_key(&projects, format!("modrinth:{}", version.project_id))
            });
        let existing = projects.get(&project_key).cloned().unwrap_or_default();
        let project_name = projects_by_id
            .get(version.project_id.as_str())
            .map(|project| project.title.clone())
            .filter(|name| !name.trim().is_empty())
            .or_else(|| non_empty(existing.name.as_str()))
            .unwrap_or_else(|| {
                mod_file
                    .file_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Modrinth mod".to_owned())
            });
        let project = InstalledContentProject {
            project_key: project_key.clone(),
            name: project_name,
            folder_name: "mods".to_owned(),
            file_path: mod_file.file_path.clone(),
            modrinth_project_id: Some(version.project_id.clone()),
            curseforge_project_id: existing.curseforge_project_id,
            selected_source: Some(ManagedContentSource::Modrinth),
            selected_version_id: Some(version.id.clone()),
            selected_version_name: non_empty(version.version_number.as_str()),
            selected_file_sha1: Some(mod_file.sha1),
            selected_file_sha512: Some(mod_file.sha512),
            pack_managed: existing.pack_managed,
            explicitly_installed: existing.explicitly_installed,
            direct_dependencies: existing.direct_dependencies,
        };
        path_keys.insert(project.file_path.clone(), project_key.clone());
        projects.insert(project_key, project);
    }

    ContentInstallManifest { projects }
}

struct DiscoveredModFile {
    absolute_path: PathBuf,
    file_path: PathBuf,
    sha1: String,
    sha512: String,
}

fn unique_project_key(
    projects: &BTreeMap<String, InstalledContentProject>,
    preferred: String,
) -> String {
    if !projects.contains_key(&preferred) {
        return preferred;
    }
    let mut index = 2usize;
    loop {
        let candidate = format!("{preferred}:{index}");
        if !projects.contains_key(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
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
    let read_dir_result = fs::read_dir(instance_root);
    if let Err(ref err) = read_dir_result {
        tracing::warn!(
            target: "vertexlauncher/vtmpack",
            path = %instance_root.display(),
            error = %err,
            "failed to list instance root entries for vtmpack export"
        );
    }
    let mut entries = read_dir_result
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
    matches!(
        entry,
        "mods" | "resourcepacks" | "shaderpacks" | "config" | "kubejs" | "tacz"
    )
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

fn append_bytes_to_archive<W: Write>(
    archive: &mut tar::Builder<W>,
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

pub(crate) fn normalize_pack_path(path: &std::path::Path) -> PathBuf {
    let normalized = path
        .to_string_lossy()
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/");
    if normalized.is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(normalized)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use managed_content::{InstalledContentProject, ManagedContentSource};

    use super::*;

    #[test]
    fn export_remembers_disabled_managed_mod_paths() {
        let root = std::env::temp_dir().join(format!(
            "vertexlauncher-vtmpack-disabled-test-{}",
            std::process::id()
        ));
        let mods_dir = root.join("mods");
        let _ = std::fs::remove_dir_all(root.as_path());
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        std::fs::write(mods_dir.join("example.jar.DISABLED"), b"disabled mod")
            .expect("write disabled mod");

        let manifest = ContentInstallManifest {
            projects: BTreeMap::from([(
                "mod::example".to_owned(),
                InstalledContentProject {
                    name: "Example".to_owned(),
                    file_path: PathBuf::from("mods/example.jar"),
                    selected_source: Some(ManagedContentSource::Modrinth),
                    selected_version_id: Some("version".to_owned()),
                    ..InstalledContentProject::default()
                },
            )]),
        };

        let updated = manifest_with_disabled_mod_paths(root.as_path(), &manifest);
        assert_eq!(
            updated
                .projects
                .get("mod::example")
                .expect("project should remain")
                .file_path,
            PathBuf::from("mods/example.jar.DISABLED")
        );

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn exported_manifest_records_disabled_managed_mod_path() {
        let root = std::env::temp_dir().join(format!(
            "vertexlauncher-vtmpack-disabled-export-test-{}",
            std::process::id()
        ));
        let mods_dir = root.join("mods");
        let package_path = root.join("disabled-pack.vtmpack");
        let _ = std::fs::remove_dir_all(root.as_path());
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        std::fs::write(mods_dir.join("example.jar.DISABLED"), b"disabled mod")
            .expect("write disabled mod");

        let manifest = ContentInstallManifest {
            projects: BTreeMap::from([(
                "mod::example".to_owned(),
                InstalledContentProject {
                    project_key: "mod::example".to_owned(),
                    name: "Example".to_owned(),
                    file_path: PathBuf::from("mods/example.jar"),
                    modrinth_project_id: Some("example".to_owned()),
                    selected_source: Some(ManagedContentSource::Modrinth),
                    selected_version_id: Some("version".to_owned()),
                    ..InstalledContentProject::default()
                },
            )]),
        };
        std::fs::write(
            content_manifest_path(root.as_path()),
            toml::to_string_pretty(&manifest).expect("serialize content manifest"),
        )
        .expect("write content manifest");

        let mut included_root_entries = BTreeMap::new();
        included_root_entries.insert("mods".to_owned(), true);
        export_instance_as_vtmpack(
            &VtmpackInstanceMetadata {
                name: "Disabled Export".to_owned(),
                game_version: "1.21.1".to_owned(),
                modloader: "Fabric".to_owned(),
                ..VtmpackInstanceMetadata::default()
            },
            root.as_path(),
            package_path.as_path(),
            &VtmpackExportOptions {
                provider_mode: VtmpackProviderMode::ExcludeCurseForge,
                compression_mode: VtmpackCompressionMode::Standard,
                included_root_entries,
            },
        )
        .expect("export vtmpack");

        let exported = crate::read_vtmpack_manifest(package_path.as_path()).expect("read vtmpack");
        assert_eq!(exported.downloadable_content.len(), 1);
        assert_eq!(
            exported.downloadable_content[0].file_path,
            PathBuf::from("mods/example.jar.DISABLED")
        );
        assert!(exported.bundled_mods.is_empty());

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn export_can_strip_curseforge_metadata() {
        let manifest = ContentInstallManifest {
            projects: BTreeMap::from([
                (
                    "mod::sodium".to_owned(),
                    InstalledContentProject {
                        name: "Sodium".to_owned(),
                        file_path: PathBuf::from("mods/sodium.jar"),
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
                        file_path: PathBuf::from("mods/embeddium.jar"),
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
                compression_mode: VtmpackCompressionMode::Standard,
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
