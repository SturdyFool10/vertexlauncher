use super::*;

pub(super) fn prefetch_bulk_update_metadata(
    state: &mut InstanceScreenState,
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader: &str,
) {
    if tab_has_known_available_update(state, installed_files) {
        return;
    }

    let _ = super::request_content_metadata_lookup_batch(
        state,
        installed_files,
        kind,
        game_version,
        loader,
        false,
        CONTENT_UPDATE_PREFETCH_BATCH_SIZE,
    );
}

pub(super) fn tab_has_known_available_update(
    state: &InstanceScreenState,
    installed_files: &[InstalledContentFile],
) -> bool {
    installed_files.iter().any(|entry| {
        state
            .content_metadata_cache
            .get(&entry.lookup_key)
            .and_then(|resolution| resolution.as_ref())
            .and_then(|resolution| resolution.update.as_ref())
            .is_some()
    })
}

pub(super) fn bulk_update_button_label(kind: InstalledContentKind) -> String {
    format!("Update all {}", kind.label().to_ascii_lowercase())
}

pub(super) fn bulk_update_button_tooltip(kind: InstalledContentKind) -> String {
    if kind == InstalledContentKind::Mods {
        "Updates all mods to the latest compatible version, you typically should not update pre-made modpacks most of the time if you are playing Multiplayer, or if your modpack is complex".to_owned()
    } else {
        format!(
            "Updates all {} to the latest compatible version for this instance.",
            kind.label().to_ascii_lowercase()
        )
    }
}

pub(super) fn installed_content_lookup_repaint_delay(
    state: &InstanceScreenState,
    installed_files: &[InstalledContentFile],
) -> Option<Duration> {
    let now = Instant::now();
    let mut next_retry: Option<Duration> = None;

    for entry in installed_files {
        if state.content_lookup_in_flight.contains(&entry.lookup_key)
            || !state.content_metadata_cache.contains_key(&entry.lookup_key)
        {
            return Some(CONTENT_LOOKUP_REPAINT_INTERVAL);
        }

        if state
            .content_metadata_cache
            .get(&entry.lookup_key)
            .is_some_and(|resolution| resolution.is_none())
        {
            match state
                .content_lookup_retry_after_by_key
                .get(&entry.lookup_key)
            {
                Some(retry_at) if *retry_at > now => {
                    let remaining = retry_at.saturating_duration_since(now);
                    next_retry = Some(match next_retry {
                        Some(current) => current.min(remaining),
                        None => remaining,
                    });
                }
                _ => return Some(CONTENT_LOOKUP_REPAINT_INTERVAL),
            }
        }
    }

    next_retry
}

