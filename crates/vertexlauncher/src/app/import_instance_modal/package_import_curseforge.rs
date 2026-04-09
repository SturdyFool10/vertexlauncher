use super::*;

pub(super) fn import_curseforge_pack(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: &ImportRequest,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<InstanceRecord, ImportPackageError> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Err(ImportPackageError::message(
            "CurseForge pack import requires a manifest file source.",
        ));
    };
    let manifest = read_curseforge_pack_manifest(package_path.as_path())
        .map_err(ImportPackageError::message)?;
    let override_steps = count_curseforge_override_entries(
        package_path.as_path(),
        manifest
            .overrides
            .as_deref()
            .unwrap_or_else(|| Path::new("overrides")),
    )
    .map_err(ImportPackageError::message)?;
    let file_count = manifest.files.iter().filter(|file| file.required).count();
    let total_steps = 5 + override_steps + (file_count * 2);
    progress(import_progress("Read CurseForge manifest.", 1, total_steps));
    progress(import_progress(
        "Resolving CurseForge pack metadata...",
        2,
        total_steps,
    ));
    let resolved = resolve_curseforge_pack_data(&manifest).map_err(ImportPackageError::message)?;
    let staged_files =
        predownload_curseforge_pack_files(&manifest, &resolved, request, total_steps, progress)?;
    let instance = create_instance(
        store,
        installations_root,
        NewInstanceSpec {
            name: request.instance_name.clone(),
            description: non_empty(manifest.author.as_str())
                .map(|author| format!("Imported CurseForge pack by {author}.")),
            thumbnail_path: None,
            modloader: resolved.dependency_info.modloader.clone(),
            game_version: resolved.dependency_info.game_version.clone(),
            modloader_version: resolved.dependency_info.modloader_version.clone(),
        },
    )
    .map_err(|err| {
        ImportPackageError::message(format!("failed to create imported profile: {err}"))
    })?;
    progress(import_progress(
        &format!(
            "Downloaded {file_count}/{file_count} mods. Created imported profile. Restoring overrides..."
        ),
        3 + file_count,
        total_steps,
    ));
    let instance_root = instance_root_path(installations_root, &instance);

    if let Err(err) = populate_curseforge_pack_instance(
        package_path.as_path(),
        &manifest,
        &resolved,
        &staged_files,
        instance_root.as_path(),
        total_steps,
        3 + file_count,
        progress,
    ) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(err);
    }

    progress(import_progress(
        "Writing managed metadata...",
        total_steps.saturating_sub(1),
        total_steps,
    ));
    let base_manifest = build_curseforge_base_manifest_from_resolved(&manifest, &resolved);
    if let Err(err) = save_content_manifest(instance_root.as_path(), &base_manifest) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(ImportPackageError::message(err));
    }
    let modpack_state = build_curseforge_install_state(&manifest, base_manifest);
    if let Err(err) = save_modpack_install_state(instance_root.as_path(), &modpack_state) {
        let _ = delete_instance(store, instance.id.as_str(), installations_root);
        return Err(ImportPackageError::message(err));
    }

    progress(import_progress(
        "Import complete.",
        total_steps,
        total_steps,
    ));
    Ok(instance)
}

pub fn attach_curseforge_modpack_install_state(
    instance_root: &Path,
    project_id: u64,
    file_id: u64,
    pack_name: &str,
    version_name: &str,
) -> Result<(), String> {
    let base_manifest = load_modpack_install_state(instance_root)
        .map(|state| state.base_manifest)
        .unwrap_or_else(|| load_content_manifest(instance_root));
    save_modpack_install_state(
        instance_root,
        &ModpackInstallState {
            format: "curseforge".to_owned(),
            pack_name: default_if_blank(pack_name, "CurseForge Pack".to_owned()),
            version_id: file_id.to_string(),
            version_name: default_if_blank(version_name, file_id.to_string()),
            modrinth_project_id: None,
            curseforge_project_id: Some(project_id),
            source: Some(ManagedContentSource::CurseForge),
            base_manifest,
        },
    )
}

