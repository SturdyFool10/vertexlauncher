use super::*;

pub(super) fn ensure_create_instance_channel(state: &mut create_instance_modal::CreateInstanceState) {
    if state.create_results_tx.is_some() && state.create_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<create_instance_modal::CreateInstanceTaskResult>();
    state.create_results_tx = Some(tx);
    state.create_results_rx = Some(rx);
}

pub(super) fn start_create_instance_task(
    app: &mut VertexApp,
    draft: create_instance_modal::CreateInstanceDraft,
) {
    if app.create_instance_state.create_in_flight {
        return;
    }

    ensure_create_instance_channel(&mut app.create_instance_state);
    let Some(tx) = app
        .create_instance_state
        .create_results_tx
        .as_ref()
        .cloned()
    else {
        return;
    };

    app.create_instance_state.error = None;
    app.create_instance_state.create_in_flight = true;
    let mut store = app.instance_store.clone();
    let installations_root = app.config.minecraft_installations_root_path().to_path_buf();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = create_instance(
            &mut store,
            &installations_root,
            draft.into_new_instance_spec(),
        )
        .map(|instance| (store, instance))
        .map_err(|err| err.to_string());
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/create_instance",
                error = %err,
                "Failed to deliver create-instance task result."
            );
        }
    });
}

pub(super) fn poll_create_instance_result(app: &mut VertexApp) {
    let Some(result) = app
        .create_instance_state
        .create_results_rx
        .as_ref()
        .map(|rx| rx.try_recv())
    else {
        return;
    };
    let result = match result {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            app.create_instance_state.create_results_tx = None;
            app.create_instance_state.create_results_rx = None;
            app.create_instance_state.create_in_flight = false;
            app.create_instance_state.error =
                Some("Create instance task stopped unexpectedly.".to_owned());
            tracing::error!(
                target: "vertexlauncher/app/create_instance",
                "Create-instance worker channel stopped unexpectedly."
            );
            return;
        }
    };

    app.create_instance_state.create_in_flight = false;
    match result {
        Ok((store, instance)) => {
            let installations_root = app.config.minecraft_installations_root_path().to_path_buf();
            let config = app.config.clone();
            app.instance_store = store;
            start_initial_instance_install(app, &instance, installations_root.as_path(), &config);
            app.selected_instance_id = Some(instance.id);
            app.active_screen = screens::AppScreen::Instance;
            app.show_create_instance_modal = false;
            app.create_instance_state.reset();
            app.refresh_instance_shortcuts();
        }
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/app/create_instance",
                error = %err,
                "Create-instance task failed."
            );
            app.create_instance_state.error = Some(format!("Failed to create instance: {err}"));
        }
    }
}

pub(super) fn ensure_import_instance_channel(state: &mut import_instance_modal::ImportInstanceState) {
    if state.import_results_tx.is_some() && state.import_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportTaskResult>();
    state.import_results_tx = Some(tx);
    state.import_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn ensure_import_instance_progress_channel(state: &mut import_instance_modal::ImportInstanceState) {
    if state.import_progress_tx.is_some() && state.import_progress_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<import_instance_modal::ImportProgress>();
    state.import_progress_tx = Some(tx);
    state.import_progress_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn start_import_instance_task(
    app: &mut VertexApp,
    mut request: import_instance_modal::ImportRequest,
) {
    if app.import_instance_state.import_in_flight
        || app.curseforge_manual_download_preflight_in_flight
        || app.pending_curseforge_manual_download.is_some()
    {
        return;
    }

    request.max_concurrent_downloads = app.config.download_max_concurrent().max(1);
    if matches!(
        request.source,
        import_instance_modal::ImportSource::ManifestFile(_)
    ) && request.manual_curseforge_files.is_empty()
    {
        start_curseforge_manual_download_preflight(app, request);
        return;
    }

    spawn_import_instance_task(app, request);
}

pub(super) fn start_curseforge_manual_download_preflight(
    app: &mut VertexApp,
    request: import_instance_modal::ImportRequest,
) {
    let (tx, rx) = mpsc::channel::<
        Result<Option<Vec<import_instance_modal::CurseForgeManualDownloadRequirement>>, String>,
    >();
    app.curseforge_manual_download_preflight_rx = Some(rx);
    app.import_instance_state.error = None;
    app.import_instance_state.import_in_flight = true;
    app.import_instance_state.import_latest_progress =
        Some(import_instance_modal::ImportProgress {
            message: "Checking CurseForge download restrictions...".to_owned(),
            completed_steps: 0,
            total_steps: 1,
        });
    app.curseforge_manual_download_preflight_request = Some(request.clone());
    app.curseforge_manual_download_preflight_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            import_instance_modal::prepare_curseforge_manual_downloads(&request)
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/import",
                error = %err,
                "Failed to deliver CurseForge manual-download preflight result."
            );
        }
    });
}

