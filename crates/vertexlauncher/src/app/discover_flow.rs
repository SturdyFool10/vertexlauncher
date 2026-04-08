use super::*;

pub(super) fn ensure_discover_install_channel(app: &mut VertexApp) {
    if app.discover_install_results_tx.is_some() && app.discover_install_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportTaskResult>();
    app.discover_install_results_tx = Some(tx);
    app.discover_install_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn ensure_discover_install_progress_channel(app: &mut VertexApp) {
    if app.discover_install_progress_tx.is_some() && app.discover_install_progress_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportProgress>();
    app.discover_install_progress_tx = Some(tx);
    app.discover_install_progress_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn start_discover_install_task(
    app: &mut VertexApp,
    request: screens::DiscoverInstallRequest,
) {
    if app.show_import_instance_modal
        || app.import_instance_state.import_in_flight
        || app.curseforge_manual_download_preflight_in_flight
        || app.discover_curseforge_manual_download_preflight_in_flight
        || app.pending_curseforge_manual_download.is_some()
    {
        return;
    }
    if matches!(
        &request.source,
        screens::DiscoverInstallSource::CurseForge {
            manual_download_path: None,
            ..
        }
    ) {
        start_discover_curseforge_manual_download_preflight(app, request);
        return;
    }
    spawn_discover_install_task(app, request);
}

pub(super) fn ensure_discover_curseforge_manual_download_preflight_channel(app: &mut VertexApp) {
    if app
        .discover_curseforge_manual_download_preflight_tx
        .is_some()
        && app
            .discover_curseforge_manual_download_preflight_rx
            .is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<
        Result<Option<import_instance_modal::CurseForgeManualDownloadRequirement>, String>,
    >();
    app.discover_curseforge_manual_download_preflight_tx = Some(tx);
    app.discover_curseforge_manual_download_preflight_rx = Some(rx);
}

pub(super) fn start_discover_curseforge_manual_download_preflight(
    app: &mut VertexApp,
    request: screens::DiscoverInstallRequest,
) {
    let (project_id, file_id) = match &request.source {
        screens::DiscoverInstallSource::CurseForge {
            project_id,
            file_id,
            ..
        } => (*project_id, *file_id),
        _ => {
            spawn_discover_install_task(app, request);
            return;
        }
    };
    ensure_discover_curseforge_manual_download_preflight_channel(app);
    let Some(tx) = app
        .discover_curseforge_manual_download_preflight_tx
        .as_ref()
        .cloned()
    else {
        return;
    };
    app.discover_state
        .begin_install("Checking CurseForge download restrictions...");
    app.discover_curseforge_manual_download_preflight_request = Some(request);
    app.discover_curseforge_manual_download_preflight_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            import_instance_modal::prepare_curseforge_manual_download_for_file(project_id, file_id)
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/discover",
                project_id,
                file_id,
                error = %err,
                "Failed to deliver discover manual-download preflight result."
            );
        }
    });
}

