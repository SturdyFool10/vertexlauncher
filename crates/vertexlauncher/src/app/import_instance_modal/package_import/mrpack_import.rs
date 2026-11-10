use super::*;

pub(crate) fn import_mrpack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err("Modrinth pack import requires a manifest file source.".to_owned());
    };
    progress(import_progress("Reading .mrpack manifest...", 0, 1));
    let manifest = read_mrpack_manifest(package_path.as_path())?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    let override_steps = count_mrpack_override_entries(package_path.as_path())?;
    let total_steps = 3 + override_steps + manifest.files.len();
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: instances::normalize_optional(
                manifest.summary.as_deref().unwrap_or_default(),
            ),
            thumbnail_path: None,
            modloader: dependency_info.modloader.clone(),
            game_version: dependency_info.game_version.clone(),
            modloader_version: dependency_info.modloader_version.clone(),
        },
    )
    .map_err(|err| format!("failed to create imported profile: {err}"))?;
    progress(import_progress(
        "Created imported profile. Restoring overrides...",
        1,
        total_steps,
    ));
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = populate_mrpack_instance(
        package_path.as_path(),
        manifest.clone(),
        instance_root.as_path(),
        total_steps,
        progress,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    let base_manifest = match build_mrpack_base_manifest(instance_root.as_path(), &manifest) {
        Ok(manifest) => manifest,
        Err(err) => {
            let _ = delete_instance(store, instance.id.as_str(), installations_root);
            return Err(err);
        }
    };
    if let Err(err) = save_content_manifest(instance_root.as_path(), &base_manifest) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }
    let modpack_state =
        build_mrpack_install_state(package_path.as_path(), &manifest, base_manifest);
    if let Err(err) = save_modpack_install_state(instance_root.as_path(), &modpack_state) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    progress(import_progress(
        "Import complete.",
        total_steps,
        total_steps,
    ));

    Ok(instance)
}