pub(super) fn update_all_installed_content(
    instance_root: &Path,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    download_policy: &DownloadPolicy,
    progress: Option<&InstallProgressCallback>,
) -> Result<String, String> {
    let mut updated_count = 0usize;
    let mut pass = 0usize;
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        kind = %kind.folder_name(),
        game_version = %game_version,
        loader = %loader_label,
        "scanning for bulk content updates"
    );

    loop {
        pass += 1;
        if pass > 512 {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                kind = %kind.folder_name(),
                updated_count,
                "aborting bulk content update after too many passes"
            );
            return Err(format!(
                "Stopped updating {} after too many passes.",
                kind.label().to_ascii_lowercase()
            ));
        }

        let managed_identities = load_managed_content_identities(instance_root);
        let installed_files = InstalledContentResolver::scan_installed_content_files(
            instance_root,
            kind,
            &managed_identities,
        );
        tracing::debug!(
            target: CONTENT_UPDATE_LOG_TARGET,
            instance_root = %instance_root.display(),
            kind = %kind.folder_name(),
            pass,
            installed_files = installed_files.len(),
            "scanned installed content for bulk update pass"
        );
        if installed_files.is_empty() {
            break;
        }

        let mut hash_cache = InstalledContentResolver::load_hash_cache(instance_root);
        let manifest = managed_content::load_content_manifest(instance_root);
        let mut cleaned_stale_duplicates = 0usize;
        let mut updates = Vec::new();
        for (file, resolution) in resolve_installed_content_metadata_batch(
            installed_files.as_slice(),
            kind,
            game_version,
            loader_label,
            &mut hash_cache,
        ) {
            let Some(update) = resolution.update.as_ref() else {
                continue;
            };
            if let Some(managed_path) = stale_managed_content_path_for_update(
                instance_root,
                &manifest,
                &file,
                &resolution,
                update.latest_version_id.as_str(),
            ) {
                tracing::warn!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    "removing stale duplicate content during bulk update pass file_path={} managed_path={} project={} latest_version_id={}",
                    file.file_path.display(),
                    managed_path.display(),
                    resolution.entry.name,
                    update.latest_version_id,
                );
                remove_stale_duplicate_content_path(file.file_path.as_path())?;
                cleaned_stale_duplicates += 1;
                continue;
            }
            updates.push(crate::screens::content_browser::BulkContentUpdate {
                entry: resolution.entry.clone(),
                installed_file_path: file.file_path.clone(),
                version_id: update.latest_version_id.clone(),
            });
        }

        if updates.is_empty() {
            if cleaned_stale_duplicates > 0 {
                tracing::info!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    "cleaned stale duplicate content during bulk update pass instance_root={} kind={} pass={} cleaned_duplicates={}",
                    instance_root.display(),
                    kind.folder_name(),
                    pass,
                    cleaned_stale_duplicates,
                );
                continue;
            }
            tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                kind = %kind.folder_name(),
                pass,
                updated_count,
                "no further bulk content updates found"
            );
            break;
        }

        tracing::info!(
            target: CONTENT_UPDATE_LOG_TARGET,
            instance_root = %instance_root.display(),
            kind = %kind.folder_name(),
            pass,
            queued_updates = updates.len(),
            "applying queued bulk content updates"
        );
        let applied = crate::screens::content_browser::bulk_update_installed_content(
            instance_root,
            updates.as_slice(),
            game_version,
            loader_label,
            download_policy,
            progress,
        )?;
        if applied == 0 {
            tracing::warn!(
                target: CONTENT_UPDATE_LOG_TARGET,
                "bulk content update pass made no progress; stopping to avoid a no-op loop instance_root={} kind={} pass={} queued_updates={}",
                instance_root.display(),
                kind.folder_name(),
                pass,
                updates.len(),
            );
            break;
        }
        updated_count += applied;
    }

    let kind_label = kind.label().to_ascii_lowercase();
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        kind = %kind.folder_name(),
        updated_count,
        "finished bulk content update scan"
    );
    if updated_count == 0 {
        Ok(format!("No {kind_label} updates available."))
    } else if updated_count == 1 {
        Ok(format!("Updated 1 {} entry.", kind.content_type_key()))
    } else {
        Ok(format!("Updated {updated_count} {kind_label}."))
    }
}

fn remove_stale_duplicate_content_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove stale directory {}: {err}", path.display()))
    } else {
        std::fs::remove_file(path)
            .map_err(|err| format!("failed to remove stale file {}: {err}", path.display()))
    }
}

fn stale_managed_content_path_for_update(
    instance_root: &Path,
    manifest: &managed_content::ContentInstallManifest,
    file: &InstalledContentFile,
    resolution: &content_resolver::ResolvedInstalledContent,
    latest_version_id: &str,
) -> Option<PathBuf> {
    let project = manifest_project_for_entry(manifest, &resolution.entry)?;
    if project.selected_version_id.as_deref() != Some(latest_version_id) {
        return None;
    }

    let managed_path = instance_root.join(project.file_path.as_path());
    if !managed_path.exists()
        || content_paths_match(managed_path.as_path(), file.file_path.as_path())
    {
        return None;
    }

    Some(managed_path)
}