pub(super) fn poll_discover_curseforge_manual_download_preflight(app: &mut VertexApp) {
    if !app.discover_curseforge_manual_download_preflight_in_flight {
        return;
    }
    let Some(rx) = app
        .discover_curseforge_manual_download_preflight_rx
        .as_ref()
    else {
        return;
    };
    let Ok(result) = rx.try_recv() else {
        return;
    };
    app.discover_curseforge_manual_download_preflight_in_flight = false;
    let request = app
        .discover_curseforge_manual_download_preflight_request
        .take();
    match (request, result) {
        (Some(request), Ok(Some(requirement))) => {
            match PendingCurseForgeManualDownloadState::new(
                ManualDownloadContinuation::DiscoverInstall(request),
                vec![requirement],
                HashMap::new(),
                app.config.minecraft_installations_root_path(),
            ) {
                Ok(mut pending) => {
                    if let Err(err) = scan_curseforge_manual_downloads(&mut pending) {
                        cleanup_pending_curseforge_manual_download(Some(pending));
                        app.discover_state.finish_install(Err(format!(
                            "Failed to prepare manual CurseForge download: {err}"
                        )));
                        return;
                    }
                    if pending.pending_files.is_empty() {
                        let mut request = match &pending.continuation {
                            ManualDownloadContinuation::DiscoverInstall(request) => request.clone(),
                            ManualDownloadContinuation::Import(_) => return,
                        };
                        if let screens::DiscoverInstallSource::CurseForge {
                            manual_download_path,
                            download_url,
                            ..
                        } = &mut request.source
                        {
                            *manual_download_path = pending.staged_files.values().next().cloned();
                            *download_url = None;
                        }
                        app.pending_curseforge_manual_download = Some(pending);
                        spawn_discover_install_task(app, request);
                    } else {
                        app.discover_state
                            .begin_install("Waiting for manual CurseForge download...");
                        app.pending_curseforge_manual_download = Some(pending);
                    }
                }
                Err(err) => {
                    app.discover_state.finish_install(Err(format!(
                        "Failed to prepare manual CurseForge download: {err}"
                    )));
                }
            }
        }
        (Some(request), Ok(None)) => {
            spawn_discover_install_task(app, request);
        }
        (_, Err(err)) => {
            app.discover_state
                .finish_install(Err(format!("Failed to prepare CurseForge install: {err}")));
        }
        (None, Ok(_)) => {}
    }
}

pub(super) fn spawn_discover_install_task(
    app: &mut VertexApp,
    request: screens::DiscoverInstallRequest,
) {
    ensure_discover_install_channel(app);
    ensure_discover_install_progress_channel(app);
    let Some(tx) = app.discover_install_results_tx.as_ref().cloned() else {
        return;
    };
    let Some(progress_tx) = app.discover_install_progress_tx.as_ref().cloned() else {
        return;
    };

    app.discover_state
        .begin_install(format!("Downloading {}...", request.version_name));
    let store = app.instance_store.clone();
    let installations_root = app.config.minecraft_installations_root_path().to_path_buf();
    let _ = tokio_runtime::spawn_detached(async move {
        let result =
            install_discover_modpack_in_background(store, installations_root, request, progress_tx);
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/discover",
                error = %err,
                "Failed to deliver discover install result."
            );
        }
    });
}

pub(super) fn poll_discover_install_progress(app: &mut VertexApp) {
    let Some(rx) = app.discover_install_progress_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/app/discover",
            "Discover install progress receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok(progress) => {
                app.discover_state.apply_install_progress(
                    progress.message,
                    progress.completed_steps,
                    progress.total_steps,
                );
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/app/discover",
                    "Discover install progress worker disconnected unexpectedly."
                );
                break;
            }
        }
    }
}

pub(super) fn poll_discover_install_result(app: &mut VertexApp) {
    let Some(rx) = app.discover_install_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/app/discover",
            "Discover install result receiver mutex was poisoned."
        );
        return;
    };
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            tracing::error!(
                target: "vertexlauncher/app/discover",
                "Discover install result worker disconnected unexpectedly."
            );
            app.discover_state
                .finish_install(Err("Install task stopped unexpectedly.".to_owned()));
            return;
        }
    };

    match result {
        Ok((store, instance)) => {
            let installations_root = app.config.minecraft_installations_root_path().to_path_buf();
            let config = app.config.clone();
            app.instance_store = store;
            cleanup_pending_curseforge_manual_download(
                app.pending_curseforge_manual_download.take(),
            );
            start_initial_instance_install(app, &instance, installations_root.as_path(), &config);
            app.selected_instance_id = Some(instance.id);
            app.active_screen = screens::AppScreen::Instance;
            app.discover_state
                .finish_install(Ok("Created instance from modpack.".to_owned()));
            app.refresh_instance_shortcuts();
        }
        Err(err) => {
            cleanup_pending_curseforge_manual_download(
                app.pending_curseforge_manual_download.take(),
            );
            tracing::error!(
                target: "vertexlauncher/app/discover",
                error = %err,
                "Discover modpack install failed."
            );
            app.discover_state.finish_install(Err(err.to_string()));
        }
    }
}