pub(super) fn poll_curseforge_manual_download_preflight(app: &mut VertexApp) {
    if !app.curseforge_manual_download_preflight_in_flight {
        return;
    }
    let Some(rx) = app.curseforge_manual_download_preflight_rx.as_ref() else {
        return;
    };
    let Ok(result) = rx.try_recv() else {
        return;
    };
    app.curseforge_manual_download_preflight_in_flight = false;
    app.import_instance_state.import_in_flight = false;
    app.import_instance_state.import_latest_progress = None;
    let request = app.curseforge_manual_download_preflight_request.take();
    match (request, result) {
        (Some(request), Ok(Some(requirements))) if !requirements.is_empty() => {
            match PendingCurseForgeManualDownloadState::new(
                ManualDownloadContinuation::Import(request),
                requirements,
                HashMap::new(),
                app.config.minecraft_installations_root_path(),
            ) {
                Ok(mut pending) => {
                    if let Err(err) = scan_curseforge_manual_downloads(&mut pending) {
                        tracing::error!(
                            target: "vertexlauncher/app/import",
                            error = %err,
                            "Failed to prepare manual CurseForge downloads after preflight."
                        );
                        app.import_instance_state.error = Some(format!(
                            "Failed to prepare manual CurseForge downloads: {err}"
                        ));
                        cleanup_pending_curseforge_manual_download(Some(pending));
                        return;
                    }
                    if pending.pending_files.is_empty() {
                        let mut request = match &pending.continuation {
                            ManualDownloadContinuation::Import(request) => request.clone(),
                            ManualDownloadContinuation::DiscoverInstall(_) => return,
                        };
                        request.manual_curseforge_files = pending.staged_files.clone();
                        request.manual_curseforge_staging_dir = Some(pending.staging_dir.clone());
                        spawn_import_instance_task(app, request);
                    } else {
                        app.pending_curseforge_manual_download = Some(pending);
                    }
                }
                Err(err) => {
                    tracing::error!(
                        target: "vertexlauncher/app/import",
                        error = %err,
                        "Failed to initialize pending manual CurseForge download state."
                    );
                    app.import_instance_state.error = Some(format!(
                        "Failed to prepare manual CurseForge downloads: {err}"
                    ));
                }
            }
        }
        (Some(request), Ok(_)) => {
            spawn_import_instance_task(app, request);
        }
        (_, Err(err)) => {
            tracing::error!(
                target: "vertexlauncher/app/import",
                error = %err,
                "CurseForge manual-download preflight failed."
            );
            app.import_instance_state.error =
                Some(format!("Failed to prepare CurseForge import: {err}"));
        }
        (None, Ok(_)) => {}
    }
}

pub(super) fn spawn_import_instance_task(app: &mut VertexApp, request: import_instance_modal::ImportRequest) {
    ensure_import_instance_channel(&mut app.import_instance_state);
    ensure_import_instance_progress_channel(&mut app.import_instance_state);
    let Some(tx) = app
        .import_instance_state
        .import_results_tx
        .as_ref()
        .cloned()
    else {
        return;
    };
    let Some(progress_tx) = app
        .import_instance_state
        .import_progress_tx
        .as_ref()
        .cloned()
    else {
        return;
    };

    app.import_instance_state.error = None;
    app.import_instance_state.import_in_flight = true;
    app.import_instance_state.import_latest_progress = None;
    app.in_flight_import_request = Some(request.clone());
    let store = app.instance_store.clone();
    let installations_root = app.config.minecraft_installations_root_path().to_path_buf();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = import_package_in_background(store, installations_root, request, progress_tx);
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/import",
                error = %err,
                "Failed to deliver import task result."
            );
        }
    });
}