fn manifest_project_for_entry<'a>(
    manifest: &'a managed_content::ContentInstallManifest,
    entry: &modprovider::UnifiedContentEntry,
) -> Option<&'a managed_content::InstalledContentProject> {
    match entry.source {
        modprovider::ContentSource::Modrinth => {
            let project_id = entry.id.strip_prefix("modrinth:")?;
            manifest
                .projects
                .values()
                .find(|project| project.modrinth_project_id.as_deref() == Some(project_id))
        }
        modprovider::ContentSource::CurseForge => {
            let project_id = entry.id.strip_prefix("curseforge:")?.parse::<u64>().ok()?;
            manifest
                .projects
                .values()
                .find(|project| project.curseforge_project_id == Some(project_id))
        }
    }
}

fn content_paths_match(left: &Path, right: &Path) -> bool {
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

fn resolve_installed_content_for_update(
    file: &InstalledContentFile,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    hash_cache: &mut InstalledContentHashCache,
) -> Option<content_resolver::ResolvedInstalledContent> {
    let request = ResolveInstalledContentRequest {
        file_path: file.file_path.clone(),
        disk_file_name: file.file_name.trim().to_owned(),
        lookup_query: file.lookup_query.trim().to_owned(),
        fallback_lookup_key: file
            .fallback_lookup_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        fallback_lookup_query: file
            .fallback_lookup_query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        managed_identity: file.managed_identity.clone(),
        kind,
        game_version: game_version.trim().to_owned(),
        loader: loader_label.trim().to_owned(),
    };
    let result = InstalledContentResolver::resolve(&request, hash_cache);
    let _ = hash_cache.apply_updates(result.hash_cache_updates);
    result.resolution
}

pub(super) fn resolve_installed_content_metadata_batch(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    hash_cache: &mut InstalledContentHashCache,
) -> Vec<(
    InstalledContentFile,
    content_resolver::ResolvedInstalledContent,
)> {
    let mut prefetched = vec![None; installed_files.len()];
    prefetch_modrinth_hash_updates(
        installed_files,
        kind,
        game_version,
        loader_label,
        hash_cache,
        prefetched.as_mut_slice(),
    );
    prefetch_managed_modrinth_updates(
        installed_files,
        kind,
        game_version,
        loader_label,
        prefetched.as_mut_slice(),
    );
    prefetch_managed_curseforge_updates(
        installed_files,
        kind,
        game_version,
        loader_label,
        prefetched.as_mut_slice(),
    );

    let mut resolved = Vec::new();
    for (index, file) in installed_files.iter().enumerate() {
        let resolution = prefetched[index].clone().or_else(|| {
            resolve_installed_content_for_update(file, kind, game_version, loader_label, hash_cache)
        });
        if let Some(resolution) = resolution {
            resolved.push((file.clone(), resolution));
        }
    }
    resolved
}

fn prefetch_modrinth_hash_updates(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    hash_cache: &mut InstalledContentHashCache,
    prefetched: &mut [Option<content_resolver::ResolvedInstalledContent>],
) {
    #[derive(Clone)]
    struct HashWorkItem {
        index: usize,
        sha1: String,
        sha512: String,
    }

    let modrinth = modrinth::Client::default();
    let loaders = if kind == InstalledContentKind::Mods {
        modrinth_loader_slugs_for_update_prefetch(loader_label)
    } else {
        Vec::new()
    };
    let game_versions = normalized_game_versions_for_update_prefetch(game_version);
    let mut pending_sha512 = Vec::new();
    let mut pending_sha1 = Vec::new();
    let mut cached_sha512 = Vec::new();
    let mut cached_sha1 = Vec::new();
    let mut cache_updates = Vec::new();

    for (index, file) in installed_files.iter().enumerate() {
        if prefetched[index].is_some() {
            continue;
        }
        if !supports_modrinth_hash_prefetch(kind, file.file_path.as_path()) {
            continue;
        }

        let Ok((sha1, sha512)) = modrinth::hash_file_sha1_and_sha512_hex(file.file_path.as_path())
        else {
            continue;
        };
        let sha512_key = format!("sha512:{sha512}");
        let sha1_key = format!("sha1:{sha1}");

        if let Some(Some(resolution)) = hash_cache.entries.get(sha512_key.as_str()) {
            cached_sha512.push((index, sha512, resolution.clone()));
            continue;
        }
        if let Some(Some(resolution)) = hash_cache.entries.get(sha1_key.as_str()) {
            cached_sha1.push((index, sha1, resolution.clone()));
            continue;
        }

        let sha512_missing = !hash_cache.entries.contains_key(sha512_key.as_str());
        let sha1_missing = !hash_cache.entries.contains_key(sha1_key.as_str());
        let item = HashWorkItem {
            index,
            sha1,
            sha512,
        };
        if sha512_missing {
            pending_sha512.push(item);
        } else if sha1_missing {
            pending_sha1.push(item);
        } else {
            cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                hash_key: sha512_key,
                resolution: None,
            });
            cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                hash_key: sha1_key,
                resolution: None,
            });
        }
    }

    let cached_sha512_latest = modrinth
        .get_latest_versions_from_hashes(
            &cached_sha512
                .iter()
                .map(|(_, hash, _)| hash.clone())
                .collect::<Vec<_>>(),
            "sha512",
            loaders.as_slice(),
            game_versions.as_slice(),
        )
        .unwrap_or_default();
    for (index, hash, mut resolution) in cached_sha512 {
        resolution.update = modrinth_update_from_latest_prefetch(
            cached_sha512_latest.get(hash.as_str()),
            resolution.installed_version_id.as_deref(),
        );
        prefetched[index] = Some(resolution);
    }

    let cached_sha1_latest = modrinth
        .get_latest_versions_from_hashes(
            &cached_sha1
                .iter()
                .map(|(_, hash, _)| hash.clone())
                .collect::<Vec<_>>(),
            "sha1",
            loaders.as_slice(),
            game_versions.as_slice(),
        )
        .unwrap_or_default();
    for (index, hash, mut resolution) in cached_sha1 {
        resolution.update = modrinth_update_from_latest_prefetch(
            cached_sha1_latest.get(hash.as_str()),
            resolution.installed_version_id.as_deref(),
        );
        prefetched[index] = Some(resolution);
    }

    let mut pending_sha1_from_sha512 = Vec::new();
    if !pending_sha512.is_empty()
        && let Ok(versions_by_hash) = modrinth.get_versions_from_hashes(
            &pending_sha512
                .iter()
                .map(|item| item.sha512.clone())
                .collect::<Vec<_>>(),
            "sha512",
        )
    {
        let projects_by_id = modrinth_projects_by_id_prefetch(
            &modrinth,
            versions_by_hash
                .values()
                .map(|version| version.project_id.clone())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        let latest_by_hash = modrinth
            .get_latest_versions_from_hashes(
                &pending_sha512
                    .iter()
                    .map(|item| item.sha512.clone())
                    .collect::<Vec<_>>(),
                "sha512",
                loaders.as_slice(),
                game_versions.as_slice(),
            )
            .unwrap_or_default();
        for item in pending_sha512 {
            if let Some(version) = versions_by_hash.get(item.sha512.as_str())
                && let Some(project) = projects_by_id.get(version.project_id.as_str())
            {
                let resolution = modrinth_resolution_from_prefetched_hash(
                    project,
                    version,
                    latest_by_hash.get(item.sha512.as_str()),
                );
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha512:{}", item.sha512),
                    resolution: Some(hash_cache_resolution_without_update_prefetch(&resolution)),
                });
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha1:{}", item.sha1),
                    resolution: Some(hash_cache_resolution_without_update_prefetch(&resolution)),
                });
                prefetched[item.index] = Some(resolution);
                continue;
            }
            pending_sha1_from_sha512.push(item);
        }
    } else {
        pending_sha1_from_sha512 = pending_sha512;
    }
    pending_sha1.extend(pending_sha1_from_sha512);

    if !pending_sha1.is_empty()
        && let Ok(versions_by_hash) = modrinth.get_versions_from_hashes(
            &pending_sha1
                .iter()
                .map(|item| item.sha1.clone())
                .collect::<Vec<_>>(),
            "sha1",
        )
    {
        let projects_by_id = modrinth_projects_by_id_prefetch(
            &modrinth,
            versions_by_hash
                .values()
                .map(|version| version.project_id.clone())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        let latest_by_hash = modrinth
            .get_latest_versions_from_hashes(
                &pending_sha1
                    .iter()
                    .map(|item| item.sha1.clone())
                    .collect::<Vec<_>>(),
                "sha1",
                loaders.as_slice(),
                game_versions.as_slice(),
            )
            .unwrap_or_default();
        for item in pending_sha1 {
            if let Some(version) = versions_by_hash.get(item.sha1.as_str())
                && let Some(project) = projects_by_id.get(version.project_id.as_str())
            {
                let resolution = modrinth_resolution_from_prefetched_hash(
                    project,
                    version,
                    latest_by_hash.get(item.sha1.as_str()),
                );
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha512:{}", item.sha512),
                    resolution: Some(hash_cache_resolution_without_update_prefetch(&resolution)),
                });
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha1:{}", item.sha1),
                    resolution: Some(hash_cache_resolution_without_update_prefetch(&resolution)),
                });
                prefetched[item.index] = Some(resolution);
            } else {
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha512:{}", item.sha512),
                    resolution: None,
                });
                cache_updates.push(content_resolver::InstalledContentHashCacheUpdate {
                    hash_key: format!("sha1:{}", item.sha1),
                    resolution: None,
                });
            }
        }
    }

    let _ = hash_cache.apply_updates(cache_updates);
}

