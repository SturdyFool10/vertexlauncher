use super::*;

#[path = "installation_core/async_helpers.rs"]
mod async_helpers;
#[path = "installation_core/install_types.rs"]
mod install_types;
#[path = "installation_core/installation_error.rs"]
mod installation_error;
#[path = "installation_core/java_update.rs"]
mod java_update;
#[path = "installation_core/version_catalog_types.rs"]
mod version_catalog_types;

use async_helpers::*;
pub use install_types::*;
pub use installation_error::*;
pub use java_update::*;
pub use version_catalog_types::*;

pub fn download_batch(
    tasks: Vec<DownloadBatchTask>,
    policy: &DownloadPolicy,
) -> Result<u32, InstallationError> {
    download_batch_with_progress(tasks, policy, InstallStage::DownloadingCore, None)
}

pub fn download_batch_with_progress(
    tasks: Vec<DownloadBatchTask>,
    policy: &DownloadPolicy,
    stage: InstallStage,
    progress: Option<&InstallProgressCallback>,
) -> Result<u32, InstallationError> {
    let tasks = tasks
        .into_iter()
        .map(|task| FileDownloadTask {
            url: task.url,
            destination: task.destination,
            expected_size: task.expected_size,
        })
        .collect();
    download_files_concurrent(stage, tasks, policy, 0, progress.map(Arc::as_ref))
}

pub fn fetch_version_catalog(
    filter: VersionCatalogFilter,
) -> Result<VersionCatalog, InstallationError> {
    fetch_version_catalog_with_refresh(filter, false)
}

pub fn fetch_version_catalog_with_refresh(
    filter: VersionCatalogFilter,
    force_refresh: bool,
) -> Result<VersionCatalog, InstallationError> {
    let cached = read_cached_version_catalog(filter).ok();
    if !force_refresh
        && let Some(cached) = cached.as_ref()
        && cached.filter == filter
        && !is_cache_expired(cached.fetched_at_unix_secs)
        && catalog_has_loader_version_data(&cached.catalog)
    {
        let mut catalog = cached.catalog.clone();
        normalize_version_catalog_ordering(&mut catalog);
        return Ok(catalog);
    }

    match fetch_version_catalog_uncached(filter) {
        Ok(catalog) => {
            let _ = write_cached_version_catalog(filter, &catalog);
            Ok(catalog)
        }
        Err(err) => {
            if let Some(cached) = cached
                && cached.filter == filter
            {
                let mut catalog = cached.catalog;
                normalize_version_catalog_ordering(&mut catalog);
                Ok(catalog)
            } else {
                Err(err)
            }
        }
    }
}

pub fn purge_cache() -> Result<(), InstallationError> {
    let cache_root = cache_root_dir();
    match fs_remove_dir_all(&cache_root) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(InstallationError::Io(err)),
    }
}

pub async fn purge_cache_async() -> Result<(), InstallationError> {
    let cache_root = cache_root_dir();
    match launcher_runtime::fs::remove_dir_all(&cache_root).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(InstallationError::Io(err)),
    }
}

pub fn fetch_loader_versions_for_game(
    loader_label: &str,
    game_version: &str,
    force_refresh: bool,
) -> Result<Vec<String>, InstallationError> {
    let game_version = game_version.trim();
    if game_version.is_empty() {
        return Ok(Vec::new());
    }

    let loader_kind = normalized_loader_label(loader_label);
    if matches!(loader_kind, LoaderKind::Vanilla | LoaderKind::Custom) {
        return Ok(Vec::new());
    }

    let cached = read_cached_loader_versions(loader_kind).ok();
    if !force_refresh
        && let Some(cached) = cached.as_ref()
        && !is_cache_expired(cached.fetched_at_unix_secs)
        && let Some(versions) = cached.versions_by_game_version.get(game_version)
    {
        return Ok(sort_loader_versions_desc(versions.clone()));
    }

    match fetch_loader_versions_for_game_uncached(loader_kind, game_version) {
        Ok(result) => {
            let LoaderVersionFetchResult {
                selected_versions,
                versions_by_game_version,
            } = result;
            let mut updated_cache = cached.unwrap_or_default();
            updated_cache.fetched_at_unix_secs = now_unix_secs();
            updated_cache.loader_label = loader_label.to_owned();
            if versions_by_game_version.is_empty() {
                updated_cache
                    .versions_by_game_version
                    .insert(game_version.to_owned(), selected_versions.clone());
            } else {
                updated_cache
                    .versions_by_game_version
                    .extend(versions_by_game_version);
                updated_cache
                    .versions_by_game_version
                    .entry(game_version.to_owned())
                    .or_insert_with(|| selected_versions.clone());
            }
            sort_loader_version_map_desc(&mut updated_cache.versions_by_game_version);
            let _ = write_cached_loader_versions(loader_kind, &updated_cache);
            Ok(sort_loader_versions_desc(selected_versions))
        }
        Err(err) => {
            if let Some(cached) = cached
                && let Some(versions) = cached.versions_by_game_version.get(game_version)
            {
                Ok(sort_loader_versions_desc(versions.clone()))
            } else {
                Err(err)
            }
        }
    }
}