pub fn format_curseforge_download_url_error(
    project_id: u64,
    file_id: u64,
    err: &curseforge::CurseForgeError,
) -> String {
    let endpoint = format!("/v1/mods/{project_id}/files/{file_id}/download-url");
    match err {
        curseforge::CurseForgeError::HttpStatus { status, body } => {
            let body = body.trim();
            if body.is_empty() {
                format!(
                    "CurseForge download URL lookup failed for project {project_id}, file {file_id} via {endpoint}: HTTP {status} with empty response body"
                )
            } else {
                format!(
                    "CurseForge download URL lookup failed for project {project_id}, file {file_id} via {endpoint}: HTTP {status}: {body}"
                )
            }
        }
        _ => format!(
            "CurseForge download URL lookup failed for project {project_id}, file {file_id} via {endpoint}: {err}"
        ),
    }
}

pub(super) fn build_curseforge_base_manifest_from_resolved(
    manifest: &CurseForgePackManifest,
    resolved: &ResolvedCurseForgePackData,
) -> ContentInstallManifest {
    let mut content_manifest = ContentInstallManifest::default();
    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        let Some(file) = resolved.files.get(&manifest_file.file_id) else {
            continue;
        };
        let project = resolved.projects.get(&manifest_file.project_id);
        let project_key = format!("curseforge:{}", manifest_file.project_id);
        content_manifest.projects.insert(
            project_key.clone(),
            InstalledContentProject {
                project_key,
                name: project
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| file.display_name.clone()),
                folder_name: "mods".to_owned(),
                file_path: PathBuf::from(format!("mods/{}", file.file_name)),
                modrinth_project_id: None,
                curseforge_project_id: Some(manifest_file.project_id),
                selected_source: Some(ManagedContentSource::CurseForge),
                selected_version_id: Some(manifest_file.file_id.to_string()),
                selected_version_name: non_empty(file.display_name.as_str()),
                pack_managed: true,
                explicitly_installed: false,
                direct_dependencies: Vec::new(),
            },
        );
    }
    content_manifest
}

#[derive(Debug)]
pub(super) struct ResolvedCurseForgePackData {
    pub(super) dependency_info: MrpackDependencyInfo,
    pub(super) files: HashMap<u64, curseforge::File>,
    pub(super) projects: HashMap<u64, curseforge::Project>,
}