fn prefetch_managed_modrinth_updates(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    prefetched: &mut [Option<content_resolver::ResolvedInstalledContent>],
) {
    #[derive(Clone)]
    struct ModrinthWorkItem {
        index: usize,
        project_id: String,
        version_id: String,
    }

    let mut work_items = Vec::new();
    for (index, file) in installed_files.iter().enumerate() {
        if prefetched[index].is_some() {
            continue;
        }
        let Some(identity) = file.managed_identity.as_ref() else {
            continue;
        };
        if identity.pack_managed {
            continue;
        }
        if identity.source != modprovider::ContentSource::Modrinth {
            continue;
        }
        let Some(project_id) = identity.modrinth_project_id.as_ref() else {
            continue;
        };
        let version_id = identity.selected_version_id.trim();
        if version_id.is_empty() {
            continue;
        }
        work_items.push(ModrinthWorkItem {
            index,
            project_id: project_id.clone(),
            version_id: version_id.to_owned(),
        });
    }
    if work_items.is_empty() {
        return;
    }

    let modrinth = modrinth::Client::default();
    let versions_by_id = match modrinth.get_versions(
        &work_items
            .iter()
            .map(|item| item.version_id.clone())
            .collect::<Vec<_>>(),
    ) {
        Ok(versions) => versions
            .into_iter()
            .map(|version| (version.id.clone(), version))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };
    let projects_by_id = modrinth_projects_by_id_prefetch(
        &modrinth,
        &work_items
            .iter()
            .map(|item| item.project_id.clone())
            .collect::<Vec<_>>(),
    );
    let loaders = if kind == InstalledContentKind::Mods {
        modrinth_loader_slugs_for_update_prefetch(loader_label)
    } else {
        Vec::new()
    };
    let game_versions = normalized_game_versions_for_update_prefetch(game_version);
    let latest_versions_by_project_id = modrinth_latest_versions_by_project_prefetch(
        &modrinth,
        &work_items
            .iter()
            .map(|item| item.project_id.clone())
            .collect::<Vec<_>>(),
        loaders.as_slice(),
        game_versions.as_slice(),
    );

    for item in work_items {
        let Some(version) = versions_by_id.get(item.version_id.as_str()) else {
            continue;
        };
        if version.project_id != item.project_id {
            continue;
        }
        let file = &installed_files[item.index];
        if !version_contains_file_name_for_update_prefetch(
            version.files.as_slice(),
            file.file_name.as_str(),
        ) {
            continue;
        }
        let Some(project) = projects_by_id.get(item.project_id.as_str()) else {
            continue;
        };

        prefetched[item.index] = Some(modrinth_resolution_from_prefetched_managed(
            project,
            version,
            latest_versions_by_project_id.get(item.project_id.as_str()),
        ));
    }
}