fn fetch_version_catalog_uncached(
    filter: VersionCatalogFilter,
) -> Result<VersionCatalog, InstallationError> {
    let (manifest, fabric, forge, neoforge, quilt) = thread::scope(|scope| {
        let manifest_task =
            scope.spawn(|| get_json::<MojangVersionManifest>(MOJANG_VERSION_MANIFEST_URL));
        let fabric_task = scope.spawn(fetch_fabric_loader_catalog_with_fallback);
        let forge_task = scope.spawn(fetch_forge_loader_catalog_with_fallback);
        let neoforge_task = scope.spawn(fetch_neoforge_loader_catalog_with_fallback);
        let quilt_task = scope.spawn(fetch_quilt_loader_catalog_with_fallback);

        let manifest = manifest_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::new(
                ErrorKind::Other,
                "minecraft version manifest task panicked",
            ))
        })??;
        let fabric = fabric_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other("fabric loader catalog task panicked"))
        })?;
        let forge = forge_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other("forge loader catalog task panicked"))
        })?;
        let neoforge = neoforge_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other(
                "neoforge loader catalog task panicked",
            ))
        })?;
        let quilt = quilt_task.join().map_err(|_| {
            InstallationError::Io(std::io::Error::other("quilt loader catalog task panicked"))
        })?;
        Ok::<_, InstallationError>((manifest, fabric, forge, neoforge, quilt))
    })?;

    let mut game_versions_with_release_time: Vec<(String, MinecraftVersionEntry)> = manifest
        .versions
        .into_iter()
        .filter_map(|entry| {
            let version_type = map_version_type(entry.version_type.as_str());
            let include = match version_type {
                MinecraftVersionType::Release => true,
                MinecraftVersionType::Snapshot | MinecraftVersionType::OldBeta => {
                    filter.include_snapshots_and_betas
                }
                MinecraftVersionType::OldAlpha => filter.include_alpha,
                MinecraftVersionType::Unknown => filter.include_experimental,
            };
            if include {
                Some((
                    entry.release_time.clone(),
                    MinecraftVersionEntry {
                        id: entry.id,
                        version_type,
                        release_time: Some(entry.release_time),
                    },
                ))
            } else {
                None
            }
        })
        .collect();
    game_versions_with_release_time.sort_by(|left, right| right.0.cmp(&left.0));
    let game_versions: Vec<MinecraftVersionEntry> = game_versions_with_release_time
        .into_iter()
        .map(|(_, entry)| entry)
        .collect();

    let loader_support = LoaderSupportIndex {
        fabric: fabric.supported_game_versions,
        forge: forge.supported_game_versions,
        neoforge: neoforge.supported_game_versions,
        quilt: quilt.supported_game_versions,
    };
    let mut loader_versions = LoaderVersionIndex {
        fabric: fabric.versions_by_game_version,
        forge: forge.versions_by_game_version,
        neoforge: neoforge.versions_by_game_version,
        quilt: quilt.versions_by_game_version,
    };
    loader_versions.sort_desc();

    Ok(VersionCatalog {
        game_versions,
        loader_support,
        loader_versions,
    })
}

