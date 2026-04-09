use super::*;

#[derive(Clone, Debug)]
pub(super) enum DependencyRef {
    ModrinthProject(String),
    CurseForgeProject(u64),
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedDownload {
    pub(super) source: ManagedContentSource,
    pub(super) version_id: String,
    version_name: String,
    pub(super) file_url: String,
    file_name: String,
    published_at: String,
    pub(super) dependencies: Vec<DependencyRef>,
}

pub(super) fn active_download_from_request(
    request: &ContentInstallRequest,
) -> ActiveContentDownload {
    match request {
        ContentInstallRequest::Latest { entry, .. } => ActiveContentDownload {
            dedupe_key: entry.dedupe_key.clone(),
            version_id: None,
        },
        ContentInstallRequest::Exact { entry, version, .. } => ActiveContentDownload {
            dedupe_key: entry.dedupe_key.clone(),
            version_id: Some(version.version_id.clone()),
        },
    }
}

pub(super) fn apply_content_install_request(
    instance_root: &Path,
    request: ContentInstallRequest,
) -> Result<ContentDownloadOutcome, String> {
    apply_content_install_request_with_prefetched_downloads(
        instance_root,
        request,
        &HashMap::new(),
        &[],
    )
}

pub(super) fn apply_content_install_request_with_prefetched_downloads(
    instance_root: &Path,
    request: ContentInstallRequest,
    prefetched_paths: &HashMap<PathBuf, PathBuf>,
    additional_cleanup_paths: &[PathBuf],
) -> Result<ContentDownloadOutcome, String> {
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let mut added_files = Vec::new();
    let mut removed_files = Vec::new();

    let (root_entry, game_version, loader, root_download) = match request {
        ContentInstallRequest::Latest {
            entry,
            game_version,
            loader,
        } => {
            let resolved = resolve_best_download(
                &entry,
                game_version.as_str(),
                loader,
                &modrinth,
                curseforge.as_ref(),
            )?
            .ok_or_else(|| format!("No compatible downloadable file found for {}.", entry.name))?;
            (entry, game_version, loader, resolved)
        }
        ContentInstallRequest::Exact {
            entry,
            version,
            game_version,
            loader,
        } => (
            entry,
            game_version,
            loader,
            resolved_download_from_version(version),
        ),
    };
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %root_entry.name,
        version_id = %root_download.version_id,
        prefetched = !prefetched_paths.is_empty(),
        "applying content install request"
    );