fn prefetch_managed_curseforge_updates(
    installed_files: &[InstalledContentFile],
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    prefetched: &mut [Option<content_resolver::ResolvedInstalledContent>],
) {
    let Some(curseforge) = curseforge::Client::from_env() else {
        return;
    };

    #[derive(Clone, Copy)]
    struct CurseForgeWorkItem {
        index: usize,
        project_id: u64,
        version_id: u64,
    }

    let mut work_items = Vec::new();
    for (index, file) in installed_files.iter().enumerate() {
        if prefetched[index].is_some() {
            continue;
        }
        let Some(identity) = file.managed_identity.as_ref() else {
            continue;
        };
        if identity.pack_managed {
            continue;
        }
        if identity.source != modprovider::ContentSource::CurseForge {
            continue;
        }
        let Some(project_id) = identity.curseforge_project_id else {
            continue;
        };
        let Ok(version_id) = identity.selected_version_id.trim().parse::<u64>() else {
            continue;
        };
        work_items.push(CurseForgeWorkItem {
            index,
            project_id,
            version_id,
        });
    }
    if work_items.is_empty() {
        return;
    }

    let installed_files_by_id = match curseforge.get_files(
        &work_items
            .iter()
            .map(|item| item.version_id)
            .collect::<Vec<_>>(),
    ) {
        Ok(files) => files
            .into_iter()
            .map(|file| (file.id, file))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };
    let projects_by_id = match curseforge.get_mods(
        &work_items
            .iter()
            .map(|item| item.project_id)
            .collect::<Vec<_>>(),
    ) {
        Ok(projects) => projects
            .into_iter()
            .map(|project| (project.id, project))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };

    let latest_file_ids = work_items
        .iter()
        .filter_map(|item| {
            projects_by_id.get(&item.project_id).and_then(|project| {
                select_curseforge_latest_file_id_prefetch(project, kind, game_version, loader_label)
            })
        })
        .collect::<Vec<_>>();
    let latest_files_by_id = match curseforge.get_files(latest_file_ids.as_slice()) {
        Ok(files) => files
            .into_iter()
            .map(|file| (file.id, file))
            .collect::<std::collections::HashMap<_, _>>(),
        Err(_) => return,
    };

    for item in work_items {
        let Some(project) = projects_by_id.get(&item.project_id) else {
            continue;
        };
        let Some(latest_file_id) =
            select_curseforge_latest_file_id_prefetch(project, kind, game_version, loader_label)
        else {
            continue;
        };
        let Some(installed_file) = installed_files_by_id.get(&item.version_id) else {
            continue;
        };
        let Some(latest_file) = latest_files_by_id.get(&latest_file_id) else {
            continue;
        };

        let file = &installed_files[item.index];
        if !file_name_matches_for_update_prefetch(
            installed_file.file_name.as_str(),
            file.file_name.as_str(),
        ) || !file
            .file_path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                file_name_matches_for_update_prefetch(value, file.file_name.as_str())
            })
        {
            continue;
        }

        let update = if latest_file.id == item.version_id || latest_file.download_url.is_none() {
            None
        } else {
            Some(content_resolver::InstalledContentUpdate {
                latest_version_id: latest_file.id.to_string(),
                latest_version_label: non_empty_owned_for_update_prefetch(
                    latest_file.display_name.as_str(),
                )
                .unwrap_or_else(|| "Unknown update".to_owned()),
            })
        };

        prefetched[item.index] = Some(content_resolver::ResolvedInstalledContent {
            entry: modprovider::UnifiedContentEntry {
                id: format!("curseforge:{}", project.id),
                name: project.name.clone(),
                summary: project.summary.trim().to_owned(),
                content_type: kind.content_type_key().to_owned(),
                source: modprovider::ContentSource::CurseForge,
                project_url: project.website_url.clone(),
                icon_url: project.icon_url.clone(),
            },
            installed_version_id: Some(installed_file.id.to_string()),
            installed_version_label: non_empty_owned_for_update_prefetch(
                installed_file.display_name.as_str(),
            ),
            resolution_kind: content_resolver::InstalledContentResolutionKind::Managed,
            warning_message: None,
            update,
        });
    }
}

