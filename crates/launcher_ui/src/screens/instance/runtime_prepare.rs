use super::*;

const RUNTIME_PREPARE_TASK_KIND: &str = "instance runtime prepare";

pub(in super::super) async fn reinstall_instance_profile_files(
    instance_root: &Path,
) -> Result<(), std::io::Error> {
    const REINSTALL_PATHS: [&str; 5] = ["versions", "libraries", "assets", "natives", "loaders"];
    for relative in REINSTALL_PATHS {
        let path = instance_root.join(relative);
        match launcher_runtime::fs::remove_dir_all(path.as_path()).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

pub(in super::super) fn request_runtime_prepare(
    state: &mut InstanceScreenState,
    operation: RuntimePrepareOperation,
    instance_root: PathBuf,
    game_version: String,
    modloader: String,
    modloader_version: Option<String>,
    required_java_major: Option<u8>,
    java_executable: Option<String>,
    download_max_concurrent: u32,
    download_speed_limit_bps: Option<u64>,
    linux_set_opengl_driver: bool,
    linux_use_zink_driver: bool,
    max_memory_mib: u128,
    extra_jvm_args: Option<String>,
    extra_env_vars: Option<String>,
    visible_username: Option<String>,
    player_name: Option<String>,
    player_uuid: Option<String>,
    access_token: Option<String>,
    xuid: Option<String>,
    user_type: Option<String>,
    launch_account_name: Option<String>,
    sync_run: Option<crate::sync_runner::SyncRunConfig>,
) {
    let game_version = game_version.trim().to_owned();
    if game_version.is_empty() || state.runtime_prepare_in_flight {
        return;
    }

    let tx = state.runtime_prepare_results.sender();
    let progress_tx = state.runtime_progress.sender();

    state.runtime_prepare_in_flight = true;
    state.runtime_latest_progress = None;
    if operation == RuntimePrepareOperation::ReinstallProfile {
        state.launch_username = None;
        state.launch_user_key = None;
    }
    state.status_message = Some(match operation {
        RuntimePrepareOperation::Launch => format!("Preparing Minecraft {game_version}..."),
        RuntimePrepareOperation::ReinstallProfile => {
            format!("Reinstalling Minecraft {game_version} profile...")
        }
    });
    let instance_root_display = display_user_path(instance_root.as_path());
    state.runtime_prepare_instance_root = Some(instance_root_display.clone());
    let game_version_for_task = game_version.clone();
    let game_version_for_result = game_version.clone();
    let modloader_for_task = modloader.trim().to_owned();
    let modloader_version_for_task = modloader_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let java_executable_for_task = java_executable;
    let extra_jvm_args_for_task = extra_jvm_args;
    let extra_env_vars_for_task = extra_env_vars;
    let visible_username_for_task = visible_username;
    let player_name_for_task = player_name;
    let player_uuid_for_task = player_uuid;
    let access_token_for_task = access_token;
    let xuid_for_task = xuid;
    let user_type_for_task = user_type;
    let launch_account_name_for_task = launch_account_name;
    let tab_user_key = player_uuid_for_task
        .as_deref()
        .or(launch_account_name_for_task.as_deref())
        .or(player_name_for_task.as_deref())
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        });
    state.runtime_prepare_user_key = tab_user_key.clone();
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: download_max_concurrent.max(1),
        max_download_bps: download_speed_limit_bps,
    };
    let modloader_version_display = modloader_version_for_task
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" {value}"))
        .unwrap_or_default();
    let java_launch_mode = if let Some(path) = java_executable_for_task
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        format!("configured Java at {path}")
    } else if let Some(runtime_major) = required_java_major {
        format!("auto-provisioned OpenJDK {runtime_major}")
    } else {
        "java from PATH".to_owned()
    };
    let username = visible_username_for_task
        .as_deref()
        .or(player_name_for_task.as_deref())
        .or(launch_account_name_for_task.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Player");
    let tab_id = console::ensure_instance_tab(
        state.name_input.as_str(),
        username,
        instance_root_display.as_str(),
        tab_user_key.as_deref(),
    );
    console::request_focus(tab_id.as_str());
    console::set_instance_tab_loading(
        instance_root_display.as_str(),
        tab_user_key.as_deref(),
        true,
    );
    console::push_log_to_tab(
        tab_id.as_str(),
        crate::log_format::LogLevel::Info,
        match operation {
            RuntimePrepareOperation::Launch => format!(
                "Launch request: root={} | Minecraft {} | {}{} | max memory={} MiB | {}",
                instance_root_display,
                game_version_for_task,
                modloader_for_task,
                modloader_version_display,
                max_memory_mib.max(512),
                java_launch_mode
            ),
            RuntimePrepareOperation::ReinstallProfile => format!(
                "Reinstall request: root={} | Minecraft {} | {}{} | {}",
                instance_root_display,
                game_version_for_task,
                modloader_for_task,
                modloader_version_display,
                java_launch_mode
            ),
        },
    );
    let instance_id_for_notifications = state.name_input.clone();
    let instance_root_for_join_log = instance_root.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let progress_tx_done = progress_tx.clone();
        let mut configured_java = None;
        let java_path_result = if let Some(path) = java_executable_for_task
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match launcher_runtime::fs::metadata(path).await {
                Ok(_) => Ok(path.to_owned()),
                Err(_) if required_java_major.is_some() => {
                    let runtime_major = required_java_major.unwrap_or_default();
                    ensure_openjdk_runtime_async(runtime_major)
                        .await
                        .map(|installed| {
                            let installed = display_user_path(installed.as_path());
                            configured_java = Some((runtime_major, installed.clone()));
                            installed
                        })
                        .map_err(|err| {
                            format!("failed to auto-install OpenJDK {runtime_major}: {err}")
                        })
                }
                Err(err) => Err(format!(
                    "configured Java path is not readable ({path}): {err}"
                )),
            }
        } else if let Some(runtime_major) = required_java_major {
            ensure_openjdk_runtime_async(runtime_major)
                .await
                .map(|installed| {
                    let installed = display_user_path(installed.as_path());
                    configured_java = Some((runtime_major, installed.clone()));
                    installed
                })
                .map_err(|err| format!("failed to auto-install OpenJDK {runtime_major}: {err}"))
        } else {
            Ok("java".to_owned())
        };

        let result = if let Ok(java_path) = java_path_result {
            'runtime_result: {
                let instance_root_for_progress = instance_root.clone();
                let game_version_for_progress = game_version_for_task.clone();
                let operation_for_progress = operation;
                let progress_callback: InstallProgressCallback = Arc::new(move |event| {
                    if let Err(err) = progress_tx.send(event) {
                        tracing::error!(
                            target: "vertexlauncher/instance_runtime",
                            instance_root = %instance_root_for_progress.display(),
                            game_version = %game_version_for_progress,
                            operation = ?operation_for_progress,
                            error = %err,
                            "Failed to deliver runtime prepare progress update."
                        );
                    }
                });
                tracing::info!(
                    target: "vertexlauncher/instance_runtime",
                    instance_root = %instance_root.display(),
                    game_version = %game_version_for_task,
                    modloader = %modloader_for_task,
                    operation = ?operation,
                    "Starting instance runtime prepare task."
                );
                tracing::info!(
                    target: "vertexlauncher/instance_runtime",
                    instance_root = %instance_root.display(),
                    game_version = %game_version_for_task,
                    java_executable = %java_path,
                    required_java_major = ?required_java_major,
                    operation = ?operation,
                    "Selected Java executable for instance runtime task."
                );
                if operation == RuntimePrepareOperation::ReinstallProfile {
                    let reinstall_result =
                        reinstall_instance_profile_files(instance_root.as_path())
                            .await
                            .map_err(|err| {
                                format!("failed to clear install artifacts before reinstall: {err}")
                            });
                    if let Err(err) = reinstall_result {
                        break 'runtime_result Err(err);
                    }
                }
                let setup = ensure_game_files_async(
                    instance_root.clone(),
                    game_version_for_task.clone(),
                    modloader_for_task.clone(),
                    modloader_version_for_task.clone(),
                    Some(java_path.clone()),
                    download_policy,
                    Some(progress_callback),
                )
                .await
                .map_err(|err| err.to_string());
                let setup = match setup {
                    Ok(setup) => setup,
                    Err(err) => {
                        tracing::warn!(
                            target: "vertexlauncher/instance_runtime",
                            instance_root = %instance_root.display(),
                            game_version = %game_version_for_task,
                            operation = ?operation,
                            error = %err,
                            "Instance runtime prepare task failed."
                        );
                        break 'runtime_result Err(err);
                    }
                };
                tracing::info!(
                    target: "vertexlauncher/instance_runtime",
                    instance_root = %instance_root.display(),
                    game_version = %game_version_for_task,
                    downloaded_files = setup.downloaded_files,
                    "Instance runtime prepare completed ensure_game_files."
                );
                let launch = if operation == RuntimePrepareOperation::Launch {
                    if let Some(sync_run) = sync_run {
                        // Pull servers, history and hotbars into this instance before the game reads them.
                        let _ = tokio_runtime::spawn_blocking(move || {
                            crate::sync_runner::run_from_saved_store_blocking(&sync_run)
                        })
                        .await;
                    }
                    let launch_request = LaunchRequest {
                        instance_root: instance_root.clone(),
                        game_version: game_version_for_task.clone(),
                        modloader: modloader_for_task.clone(),
                        modloader_version: modloader_version_for_task.clone(),
                        account_key: launch_account_name_for_task.clone(),
                        java_executable: Some(java_path.clone()),
                        max_memory_mib,
                        extra_jvm_args: extra_jvm_args_for_task.clone(),
                        extra_env_vars: extra_env_vars_for_task.clone(),
                        player_name: player_name_for_task
                            .clone()
                            .or(launch_account_name_for_task.clone()),
                        player_uuid: player_uuid_for_task.clone(),
                        auth_access_token: access_token_for_task.clone(),
                        auth_xuid: xuid_for_task.clone(),
                        auth_user_type: user_type_for_task.clone(),
                        quick_play_singleplayer: None,
                        quick_play_multiplayer: None,
                        linux_set_opengl_driver,
                        linux_use_zink_driver,
                    };
                    tracing::info!(
                        target: "vertexlauncher/instance_runtime",
                        instance_root = %instance_root.display(),
                        game_version = %game_version_for_task,
                        "Launching prepared instance runtime."
                    );
                    match tokio_runtime::spawn_blocking(move || launch_instance(&launch_request))
                        .await
                        .map_err(|err| format!("{RUNTIME_PREPARE_TASK_KIND} failed: {err}"))
                        .and_then(|result| result.map_err(|err| err.to_string()))
                    {
                        Ok(launch) => Some(launch),
                        Err(err) => break 'runtime_result Err(err),
                    }
                } else {
                    None
                };
                tracing::info!(
                    target: "vertexlauncher/instance_runtime",
                    instance_root = %instance_root.display(),
                    game_version = %game_version_for_task,
                    operation = ?operation,
                    "Instance runtime prepare task finished successfully."
                );
                Ok(RuntimePrepareOutcome {
                    operation,
                    setup,
                    configured_java,
                    launch,
                })
            }
        } else {
            Err(java_path_result
                .err()
                .unwrap_or_else(|| "failed to select Java executable".to_owned()))
        };
        if let Err(err) = tx.send((
            game_version_for_result.clone(),
            instance_root_display.clone(),
            result,
        )) {
            tracing::error!(
                target: "vertexlauncher/instance_runtime",
                instance_root = %instance_root_for_join_log.display(),
                game_version = %game_version_for_result,
                operation = ?operation,
                error = %err,
                "Failed to deliver runtime prepare result."
            );
        }
        if let Err(err) = progress_tx_done.send(InstallProgress {
            stage: InstallStage::Complete,
            message: format!("Install task ended for {instance_id_for_notifications}."),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: Some(0),
        }) {
            tracing::error!(
                target: "vertexlauncher/instance_runtime",
                instance_root = %instance_root_for_join_log.display(),
                game_version = %game_version_for_result,
                operation = ?operation,
                error = %err,
                "Failed to deliver runtime prepare completion progress update."
            );
        }
    });
}

