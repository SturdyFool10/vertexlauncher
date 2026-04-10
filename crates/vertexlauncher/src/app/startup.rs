use super::*;

impl eframe::App for VertexApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.update_inner(ui.ctx(), frame)))
        {
            log_unexpected_panic("ui update", payload.as_ref());
            resume_unwind(payload);
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        #[cfg(target_os = "windows")]
        {
            if transparent_viewport_enabled(&self.config) {
                return egui::Rgba::TRANSPARENT.to_array();
            }
        }

        egui::Rgba::from(visuals.panel_fill).to_array()
    }
}

pub(super) fn panic_payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        return (*text).to_owned();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "non-string panic payload".to_owned()
}

pub(super) fn log_unexpected_panic(context: &'static str, payload: &(dyn Any + Send)) {
    tracing::error!(
        target: "vertexlauncher/app/stability",
        context,
        message = %panic_payload_text(payload),
        "Launcher hit an unrecoverable panic."
    );
}

pub(super) fn apply_install_activity_os_feedback(ctx: &egui::Context, frame: &eframe::Frame) {
    if let Some(activity) = install_activity::snapshot() {
        let fraction = if activity.total_files > 0 {
            (activity.downloaded_files as f32 / activity.total_files as f32).clamp(0.0, 1.0)
        } else if let Some(total) = activity.total_bytes {
            if total > 0 {
                (activity.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            }
        } else {
            0.0
        };
        let percent = (fraction * 100.0).round() as u32;
        let speed_mib = activity.bytes_per_second / (1024.0 * 1024.0);
        let eta_suffix = activity
            .eta_seconds
            .map(|eta| format!(" ETA {}s", eta))
            .unwrap_or_default();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "Vertex Launcher · Installing {}% · {:.1} MiB/s{}",
            percent, speed_mib, eta_suffix
        )));
        ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::Progress);
        taskbar_progress::set_install_progress(frame, Some(fraction));
        return;
    }

    ctx.send_viewport_cmd(egui::ViewportCommand::Title("Vertex Launcher".to_owned()));
    taskbar_progress::set_install_progress(frame, None);
}