pub(super) fn install_discover_modpack_in_background(
    store: InstanceStore,
    installations_root: PathBuf,
    request: screens::DiscoverInstallRequest,
    progress_tx: mpsc::Sender<import_instance_modal::ImportProgress>,
) -> import_instance_modal::ImportTaskResult {
    let instance_name = request.instance_name.clone();
    let project_summary = request.project_summary.clone();
    let icon_url = request.icon_url.clone();
    match request.source {
        screens::DiscoverInstallSource::Modrinth {
            file_url,
            file_name,
            ..
        } => {
            let temp_path = download_discover_modpack_file(
                file_url.as_str(),
                file_name.as_str(),
                &progress_tx,
            )?;
            let import_request = import_instance_modal::ImportRequest {
                source: import_instance_modal::ImportSource::ManifestFile(temp_path.clone()),
                instance_name: instance_name.clone(),
                manual_curseforge_files: HashMap::new(),
                manual_curseforge_staging_dir: None,
                max_concurrent_downloads: 4,
            };
            let result = import_package_in_background(
                store,
                installations_root.clone(),
                import_request,
                progress_tx,
            );
            if let Err(err) = fs::remove_file(temp_path.as_path()) {
                tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %temp_path.display(), error = %err, context = "cleanup discover temp package");
            }
            finalize_discover_instance(
                result,
                installations_root.as_path(),
                instance_name.as_str(),
                project_summary.as_deref(),
                icon_url.as_deref(),
            )
        }
        screens::DiscoverInstallSource::CurseForge {
            project_id,
            file_id,
            file_name,
            download_url,
            manual_download_path,
        } => {
            let temp_path = if let Some(path) = manual_download_path {
                path
            } else {
                let download_url = match download_url {
                    Some(url) => url,
                    None => curseforge::Client::from_env()
                        .ok_or_else(|| "CurseForge API key missing in settings.".to_owned())?
                        .get_mod_file_download_url(project_id, file_id)
                        .map_err(|err| {
                            import_instance_modal::format_curseforge_download_url_error(
                                project_id, file_id, &err,
                            )
                        })?
                        .ok_or_else(|| {
                            format!(
                                "CurseForge file {file_id} for project {project_id} has no download URL"
                            )
                        })?,
                };
                download_discover_modpack_file(
                    download_url.as_str(),
                    file_name.as_str(),
                    &progress_tx,
                )?
            };
            let import_request = import_instance_modal::ImportRequest {
                source: import_instance_modal::ImportSource::ManifestFile(temp_path.clone()),
                instance_name: instance_name.clone(),
                manual_curseforge_files: HashMap::new(),
                manual_curseforge_staging_dir: None,
                max_concurrent_downloads: 4,
            };
            let result = import_package_in_background(
                store,
                installations_root.clone(),
                import_request,
                progress_tx,
            );
            let final_result = result.and_then(|(store, instance)| {
                let instance_root = instance_root_path(installations_root.as_path(), &instance);
                import_instance_modal::attach_curseforge_modpack_install_state(
                    instance_root.as_path(),
                    project_id,
                    file_id,
                    instance_name.as_str(),
                    request.version_name.as_str(),
                )?;
                Ok((store, instance))
            });
            if let Err(err) = fs::remove_file(temp_path.as_path()) {
                tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %temp_path.display(), error = %err, context = "cleanup discover temp package");
            }
            finalize_discover_instance(
                final_result,
                installations_root.as_path(),
                instance_name.as_str(),
                project_summary.as_deref(),
                icon_url.as_deref(),
            )
        }
    }
}