pub fn ensure_game_files(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    modloader_version: Option<&str>,
    java_executable: Option<&str>,
    download_policy: &DownloadPolicy,
    progress: Option<InstallProgressCallback>,
) -> Result<GameSetupResult, InstallationError> {
    let game_version = game_version.trim();
    if game_version.is_empty() {
        return Err(InstallationError::UnknownMinecraftVersion(String::new()));
    }
    tracing::info!(
        target: "vertexlauncher/installation/process",
        instance_root = %instance_root.display(),
        game_version = %game_version,
        modloader = %modloader,
        requested_modloader_version = %modloader_version.unwrap_or(""),
        "Starting ensure_game_files."
    );

    let versions_dir = instance_root.join("versions").join(game_version);
    fs_create_dir_all(&versions_dir)?;
    let version_json_path = versions_dir.join(format!("{game_version}.json"));
    let client_jar_path = versions_dir.join(format!("{game_version}.jar"));
    fs_create_dir_all(instance_root.join("mods"))?;
    fs_create_dir_all(instance_root.join("assets"))?;
    fs_create_dir_all(instance_root.join("libraries"))?;
    fs_create_dir_all(instance_root.join("resourcepacks"))?;
    fs_create_dir_all(instance_root.join("shaderpacks"))?;
    report_install_progress(
        progress.as_deref(),
        InstallProgress {
            stage: InstallStage::PreparingFolders,
            message: format!("Prepared instance folders for Minecraft {game_version}."),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
    tracing::info!(
        target: "vertexlauncher/installation/process",
        instance_root = %instance_root.display(),
        game_version = %game_version,
        "Prepared instance folders."
    );

    let mut downloaded_files = 0;

    if !version_json_path.exists() || !client_jar_path.exists() {
        downloaded_files += download_missing_core_files(
            instance_root,
            game_version,
            version_json_path.as_path(),
            client_jar_path.as_path(),
            download_policy,
            downloaded_files,
            progress.as_deref(),
        )?;
    }
    downloaded_files += download_version_dependencies(
        instance_root,
        version_json_path.as_path(),
        download_policy,
        downloaded_files,
        progress.as_deref(),
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/process",
            instance_root = %instance_root.display(),
            game_version = %game_version,
            version_json_path = %version_json_path.display(),
            error = %err,
            "Version dependency download phase failed during ensure_game_files."
        );
        err
    })?;
    tracing::info!(
        target: "vertexlauncher/installation/process",
        game_version = %game_version,
        downloaded_files,
        "Version dependency download phase completed."
    );

    let resolved_modloader_version = install_selected_modloader(
        instance_root,
        game_version,
        modloader,
        modloader_version,
        java_executable,
        download_policy,
        &mut downloaded_files,
        progress.as_deref(),
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/process",
            instance_root = %instance_root.display(),
            game_version = %game_version,
            modloader = %modloader,
            requested_modloader_version = %modloader_version.unwrap_or(""),
            error = %err,
            "Modloader installation phase failed during ensure_game_files."
        );
        err
    })?;
    tracing::info!(
        target: "vertexlauncher/installation/process",
        game_version = %game_version,
        modloader = %modloader,
        resolved_modloader_version = %resolved_modloader_version.as_deref().unwrap_or(""),
        downloaded_files,
        "Modloader installation phase completed."
    );
    if let Some(loader_version) = resolved_modloader_version.as_deref() {
        downloaded_files += download_loader_profile_dependencies(
            instance_root,
            game_version,
            modloader,
            loader_version,
            download_policy,
            downloaded_files,
            progress.as_deref(),
        )?;
    }

    report_install_progress(
        progress.as_deref(),
        InstallProgress {
            stage: InstallStage::Complete,
            message: format!("Installation prepared for Minecraft {game_version}."),
            downloaded_files,
            total_files: downloaded_files.max(1),
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: Some(0),
        },
    );
    tracing::info!(
        target: "vertexlauncher/installation/process",
        instance_root = %instance_root.display(),
        game_version = %game_version,
        final_downloaded_files = downloaded_files,
        resolved_modloader_version = %resolved_modloader_version.as_deref().unwrap_or(""),
        "ensure_game_files completed successfully."
    );
    Ok(GameSetupResult {
        version_json_path,
        client_jar_path,
        downloaded_files,
        resolved_modloader_version,
    })
}