fn modrinth_projects_by_id_prefetch(
    modrinth: &modrinth::Client,
    project_ids: &[String],
) -> std::collections::HashMap<String, modrinth::Project> {
    modrinth
        .get_projects(project_ids)
        .unwrap_or_default()
        .into_iter()
        .map(|project| (project.project_id.clone(), project))
        .collect()
}

fn modrinth_latest_versions_by_project_prefetch(
    modrinth: &modrinth::Client,
    project_ids: &[String],
    loaders: &[String],
    game_versions: &[String],
) -> std::collections::HashMap<String, modrinth::ProjectVersion> {
    let mut latest_versions = std::collections::HashMap::new();
    let mut seen = HashSet::new();

    for project_id in project_ids {
        if !seen.insert(project_id.clone()) {
            continue;
        }
        let Ok(versions) =
            modrinth.list_project_versions(project_id.as_str(), loaders, game_versions)
        else {
            continue;
        };
        let Some(latest) = versions
            .into_iter()
            .filter(|version| !version.files.is_empty())
            .max_by(|left, right| left.date_published.cmp(&right.date_published))
        else {
            continue;
        };
        latest_versions.insert(project_id.clone(), latest);
    }

    latest_versions
}

fn modrinth_resolution_from_prefetched_hash(
    project: &modrinth::Project,
    version: &modrinth::ProjectVersion,
    latest: Option<&modrinth::ProjectVersion>,
) -> content_resolver::ResolvedInstalledContent {
    content_resolver::ResolvedInstalledContent {
        entry: modprovider::UnifiedContentEntry {
            id: format!("modrinth:{}", project.project_id),
            name: project.title.clone(),
            summary: project.description.trim().to_owned(),
            content_type: project.project_type.clone(),
            source: modprovider::ContentSource::Modrinth,
            project_url: Some(project.project_url.clone()),
            icon_url: project.icon_url.clone(),
        },
        installed_version_id: non_empty_owned_for_update_prefetch(version.id.as_str()),
        installed_version_label: non_empty_owned_for_update_prefetch(
            version.version_number.as_str(),
        ),
        resolution_kind: content_resolver::InstalledContentResolutionKind::ExactHash,
        warning_message: None,
        update: modrinth_update_from_latest_prefetch(latest, Some(version.id.as_str())),
    }
}