    let mut manifest = load_content_manifest(instance_root);
    let mut deferred_cleanup = (!prefetched_paths.is_empty()).then(DeferredContentCleanup::default);
    let existing_project = installed_project_for_entry(&manifest, &root_entry)
        .map(|(key, project)| (key.to_owned(), project.clone()));
    let root_project_key = existing_project
        .as_ref()
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| root_entry.dedupe_key.clone());

    if let Some((existing_project_key, existing)) = existing_project {
        if existing.selected_source == Some(root_download.source)
            && existing.selected_version_id.as_deref() == Some(root_download.version_id.as_str())
        {
            if let Some(record) = manifest.projects.get_mut(existing_project_key.as_str()) {
                record.pack_managed = false;
                record.explicitly_installed = true;
            }
            for path in additional_cleanup_paths {
                if !path.exists() {
                    continue;
                }
                remove_content_path(path.as_path())?;
                removed_files.push(path.display().to_string());
            }
            save_content_manifest(instance_root, &manifest)?;
            return Ok(ContentDownloadOutcome {
                project_name: root_entry.name,
                added_files,
                removed_files,
            });
        }
        let dependents = manifest_dependents(&manifest, existing_project_key.as_str());
        if !dependents.is_empty() {
            tracing::warn!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %root_entry.name,
                existing_project_key = %existing_project_key,
                dependents = %dependents.join(", "),
                "rejecting content switch because dependents are still installed"
            );
            return Err(format!(
                "Cannot switch {} while it is required by {}.",
                root_entry.name,
                dependents.join(", ")
            ));
        }
        remove_installed_project(
            instance_root,
            &mut manifest,
            existing_project_key.as_str(),
            true,
            &mut removed_files,
            deferred_cleanup.as_mut(),
        )?;
    }

    let mut visited = HashSet::new();
    install_project_recursive(
        instance_root,
        &mut manifest,
        &root_entry,
        root_download,
        game_version.as_str(),
        loader,
        Some(root_project_key.as_str()),
        &modrinth,
        curseforge.as_ref(),
        None,
        true,
        prefetched_paths,
        &mut visited,
        &mut added_files,
        &mut removed_files,
        deferred_cleanup.as_mut(),
    )?;
    if let Some(cleanup) = deferred_cleanup.as_mut() {
        cleanup.stale_paths.extend(
            additional_cleanup_paths
                .iter()
                .filter(|path| path.exists())
                .cloned(),
        );
    } else {
        for path in additional_cleanup_paths {
            if !path.exists() {
                continue;
            }
            remove_content_path(path.as_path())?;
            removed_files.push(path.display().to_string());
        }
    }
    if let Some(cleanup) = deferred_cleanup.as_ref() {
        apply_deferred_content_cleanup(instance_root, &manifest, cleanup, &mut removed_files)?;
    }
    save_content_manifest(instance_root, &manifest)?;
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %root_entry.name,
        added_files = added_files.len(),
        removed_files = removed_files.len(),
        "finished content install request"
    );

    Ok(ContentDownloadOutcome {
        project_name: root_entry.name,
        added_files,
        removed_files,
    })
}