pub(super) fn resolve_curseforge_pack_data(
    manifest: &CurseForgePackManifest,
) -> Result<ResolvedCurseForgePackData, String> {
    let client = CurseForgeClient::from_env().ok_or_else(|| {
        "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack."
            .to_owned()
    })?;
    let dependency_info = resolve_curseforge_pack_dependencies(&manifest.minecraft)?;
    let required_files = manifest
        .files
        .iter()
        .filter(|file| file.required)
        .collect::<Vec<_>>();
    let files = client
        .get_files(
            required_files
                .iter()
                .map(|file| file.file_id)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .map_err(|err| format!("failed to fetch CurseForge pack files: {err}"))?
        .into_iter()
        .map(|file| (file.id, file))
        .collect::<HashMap<_, _>>();
    let projects = client
        .get_mods(
            required_files
                .iter()
                .map(|file| file.project_id)
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .map_err(|err| format!("failed to fetch CurseForge pack projects: {err}"))?
        .into_iter()
        .map(|project| (project.id, project))
        .collect::<HashMap<_, _>>();

    Ok(ResolvedCurseForgePackData {
        dependency_info,
        files,
        projects,
    })
}

#[derive(Clone, Debug)]
pub(super) struct CurseForgeDownloadPlan {
    pub(super) requirement: CurseForgeManualDownloadRequirement,
    pub(super) download_url: String,
    pub(super) source_label: &'static str,
}

pub(super) fn predownload_curseforge_pack_files(
    manifest: &CurseForgePackManifest,
    resolved: &ResolvedCurseForgePackData,
    request: &ImportRequest,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<HashMap<u64, PathBuf>, ImportPackageError> {
    let mut staged_files = request.manual_curseforge_files.clone();
    let mut download_plans = Vec::new();
    let mut manual_requirements = Vec::new();
    let client = CurseForgeClient::from_env().ok_or_else(|| {
        ImportPackageError::message(
            "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack.",
        )
    })?;

    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        if staged_files.contains_key(&manifest_file.file_id) {
            continue;
        }
        let file = resolved.files.get(&manifest_file.file_id).ok_or_else(|| {
            ImportPackageError::message(format!(
                "CurseForge file {} for project {} was not found.",
                manifest_file.file_id, manifest_file.project_id
            ))
        })?;
        let project = resolved.projects.get(&manifest_file.project_id);
        let requirement = build_curseforge_manual_download_requirement(
            manifest_file.project_id,
            manifest_file.file_id,
            file,
            project,
        );
        match resolve_curseforge_download_plan(
            &client,
            file,
            project.map(|project| project.name.as_str()),
            manifest_file.project_id,
            manifest_file.file_id,
            resolved.dependency_info.game_version.as_str(),
            resolved.dependency_info.modloader.as_str(),
        )
        .map_err(ImportPackageError::message)?
        {
            Some((download_url, source_label)) => download_plans.push(CurseForgeDownloadPlan {
                requirement,
                download_url,
                source_label,
            }),
            None => manual_requirements.push(requirement),
        }
    }

    if !download_plans.is_empty() {
        progress(import_progress(
            &format!(
                "Preparing {} CurseForge mod downloads...",
                download_plans.len()
            ),
            2,
            total_steps,
        ));
        let download_results = download_curseforge_plans_concurrently(
            download_plans,
            request.max_concurrent_downloads.max(1) as usize,
            total_steps,
            progress,
        )
        .map_err(ImportPackageError::message)?;
        staged_files.extend(download_results.staged_files);
        manual_requirements.extend(download_results.failed_requirements);
    }

    if !manual_requirements.is_empty() {
        manual_requirements.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        manual_requirements.dedup_by(|left, right| left.file_id == right.file_id);
        return Err(ImportPackageError::ManualCurseForgeDownloads {
            requirements: manual_requirements,
            staged_files,
        });
    }

    Ok(staged_files)
}

pub(super) fn resolve_curseforge_download_plan(
    client: &CurseForgeClient,
    curseforge_file: &curseforge::File,
    curseforge_project_name: Option<&str>,
    project_id: u64,
    file_id: u64,
    game_version: &str,
    modloader: &str,
) -> Result<Option<(String, &'static str)>, String> {
    if let Some(url) = curseforge_file
        .download_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
    {
        return Ok(Some((url.to_owned(), "CurseForge")));
    }
    match client.get_mod_file_download_url(project_id, file_id) {
        Ok(Some(url)) if !url.trim().is_empty() => return Ok(Some((url, "CurseForge"))),
        Ok(_) => {}
        Err(curseforge::CurseForgeError::HttpStatus { status: 403, .. }) => {}
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/import",
                curseforge_project_id = project_id,
                curseforge_file_id = file_id,
                error = %format_curseforge_download_url_error(project_id, file_id, &err),
                "CurseForge download URL resolution failed during pack predownload"
            );
        }
    }
    Ok(resolve_modrinth_backup_download_url_for_curseforge_file(
        curseforge_file,
        curseforge_project_name,
        game_version,
        modloader,
    )?
    .map(|url| (url, "Modrinth backup")))
}

pub(super) struct CurseForgeConcurrentDownloadResult {
    pub(super) staged_files: HashMap<u64, PathBuf>,
    pub(super) failed_requirements: Vec<CurseForgeManualDownloadRequirement>,
}