fn modrinth_resolution_from_prefetched_managed(
    project: &modrinth::Project,
    version: &modrinth::ProjectVersion,
    latest: Option<&modrinth::ProjectVersion>,
) -> content_resolver::ResolvedInstalledContent {
    content_resolver::ResolvedInstalledContent {
        entry: modprovider::UnifiedContentEntry {
            id: format!("modrinth:{}", project.project_id),
            name: project.title.clone(),
            summary: project.description.trim().to_owned(),
            content_type: project.project_type.clone(),
            source: modprovider::ContentSource::Modrinth,
            project_url: Some(project.project_url.clone()),
            icon_url: project.icon_url.clone(),
        },
        installed_version_id: non_empty_owned_for_update_prefetch(version.id.as_str()),
        installed_version_label: non_empty_owned_for_update_prefetch(
            version.version_number.as_str(),
        ),
        resolution_kind: content_resolver::InstalledContentResolutionKind::Managed,
        warning_message: None,
        update: modrinth_update_from_latest_prefetch(latest, Some(version.id.as_str())),
    }
}

fn modrinth_update_from_latest_prefetch(
    latest: Option<&modrinth::ProjectVersion>,
    installed_version_id: Option<&str>,
) -> Option<content_resolver::InstalledContentUpdate> {
    let latest = latest?;
    if installed_version_id.is_some_and(|value| value == latest.id) {
        return None;
    }

    Some(content_resolver::InstalledContentUpdate {
        latest_version_id: latest.id.clone(),
        latest_version_label: non_empty_owned_for_update_prefetch(latest.version_number.as_str())
            .unwrap_or_else(|| "Unknown update".to_owned()),
    })
}