#[allow(clippy::too_many_arguments)]
fn install_project_recursive(
    instance_root: &Path,
    manifest: &mut ContentInstallManifest,
    entry: &BrowserProjectEntry,
    resolved: ResolvedDownload,
    game_version: &str,
    loader: BrowserLoader,
    project_key_override: Option<&str>,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    parent_key: Option<&str>,
    explicit: bool,
    prefetched_paths: &HashMap<PathBuf, PathBuf>,
    visited: &mut HashSet<String>,
    added_files: &mut Vec<String>,
    removed_files: &mut Vec<String>,
    mut deferred_cleanup: Option<&mut DeferredContentCleanup>,
) -> Result<(), String> {
    let project_key = project_key_override
        .map(str::to_owned)
        .unwrap_or_else(|| entry.dedupe_key.clone());
    if let Some(parent_key) = parent_key {
        append_project_dependency(manifest, parent_key, project_key.as_str());
    }

    if !visited.insert(project_key.clone()) {
        if explicit && let Some(existing) = manifest.projects.get_mut(&project_key) {
            existing.explicitly_installed = true;
        }
        return Ok(());
    }

    let existing = manifest.projects.get(&project_key).cloned();
    let target_dir = instance_root.join(entry.content_type.folder_name());
    std::fs::create_dir_all(target_dir.as_path())
        .map_err(|err| format!("failed to create content folder {:?}: {err}", target_dir))?;
    let target_name = normalized_filename(resolved.file_name.as_str(), resolved.file_url.as_str());
    let target_path = target_dir.join(target_name.as_str());
    if let Some(existing) = existing.as_ref()
        && existing.selected_source == Some(resolved.source)
        && existing.selected_version_id.as_deref() == Some(resolved.version_id.as_str())
    {
        if explicit && let Some(record) = manifest.projects.get_mut(&project_key) {
            record.explicitly_installed = true;
        }
        return Ok(());
    }

    let previous_file_path = existing
        .as_ref()
        .map(|project| instance_root.join(project.file_path.as_path()));
    let staged_previous_path = match previous_file_path.as_ref() {
        Some(previous_file_path) => {
            stage_existing_file_for_update(previous_file_path.as_path(), target_path.as_path())?
        }
        None => None,
    };
    let previous_dependency_keys = existing
        .as_ref()
        .map(|project| project.direct_dependencies.clone())
        .unwrap_or_default();
    let explicitly_installed = explicit
        || existing
            .as_ref()
            .is_some_and(|project| project.explicitly_installed);
    let prefetched_path = prefetched_paths.get(&target_path).cloned();

    let install_result = (|| -> Result<(), String> {
        if existing.is_some() || !target_path.exists() || prefetched_path.is_some() {
            if let Some(prefetched_path) = prefetched_path.as_ref() {
                if !prefetched_path.exists() {
                    return Err(format!(
                        "prefetched content file missing at {}",
                        prefetched_path.display()
                    ));
                }
                if target_path.exists() {
                    remove_content_path(target_path.as_path())?;
                }
                std::fs::rename(prefetched_path, target_path.as_path()).map_err(|err| {
                    format!(
                        "failed to place prefetched content {} at {}: {err}",
                        prefetched_path.display(),
                        target_path.display()
                    )
                })?;
            } else {
                download_file(resolved.file_url.as_str(), target_path.as_path())?;
            }
            if !added_files
                .iter()
                .any(|path| path == &target_path.display().to_string())
            {
                added_files.push(target_path.display().to_string());
            }
        }

        let file_path = target_path
            .strip_prefix(instance_root)
            .unwrap_or(target_path.as_path())
            .display()
            .to_string();
        manifest.projects.insert(
            project_key.clone(),
            InstalledContentProject {
                project_key: project_key.clone(),
                name: entry.name.clone(),
                folder_name: entry.content_type.folder_name().to_owned(),
                file_path: PathBuf::from(file_path),
                modrinth_project_id: entry.modrinth_project_id.clone(),
                curseforge_project_id: entry.curseforge_project_id,
                selected_source: Some(resolved.source),
                selected_version_id: Some(resolved.version_id.clone()),
                selected_version_name: Some(resolved.version_name.clone()),
                pack_managed: false,
                explicitly_installed,
                direct_dependencies: Vec::new(),
            },
        );

        let mut dependency_keys = Vec::new();
        for dep_entry in
            dependency_to_browser_entries(resolved.dependencies.as_slice(), modrinth, curseforge)?
        {
            let dep_resolved =
                resolve_best_download(&dep_entry, game_version, loader, modrinth, curseforge)?
                    .ok_or_else(|| {
                        format!(
                            "No compatible downloadable file found for dependency {}.",
                            dep_entry.name
                        )
                    })?;
            dependency_keys.push(dep_entry.dedupe_key.clone());
            install_project_recursive(
                instance_root,
                manifest,
                &dep_entry,
                dep_resolved,
                game_version,
                loader,
                None,
                modrinth,
                curseforge,
                Some(project_key.as_str()),
                false,
                prefetched_paths,
                visited,
                added_files,
                removed_files,
                deferred_cleanup.as_deref_mut(),
            )?;
        }

        if let Some(record) = manifest.projects.get_mut(&project_key) {
            record.direct_dependencies = dependency_keys.clone();
            if explicit {
                record.explicitly_installed = true;
            }
        }

        if let Some(previous_file_path) = previous_file_path.as_ref() {
            finalize_updated_file_replacement(
                previous_file_path.as_path(),
                target_path.as_path(),
                staged_previous_path.as_deref(),
                removed_files,
                deferred_cleanup.as_deref_mut(),
            )?;
        }

        for dependency_key in previous_dependency_keys {
            if dependency_keys
                .iter()
                .any(|current| current == &dependency_key)
            {
                continue;
            }
            remove_installed_project(
                instance_root,
                manifest,
                dependency_key.as_str(),
                false,
                removed_files,
                deferred_cleanup.as_deref_mut(),
            )?;
        }

        Ok(())
    })();

    match install_result {
        Ok(()) => Ok(()),
        Err(err) => {
            if let (Some(staged_previous_path), Some(previous_file_path)) =
                (staged_previous_path.as_ref(), previous_file_path.as_ref())
            {
                restore_staged_update_file(
                    staged_previous_path.as_path(),
                    previous_file_path.as_path(),
                )
                .map_err(|restore_err| {
                    format!("{err} (also failed to restore original file: {restore_err})")
                })?;
            }
            Err(err)
        }
    }
}