pub(super) fn start_initial_instance_install(
    app: &mut VertexApp,
    instance: &InstanceRecord,
    installations_root: &Path,
    config: &Config,
) {
    ensure_initial_instance_install_channel(app);
    let initial_install_results_tx = app.initial_install_results_tx.as_ref().cloned();
    let instance_id = instance.id.clone();
    let instance_name = instance.name.clone();
    let activity_instance = instance_name.clone();
    let game_version = instance.game_version.trim().to_owned();
    let modloader = instance.modloader.trim().to_owned();
    let modloader_version = {
        let trimmed = instance.modloader_version.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };
    if game_version.is_empty() || modloader.is_empty() {
        return;
    }

    let instance_root = instance_root_path(installations_root, instance);
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: config.download_max_concurrent().max(1),
        max_download_bps: config.parsed_download_speed_limit_bps(),
    };
    let java_8 = config
        .java_runtime_path_ref(JavaRuntimeVersion::Java8)
        .map(|path| path.as_os_str().to_string_lossy().into_owned());
    let java_16 = config
        .java_runtime_path_ref(JavaRuntimeVersion::Java16)
        .map(|path| path.as_os_str().to_string_lossy().into_owned());
    let java_17 = config
        .java_runtime_path_ref(JavaRuntimeVersion::Java17)
        .map(|path| path.as_os_str().to_string_lossy().into_owned());
    let java_21 = config
        .java_runtime_path_ref(JavaRuntimeVersion::Java21)
        .map(|path| path.as_os_str().to_string_lossy().into_owned());
    let java_25 = config
        .java_runtime_path_ref(JavaRuntimeVersion::Java25)
        .map(|path| path.as_os_str().to_string_lossy().into_owned());

    let notification_source = format!("installation/{instance_name}");
    install_activity::set_progress(
        activity_instance.as_str(),
        &InstallProgress {
            stage: InstallStage::PreparingFolders,
            message: format!(
                "Starting installation for Minecraft {} ({})...",
                game_version, modloader
            ),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
    notification::progress!(
        notification::Severity::Info,
        notification_source.clone(),
        0.0f32,
        "Starting initial install: Minecraft {} / {}.",
        game_version,
        modloader
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let last_emit = Arc::new(Mutex::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        ));
        let notification_source_for_progress = notification_source.clone();
        let activity_instance_for_progress = activity_instance.clone();
        let result: Result<_, String> = (|| {
            let progress_callback: InstallProgressCallback = {
                let last_emit = Arc::clone(&last_emit);
                Arc::new(move |progress: InstallProgress| {
                    install_activity::set_progress(
                        activity_instance_for_progress.as_str(),
                        &progress,
                    );
                    let should_emit = if let Ok(mut last) = last_emit.lock() {
                        if last.elapsed() >= std::time::Duration::from_millis(250) {
                            *last = std::time::Instant::now();
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !should_emit {
                        return;
                    }
                    let fraction = if progress.total_files > 0 {
                        (progress.downloaded_files as f32 / progress.total_files as f32)
                            .clamp(0.0, 1.0)
                    } else if let Some(total) = progress.total_bytes {
                        if total > 0 {
                            (progress.downloaded_bytes as f32 / total as f32).clamp(0.0, 1.0)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    };
                    notification::progress!(
                        notification::Severity::Info,
                        notification_source_for_progress.clone(),
                        fraction,
                        "{} · {:.1} MiB/s{}",
                        progress.message,
                        progress.bytes_per_second / (1024.0 * 1024.0),
                        progress
                            .eta_seconds
                            .map(|eta| format!(" · ETA {}s", eta))
                            .unwrap_or_default()
                    );
                })
            };
            let runtime = recommended_java_runtime_for_game(game_version.as_str());
            let configured_java = runtime.and_then(|runtime| match runtime {
                JavaRuntimeVersion::Java8 => java_8.as_deref(),
                JavaRuntimeVersion::Java16 => java_16.as_deref(),
                JavaRuntimeVersion::Java17 => java_17.as_deref(),
                JavaRuntimeVersion::Java21 => java_21.as_deref(),
                JavaRuntimeVersion::Java25 => java_25.as_deref(),
            });
            let java_path = configured_java
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .filter(|value| Path::new(value).exists())
                .map(str::to_owned)
                .or_else(|| {
                    runtime.and_then(|runtime| {
                        ensure_openjdk_runtime(runtime.major())
                            .ok()
                            .map(|path| display_user_path(path.as_path()))
                    })
                })
                .unwrap_or_else(|| "java".to_owned());

            ensure_game_files(
                instance_root.as_path(),
                game_version.as_str(),
                modloader.as_str(),
                modloader_version.as_deref(),
                Some(java_path.as_str()),
                &download_policy,
                Some(progress_callback),
            )
            .map_err(|err| err.to_string())
        })();

        match result {
            Ok(setup) => {
                install_activity::clear_instance(activity_instance.as_str());
                notification::progress!(
                    notification::Severity::Info,
                    notification_source.clone(),
                    1.0f32,
                    "Initial install complete ({} files, loader {}).",
                    setup.downloaded_files,
                    setup.resolved_modloader_version.as_deref().unwrap_or("n/a")
                );
                notification::info!(
                    "Installation Complete!",
                    "{} installed successfully.",
                    instance_name
                );
            }
            Err(err) => {
                install_activity::clear_instance(activity_instance.as_str());
                tracing::error!(
                    target: "vertexlauncher/app/initial_install",
                    instance_name = %instance_name,
                    instance_root = %instance_root.display(),
                    game_version = %game_version,
                    modloader = %modloader,
                    requested_modloader_version = %modloader_version.as_deref().unwrap_or(""),
                    error = %err,
                    "Initial install failed for newly created instance."
                );
                notification::error!(
                    notification_source,
                    "{}: initial install failed: {}",
                    instance_name,
                    err
                );
                if let Some(tx) = initial_install_results_tx {
                    if let Err(send_err) = tx.send(InitialInstanceInstallResult::Failed {
                        instance_id,
                        instance_name,
                        error: err,
                    }) {
                        tracing::error!(
                            target: "vertexlauncher/app/initial_install",
                            error = %send_err,
                            "Failed to deliver initial-install failure result."
                        );
                    }
                }
            }
        }
    });
}

pub(super) fn recommended_java_runtime_for_game(game_version: &str) -> Option<JavaRuntimeVersion> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        // New versioning scheme (e.g. 26.x): Java version is major - 1
        return Some(JavaRuntimeVersion::Java25);
    }
    if minor <= 16 {
        return Some(JavaRuntimeVersion::Java8);
    }
    if minor == 17 {
        return Some(JavaRuntimeVersion::Java16);
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        return Some(JavaRuntimeVersion::Java21);
    }
    Some(JavaRuntimeVersion::Java17)
}

pub fn run() -> Result<(), RunError> {
    match catch_unwind(AssertUnwindSafe(run_inner)) {
        Ok(result) => result,
        Err(payload) => {
            log_unexpected_panic("launcher runtime", payload.as_ref());
            resume_unwind(payload)
        }
    }
}

pub(super) fn run_inner() -> Result<(), RunError> {
    let log_path = init_tracing();
    if let Some(log_path) = log_path.as_deref() {
        tracing::info!(
            target: "vertexlauncher/app/startup",
            "Launcher started. Log file: {}",
            log_path.display()
        );
    } else {
        tracing::info!(
            target: "vertexlauncher/app/startup",
            "Launcher started. File logging unavailable; using stderr/console only."
        );
    }
    launcher_runtime::init().map_err(RunError::RuntimeBootstrap)?;
    let config_state = load_config();
    let startup_config = match &config_state {
        LoadConfigResult::Loaded(config) => config.clone(),
        LoadConfigResult::Missing { .. } => Config::default(),
    };

    tracing::info!(
        target: "vertexlauncher/app/startup",
        "Building native window and renderer options."
    );
    let options = native_options::build(&startup_config);
    tracing::info!(
        target: "vertexlauncher/app/startup",
        "Starting eframe runtime."
    );

    eframe::run_native(
        "Vertex Launcher",
        options,
        Box::new(move |cc| {
            tracing::info!(
                target: "vertexlauncher/app/startup",
                "Renderer initialized; constructing application state."
            );
            match catch_unwind(AssertUnwindSafe(|| {
                VertexApp::new(cc, config_state.clone())
            })) {
                Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
                Err(payload) => {
                    log_unexpected_panic("ui startup", payload.as_ref());
                    resume_unwind(payload)
                }
            }
        }),
    )
    .map_err(RunError::Ui)
}

pub fn maybe_run_webview_helper() -> Result<bool, String> {
    webview_sign_in::maybe_run_helper_from_args()
}

pub fn maybe_run_cli_command() -> Result<bool, String> {
    cli::maybe_run_from_args()
}
