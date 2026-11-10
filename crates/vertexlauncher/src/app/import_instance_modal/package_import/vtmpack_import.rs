use vertex_i18n::tr;
use super::*;

pub(crate) fn import_vtmpack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err(tr!("Vertex pack import requires a manifest file source.").into_owned());
    };
    progress(import_progress(vertex_i18n::src!("Reading XZ .vtmpack manifest..."), 0, 1));
    let manifest = read_vtmpack_manifest_with_progress(package_path.as_path(), |_| {})?;
    let extract_steps = count_vtmpack_manifest_payload_entries(&manifest);
    let total_steps = 3 + extract_steps + manifest.downloadable_content.len();
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: None,
            thumbnail_path: None,
            modloader: default_if_blank(manifest.instance.modloader.as_str(), tr!("Vanilla").into_owned()),
            game_version: default_if_blank(
                manifest.instance.game_version.as_str(),
                "latest".to_owned(),
            ),
            modloader_version: manifest.instance.modloader_version.clone(),
        },
    )
    .map_err(|err| tr!("failed to create imported profile: {err}", err = err).into_owned())?;
    progress(import_progress(
        vertex_i18n::src!("Created imported profile. Restoring packaged files..."),
        1,
        total_steps,
    ));
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = populate_vtmpack_instance(
        package_path.as_path(),
        manifest,
        instance_root.as_path(),
        total_steps,
        progress,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    progress(import_progress(
        vertex_i18n::src!("Import complete."),
        total_steps,
        total_steps,
    ));
    let _ = remove_modpack_install_state(instance_root.as_path());

    Ok(instance)
}

fn count_vtmpack_manifest_payload_entries(manifest: &VtmpackManifest) -> usize {
    usize::from(!manifest.downloadable_content.is_empty())
        + manifest.bundled_mods.len()
        + manifest.configs.len()
        + manifest.additional_paths.len()
}

pub(crate) fn populate_vtmpack_instance(
    package_path: &Path,
    manifest: VtmpackManifest,
    instance_root: &Path,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let mut completed_steps = 1usize;
    extract_vtmpack_payload(
        package_path,
        instance_root,
        total_steps,
        &mut completed_steps,
        progress,
    )?;

    // Each file's download URL is worked out first, then all are downloaded together under the user's download
    // limits, and the hashes are checked afterwards.
    let mut tasks = Vec::new();
    let mut to_verify = Vec::new();
    for downloadable in &manifest.downloadable_content {
        if downloadable.file_path.as_os_str().is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, downloadable.file_path.as_path())?;
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                tr!("failed to create import directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
            })?;
        }
        completed_steps += 1;
        progress(import_progress(
            &tr!("Resolving {name}", name = downloadable.name),
            completed_steps,
            total_steps,
        ));
        let (url, verify) = resolve_vtmpack_entry_download(downloadable)?;
        if verify {
            to_verify.push((downloadable, destination.clone()));
        }
        tasks.push(installation::DownloadBatchTask {
            url,
            destination,
            expected_size: None,
        });
    }
    if !tasks.is_empty() {
        progress(import_progress(
            &tr!("Downloading {count} files...", count = tasks.len()),
            completed_steps,
            total_steps,
        ));
        installation::download_batch(tasks, &installation::shared_download_policy())
            .map_err(|err| tr!("failed to download pack files: {err}", err = err).into_owned())?;
        for (downloadable, destination) in to_verify {
            verify_vtmpack_download_hash(downloadable, destination.as_path())?;
        }
    }

    Ok(())
}

pub(crate) fn extract_vtmpack_payload(
    package_path: &Path,
    instance_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let mut archive = open_vtmpack_tar_archive(package_path)?;
    extract_vtmpack_payload_from_archive(
        package_path,
        &mut archive,
        instance_root,
        total_steps,
        completed_steps,
        progress,
    )
}