fn append_project_dependency(
    manifest: &mut ContentInstallManifest,
    parent_key: &str,
    dependency_key: &str,
) {
    if let Some(parent) = manifest.projects.get_mut(parent_key)
        && !parent
            .direct_dependencies
            .iter()
            .any(|existing| existing == dependency_key)
    {
        parent.direct_dependencies.push(dependency_key.to_owned());
    }
}

fn remove_installed_project(
    instance_root: &Path,
    manifest: &mut ContentInstallManifest,
    project_key: &str,
    force: bool,
    removed_files: &mut Vec<String>,
    mut deferred_cleanup: Option<&mut DeferredContentCleanup>,
) -> Result<(), String> {
    let Some(existing) = manifest.projects.get(project_key).cloned() else {
        return Ok(());
    };
    if !force {
        if existing.explicitly_installed {
            return Ok(());
        }
        if !manifest_dependents(manifest, project_key).is_empty() {
            return Ok(());
        }
    }

    manifest.projects.remove(project_key);
    for project in manifest.projects.values_mut() {
        project
            .direct_dependencies
            .retain(|dependency| dependency != project_key);
    }

    let file_path = instance_root.join(existing.file_path.as_path());
    if file_path.exists() {
        if let Some(cleanup) = deferred_cleanup.as_deref_mut() {
            cleanup.stale_paths.push(file_path.clone());
        } else {
            std::fs::remove_file(file_path.as_path())
                .map_err(|err| format!("failed to remove {}: {err}", file_path.display()))?;
            removed_files.push(file_path.display().to_string());
        }
    }

    for dependency_key in existing.direct_dependencies {
        remove_installed_project(
            instance_root,
            manifest,
            dependency_key.as_str(),
            false,
            removed_files,
            deferred_cleanup.as_deref_mut(),
        )?;
    }

    Ok(())
}

pub(super) fn resolved_download_from_version(version: BrowserVersionEntry) -> ResolvedDownload {
    ResolvedDownload {
        source: version.source,
        version_id: version.version_id,
        version_name: version.version_name,
        file_url: version.file_url,
        file_name: version.file_name,
        published_at: version.published_at,
        dependencies: version.dependencies,
    }
}

pub(super) fn content_target_path(
    instance_root: &Path,
    entry: &BrowserProjectEntry,
    version: &BrowserVersionEntry,
) -> PathBuf {
    let target_dir = instance_root.join(entry.content_type.folder_name());
    let target_name = normalized_filename(version.file_name.as_str(), version.file_url.as_str());
    target_dir.join(target_name)
}

pub(super) fn content_target_path_for_resolved_download(
    instance_root: &Path,
    entry: &BrowserProjectEntry,
    resolved: &ResolvedDownload,
) -> PathBuf {
    let target_dir = instance_root.join(entry.content_type.folder_name());
    let target_name = normalized_filename(resolved.file_name.as_str(), resolved.file_url.as_str());
    target_dir.join(target_name)
}

pub(super) fn stage_existing_file_for_update(
    existing_file_path: &Path,
    target_path: &Path,
) -> Result<Option<PathBuf>, String> {
    if !paths_match_for_update(existing_file_path, target_path) || !existing_file_path.exists() {
        return Ok(None);
    }

    let staged_path = staged_update_backup_path(existing_file_path);
    std::fs::rename(existing_file_path, staged_path.as_path()).map_err(|err| {
        format!(
            "failed to stage existing content {} for replacement: {err}",
            existing_file_path.display()
        )
    })?;
    Ok(Some(staged_path))
}