pub(in super::super) fn poll_runtime_progress(state: &mut InstanceScreenState) {
    let drained = state.runtime_progress.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/worker", "runtime_progress worker channel stopped unexpectedly.");
    }
    let updates = drained.items;

    for progress in updates {
        state.runtime_latest_progress = Some(progress.clone());
        install_activity::set_progress(state.name_input.as_str(), &progress);
        if should_emit_progress_notification(state, &progress) {
            let source = format!("installation/{}", state.name_input);
            let fraction = progress_fraction(&progress);
            notification::progress!(
                notification::Severity::Info,
                source,
                fraction,
                "{} · {:.1} MiB/s{}",
                stage_label(progress.stage),
                progress.bytes_per_second / (1024.0 * 1024.0),
                progress
                    .eta_seconds
                    .map(|eta| format!(" · ETA {}s", eta))
                    .unwrap_or_default()
            );
            state.runtime_last_notification_at = Some(Instant::now());
        }
    }
}

pub(in super::super) fn poll_runtime_prepare(
    state: &mut InstanceScreenState,
    config: &mut Config,
    instances: &mut InstanceStore,
    instance_id: &str,
) {
    let drained = state.runtime_prepare_results.drain();
    let updates = drained.items;

    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/worker", "runtime_prepare_results worker channel stopped unexpectedly.");
        let prepare_user_key = state.runtime_prepare_user_key.take();
        if let Some(root) = state.runtime_prepare_instance_root.take() {
            console::set_instance_tab_loading(root.as_str(), prepare_user_key.as_deref(), false);
        }
        state.runtime_prepare_in_flight = false;
        state.runtime_progress.reset();
    }

    for (game_version, instance_root_display, result) in updates {
        let prepare_user_key = state.runtime_prepare_user_key.take();
        let had_install_progress = state.runtime_latest_progress.is_some();
        state.runtime_prepare_instance_root = None;
        console::set_instance_tab_loading(
            instance_root_display.as_str(),
            prepare_user_key.as_deref(),
            false,
        );
        state.runtime_prepare_in_flight = false;
        match result {
            Ok(outcome) => {
                let operation = outcome.operation;
                if let Some((runtime_major, path)) = outcome.configured_java
                    && let Some(runtime) = JavaRuntimeVersion::from_major(runtime_major)
                {
                    config.set_java_runtime_path_ref(runtime, Some(Path::new(path.as_str())));
                }
                let setup = outcome.setup;
                if let Some(launch) = outcome.launch {
                    let _ = record_instance_launch_usage(instances, instance_id);
                    let username = state
                        .launch_username
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("Player");
                    let tab_id = console::ensure_instance_tab(
                        state.name_input.as_str(),
                        username,
                        instance_root_display.as_str(),
                        state.launch_user_key.as_deref(),
                    );
                    console::attach_launch_log(
                        tab_id.as_str(),
                        instance_root_display.as_str(),
                        launch.launch_log_path.as_path(),
                    );
                    console::push_log_to_tab(
                        tab_id.as_str(),
                        crate::log_format::LogLevel::Info,
                        format!(
                            "Launched Minecraft {} (pid {}, profile {}).",
                            game_version, launch.pid, launch.profile_id
                        ),
                    );
                    state.running = true;
                    install_activity::clear_instance(state.name_input.as_str());
                    state.status_message = Some(format!(
                        "Launched Minecraft {} in {} (pid {}, profile {}, {} file(s) downloaded, loader: {}).",
                        game_version,
                        instance_root_display,
                        launch.pid,
                        launch.profile_id,
                        setup.downloaded_files,
                        setup.resolved_modloader_version.as_deref().unwrap_or("n/a")
                    ));
                    notification::progress!(
                        notification::Severity::Info,
                        format!("installation/{}", state.name_input),
                        1.0f32,
                        "Launched Minecraft {} (pid {}, {} files).",
                        game_version,
                        launch.pid,
                        setup.downloaded_files
                    );
                    if had_install_progress {
                        notification::info!(
                            "Installation Complete!",
                            "{} finished installing files and launched successfully.",
                            state.name_input
                        );
                    }
                } else {
                    state.running = false;
                    install_activity::clear_instance(state.name_input.as_str());
                    let source = format!("installation/{}", state.name_input);
                    match operation {
                        RuntimePrepareOperation::ReinstallProfile => {
                            state.status_message = Some(format!(
                                "Reinstalled Minecraft {} in {} ({} file(s) downloaded, loader: {}).",
                                game_version,
                                instance_root_display,
                                setup.downloaded_files,
                                setup.resolved_modloader_version.as_deref().unwrap_or("n/a")
                            ));
                            notification::progress!(
                                notification::Severity::Info,
                                source,
                                1.0f32,
                                "Reinstalled Minecraft {} ({} files).",
                                game_version,
                                setup.downloaded_files
                            );
                            if had_install_progress {
                                notification::info!(
                                    "Installation Complete!",
                                    "{} finished installing successfully.",
                                    state.name_input
                                );
                            }
                        }
                        RuntimePrepareOperation::Launch => {
                            state.status_message = Some(format!(
                                "Installed Minecraft {} in {} ({} file(s) downloaded).",
                                game_version, instance_root_display, setup.downloaded_files
                            ));
                            notification::progress!(
                                notification::Severity::Info,
                                source,
                                1.0f32,
                                "Installed Minecraft {} ({} files).",
                                game_version,
                                setup.downloaded_files
                            );
                            if had_install_progress {
                                notification::info!(
                                    "Installation Complete!",
                                    "{} finished installing successfully.",
                                    state.name_input
                                );
                            }
                        }
                    }
                }
            }
            Err(err) => {
                state.running = false;
                install_activity::clear_instance(state.name_input.as_str());
                state.status_message = Some(format!("Failed to prepare game files: {err}"));
                notification::error!(
                    format!("installation/{}", state.name_input),
                    "{} installation failed: {}",
                    state.name_input,
                    err
                );
            }
        }
    }
}