fn download_missing_core_files(
    instance_root: &Path,
    game_version: &str,
    version_json_path: &Path,
    client_jar_path: &Path,
    download_policy: &DownloadPolicy,
    downloaded_files: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    tracing::info!(
        target: "vertexlauncher/installation/process",
        game_version = %game_version,
        "Core version files missing; resolving metadata and scheduling core downloads."
    );
    report_install_progress(
        progress,
        InstallProgress {
            stage: InstallStage::ResolvingMetadata,
            message: format!("Resolving Minecraft {game_version} metadata..."),
            downloaded_files,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
    let manifest: MojangVersionManifest = get_json(MOJANG_VERSION_MANIFEST_URL)?;
    let version_entry = manifest
        .versions
        .into_iter()
        .find(|entry| entry.id == game_version)
        .ok_or_else(|| InstallationError::UnknownMinecraftVersion(game_version.to_owned()))?;

    let version_meta: MojangVersionMeta = get_json(&version_entry.url)?;
    let client_download = version_meta
        .downloads
        .and_then(|downloads| downloads.client)
        .ok_or_else(|| InstallationError::MissingClientDownload(game_version.to_owned()))?;

    let mut tasks = Vec::new();
    if !version_json_path.exists() {
        tasks.push(FileDownloadTask {
            url: version_entry.url,
            destination: version_json_path.to_path_buf(),
            expected_size: None,
        });
    }
    if !client_jar_path.exists() {
        tasks.push(FileDownloadTask {
            url: client_download.url,
            destination: client_jar_path.to_path_buf(),
            expected_size: client_download.size,
        });
    }
    let downloaded = download_files_concurrent(
        InstallStage::DownloadingCore,
        tasks,
        download_policy,
        downloaded_files,
        progress,
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/process",
            instance_root = %instance_root.display(),
            game_version = %game_version,
            version_json_path = %version_json_path.display(),
            client_jar_path = %client_jar_path.display(),
            error = %err,
            "Core file download batch failed during ensure_game_files."
        );
        err
    })?;
    tracing::info!(
        target: "vertexlauncher/installation/process",
        game_version = %game_version,
        downloaded_files = downloaded_files + downloaded,
        "Core file download batch completed."
    );
    Ok(downloaded)
}

fn download_loader_profile_dependencies(
    instance_root: &Path,
    game_version: &str,
    modloader: &str,
    loader_version: &str,
    download_policy: &DownloadPolicy,
    downloaded_files: u32,
    progress: Option<&InstallProgressSink>,
) -> Result<u32, InstallationError> {
    let loader_kind = normalized_loader_label(modloader);
    if !matches!(loader_kind, LoaderKind::Fabric | LoaderKind::Quilt) {
        return Ok(0);
    }
    let id_prefix = if loader_kind == LoaderKind::Fabric {
        "fabric-loader"
    } else {
        "quilt-loader"
    };
    let version_id = format!("{id_prefix}-{loader_version}-{game_version}");
    let loader_profile_path = instance_root
        .join("versions")
        .join(version_id.as_str())
        .join(format!("{version_id}.json"));
    download_version_dependencies(
        instance_root,
        loader_profile_path.as_path(),
        download_policy,
        downloaded_files,
        progress,
    )
    .map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/process",
            instance_root = %instance_root.display(),
            game_version = %game_version,
            loader_profile_path = %loader_profile_path.display(),
            error = %err,
            "Loader profile dependency download phase failed during ensure_game_files."
        );
        err
    })
}

pub async fn ensure_game_files_async(
    instance_root: PathBuf,
    game_version: String,
    modloader: String,
    modloader_version: Option<String>,
    java_executable: Option<String>,
    download_policy: DownloadPolicy,
    progress: Option<InstallProgressCallback>,
) -> Result<GameSetupResult, InstallationError> {
    launcher_runtime::spawn_blocking(move || {
        ensure_game_files(
            instance_root.as_path(),
            game_version.as_str(),
            modloader.as_str(),
            modloader_version.as_deref(),
            java_executable.as_deref(),
            &download_policy,
            progress,
        )
    })
    .await
    .map_err(map_join_error)?
}