pub(super) fn finalize_updated_file_replacement(
    previous_file_path: &Path,
    target_path: &Path,
    staged_previous_path: Option<&Path>,
    removed_files: &mut Vec<String>,
    deferred_cleanup: Option<&mut DeferredContentCleanup>,
) -> Result<(), String> {
    if let Some(staged_previous_path) = staged_previous_path {
        if let Some(cleanup) = deferred_cleanup {
            cleanup
                .staged_paths
                .push(staged_previous_path.to_path_buf());
        } else {
            remove_content_path(staged_previous_path)?;
            removed_files.push(staged_previous_path.display().to_string());
        }
        return Ok(());
    }

    if paths_match_for_update(previous_file_path, target_path) || !previous_file_path.exists() {
        return Ok(());
    }

    if let Some(cleanup) = deferred_cleanup {
        cleanup.stale_paths.push(previous_file_path.to_path_buf());
    } else {
        remove_content_path(previous_file_path)?;
        removed_files.push(previous_file_path.display().to_string());
    }
    Ok(())
}

pub(super) fn apply_deferred_content_cleanup(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    cleanup: &DeferredContentCleanup,
    removed_files: &mut Vec<String>,
) -> Result<(), String> {
    let active_paths = manifest
        .projects
        .values()
        .map(|project| instance_root.join(project.file_path.as_path()))
        .collect::<HashSet<_>>();

    for staged_path in &cleanup.staged_paths {
        if !staged_path.exists() {
            continue;
        }
        remove_content_path(staged_path.as_path())?;
        removed_files.push(staged_path.display().to_string());
    }

    for stale_path in &cleanup.stale_paths {
        if active_paths.contains(stale_path) || !stale_path.exists() {
            continue;
        }
        remove_content_path(stale_path.as_path())?;
        removed_files.push(stale_path.display().to_string());
    }

    Ok(())
}

pub(super) fn restore_staged_update_file(
    staged_previous_path: &Path,
    previous_file_path: &Path,
) -> Result<(), String> {
    if !staged_previous_path.exists() {
        return Ok(());
    }

    if previous_file_path.exists() {
        remove_content_path(previous_file_path)?;
    }

    std::fs::rename(staged_previous_path, previous_file_path).map_err(|err| {
        format!(
            "failed to restore {} from {}: {err}",
            previous_file_path.display(),
            staged_previous_path.display()
        )
    })
}

fn staged_update_backup_path(existing_file_path: &Path) -> PathBuf {
    let parent = existing_file_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = existing_file_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("content.bin");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    let mut attempt = 0u32;
    loop {
        let candidate = parent.join(format!(
            ".vertex-update-backup-{file_name}-{timestamp}-{attempt}"
        ));
        if !candidate.exists() {
            return candidate;
        }
        attempt += 1;
    }
}

pub(super) fn remove_content_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {err}", path.display()))
    } else {
        std::fs::remove_file(path)
            .map_err(|err| format!("failed to remove {}: {err}", path.display()))
    }
}

pub(super) fn paths_match_for_update(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }

    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

fn manifest_dependents(manifest: &ContentInstallManifest, project_key: &str) -> Vec<String> {
    manifest
        .projects
        .iter()
        .filter(|(key, project)| {
            key.as_str() != project_key
                && project
                    .direct_dependencies
                    .iter()
                    .any(|dependency| dependency == project_key)
        })
        .map(|(_, project)| project.name.clone())
        .collect()
}