pub(super) fn cleanup_import_request_manual_staging(request: Option<&import_instance_modal::ImportRequest>) {
    let Some(request) = request else {
        return;
    };
    if let Some(staging_dir) = request.manual_curseforge_staging_dir.as_ref() {
        if let Err(err) = fs::remove_dir_all(staging_dir.as_path()) {
            tracing::warn!(
                target: "vertexlauncher/io",
                op = "remove_dir_all",
                path = %staging_dir.display(),
                error = %err,
                context = "cleanup import manual CurseForge staging"
            );
        }
    }
}

pub(super) fn poll_import_instance_progress(app: &mut VertexApp) {
    let Some(rx) = app
        .import_instance_state
        .import_progress_rx
        .as_ref()
        .cloned()
    else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/app/import",
            "Import progress receiver mutex was poisoned."
        );
        return;
    };
    while let Ok(progress) = receiver.try_recv() {
        app.import_instance_state.import_latest_progress = Some(progress);
    }
}

pub(super) fn poll_import_instance_result(app: &mut VertexApp) {
    let Some(rx) = app
        .import_instance_state
        .import_results_rx
        .as_ref()
        .cloned()
    else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/app/import",
            "Import result receiver mutex was poisoned."
        );
        return;
    };
    let Ok(result) = receiver.try_recv() else {
        return;
    };

    app.import_instance_state.import_in_flight = false;
    app.import_instance_state.import_latest_progress = None;
    let mut original_request = app.in_flight_import_request.take();
    match result {
        Ok((store, instance)) => {
            let installations_root = app.config.minecraft_installations_root_path().to_path_buf();
            let config = app.config.clone();
            app.instance_store = store;
            cleanup_import_request_manual_staging(original_request.as_ref());
            cleanup_pending_curseforge_manual_download(
                app.pending_curseforge_manual_download.take(),
            );
            start_initial_instance_install(app, &instance, installations_root.as_path(), &config);
            app.selected_instance_id = Some(instance.id);
            app.active_screen = screens::AppScreen::Instance;
            app.show_import_instance_modal = false;
            app.import_instance_state.reset();
            app.refresh_instance_shortcuts();
        }
        Err(err) => {
            if let (
                Some(request),
                import_instance_modal::ImportPackageError::ManualCurseForgeDownloads {
                    requirements,
                    staged_files,
                },
            ) = (original_request.take(), err.clone())
            {
                cleanup_import_request_manual_staging(Some(&request));
                match PendingCurseForgeManualDownloadState::new(
                    ManualDownloadContinuation::Import(request),
                    requirements,
                    staged_files,
                    app.config.minecraft_installations_root_path(),
                ) {
                    Ok(mut pending) => {
                        if let Err(scan_err) = scan_curseforge_manual_downloads(&mut pending) {
                            tracing::error!(
                                target: "vertexlauncher/app/import",
                                error = %scan_err,
                                "Failed to rescan reopened manual CurseForge downloads."
                            );
                            cleanup_pending_curseforge_manual_download(Some(pending));
                            app.import_instance_state.error = Some(format!(
                                "Failed to reopen manual CurseForge downloads: {scan_err}"
                            ));
                            return;
                        }
                        if pending.pending_files.is_empty() {
                            let mut request = match &pending.continuation {
                                ManualDownloadContinuation::Import(request) => request.clone(),
                                ManualDownloadContinuation::DiscoverInstall(_) => return,
                            };
                            request.manual_curseforge_files = pending.staged_files.clone();
                            request.manual_curseforge_staging_dir =
                                Some(pending.staging_dir.clone());
                            spawn_import_instance_task(app, request);
                        } else {
                            app.pending_curseforge_manual_download = Some(pending);
                        }
                        return;
                    }
                    Err(setup_err) => {
                        tracing::error!(
                            target: "vertexlauncher/app/import",
                            error = %setup_err,
                            "Failed to rebuild manual CurseForge download state from import error."
                        );
                        app.import_instance_state.error = Some(format!(
                            "Failed to reopen manual CurseForge downloads: {setup_err}"
                        ));
                        return;
                    }
                }
            }
            cleanup_import_request_manual_staging(original_request.as_ref());
            cleanup_pending_curseforge_manual_download(
                app.pending_curseforge_manual_download.take(),
            );
            tracing::error!(
                target: "vertexlauncher/app/import",
                error = %err,
                "Import profile task failed."
            );
            app.import_instance_state.error = Some(format!("Failed to import profile: {err}"));
        }
    }
}

#[derive(Debug)]
pub(super) enum ManualDownloadContinuation {
    Import(import_instance_modal::ImportRequest),
    DiscoverInstall(screens::DiscoverInstallRequest),
}