pub(super) fn download_curseforge_plans_concurrently(
    plans: Vec<CurseForgeDownloadPlan>,
    max_concurrent_downloads: usize,
    total_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<CurseForgeConcurrentDownloadResult, String> {
    if plans.is_empty() {
        return Ok(CurseForgeConcurrentDownloadResult {
            staged_files: HashMap::new(),
            failed_requirements: Vec::new(),
        });
    }
    let staging_dir = std::env::temp_dir().join(format!(
        "vertexlauncher-cf-download-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    fs_create_dir_all_logged(staging_dir.as_path())
        .map_err(|err| format!("failed to create CurseForge staging directory: {err}"))?;
    let total_downloads = plans.len();
    let queue = Arc::new(Mutex::new(VecDeque::from(plans)));
    let (tx, rx) = mpsc::channel::<(
        CurseForgeManualDownloadRequirement,
        Result<PathBuf, String>,
        &'static str,
    )>();
    let worker_count = max_concurrent_downloads.max(1).min(total_downloads.max(1));
    let mut handles = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let queue = queue.clone();
        let tx = tx.clone();
        let staging_dir = staging_dir.clone();
        handles.push(thread::spawn(move || {
            loop {
                let next = {
                    let mut guard = match queue.lock() {
                        Ok(guard) => guard,
                        Err(_) => return,
                    };
                    guard.pop_front()
                };
                let Some(plan) = next else {
                    return;
                };
                let staged_path = staging_dir.join(format!(
                    "{}-{}",
                    plan.requirement.file_id, plan.requirement.file_name
                ));
                let result = download_file(plan.download_url.as_str(), staged_path.as_path())
                    .map(|_| staged_path);
                if let Err(err) = tx.send((plan.requirement, result, plan.source_label)) {
                    tracing::error!(
                        target: "vertexlauncher/import_instance",
                        source = %plan.source_label,
                        error = %err,
                        "Failed to deliver manual CurseForge download worker result."
                    );
                }
            }
        }));
    }
    drop(tx);

    let mut completed_downloads = 0usize;
    let mut staged_files = HashMap::new();
    let mut failed_requirements = Vec::new();
    while let Ok((requirement, result, source_label)) = rx.recv() {
        completed_downloads += 1;
        match result {
            Ok(path) => {
                progress(import_progress(
                    &format!(
                        "Downloaded {} via {} ({}/{total_downloads} mods)",
                        requirement.display_name, source_label, completed_downloads
                    ),
                    2 + completed_downloads,
                    total_steps,
                ));
                staged_files.insert(requirement.file_id, path);
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_project_id = requirement.project_id,
                    curseforge_file_id = requirement.file_id,
                    error = %err,
                    source = source_label,
                    "CurseForge pack predownload failed; requiring manual download"
                );
                progress(import_progress(
                    &format!(
                        "Queued {} for manual download ({}/{total_downloads} mods checked)",
                        requirement.display_name, completed_downloads
                    ),
                    2 + completed_downloads,
                    total_steps,
                ));
                failed_requirements.push(requirement);
            }
        }
    }
    for handle in handles {
        let _ = handle.join();
    }

    Ok(CurseForgeConcurrentDownloadResult {
        staged_files,
        failed_requirements,
    })
}

pub(super) fn build_curseforge_install_state(
    manifest: &CurseForgePackManifest,
    base_manifest: ContentInstallManifest,
) -> ModpackInstallState {
    ModpackInstallState {
        format: "curseforge".to_owned(),
        pack_name: non_empty(manifest.name.as_str())
            .unwrap_or_else(|| "CurseForge Pack".to_owned()),
        version_id: non_empty(manifest.version.as_str()).unwrap_or_else(|| "unknown".to_owned()),
        version_name: non_empty(manifest.version.as_str()).unwrap_or_else(|| "unknown".to_owned()),
        modrinth_project_id: None,
        curseforge_project_id: None,
        source: Some(ManagedContentSource::CurseForge),
        base_manifest,
    }
}

pub(super) fn find_curseforge_file(
    client: &CurseForgeClient,
    project_id: u64,
    file_id: u64,
) -> Result<curseforge::File, String> {
    client
        .get_files(&[file_id])
        .map_err(|err| format!("failed to fetch CurseForge file {file_id}: {err}"))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("CurseForge file {file_id} was not found for project {project_id}"))
}

