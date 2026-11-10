use vertex_i18n::tr;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use config::{Config, JavaRuntimeVersion, load_config};
use installation::{
    DownloadPolicy, InstallProgress, InstallProgressCallback, InstallStage, display_user_path,
    ensure_game_files_async, ensure_openjdk_runtime_async,
};
use instances::{InstanceRecord, instance_root_path};
use launcher_runtime as tokio_runtime;
use launcher_ui::{install_activity, notification};

use super::{cli, se5, tracing_setup, webview_sign_in};

#[derive(Debug)]
pub enum RunError {
    RuntimeBootstrap(launcher_runtime::RuntimeBootstrapError),
}

#[derive(Debug)]
pub(super) enum InitialInstanceInstallResult {
    Failed {
        instance_id: String,
        instance_name: String,
        error: String,
    },
}

pub fn init_tracing() -> Option<PathBuf> {
    launcher_runtime::set_detached_task_reporter(report_detached_task_failure);
    tracing_setup::init_tracing()
}

fn report_detached_task_failure(task_kind: &str, error: &launcher_runtime::TaskError) {
    notification::emit_replace(
        notification::Severity::Error,
        "background-task",
        tr!("A background task failed ({task_kind}): {error}", task_kind = task_kind, error = error).into_owned(),
        format!("background-task/{task_kind}"),
    );
}

fn panic_payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        return (*text).to_owned();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    tr!("non-string panic payload").into_owned()
}

fn log_unexpected_panic(context: &'static str, payload: &(dyn Any + Send)) {
    tracing::error!(
        target: "vertexlauncher/app/stability",
        context,
        message = %panic_payload_text(payload),
        "Launcher hit an unrecoverable panic."
    );
}

/// Downloads the game files for a newly created instance. Failures are sent to `initial_install_results_tx`.
pub(super) fn start_initial_instance_install_with(
    initial_install_results_tx: Option<mpsc::Sender<InitialInstanceInstallResult>>,
    instance: &InstanceRecord,
    installations_root: &Path,
    config: &Config,
) {
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
            message: tr!("Starting installation for Minecraft {game_version} ({modloader})...", game_version = game_version, modloader = modloader).into_owned(),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: None,
        },
    );
    notification::progress!(notification::Severity::Info, notification_source.clone(), 0.0f32, "Starting initial install: Minecraft {game_version} / {modloader}.", game_version = game_version, modloader = modloader);

    let _ = tokio_runtime::spawn_detached(async move {
        let last_emit = Arc::new(Mutex::new(
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        ));
        let notification_source_for_progress = notification_source.clone();
        let activity_instance_for_progress = activity_instance.clone();
        let runtime = JavaRuntimeVersion::recommended_for_game(game_version.as_str());
        let configured_java = runtime.and_then(|runtime| match runtime {
            JavaRuntimeVersion::Java8 => java_8.as_deref(),
            JavaRuntimeVersion::Java16 => java_16.as_deref(),
            JavaRuntimeVersion::Java17 => java_17.as_deref(),
            JavaRuntimeVersion::Java21 => java_21.as_deref(),
            JavaRuntimeVersion::Java25 => java_25.as_deref(),
        });
        let java_path_result = if let Some(path) = configured_java
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match launcher_runtime::fs::metadata(path).await {
                Ok(_) => Ok(path.to_owned()),
                Err(_) if runtime.is_some() => {
                    let runtime = runtime.expect("runtime checked above");
                    ensure_openjdk_runtime_async(runtime.major())
                        .await
                        .map(|path| display_user_path(path.as_path()))
                        .map_err(|err| {
                            tr!("failed to auto-install OpenJDK {version}: {err}", version = runtime.major(), err = err).into_owned()
                        })
                }
                Err(err) => Err(tr!("configured Java path is not readable ({path}): {err}", path = path, err = err).into_owned()),
            }
        } else if let Some(runtime) = runtime {
            ensure_openjdk_runtime_async(runtime.major())
                .await
                .map(|path| display_user_path(path.as_path()))
                .map_err(|err| tr!("failed to auto-install OpenJDK {version}: {err}", version = runtime.major(), err = err).into_owned())
        } else {
            Ok("java".to_owned())
        };

        let result: Result<_, String> = if let Ok(java_path) = java_path_result {
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
                    notification::progress!(notification::Severity::Info, notification_source_for_progress.clone(), fraction, "{message} · {speed:.1} MiB/s{eta}", message = progress.message, speed = progress.bytes_per_second / (1024.0 * 1024.0), eta = progress.eta_seconds.map(|seconds| tr!(" · ETA {seconds}s", seconds = seconds).into_owned()).unwrap_or_default());
                })
            };

            ensure_game_files_async(
                instance_root.clone(),
                game_version.clone(),
                modloader.clone(),
                modloader_version.clone(),
                Some(java_path),
                download_policy,
                Some(progress_callback),
            )
            .await
            .map_err(|err| err.to_string())
        } else {
            Err(java_path_result
                .err()
                .unwrap_or_else(|| tr!("failed to select Java executable").into_owned()))
        };

        match result {
            Ok(setup) => {
                install_activity::clear_instance(activity_instance.as_str());
                notification::progress!(notification::Severity::Info, notification_source.clone(), 1.0f32, "Initial install complete ({downloaded_files} files, loader {loader}).", downloaded_files = setup.downloaded_files, loader = setup.resolved_modloader_version.as_deref().unwrap_or("n/a"));
                notification::info!("Installation Complete!", "{instance_name} was installed successfully.", instance_name = instance_name);
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
                notification::error!(notification_source, "{instance_name}: initial install failed: {err}", instance_name = instance_name, err = err);
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

/// Runs the SturdyEngine5 front end (the default UI).
pub fn run_se5() -> Result<std::process::ExitCode, RunError> {
    // The engine keeps compiled shaders and compressed textures in the working directory unless told otherwise, which
    // loses them whenever the launcher starts from somewhere else and makes the first frames slow again. Pin them
    // to the launcher's cache folder (set before the engine starts, while only this thread runs).
    let engine_cache = app_paths::cache_root().join("engine");
    let _ = std::fs::create_dir_all(&engine_cache);
    // SAFETY: called at the top of startup, before any other thread reads the environment.
    unsafe { std::env::set_var("STURDY_CACHE_DIR", &engine_cache) };
    match catch_unwind(AssertUnwindSafe(|| {
        log_startup();
        launcher_runtime::init().map_err(RunError::RuntimeBootstrap)?;
        Ok(se5::run(load_config()))
    })) {
        Ok(result) => result,
        Err(payload) => {
            log_unexpected_panic("launcher runtime", payload.as_ref());
            resume_unwind(payload)
        }
    }
}

fn log_startup() {
    let log_path = init_tracing();
    match log_path.as_deref() {
        Some(log_path) => tracing::info!(
            target: "vertexlauncher/app/startup",
            "Launcher started. Log file: {}",
            log_path.display()
        ),
        None => tracing::info!(
            target: "vertexlauncher/app/startup",
            "Launcher started. File logging unavailable; using stderr/console only."
        ),
    }
}

pub fn maybe_run_webview_helper() -> Result<bool, String> {
    webview_sign_in::maybe_run_helper_from_args()
}

pub fn maybe_run_cli_command() -> Result<bool, String> {
    cli::maybe_run_from_args()
}