#[derive(Debug)]
pub(super) struct PendingCurseForgeManualDownloadState {
    pub(super) continuation: ManualDownloadContinuation,
    pub(super) downloads_dir: PathBuf,
    pub(super) staging_dir: PathBuf,
    pub(super) pending_files: Vec<import_instance_modal::CurseForgeManualDownloadRequirement>,
    pub(super) staged_files: HashMap<u64, PathBuf>,
    pub(super) last_scan_at: Instant,
    pub(super) error: Option<String>,
}

impl PendingCurseForgeManualDownloadState {
    pub(super) fn new(
        continuation: ManualDownloadContinuation,
        pending_files: Vec<import_instance_modal::CurseForgeManualDownloadRequirement>,
        initial_staged_files: HashMap<u64, PathBuf>,
        installations_root: &Path,
    ) -> Result<Self, String> {
        let downloads_dir = default_downloads_dir(installations_root);
        let staging_dir = std::env::temp_dir().join(format!(
            "vertexlauncher-cf-manual-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        fs::create_dir_all(staging_dir.as_path()).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %staging_dir.display(), error = %err, context = "create manual CurseForge staging");
            format!("failed to create staging directory: {err}")
        })?;
        let mut staged_files = HashMap::new();
        for (file_id, source_path) in initial_staged_files {
            let file_name = source_path
                .file_name()
                .ok_or_else(|| {
                    format!(
                        "staged CurseForge retry file {} had no file name",
                        source_path.display()
                    )
                })?
                .to_owned();
            let destination = staging_dir.join(file_name);
            if source_path != destination {
                fs::copy(source_path.as_path(), destination.as_path()).map_err(|err| {
                    tracing::warn!(target: "vertexlauncher/io", op = "copy", from = %source_path.display(), to = %destination.display(), error = %err, context = "stage CurseForge retry file");
                    format!(
                        "failed to copy staged CurseForge retry file {} into {}: {err}",
                        source_path.display(),
                        destination.display()
                    )
                })?;
            }
            staged_files.insert(file_id, destination);
        }
        Ok(Self {
            continuation,
            downloads_dir,
            staging_dir,
            pending_files,
            staged_files,
            last_scan_at: Instant::now() - Duration::from_secs(1),
            error: None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ManualCurseForgeDownloadAction {
    None,
    Cancel,
    OpenDownloadsFolder,
}

pub(super) fn render_curseforge_manual_download_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut PendingCurseForgeManualDownloadState,
) -> ManualCurseForgeDownloadAction {
    let mut action = ManualCurseForgeDownloadAction::None;
    let response = show_dialog(
        ctx,
        dialog_options("curseforge_manual_download_modal", DialogPreset::Form),
        |ui| {
            let modal_max_height = ui.max_rect().height();
            let body_style = LabelOptions {
                color: ui.visuals().text_color(),
                wrap: true,
                ..LabelOptions::default()
            };
            let subtle_style = LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            };
            let error_style = LabelOptions {
                color: ui.visuals().error_fg_color,
                wrap: true,
                ..LabelOptions::default()
            };
            let _ = text_ui.label(
                ui,
                "cf_manual_download_intro",
                "Some files in this CurseForge pack cannot be downloaded through the third-party API. Download them from CurseForge, and Vertex will continue automatically when they appear.",
                &body_style,
            );
            ui.add_space(ui::style::SPACE_SM);
            ui.horizontal(|ui| {
                ui.spinner();
                let message = format!(
                    "{} file{} remaining",
                    state.pending_files.len(),
                    if state.pending_files.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                );
                let _ = text_ui.label(
                    ui,
                    "cf_manual_download_status",
                    message.as_str(),
                    &body_style,
                );
            });
            let watched_path = format!(
                "Watching {}",
                display_user_path(state.downloads_dir.as_path())
            );
            let _ = text_ui.label(
                ui,
                "cf_manual_download_watched_path",
                watched_path.as_str(),
                &subtle_style,
            );
            if let Some(error) = state.error.as_deref() {
                ui.add_space(ui::style::SPACE_XS);
                let _ = text_ui.label(ui, "cf_manual_download_error", error, &error_style);
            }
            ui.add_space(ui::style::SPACE_SM);
            ui.horizontal(|ui| {
                if text_ui
                    .button(
                        ui,
                        "cf_manual_download_open_downloads",
                        "Open Downloads Folder",
                        &secondary_button(ui, egui::vec2(190.0, ui::style::CONTROL_HEIGHT)),
                    )
                    .clicked()
                {
                    action = ManualCurseForgeDownloadAction::OpenDownloadsFolder;
                }
                let staged_message = format!(
                    "{} of {} detected",
                    state.staged_files.len(),
                    state.staged_files.len() + state.pending_files.len()
                );
                let _ = text_ui.label(
                    ui,
                    "cf_manual_download_detected_count",
                    staged_message.as_str(),
                    &subtle_style,
                );
            });
            ui.add_space(ui::style::SPACE_SM);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height((modal_max_height - 210.0).max(140.0))
                .show(ui, |ui| {
                    for requirement in &state.pending_files {
                        ui.group(|ui| {
                            let title = format!(
                                "{}\n{}",
                                requirement.project_name, requirement.display_name
                            );
                            let _ = text_ui.label(
                                ui,
                                ("cf_manual_download_title", requirement.file_id),
                                title.as_str(),
                                &body_style,
                            );
                            let file_name = format!("Expected file: {}", requirement.file_name);
                            let _ = text_ui.label(
                                ui,
                                ("cf_manual_download_file", requirement.file_id),
                                file_name.as_str(),
                                &subtle_style,
                            );
                            let reference = format!(
                                "CurseForge project {}, file {}",
                                requirement.project_id, requirement.file_id
                            );
                            let _ = text_ui.label(
                                ui,
                                ("cf_manual_download_ids", requirement.file_id),
                                reference.as_str(),
                                &subtle_style,
                            );
                            ui.hyperlink_to(
                                "Open CurseForge file page",
                                requirement.download_page_url.as_str(),
                            );
                        });
                        ui.add_space(ui::style::SPACE_XS);
                    }
                });
            ui.add_space(ui::style::SPACE_SM);
            let cancel_label = match state.continuation {
                ManualDownloadContinuation::Import(_) => "Cancel Import",
                ManualDownloadContinuation::DiscoverInstall(_) => "Cancel Install",
            };
            if text_ui
                .button(
                    ui,
                    "cf_manual_download_cancel",
                    cancel_label,
                    &secondary_button(ui, egui::vec2(160.0, ui::style::CONTROL_HEIGHT)),
                )
                .clicked()
            {
                action = ManualCurseForgeDownloadAction::Cancel;
            }
        },
    );
    if response.close_requested && matches!(action, ManualCurseForgeDownloadAction::None) {
        action = ManualCurseForgeDownloadAction::Cancel;
    }
    action
}

pub(super) fn poll_pending_curseforge_manual_download(app: &mut VertexApp) {
    let mut should_resume = false;
    {
        let Some(state) = app.pending_curseforge_manual_download.as_mut() else {
            return;
        };
        if state.last_scan_at.elapsed() < Duration::from_millis(400) {
            return;
        }
        state.last_scan_at = Instant::now();
        match scan_curseforge_manual_downloads(state) {
            Ok(()) => {
                if state.pending_files.is_empty() {
                    should_resume = true;
                }
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/app/import",
                    error = %err,
                    remaining_files = state.pending_files.len(),
                    "Pending manual CurseForge download scan failed."
                );
                state.error = Some(err);
            }
        }
    }
    if !should_resume {
        return;
    }
    let Some(mut pending) = app.pending_curseforge_manual_download.take() else {
        return;
    };
    pending.error = None;
    match &mut pending.continuation {
        ManualDownloadContinuation::Import(request) => {
            request.manual_curseforge_files = pending.staged_files.clone();
            request.manual_curseforge_staging_dir = Some(pending.staging_dir.clone());
            let request = request.clone();
            spawn_import_instance_task(app, request);
        }
        ManualDownloadContinuation::DiscoverInstall(request) => {
            let Some(staged_path) = pending.staged_files.values().next().cloned() else {
                pending.error = Some("Manual CurseForge download staging was empty.".to_owned());
                app.pending_curseforge_manual_download = Some(pending);
                return;
            };
            if let screens::DiscoverInstallSource::CurseForge {
                manual_download_path,
                download_url,
                ..
            } = &mut request.source
            {
                *manual_download_path = Some(staged_path);
                *download_url = None;
            }
            let request = request.clone();
            cleanup_pending_curseforge_manual_download(Some(pending));
            spawn_discover_install_task(app, request);
        }
    }
}

pub(super) fn scan_curseforge_manual_downloads(
    state: &mut PendingCurseForgeManualDownloadState,
) -> Result<(), String> {
    let entries = fs::read_dir(state.downloads_dir.as_path())
        .map_err(|err| format!("failed to read downloads folder: {err}"))?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to inspect downloads folder: {err}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        candidates.push((file_name.to_owned(), path));
    }