fn extract_vtmpack_payload_from_archive(
    package_path: &Path,
    archive: &mut tar::Archive<Box<dyn Read>>,
    instance_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    for entry in archive
        .entries()
        .map_err(|err| tr!("failed to read {package_path}: {err}", package_path = package_path.display(), err = err).into_owned())?
    {
        let mut entry = entry.map_err(|err| {
            tr!("failed to read archive entry in {package_path}: {err}", package_path = package_path.display(), err = err).into_owned()
        })?;
        let entry_path = entry
            .path()
            .map_err(|err| tr!("failed to decode archive path: {err}", err = err).into_owned())?
            .to_path_buf();
        let entry_string = entry_path.to_string_lossy().replace('\\', "/");

        if entry_string == "manifest.toml" {
            continue;
        }
        *completed_steps += 1;
        if entry_string == format!("metadata/{CONTENT_MANIFEST_FILE_NAME}") {
            let destination = instance_root.join(CONTENT_MANIFEST_FILE_NAME);
            progress(import_progress(
                vertex_i18n::src!("Restoring managed metadata..."),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    tr!("failed to create metadata directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                tr!("failed to restore managed metadata into {destination}: {err}", destination = destination.display(), err = err).into_owned()
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("bundled_mods/") {
            let destination = join_safe(&instance_root.join("mods"), Path::new(relative))?;
            progress(import_progress(
                &tr!("Restoring bundled mod {relative}", relative = relative),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    tr!("failed to create bundled mod directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                tr!("failed to import bundled mod {destination}: {err}", destination = destination.display(), err = err).into_owned()
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("configs/") {
            let destination = join_safe(&instance_root.join("config"), Path::new(relative))?;
            progress(import_progress(
                &tr!("Restoring config {relative}", relative = relative),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    tr!("failed to create config directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                tr!("failed to import config {destination}: {err}", destination = destination.display(), err = err).into_owned()
            })?;
            continue;
        }
        if let Some(relative) = entry_string.strip_prefix("root_entries/") {
            let destination = join_safe(instance_root, Path::new(relative))?;
            progress(import_progress(
                &tr!("Restoring {relative}", relative = relative),
                *completed_steps,
                total_steps,
            ));
            if let Some(parent) = destination.parent() {
                fs_create_dir_all_logged(parent).map_err(|err| {
                    tr!("failed to create imported root entry directory {parent}: {err}", parent = parent.display(), err = err).into_owned()
                })?;
            }
            entry.unpack(destination.as_path()).map_err(|err| {
                tr!("failed to import extra root entry {destination}: {err}", destination = destination.display(), err = err).into_owned()
            })?;
        }
    }
    Ok(())
}

/// The URL `entry` downloads from, and whether the downloaded file's hash should be checked against the pack.
pub(crate) fn resolve_vtmpack_entry_download(
    entry: &VtmpackDownloadableEntry,
) -> Result<(String, bool), String> {
    match normalize_source_name(entry.selected_source.as_deref()) {
        Some(ManagedSource::Modrinth) => {
            let version_id = entry
                .selected_version_id
                .as_deref()
                .ok_or_else(|| tr!("missing Modrinth version id for {name}", name = entry.name).into_owned())?;
            let version = ModrinthClient::default()
                .get_version(version_id)
                .map_err(|err| tr!("failed to fetch Modrinth version {version_id}: {err}", version_id = version_id, err = err).into_owned())?;
            let file = select_vtmpack_modrinth_file(entry, &version)?;
            Ok((file.url.clone(), true))
        }
        Some(ManagedSource::CurseForge) => {
            let project_id = entry
                .curseforge_project_id
                .ok_or_else(|| tr!("missing CurseForge project id for {name}", name = entry.name).into_owned())?;
            let file_id = entry
                .selected_version_id
                .as_deref()
                .ok_or_else(|| tr!("missing CurseForge file id for {name}", name = entry.name).into_owned())?
                .parse::<u64>()
                .map_err(|err| tr!("invalid CurseForge file id for {name}: {err}", name = entry.name, err = err).into_owned())?;
            let client = CurseForgeClient::from_env().ok_or_else(|| {
                "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack."
                    .to_owned()
            })?;
            let file = find_curseforge_file(&client, project_id, file_id)?;
            let download_url = file.download_url.ok_or_else(|| {
                tr!("CurseForge file {file_id} for project {project_id} has no download URL", file_id = file_id, project_id = project_id).into_owned()
            })?;
            Ok((download_url, false))
        }
        None => {
            if let Some(version_id) = entry.selected_version_id.as_deref() {
                let version = ModrinthClient::default()
                    .get_version(version_id)
                    .map_err(|err| {
                        tr!("failed to fetch Modrinth fallback version {version_id}: {err}", version_id = version_id, err = err).into_owned()
                    })?;
                let file = select_vtmpack_modrinth_file(entry, &version)?;
                return Ok((file.url.clone(), true));
            }
            Err(tr!("download source for {name} could not be determined from the pack metadata", name = entry.name).into_owned())
        }
    }
}

fn select_vtmpack_modrinth_file<'a>(
    entry: &VtmpackDownloadableEntry,
    version: &'a modrinth::ProjectVersion,
) -> Result<&'a modrinth::ProjectVersionFile, String> {
    if let Some(expected_sha512) = entry.selected_file_sha512.as_deref() {
        let expected = expected_sha512.trim();
        if !expected.is_empty() {
            return version
                .files
                .iter()
                .find(|file| {
                    file.hashes
                        .get("sha512")
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(expected))
                })
                .ok_or_else(|| {
                    tr!("no Modrinth file on version {id} matched recorded sha512 for {name}", id = version.id, name = entry.name).into_owned()
                });
        }
    }
    if let Some(expected_sha1) = entry.selected_file_sha1.as_deref() {
        let expected = expected_sha1.trim();
        if !expected.is_empty() {
            return version
                .files
                .iter()
                .find(|file| {
                    file.hashes
                        .get("sha1")
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(expected))
                })
                .ok_or_else(|| {
                    tr!("no Modrinth file on version {id} matched recorded sha1 for {name}", id = version.id, name = entry.name).into_owned()
                });
        }
    }

    version
        .files
        .iter()
        .find(|file| file.primary)
        .or_else(|| version.files.first())
        .ok_or_else(|| {
            tr!("no downloadable file found for Modrinth version {id}", id = version.id).into_owned()
        })
}

fn verify_vtmpack_download_hash(
    entry: &VtmpackDownloadableEntry,
    destination: &Path,
) -> Result<(), String> {
    if entry.selected_file_sha1.is_none() && entry.selected_file_sha512.is_none() {
        return Ok(());
    }
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(destination)
        .map_err(|err| tr!("failed to hash downloaded {destination}: {err}", destination = destination.display(), err = err).into_owned())?;
    if let Some(expected) = entry.selected_file_sha512.as_deref() {
        let expected = expected.trim();
        if !expected.is_empty() && !sha512.eq_ignore_ascii_case(expected) {
            return Err(tr!("downloaded {destination} did not match recorded sha512 for {name}", destination = destination.display(), name = entry.name).into_owned());
        }
    }
    if let Some(expected) = entry.selected_file_sha1.as_deref() {
        let expected = expected.trim();
        if !expected.is_empty() && !sha1.eq_ignore_ascii_case(expected) {
            return Err(tr!("downloaded {destination} did not match recorded sha1 for {name}", destination = destination.display(), name = entry.name).into_owned());
        }
    }
    Ok(())
}