pub(super) fn resolve_best_download(
    entry: &BrowserProjectEntry,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Option<ResolvedDownload>, String> {
    let modrinth_candidate = resolve_modrinth_download(entry, game_version, loader, modrinth)?;
    let curseforge_candidate =
        resolve_curseforge_download(entry, game_version, loader, curseforge)?;
    Ok(match (modrinth_candidate, curseforge_candidate) {
        (Some(left), Some(right)) => {
            if left.published_at >= right.published_at {
                Some(left)
            } else {
                Some(right)
            }
        }
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    })
}

fn resolve_modrinth_download(
    entry: &BrowserProjectEntry,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
) -> Result<Option<ResolvedDownload>, String> {
    let Some(project_id) = entry.modrinth_project_id.as_deref() else {
        return Ok(None);
    };

    let mut loaders = Vec::new();
    if matches!(entry.content_type, BrowserContentType::Mod)
        && let Some(loader_slug) = loader.modrinth_slug()
    {
        loaders.push(loader_slug.to_owned());
    }
    let game_versions = if game_version.trim().is_empty() {
        Vec::new()
    } else {
        vec![game_version.trim().to_owned()]
    };

    let versions = modrinth
        .list_project_versions(project_id, &loaders, &game_versions)
        .map_err(|err| format!("Modrinth versions failed for {project_id}: {err}"))?;
    let dependency_version_projects = modrinth_dependency_project_ids(
        modrinth,
        versions
            .iter()
            .flat_map(|version| version.dependencies.iter().cloned())
            .collect::<Vec<_>>()
            .as_slice(),
    );

    Ok(versions
        .into_iter()
        .filter_map(|version| {
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())?;
            let dependencies = modrinth_dependency_refs(
                version.dependencies.as_slice(),
                &dependency_version_projects,
            );
            Some(ResolvedDownload {
                source: ManagedContentSource::Modrinth,
                version_id: version.id.clone(),
                version_name: version.version_number.clone(),
                file_url: file.url.clone(),
                file_name: file.filename.clone(),
                published_at: version.date_published,
                dependencies,
            })
        })
        .max_by(|left, right| left.published_at.cmp(&right.published_at)))
}

fn resolve_curseforge_download(
    entry: &BrowserProjectEntry,
    game_version: &str,
    loader: BrowserLoader,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Option<ResolvedDownload>, String> {
    let Some(curseforge) = curseforge else {
        return Ok(None);
    };
    let Some(project_id) = entry.curseforge_project_id else {
        return Ok(None);
    };

    let mod_loader_type = if matches!(entry.content_type, BrowserContentType::Mod) {
        loader.curseforge_mod_loader_type()
    } else {
        None
    };
    let project = curseforge
        .get_mod(project_id)
        .map_err(|err| format!("CurseForge project lookup failed for {project_id}: {err}"))?;
    let Some(file_id) = project
        .latest_files_indexes
        .iter()
        .filter(|index| {
            normalize_optional(game_version)
                .as_deref()
                .is_none_or(|value| index.game_version.trim() == value)
        })
        .filter(|index| mod_loader_type.is_none_or(|value| index.mod_loader == Some(value)))
        .map(|index| index.file_id)
        .max()
    else {
        return Ok(None);
    };
    let Some(file) = curseforge
        .get_files(&[file_id])
        .map_err(|err| format!("CurseForge file lookup failed for {file_id}: {err}"))?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let Some(url) = file.download_url.clone() else {
        return Ok(None);
    };
    let mut dependencies = Vec::new();
    for dep in file.dependencies {
        if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
            dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
        }
    }
    Ok(Some(ResolvedDownload {
        source: ManagedContentSource::CurseForge,
        version_id: file.id.to_string(),
        version_name: file.display_name.clone(),
        file_url: url,
        file_name: file.file_name,
        published_at: file.file_date,
        dependencies,
    }))
}