    let mut found = Vec::new();
    for requirement in &state.pending_files {
        let Some((_, source_path)) = candidates.iter().find(|(candidate_name, _)| {
            downloaded_filename_matches(candidate_name.as_str(), requirement.file_name.as_str())
        }) else {
            continue;
        };
        let staged_path = state.staging_dir.join(requirement.file_name.as_str());
        fs::copy(source_path, staged_path.as_path()).map_err(|err| {
            tracing::warn!(target: "vertexlauncher/io", op = "copy", from = %source_path.display(), to = %staged_path.display(), error = %err, context = "stage manual CurseForge file");
            format!(
                "failed to stage {} from downloads folder: {err}",
                requirement.file_name
            )
        })?;
        found.push((requirement.file_id, staged_path));
    }
    if found.is_empty() {
        return Ok(());
    }
    state.pending_files.retain(|requirement| {
        !found
            .iter()
            .any(|(file_id, _)| *file_id == requirement.file_id)
    });
    for (file_id, staged_path) in found {
        state.staged_files.insert(file_id, staged_path);
    }
    state.error = None;
    Ok(())
}

pub(super) fn downloaded_filename_matches(candidate_name: &str, expected_name: &str) -> bool {
    if candidate_name == expected_name {
        return true;
    }
    let expected_path = Path::new(expected_name);
    let candidate_path = Path::new(candidate_name);
    let Some(expected_stem) = expected_path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let Some(candidate_stem) = candidate_path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    if expected_path.extension() != candidate_path.extension() {
        return false;
    }
    let Some(suffix) = candidate_stem.strip_prefix(expected_stem) else {
        return false;
    };
    suffix.starts_with(" (")
        && suffix.ends_with(')')
        && suffix[2..suffix.len() - 1]
            .chars()
            .all(|ch| ch.is_ascii_digit())
}