pub(super) fn populate_curseforge_pack_instance(
    package_path: &Path,
    manifest: &CurseForgePackManifest,
    resolved: &ResolvedCurseForgePackData,
    manual_curseforge_files: &HashMap<u64, PathBuf>,
    instance_root: &Path,
    total_steps: usize,
    starting_completed_steps: usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), ImportPackageError> {
    let mut completed_steps = starting_completed_steps;
    let total_mods = manifest.files.iter().filter(|file| file.required).count();
    let mut applied_mods = 0usize;
    let overrides_root = manifest
        .overrides
        .as_deref()
        .unwrap_or_else(|| Path::new("overrides"));
    extract_curseforge_overrides(
        package_path,
        instance_root,
        overrides_root,
        total_steps,
        &mut completed_steps,
        progress,
    )
    .map_err(ImportPackageError::message)?;

    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        let file = resolved.files.get(&manifest_file.file_id).ok_or_else(|| {
            ImportPackageError::message(format!(
                "CurseForge file {} for project {} was not found.",
                manifest_file.file_id, manifest_file.project_id
            ))
        })?;
        let source_path = manual_curseforge_files
            .get(&manifest_file.file_id)
            .ok_or_else(|| {
                ImportPackageError::message(format!(
                    "CurseForge file {} was not predownloaded before installation.",
                    manifest_file.file_id
                ))
            })?;
        let detected_kind =
            detect_installed_content_kind(source_path.as_path()).unwrap_or_else(|| {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_project_id = manifest_file.project_id,
                    curseforge_file_id = manifest_file.file_id,
                    file_name = %file.file_name,
                    "Could not detect installed content kind for staged CurseForge file; defaulting to mods."
                );
                InstalledContentKind::Mods
            });
        let target_dir = instance_root.join(detected_kind.folder_name());
        let destination = target_dir.join(file.file_name.as_str());
        if let Some(parent) = destination.parent() {
            fs_create_dir_all_logged(parent).map_err(|err| {
                ImportPackageError::message(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        completed_steps += 1;
        applied_mods += 1;
        progress(import_progress(
            &format!(
                "Applying staged {} for {} ({applied_mods}/{total_mods} files)",
                detected_kind.content_type_key(),
                file.display_name
            ),
            completed_steps,
            total_steps,
        ));
        fs_copy_logged(source_path, destination.as_path()).map_err(|err| {
            ImportPackageError::message(format!(
                "failed to copy predownloaded file {} into {}: {err}",
                source_path.display(),
                destination.display()
            ))
        })?;
    }

    Ok(())
}

pub(super) fn extract_curseforge_overrides(
    package_path: &Path,
    instance_root: &Path,
    overrides_root: &Path,
    total_steps: usize,
    completed_steps: &mut usize,
    progress: &mut dyn FnMut(ImportProgress),
) -> Result<(), String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    let normalized_root = format!(
        "{}/",
        overrides_root
            .to_string_lossy()
            .trim()
            .trim_matches('/')
            .replace('\\', "/")
    );

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        let Some(relative) = entry_name.strip_prefix(normalized_root.as_str()) else {
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
            fs_create_dir_all_logged(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
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

pub(super) fn count_curseforge_override_entries(
    package_path: &Path,
    overrides_root: &Path,
) -> Result<usize, String> {
    let file = fs_file_open_logged(package_path)
        .map_err(|err| format!("failed to open {}: {err}", package_path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    let normalized_root = format!(
        "{}/",
        overrides_root
            .to_string_lossy()
            .trim()
            .trim_matches('/')
            .replace('\\', "/")
    );
    let mut count = 0usize;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|err| {
            format!(
                "failed to read zip entry in {}: {err}",
                package_path.display()
            )
        })?;
        let entry_name = entry.name().replace('\\', "/");
        if entry_name.starts_with(normalized_root.as_str()) && !entry.is_dir() {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn read_curseforge_pack_manifest(path: &Path) -> Result<CurseForgePackManifest, String> {
    let file = fs_file_open_logged(path)
        .map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let mut manifest = archive
        .by_name("manifest.json")
        .map_err(|err| format!("missing manifest.json in {}: {err}", path.display()))?;
    let mut raw = String::new();
    manifest
        .read_to_string(&mut raw)
        .map_err(|err| format!("failed to read manifest.json: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("failed to parse manifest.json: {err}"))
}

pub(super) fn resolve_curseforge_pack_dependencies(
    minecraft: &CurseForgePackMinecraft,
) -> Result<MrpackDependencyInfo, String> {
    let game_version =
        normalize_minecraft_game_version(minecraft.version.as_str()).ok_or_else(|| {
            format!(
                "CurseForge pack declared an invalid Minecraft version: {}",
                minecraft.version.trim()
            )
        })?;

    let loader = minecraft
        .mod_loaders
        .iter()
        .find(|loader| loader.primary)
        .or_else(|| minecraft.mod_loaders.first());
    let Some(loader) = loader else {
        return Ok(MrpackDependencyInfo {
            game_version,
            modloader: "Vanilla".to_owned(),
            modloader_version: String::new(),
        });
    };

    let id = loader.id.trim();
    let (modloader, modloader_version) = if let Some(version) = id.strip_prefix("forge-") {
        ("Forge".to_owned(), version.to_owned())
    } else if let Some(version) = id.strip_prefix("fabric-") {
        ("Fabric".to_owned(), version.to_owned())
    } else if let Some(version) = id.strip_prefix("quilt-") {
        ("Quilt".to_owned(), version.to_owned())
    } else if let Some(version) = id.strip_prefix("neoforge-") {
        ("NeoForge".to_owned(), version.to_owned())
    } else {
        (id.to_owned(), String::new())
    };

    Ok(MrpackDependencyInfo {
        game_version,
        modloader,
        modloader_version,
    })
}

pub(super) fn resolve_modrinth_backup_download_url_for_curseforge_file(
    curseforge_file: &curseforge::File,
    curseforge_project_name: Option<&str>,
    game_version: &str,
    modloader: &str,
) -> Result<Option<String>, String> {
    let modrinth = ModrinthClient::default();
    if let Some(url) = resolve_modrinth_hash_backup_download_url_for_curseforge_file(
        &modrinth,
        curseforge_file,
        game_version,
        modloader,
    )? {
        return Ok(Some(url));
    }

    let queries = modrinth_fallback_queries(curseforge_file, curseforge_project_name);
    if queries.is_empty() {
        return Ok(None);
    }

    let loader_slug = modrinth_loader_slug(modloader);
    let mut loaders = Vec::new();
    if let Some(loader) = loader_slug {
        loaders.push(loader.to_owned());
    }
    let normalized_game_version = normalize_minecraft_game_version(game_version);
    let mut game_versions = Vec::new();
    if let Some(version) = normalized_game_version.as_deref() {
        game_versions.push(version.to_owned());
    }

    for query in queries {
        let projects = match modrinth.search_projects_with_filters(
            query.as_str(),
            8,
            0,
            Some("mod"),
            normalized_game_version.as_deref(),
            loader_slug,
            None,
        ) {
            Ok(projects) => projects,
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    query = %query,
                    error = %err,
                    "Modrinth fallback search failed"
                );
                continue;
            }
        };

        for project in projects.into_iter().take(5) {
            let versions = match modrinth.list_project_versions(
                project.project_id.as_str(),
                loaders.as_slice(),
                game_versions.as_slice(),
            ) {
                Ok(versions) => versions,
                Err(_) => continue,
            };
            for version in versions {
                let Some(file) = select_modrinth_backup_file(
                    &version,
                    curseforge_file,
                    game_version,
                    modloader,
                    true,
                ) else {
                    continue;
                };
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_file_id = curseforge_file.id,
                    modrinth_project_id = %project.project_id,
                    modrinth_version_id = %version.id,
                    "Using Modrinth fallback download for CurseForge file"
                );
                return Ok(Some(file.url.clone()));
            }
        }
    }
    Ok(None)
}

#[derive(Clone, Debug)]
pub struct CurseForgeManualDownloadRequirement {
    pub project_id: u64,
    pub file_id: u64,
    pub project_name: String,
    pub file_name: String,
    pub display_name: String,
    pub download_page_url: String,
}

pub fn prepare_curseforge_manual_downloads(
    request: &ImportRequest,
) -> Result<Option<Vec<CurseForgeManualDownloadRequirement>>, String> {
    let ImportSource::ManifestFile(package_path) = &request.source else {
        return Ok(None);
    };
    if inspect_package(package_path.as_path())
        .map(|preview| preview.kind)
        .ok()
        != Some(ImportPreviewKind::Manifest(
            ImportPackageKind::CurseForgePack,
        ))
    {
        return Ok(None);
    }
    let manifest = read_curseforge_pack_manifest(package_path.as_path())?;
    let resolved = resolve_curseforge_pack_data(&manifest)?;
    let mut blocked = Vec::new();
    for manifest_file in manifest.files.iter().filter(|file| file.required) {
        let Some(file) = resolved.files.get(&manifest_file.file_id) else {
            continue;
        };
        if curseforge_file_has_api_download(file) {
            continue;
        }
        blocked.push(build_curseforge_manual_download_requirement(
            manifest_file.project_id,
            manifest_file.file_id,
            file,
            resolved.projects.get(&manifest_file.project_id),
        ));
    }
    Ok((!blocked.is_empty()).then_some(blocked))
}

pub fn prepare_curseforge_manual_download_for_file(
    project_id: u64,
    file_id: u64,
) -> Result<Option<CurseForgeManualDownloadRequirement>, String> {
    let client = CurseForgeClient::from_env().ok_or_else(|| {
        "CurseForge API key missing. Add one in Settings or set VERTEX_CURSEFORGE_API_KEY/CURSEFORGE_API_KEY to import this pack."
            .to_owned()
    })?;
    let file = find_curseforge_file(&client, project_id, file_id)?;
    if curseforge_file_has_api_download(&file) {
        return Ok(None);
    }
    let project = client
        .get_mods(&[project_id])
        .map_err(|err| format!("failed to fetch CurseForge project {project_id}: {err}"))?
        .into_iter()
        .next();
    Ok(Some(build_curseforge_manual_download_requirement(
        project_id,
        file_id,
        &file,
        project.as_ref(),
    )))
}

pub(super) fn curseforge_file_has_api_download(file: &curseforge::File) -> bool {
    file.download_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
}

pub(super) fn build_curseforge_manual_download_requirement(
    project_id: u64,
    file_id: u64,
    file: &curseforge::File,
    project: Option<&curseforge::Project>,
) -> CurseForgeManualDownloadRequirement {
    let project_name = project
        .map(|project| project.name.clone())
        .unwrap_or_else(|| file.display_name.clone());
    let download_page_url = project
        .and_then(|project| project.website_url.clone())
        .map(|base| format!("{}/files/{}", base.trim_end_matches('/'), file_id))
        .unwrap_or_else(|| {
            format!("https://www.curseforge.com/minecraft/mc-mods/{project_id}/files/{file_id}")
        });
    CurseForgeManualDownloadRequirement {
        project_id,
        file_id,
        project_name,
        file_name: file.file_name.clone(),
        display_name: file.display_name.clone(),
        download_page_url,
    }
}

pub(super) fn resolve_modrinth_hash_backup_download_url_for_curseforge_file(
    modrinth: &ModrinthClient,
    curseforge_file: &curseforge::File,
    game_version: &str,
    modloader: &str,
) -> Result<Option<String>, String> {
    let loader_slug = modrinth_loader_slug(modloader);
    let normalized_game_version = normalize_minecraft_game_version(game_version);

    let mut hash_candidates = Vec::new();
    if let Some(sha512) = curseforge_file.sha512_hash() {
        hash_candidates.push(("sha512", sha512));
    }
    if let Some(sha1) = curseforge_file.sha1_hash() {
        hash_candidates.push(("sha1", sha1));
    }
    if hash_candidates.is_empty() {
        return Ok(None);
    }

    for (algorithm, hash) in hash_candidates {
        let version = match modrinth.get_version_from_hash(hash, algorithm) {
            Ok(Some(version)) => version,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/import",
                    curseforge_file_id = curseforge_file.id,
                    algorithm,
                    error = %err,
                    "Modrinth hash lookup failed during fallback"
                );
                continue;
            }
        };
        if let Some(game_version) = normalized_game_version.as_deref()
            && !version.game_versions.is_empty()
            && !version
                .game_versions
                .iter()
                .any(|value| value.eq_ignore_ascii_case(game_version))
        {
            continue;
        }
        if let Some(loader_slug) = loader_slug
            && !version.loaders.is_empty()
            && !version
                .loaders
                .iter()
                .any(|value| value.eq_ignore_ascii_case(loader_slug))
        {
            continue;
        }

        let Some(file) =
            select_modrinth_backup_file(&version, curseforge_file, game_version, modloader, false)
        else {
            continue;
        };
        tracing::warn!(
            target: "vertexlauncher/import",
            curseforge_file_id = curseforge_file.id,
            modrinth_version_id = %version.id,
            algorithm,
            "Using exact Modrinth hash fallback for CurseForge file"
        );
        return Ok(Some(file.url.clone()));
    }
    Ok(None)
}

pub(super) fn modrinth_fallback_queries(
    file: &curseforge::File,
    curseforge_project_name: Option<&str>,
) -> Vec<String> {
    let mut queries = Vec::new();
    let raw_candidates = [
        curseforge_project_name.unwrap_or_default(),
        file.display_name.as_str(),
        file.file_name.as_str(),
        file.file_name
            .strip_suffix(".jar")
            .unwrap_or(file.file_name.as_str()),
    ];
    for candidate in raw_candidates {
        let query = candidate
            .replace(['[', ']', '(', ')', '{', '}'], " ")
            .replace(['_', '-'], " ")
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ");
        if !query.is_empty() && !queries.iter().any(|entry| entry == &query) {
            queries.push(query);
        }
    }
    queries
}

pub(super) fn select_modrinth_backup_file<'a>(
    version: &'a modrinth::ProjectVersion,
    curseforge_file: &curseforge::File,
    game_version: &str,
    modloader: &str,
    require_exact_filename: bool,
) -> Option<&'a modrinth::ProjectVersionFile> {
    let expected_name = normalized_name(curseforge_file.file_name.as_str());
    if let Some(file) = version
        .files
        .iter()
        .find(|candidate| normalized_name(candidate.filename.as_str()) == expected_name)
    {
        return Some(file);
    }
    if require_exact_filename || version.files.len() != 1 {
        return None;
    }
    let file = version.files.first()?;
    modrinth_backup_filename_looks_compatible(file.filename.as_str(), game_version, modloader)
        .then_some(file)
}

pub(super) fn modrinth_backup_filename_looks_compatible(
    filename: &str,
    game_version: &str,
    modloader: &str,
) -> bool {
    let desired_loader = modloader_loader_family(modloader);
    let candidate_loader = modloader_loader_family(filename);
    if let (Some(desired_loader), Some(candidate_loader)) = (desired_loader, candidate_loader)
        && desired_loader != candidate_loader
    {
        return false;
    }
    if let Some(candidate_game_version) = find_minecraft_version_in_text(filename)
        && let Some(desired_game_version) = normalize_minecraft_game_version(game_version)
        && candidate_game_version != desired_game_version
    {
        return false;
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModloaderFamily {
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

pub(super) fn modloader_loader_family(value: &str) -> Option<ModloaderFamily> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.contains("neoforge") || lower.contains("-neo-") {
        Some(ModloaderFamily::NeoForge)
    } else if lower.contains("fabric") {
        Some(ModloaderFamily::Fabric)
    } else if lower.contains("quilt") {
        Some(ModloaderFamily::Quilt)
    } else if lower.contains("forge") {
        Some(ModloaderFamily::Forge)
    } else {
        None
    }
}

pub(super) fn modrinth_loader_slug(loader: &str) -> Option<&'static str> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric"),
        "forge" => Some("forge"),
        "quilt" => Some("quilt"),
        "neoforge" => Some("neoforge"),
        _ => None,
    }
}

pub(super) fn normalized_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackManifest {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) author: String,
    pub(crate) minecraft: CurseForgePackMinecraft,
    #[serde(default)]
    pub(crate) files: Vec<CurseForgePackFile>,
    #[serde(default)]
    pub(crate) overrides: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackMinecraft {
    pub(crate) version: String,
    #[serde(rename = "modLoaders", default)]
    pub(crate) mod_loaders: Vec<CurseForgePackModLoader>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackModLoader {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackFile {
    #[serde(rename = "projectID")]
    pub(crate) project_id: u64,
    #[serde(rename = "fileID")]
    pub(crate) file_id: u64,
    #[serde(default)]
    pub(crate) required: bool,
}