pub(super) fn finalize_discover_instance(
    result: import_instance_modal::ImportTaskResult,
    installations_root: &Path,
    instance_name: &str,
    project_summary: Option<&str>,
    icon_url: Option<&str>,
) -> import_instance_modal::ImportTaskResult {
    let (mut store, instance) = result?;
    apply_discover_instance_metadata(
        &mut store,
        installations_root,
        instance.id.as_str(),
        instance_name,
        project_summary,
        icon_url,
    )?;
    let updated = store
        .find(instance.id.as_str())
        .cloned()
        .ok_or_else(|| format!("instance {} disappeared after install", instance.id))?;
    Ok((store, updated))
}

pub(super) fn apply_discover_instance_metadata(
    store: &mut InstanceStore,
    installations_root: &Path,
    instance_id: &str,
    instance_name: &str,
    project_summary: Option<&str>,
    icon_url: Option<&str>,
) -> Result<(), String> {
    let instance = store
        .find_mut(instance_id)
        .ok_or_else(|| format!("instance {instance_id} disappeared during discover install"))?;
    instance.name = instance_name.trim().to_owned();
    if let Some(summary) = project_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        instance.description = Some(summary.to_owned());
    }

    let instance_root = instance_root_path(installations_root, instance);
    if let Some(icon_url) = icon_url.map(str::trim).filter(|value| !value.is_empty()) {
        match download_discover_thumbnail(icon_url, instance_root.as_path(), instance_id) {
            Ok(Some(path)) => instance.thumbnail_path = Some(path),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/app/discover",
                    instance_id,
                    error = %err,
                    "failed to persist discover thumbnail"
                );
            }
        }
    }

    save_instance_store(store).map_err(|err| format!("failed to save instance metadata: {err}"))
}

pub(super) fn download_discover_modpack_file(
    url: &str,
    file_name: &str,
    progress_tx: &mpsc::Sender<import_instance_modal::ImportProgress>,
) -> Result<PathBuf, String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to download modpack from {url}: {err}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read modpack download from {url}: {err}"))?;
    if let Err(err) = progress_tx.send(import_instance_modal::ImportProgress {
        message: "Downloaded modpack package. Importing instance...".to_owned(),
        completed_steps: 1,
        total_steps: 1,
    }) {
        tracing::error!(
            target: "vertexlauncher/app/discover",
            url,
            error = %err,
            "Failed to deliver discover-download progress update."
        );
    }
    let temp_path = std::env::temp_dir().join(format!(
        "vertex-discover-{}-{}",
        std::process::id(),
        sanitize_temp_file_name(file_name)
    ));
    let mut file = fs::File::create(temp_path.as_path()).map_err(|err| {
        tracing::warn!(target: "vertexlauncher/io", op = "file_create", path = %temp_path.display(), error = %err, context = "create discover temp package");
        format!(
            "failed to create temp package {}: {err}",
            temp_path.display()
        )
    })?;
    file.write_all(bytes.as_slice()).map_err(|err| {
        format!(
            "failed to write temp package {}: {err}",
            temp_path.display()
        )
    })?;
    Ok(temp_path)
}

pub(super) fn sanitize_temp_file_name(file_name: &str) -> String {
    let sanitized = file_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.trim().is_empty() {
        "modpack.mrpack".to_owned()
    } else {
        sanitized
    }
}

pub(super) fn download_discover_thumbnail(
    url: &str,
    instance_root: &Path,
    instance_id: &str,
) -> Result<Option<PathBuf>, String> {
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to download thumbnail from {url}: {err}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read thumbnail from {url}: {err}"))?;
    if bytes.is_empty() {
        return Ok(None);
    }

    let extension = thumbnail_extension_from_url(url);
    let path = instance_root.join(format!(
        ".vertex-discover-thumbnail-{instance_id}.{extension}"
    ));
    fs::write(path.as_path(), bytes).map_err(|err| {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err, context = "write discover thumbnail");
        format!("failed to write thumbnail {}: {err}", path.display())
    })?;
    Ok(Some(path))
}

pub(super) fn thumbnail_extension_from_url(url: &str) -> &'static str {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "jpg"
    } else if path.ends_with(".webp") {
        "webp"
    } else if path.ends_with(".svg") {
        "svg"
    } else {
        "png"
    }
}