pub(super) fn cleanup_pending_curseforge_manual_download(
    pending: Option<PendingCurseForgeManualDownloadState>,
) {
    let Some(pending) = pending else {
        return;
    };
    if let Err(err) = fs::remove_dir_all(pending.staging_dir.as_path()) {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_dir_all", path = %pending.staging_dir.display(), error = %err, context = "cleanup manual CurseForge staging");
    }
}

pub(super) fn cancel_pending_curseforge_manual_download(app: &mut VertexApp) {
    let continuation = app
        .pending_curseforge_manual_download
        .as_ref()
        .map(|pending| match pending.continuation {
            ManualDownloadContinuation::Import(_) => 0u8,
            ManualDownloadContinuation::DiscoverInstall(_) => 1u8,
        });
    cleanup_pending_curseforge_manual_download(app.pending_curseforge_manual_download.take());
    match continuation {
        Some(0) => {
            app.show_import_instance_modal = false;
            app.import_instance_state.reset();
        }
        Some(1) => {
            app.discover_state
                .finish_install(Err("CurseForge install canceled.".to_owned()));
        }
        _ => {}
    }
}

pub(super) fn default_downloads_dir(installations_root: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(profile_dir) = std::env::var_os("USERPROFILE") {
            let candidate = PathBuf::from(profile_dir).join("Downloads");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home_dir) = std::env::var_os("HOME") {
            let candidate = PathBuf::from(home_dir).join("Downloads");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    installations_root.to_path_buf()
}

pub(super) fn import_package_in_background(
    mut store: InstanceStore,
    installations_root: PathBuf,
    request: import_instance_modal::ImportRequest,
    progress_tx: mpsc::Sender<import_instance_modal::ImportProgress>,
) -> import_instance_modal::ImportTaskResult {
    let instance = import_instance_modal::import_package_with_progress(
        &mut store,
        installations_root.as_path(),
        request,
        |progress| {
            if let Err(err) = progress_tx.send(progress) {
                tracing::error!(
                    target: "vertexlauncher/app/import",
                    error = %err,
                    "Failed to deliver import progress update."
                );
            }
        },
    )?;
    Ok((store, instance))
}