fn hash_cache_resolution_without_update_prefetch(
    resolution: &content_resolver::ResolvedInstalledContent,
) -> content_resolver::ResolvedInstalledContent {
    let mut cached = resolution.clone();
    cached.update = None;
    cached
}

fn select_curseforge_latest_file_id_prefetch(
    project: &curseforge::Project,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
) -> Option<u64> {
    let game_version = normalize_optional_for_update_prefetch(game_version);
    let mod_loader_type = if kind == InstalledContentKind::Mods {
        curseforge_loader_type_for_update_prefetch(loader_label)
    } else {
        None
    };

    project
        .latest_files_indexes
        .iter()
        .filter(|index| {
            game_version
                .as_deref()
                .is_none_or(|value| index.game_version.trim() == value)
        })
        .filter(|index| mod_loader_type.is_none_or(|value| index.mod_loader == Some(value)))
        .map(|index| index.file_id)
        .max()
}

fn supports_modrinth_hash_prefetch(kind: InstalledContentKind, path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };

    match kind {
        InstalledContentKind::Mods => extension.eq_ignore_ascii_case("jar"),
        InstalledContentKind::ResourcePacks
        | InstalledContentKind::ShaderPacks
        | InstalledContentKind::DataPacks => extension.eq_ignore_ascii_case("zip"),
    }
}

fn file_name_matches_for_update_prefetch(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && left.eq_ignore_ascii_case(right)
}

fn version_contains_file_name_for_update_prefetch(
    files: &[modrinth::ProjectVersionFile],
    disk_file_name: &str,
) -> bool {
    files
        .iter()
        .any(|file| file_name_matches_for_update_prefetch(file.filename.as_str(), disk_file_name))
}

fn modrinth_loader_slugs_for_update_prefetch(loader: &str) -> Vec<String> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "fabric" => vec!["fabric".to_owned()],
        "forge" => vec!["forge".to_owned()],
        "neoforge" => vec!["neoforge".to_owned()],
        "quilt" => vec!["quilt".to_owned()],
        _ => Vec::new(),
    }
}

fn curseforge_loader_type_for_update_prefetch(loader: &str) -> Option<u32> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

fn normalized_game_versions_for_update_prefetch(game_version: &str) -> Vec<String> {
    normalize_optional_for_update_prefetch(game_version)
        .into_iter()
        .collect()
}

fn normalize_optional_for_update_prefetch(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn non_empty_owned_for_update_prefetch(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