pub(crate) fn build_mrpack_base_manifest(
    instance_root: &Path,
    manifest: &MrpackManifest,
) -> Result<ContentInstallManifest, String> {
    let modrinth = ModrinthClient::default();
    let mut content_manifest = ContentInstallManifest::default();

    for file in &manifest.files {
        if matches!(
            file.env.as_ref().and_then(|env| env.client.as_deref()),
            Some("unsupported")
        ) {
            continue;
        }
        let content_folder = managed_content_folder_for_relative_path(file.path.as_path());
        let Some(folder_name) = content_folder else {
            continue;
        };
        let absolute_path = join_safe(instance_root, file.path.as_path())?;
        if !absolute_path.exists() || absolute_path.is_dir() {
            continue;
        }
        let Some((project, version)) = resolve_mrpack_manifest_project_version(&modrinth, file)
            .or_else(|| {
                resolve_modrinth_project_version_from_file(&modrinth, absolute_path.as_path())
            })
        else {
            continue;
        };
        let relative_path = absolute_path
            .strip_prefix(instance_root)
            .unwrap_or(absolute_path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let project_key = format!("modrinth:{}", project.project_id);
        content_manifest.projects.insert(
            project_key.clone(),
            InstalledContentProject {
                project_key,
                name: project.title,
                folder_name: folder_name.to_owned(),
                file_path: PathBuf::from(relative_path),
                modrinth_project_id: Some(project.project_id),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some(version.id),
                selected_version_name: instances::normalize_optional(
                    version.version_number.as_str(),
                ),
                selected_file_sha1: None,
                selected_file_sha512: None,
                pack_managed: true,
                explicitly_installed: false,
                direct_dependencies: Vec::new(),
            },
        );
    }

    Ok(content_manifest)
}

pub(crate) fn resolve_mrpack_manifest_project_version(
    client: &ModrinthClient,
    file: &MrpackFile,
) -> Option<(modrinth::Project, modrinth::ProjectVersion)> {
    let resolved = file
        .downloads
        .iter()
        .find_map(|url| parse_modrinth_download_source(url.as_str()))?;
    let project = client.get_project(resolved.project_id.as_str()).ok()?;
    let version = client.get_version(resolved.version_id.as_str()).ok()?;
    (version.project_id == resolved.project_id).then_some((project, version))
}

pub(crate) fn build_mrpack_install_state(
    package_path: &Path,
    manifest: &MrpackManifest,
    base_manifest: ContentInstallManifest,
) -> ModpackInstallState {
    let resolved = resolve_mrpack_modpack_source(package_path);
    ModpackInstallState {
        format: "mrpack".to_owned(),
        pack_name: instances::normalize_optional(manifest.name.as_str())
            .unwrap_or_else(|| "Modpack".to_owned()),
        version_id: resolved
            .as_ref()
            .map(|resolved| resolved.version_id.clone())
            .or_else(|| instances::normalize_optional(manifest.version_id.as_str()))
            .unwrap_or_else(|| "unknown".to_owned()),
        version_name: resolved
            .as_ref()
            .and_then(|resolved| instances::normalize_optional(resolved.version_name.as_str()))
            .or_else(|| instances::normalize_optional(manifest.version_id.as_str()))
            .unwrap_or_else(|| "unknown".to_owned()),
        modrinth_project_id: resolved
            .as_ref()
            .map(|resolved| resolved.project_id.clone()),
        curseforge_project_id: None,
        source: resolved.map(|_| ManagedContentSource::Modrinth),
        base_manifest,
    }
}

pub(crate) fn resolve_mrpack_modpack_source(package_path: &Path) -> Option<ResolvedMrpackSource> {
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(package_path).ok()?;
    let client = ModrinthClient::default();
    let version = client
        .get_version_from_hash(sha512.as_str(), "sha512")
        .ok()
        .flatten()
        .or_else(|| {
            client
                .get_version_from_hash(sha1.as_str(), "sha1")
                .ok()
                .flatten()
        })?;
    Some(ResolvedMrpackSource {
        project_id: version.project_id,
        version_id: version.id,
        version_name: version.version_number,
    })
}

pub(crate) fn resolve_modrinth_project_version_from_file(
    client: &ModrinthClient,
    path: &Path,
) -> Option<(modrinth::Project, modrinth::ProjectVersion)> {
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(path).ok()?;
    let version = client
        .get_version_from_hash(sha512.as_str(), "sha512")
        .ok()
        .flatten()
        .or_else(|| {
            client
                .get_version_from_hash(sha1.as_str(), "sha1")
                .ok()
                .flatten()
        })?;
    let project = client.get_project(version.project_id.as_str()).ok()?;
    Some((project, version))
}

pub(crate) fn parse_modrinth_download_source(url: &str) -> Option<ResolvedModrinthDownloadSource> {
    let path = url.split(['?', '#']).next()?.trim_matches('/');
    let mut segments = path.split('/');
    while let Some(segment) = segments.next() {
        if segment != "data" {
            continue;
        }
        let project_id = instances::normalize_optional(segments.next()?)?;
        if segments.next()? != "versions" {
            return None;
        }
        let version_id = instances::normalize_optional(segments.next()?)?;
        return Some(ResolvedModrinthDownloadSource {
            project_id,
            version_id,
        });
    }
    None
}

pub(crate) fn managed_content_folder_for_relative_path(
    relative_path: &Path,
) -> Option<&'static str> {
    let normalized = relative_path.to_string_lossy().replace('\\', "/");
    let head = normalized.split('/').next()?.to_ascii_lowercase();
    match head.as_str() {
        "mods" => Some("mods"),
        "resourcepacks" => Some("resourcepacks"),
        "shaderpacks" => Some("shaderpacks"),
        "datapacks" => Some("datapacks"),
        _ => None,
    }
}