pub(super) fn dependency_to_browser_entries(
    dependencies: &[DependencyRef],
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Vec<BrowserProjectEntry>, String> {
    let modrinth_ids = dependencies
        .iter()
        .filter_map(|dependency| match dependency {
            DependencyRef::ModrinthProject(project_id) => Some(project_id.clone()),
            DependencyRef::CurseForgeProject(_) => None,
        })
        .collect::<Vec<_>>();
    let curseforge_ids = dependencies
        .iter()
        .filter_map(|dependency| match dependency {
            DependencyRef::CurseForgeProject(project_id) => Some(*project_id),
            DependencyRef::ModrinthProject(_) => None,
        })
        .collect::<Vec<_>>();

    let modrinth_projects = modrinth
        .get_projects(modrinth_ids.as_slice())
        .unwrap_or_default()
        .into_iter()
        .map(|project| (project.project_id.clone(), project))
        .collect::<HashMap<_, _>>();
    let curseforge_projects = if let Some(curseforge) = curseforge {
        curseforge
            .get_mods(curseforge_ids.as_slice())
            .unwrap_or_default()
            .into_iter()
            .map(|project| (project.id, project))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };

    let mut entries = Vec::new();
    for dependency in dependencies {
        match dependency {
            DependencyRef::ModrinthProject(project_id) => {
                let Some(project) = modrinth_projects.get(project_id.as_str()) else {
                    continue;
                };
                if let Some(entry) = browser_entry_from_modrinth_dependency_project(project) {
                    entries.push(entry);
                }
            }
            DependencyRef::CurseForgeProject(project_id) => {
                let Some(project) = curseforge_projects.get(project_id) else {
                    continue;
                };
                if let Some(entry) = browser_entry_from_curseforge_dependency_project(project) {
                    entries.push(entry);
                }
            }
        }
    }
    Ok(entries)
}

fn browser_entry_from_modrinth_dependency_project(
    project: &modrinth::Project,
) -> Option<BrowserProjectEntry> {
    let content_type = parse_content_type(project.project_type.as_str())?;
    let name_key = normalize_search_key(project.title.as_str());
    if name_key.is_empty() {
        return None;
    }
    Some(BrowserProjectEntry {
        dedupe_key: format!("{}::{name_key}", content_type.label().to_ascii_lowercase()),
        name: project.title.clone(),
        summary: project.description.clone(),
        content_type,
        icon_url: project.icon_url.clone(),
        modrinth_project_id: Some(project.project_id.clone()),
        curseforge_project_id: None,
        sources: vec![ContentSource::Modrinth],
        popularity_score: None,
        updated_at: None,
        relevance_rank: u32::MAX,
    })
}

fn browser_entry_from_curseforge_dependency_project(
    project: &curseforge::Project,
) -> Option<BrowserProjectEntry> {
    let name_key = normalize_search_key(project.name.as_str());
    if name_key.is_empty() {
        return None;
    }
    Some(BrowserProjectEntry {
        dedupe_key: format!("mod::{name_key}"),
        name: project.name.clone(),
        summary: project.summary.clone(),
        content_type: BrowserContentType::Mod,
        icon_url: project.icon_url.clone(),
        modrinth_project_id: None,
        curseforge_project_id: Some(project.id),
        sources: vec![ContentSource::CurseForge],
        popularity_score: None,
        updated_at: None,
        relevance_rank: u32::MAX,
    })
}

pub(super) fn normalize_optional(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn vertex_prefetch_root(instance_root: &Path) -> PathBuf {
    instance_root.join(VERTEX_PREFETCH_DIR_NAME)
}

pub(super) fn prefetched_target_path(
    instance_root: &Path,
    content_type: BrowserContentType,
    target_path: &Path,
) -> PathBuf {
    let file_name = target_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("content.bin");
    vertex_prefetch_root(instance_root)
        .join(content_type.folder_name())
        .join(file_name)
}

pub(super) fn cleanup_prefetched_downloads(instance_root: &Path) -> Result<(), String> {
    let prefetch_root = vertex_prefetch_root(instance_root);
    if prefetch_root.exists() {
        remove_content_path(prefetch_root.as_path())?;
    }
    Ok(())
}

pub(super) fn modrinth_dependency_project_ids(
    modrinth: &ModrinthClient,
    dependencies: &[modrinth::ProjectDependency],
) -> HashMap<String, String> {
    let version_ids = dependencies
        .iter()
        .filter(|dependency| dependency.project_id.is_none())
        .filter_map(|dependency| dependency.version_id.as_ref())
        .cloned()
        .collect::<Vec<_>>();
    modrinth
        .get_versions(version_ids.as_slice())
        .unwrap_or_default()
        .into_iter()
        .filter(|version| !version.project_id.trim().is_empty())
        .map(|version| (version.id, version.project_id))
        .collect()
}

pub(super) fn modrinth_dependency_refs(
    dependencies: &[modrinth::ProjectDependency],
    version_projects: &HashMap<String, String>,
) -> Vec<DependencyRef> {
    let mut resolved = Vec::new();
    for dependency in dependencies {
        if !dependency.dependency_type.eq_ignore_ascii_case("required") {
            continue;
        }
        if let Some(project_id) = dependency.project_id.as_ref() {
            resolved.push(DependencyRef::ModrinthProject(project_id.clone()));
            continue;
        }
        if let Some(version_id) = dependency.version_id.as_ref()
            && let Some(project_id) = version_projects.get(version_id.as_str())
        {
            resolved.push(DependencyRef::ModrinthProject(project_id.clone()));
        }
    }
    resolved
}

pub(super) fn identify_mod_file_by_hash(path: &Path) -> Result<UnifiedContentEntry, String> {
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(path)
        .map_err(|err| format!("failed to hash file: {err}"))?;
    let modrinth = ModrinthClient::default();

    for (algorithm, hash) in [("sha512", sha512.as_str()), ("sha1", sha1.as_str())] {
        let Some(version) = modrinth
            .get_version_from_hash(hash, algorithm)
            .map_err(|err| format!("Modrinth hash lookup failed: {err}"))?
        else {
            continue;
        };
        let project = modrinth
            .get_project(version.project_id.as_str())
            .map_err(|err| format!("Modrinth project lookup failed: {err}"))?;
        return Ok(UnifiedContentEntry {
            id: format!("modrinth:{}", project.project_id),
            name: project.title,
            summary: project.description.trim().to_owned(),
            content_type: project.project_type,
            source: ContentSource::Modrinth,
            project_url: Some(project.project_url),
            icon_url: project.icon_url,
        });
    }

    Err("no Modrinth project matched this file hash".to_owned())
}

fn normalized_filename(name: &str, url: &str) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    url.rsplit('/').next().unwrap_or("download.bin").to_owned()
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    throttle_download_url(url);
    let response = ureq::get(url)
        .call()
        .map_err(|err| format!("download request failed for {url}: {err}"))?;
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read download body from {url}: {err}"))?;
    let mut file = std::fs::File::create(destination)
        .map_err(|err| format!("failed to create {:?}: {err}", destination))?;
    file.write_all(&bytes)
        .map_err(|err| format!("failed to write {:?}: {err}", destination))?;
    Ok(())
}

fn throttle_download_url(url: &str) {
    let Some(spacing) = download_spacing_for_url(url) else {
        return;
    };
    let lock = download_throttle_store(url);
    let Ok(mut next_allowed) = lock.lock() else {
        tracing::error!(
            target: "vertexlauncher/content_browser",
            url,
            throttle_spacing_ms = spacing.as_millis() as u64,
            "Content browser download throttle mutex was poisoned."
        );
        return;
    };
    let now = Instant::now();
    if *next_allowed > now {
        thread::sleep(next_allowed.saturating_duration_since(now));
    }
    *next_allowed = Instant::now() + spacing;
}

fn download_spacing_for_url(url: &str) -> Option<Duration> {
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        Some(MODRINTH_DOWNLOAD_MIN_SPACING)
    } else if host.contains("curseforge.com") || host.contains("forgecdn.net") {
        Some(CURSEFORGE_DOWNLOAD_MIN_SPACING)
    } else {
        None
    }
}

fn download_throttle_store(url: &str) -> &'static Mutex<Instant> {
    static MODRINTH: OnceLock<Mutex<Instant>> = OnceLock::new();
    static CURSEFORGE: OnceLock<Mutex<Instant>> = OnceLock::new();
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        MODRINTH.get_or_init(|| Mutex::new(Instant::now()))
    } else {
        CURSEFORGE.get_or_init(|| Mutex::new(Instant::now()))
    }
}