pub(in super::super) fn should_emit_progress_notification(
    state: &InstanceScreenState,
    _progress: &InstallProgress,
) -> bool {
    match state.runtime_last_notification_at {
        Some(last) => last.elapsed() >= Duration::from_millis(250),
        None => true,
    }
}

pub(in super::super) fn progress_fraction(progress: &InstallProgress) -> f32 {
    if let Some(total_bytes) = progress.total_bytes
        && total_bytes > 0
    {
        return (progress.downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
    }
    if progress.total_files > 0 {
        return (progress.downloaded_files as f32 / progress.total_files as f32).clamp(0.0, 1.0);
    }
    if matches!(progress.stage, InstallStage::Complete) {
        1.0
    } else {
        0.0
    }
}

pub(in super::super) fn progress_fraction_from_activity(
    progress: &install_activity::InstallActivitySnapshot,
) -> f32 {
    if let Some(total_bytes) = progress.total_bytes
        && total_bytes > 0
    {
        return (progress.downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
    }
    if progress.total_files > 0 {
        return (progress.downloaded_files as f32 / progress.total_files as f32).clamp(0.0, 1.0);
    }
    if matches!(progress.stage, InstallStage::Complete) {
        1.0
    } else {
        0.0
    }
}

pub(in super::super) fn stage_label(stage: InstallStage) -> &'static str {
    match stage {
        InstallStage::PreparingFolders => "Preparing folders",
        InstallStage::ResolvingMetadata => "Resolving metadata",
        InstallStage::DownloadingCore => "Downloading core files",
        InstallStage::InstallingModloader => "Installing modloader",
        InstallStage::Complete => "Complete",
    }
}