pub(crate) fn populate_mrpack_instance(
    package_path: &Path,
    manifest: MrpackManifest,
    instance_root: &Path,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let mut completed_steps = 1usize;
    extract_mrpack_overrides(
        package_path,
        instance_root,
        total_steps,
        &mut completed_steps,
        progress,
    )?;
    // Every file is resolved first, then all are downloaded together under the user's download limits.
    let mut tasks = Vec::new();
    for file in manifest.files {
        if matches!(
            file.env.as_ref().and_then(|env| env.client.as_deref()),
            Some("unsupported")
        ) {
            continue;
        }
        let destination = join_safe(instance_root, file.path.as_path())?;
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                format!(
                    "failed to create import directory {}: {err}",
                    parent.display()
                )
            })?;
        }
        let download_url = file.downloads.first().cloned().ok_or_else(|| {
            format!(
                "Modrinth pack entry {} has no download URL",
                file.path.display()
            )
        })?;
        tasks.push(installation::DownloadBatchTask {
            url: download_url,
            destination,
            expected_size: None,
        });
    }
    if !tasks.is_empty() {
        progress(import_progress(
            &format!("Downloading {} files...", tasks.len()),
            completed_steps + 1,
            total_steps,
        ));
        let count = tasks.len();
        installation::download_batch(tasks, &installation::shared_download_policy())
            .map_err(|err| format!("failed to download pack files: {err}"))?;
        completed_steps += count;
        progress(import_progress(
            "Downloaded pack files.",
            completed_steps,
            total_steps,
        ));
    }
    Ok(())
}

pub(crate) fn extract_mrpack_overrides(
    package_path: &Path,
    instance_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        let Some(relative) = entry_name
            .strip_prefix("overrides/")
            .or_else(|| entry_name.strip_prefix("client-overrides/"))
        else {
            continue;
        };
        if relative.is_empty() {
            continue;
        }
        let destination = join_safe(instance_root, Path::new(relative))?;
        *completed_steps += 1;
        progress(import_progress(
            &format!("Restoring override {}", relative),
            *completed_steps,
            total_steps,
        ));
        if entry.is_dir() {
            fs_create_dir_all_logged(destination.as_path()).map_err(|err| {
                format!(
                    "failed to create override directory {}: {err}",
                    destination.display()
                )
            })?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                format!(
                    "failed to create override parent {}: {err}",
                    parent.display()
                )
            })?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|err| {
            format!(
                "failed to read override {} from {}: {err}",
                entry_name,
                package_path.display()
            )
        })?;
        fs_write_logged(destination.as_path(), bytes)
            .map_err(|err| format!("failed to write override {}: {err}", destination.display()))?;
    }

    Ok(())
}

pub(crate) fn count_mrpack_override_entries(package_path: &Path) -> Result<usize, String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    let mut count = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        if entry_name
            .strip_prefix("overrides/")
            .or_else(|| entry_name.strip_prefix("client-overrides/"))
            .is_some_and(|relative| !relative.is_empty())
        {
            count += 1;
        }
    }
    Ok(count)
}

pub(crate) fn read_mrpack_manifest(path: &Path) -> Result<MrpackManifest, String> {
    let file = fs_file_open_logged(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut manifest = archive
        .by_name("modrinth.index.json")
        .map_err(|err| format!("missing modrinth.index.json in {}: {err}", path.display()))?;
    let mut raw = String::new();
    manifest
        .read_to_string(&mut raw)
        .map_err(|err| format!("failed to read modrinth.index.json: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse modrinth.index.json: {err}"))
}

pub(crate) fn resolve_mrpack_dependencies(
    dependencies: &HashMap<String, String>,
) -> Result<MrpackDependencyInfo, String> {
    let raw_game_version = dependencies
        .get("minecraft")
        .ok_or_else(|| "Modrinth pack is missing the required minecraft dependency.".to_owned())?;
    let game_version = normalize_minecraft_game_version(raw_game_version).ok_or_else(|| {
        format!(
            "Modrinth pack declared an invalid Minecraft version: {}",
            raw_game_version.trim()
        )
    })?;

    let loader_candidates = [
        ("neoforge", "NeoForge"),
        ("forge", "Forge"),
        ("fabric-loader", "Fabric"),
        ("quilt-loader", "Quilt"),
    ];
    for (key, label) in loader_candidates {
        if let Some(version) = dependencies.get(key) {
            return Ok(MrpackDependencyInfo {
                game_version,
                modloader: label.to_owned(),
                modloader_version: version.clone(),
            });
        }
    }

    Ok(MrpackDependencyInfo {
        game_version,
        modloader: "Vanilla".to_owned(),
        modloader_version: String::new(),
    })
}
