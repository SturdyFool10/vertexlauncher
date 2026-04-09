use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

use config::{Config, JavaRuntimeVersion};
use installation::{
    DownloadPolicy, InstallProgressCallback, LaunchRequest, LaunchResult, display_user_path,
    ensure_game_files, ensure_openjdk_runtime, launch_instance,
};
use instances::{
    InstanceRecord, InstanceStore, delete_instance_root_path, instance_root_path,
    record_instance_launch_usage, remove_instance_record,
};

use crate::{app::tokio_runtime, console, install_activity, notification};

use super::LIBRARY_RUNTIME_LAUNCH_TASK_KIND;

#[derive(Debug, Clone, Default)]
pub(super) struct LibraryRuntimeState {
    results_tx: Option<mpsc::Sender<RuntimeLaunchResult>>,
    results_rx: Option<Arc<Mutex<mpsc::Receiver<RuntimeLaunchResult>>>>,
    pub(super) pending_launches: HashSet<String>,
    pending_launch_contexts: HashMap<String, PendingLaunchContext>,
    pub(super) status_by_instance: HashMap<String, String>,
    pub(super) last_handled_launch_intent_nonce: Option<u64>,
    pub(super) delete_target_instance_id: Option<String>,
    pub(super) delete_error: Option<String>,
    pub(super) delete_in_flight: bool,
    pub(super) delete_results_tx: Option<mpsc::Sender<Result<InstanceRecord, String>>>,
    pub(super) delete_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<InstanceRecord, String>>>>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LibraryLaunchIdentity {
    pub(super) account: Option<String>,
    pub(super) display_name: Option<String>,
    pub(super) player_uuid: Option<String>,
    pub(super) access_token: Option<String>,
    pub(super) xuid: Option<String>,
    pub(super) user_type: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingLaunchContext {
    instance_name: String,
    instance_root_display: String,
    tab_user_key: Option<String>,
    tab_username: String,
}

#[derive(Debug, Clone)]
struct RuntimeLaunchResult {
    instance_id: String,
    result: Result<RuntimeLaunchOutcome, String>,
}

#[derive(Debug, Clone)]
struct RuntimeLaunchOutcome {
    launch: LaunchResult,
    downloaded_files: u32,
    resolved_modloader_version: Option<String>,
    configured_java: Option<(u8, String)>,
}

fn ensure_result_channel(state: &mut LibraryRuntimeState) {
    if state.results_tx.is_some() && state.results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<RuntimeLaunchResult>();
    state.results_tx = Some(tx);
    state.results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn request_runtime_launch(
    state: &mut LibraryRuntimeState,
    instance: &InstanceRecord,
    instance_root: PathBuf,
    config: &Config,
    player_name: Option<String>,
    player_uuid: Option<String>,
    access_token: Option<String>,
    xuid: Option<String>,
    user_type: Option<String>,
    launch_account_name: Option<String>,
    quick_play_singleplayer: Option<String>,
    quick_play_multiplayer: Option<String>,
) -> bool {
    if state.pending_launches.contains(instance.id.as_str()) {
        return false;
    }

    let game_version = instance.game_version.trim().to_owned();
    if game_version.is_empty() {
        state.status_by_instance.insert(
            instance.id.clone(),
            "Cannot launch: choose a Minecraft game version first.".to_owned(),
        );
        return false;
    }

    ensure_result_channel(state);
    let Some(tx) = state.results_tx.as_ref().cloned() else {
        return false;
    };

    let instance_id = instance.id.clone();
    let instance_name = instance.name.clone();
    state.pending_launches.insert(instance_id.clone());
    state.status_by_instance.insert(
        instance_id.clone(),
        format!("Preparing Minecraft {}...", game_version),
    );

    let modloader = instance.modloader.trim().to_owned();
    let modloader_version = normalize_optional(instance.modloader_version.as_str());
    let modloader_version_display = modloader_version
        .as_deref()
        .map(|value| format!(" {value}"))
        .unwrap_or_default();
    let required_java_major = effective_required_java_major(config, game_version.as_str());
    let java_executable = choose_java_executable(config, instance, required_java_major);
    let download_max_concurrent = config.download_max_concurrent().max(1);
    let download_speed_limit_bps = config.parsed_download_speed_limit_bps();
    let default_instance_max_memory_mib = config.default_instance_max_memory_mib();
    let default_instance_cli_args = normalize_optional(config.default_instance_cli_args());
    let global_linux_set_opengl_driver = config.linux_set_opengl_driver();
    let global_linux_use_zink_driver = config.linux_use_zink_driver();
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: download_max_concurrent,
        max_download_bps: download_speed_limit_bps,
    };
    let max_memory_mib = instance
        .max_memory_mib
        .unwrap_or(default_instance_max_memory_mib);
    let extra_jvm_args = instance
        .cli_args
        .as_deref()
        .and_then(normalize_optional)
        .or(default_instance_cli_args);
    let (linux_set_opengl_driver, linux_use_zink_driver) =
        instances::effective_linux_graphics_settings(
            instance,
            global_linux_set_opengl_driver,
            global_linux_use_zink_driver,
        );
    let instance_root_display = display_user_path(instance_root.as_path());
    let tab_user_key = player_uuid
        .as_deref()
        .or(launch_account_name.as_deref())
        .or(player_name.as_deref())
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        });
    let tab_username = player_name
        .as_deref()
        .or(launch_account_name.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Player")
        .to_owned();
    let tab_id = console::ensure_instance_tab(
        instance_name.as_str(),
        tab_username.as_str(),
        instance_root_display.as_str(),
        tab_user_key.as_deref(),
    );
    console::set_instance_tab_loading(
        instance_root_display.as_str(),
        tab_user_key.as_deref(),
        true,
    );
    console::push_line_to_tab(
        tab_id.as_str(),
        format!(
            "Launch request: root={} | Minecraft {} | {}{} | max memory={} MiB",
            instance_root_display,
            game_version,
            modloader,
            modloader_version_display,
            max_memory_mib.max(512),
        ),
    );
    state.pending_launch_contexts.insert(
        instance_id.clone(),
        PendingLaunchContext {
            instance_name,
            instance_root_display: instance_root_display.clone(),
            tab_user_key: tab_user_key.clone(),
            tab_username: tab_username.clone(),
        },
    );

    let instance_id_for_join_log = instance_id.clone();
    let instance_id_for_result = instance_id.clone();
    let instance_root_for_join_log = instance_root.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            tracing::info!(
                target: "vertexlauncher/library_runtime",
                instance_id = %instance_id,
                instance_root = %instance_root.display(),
                game_version = %game_version,
                modloader = %modloader,
                "Starting library runtime launch task."
            );
            let result = (|| -> Result<RuntimeLaunchOutcome, String> {
                let mut configured_java = None;
                let java_path = if let Some(path) = java_executable
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .filter(|value| Path::new(value).exists())
                    .map(str::to_owned)
                {
                    path
                } else if let Some(runtime_major) = required_java_major {
                    let installed = ensure_openjdk_runtime(runtime_major).map_err(|err| {
                        format!("failed to auto-install OpenJDK {runtime_major}: {err}")
                    })?;
                    let installed = display_user_path(installed.as_path());
                    configured_java = Some((runtime_major, installed.clone()));
                    installed
                } else {
                    "java".to_owned()
                };

                let progress_instance_id = instance_id.clone();
                install_activity::set_status(
                    instance_id.as_str(),
                    installation::InstallStage::ResolvingMetadata,
                    format!("Preparing Minecraft {}...", game_version),
                );
                let progress_cb: InstallProgressCallback =
                    Arc::new(move |progress: installation::InstallProgress| {
                        install_activity::set_progress(progress_instance_id.as_str(), &progress);
                    });

                let setup = ensure_game_files(
                    instance_root.as_path(),
                    game_version.as_str(),
                    modloader.as_str(),
                    modloader_version.as_deref(),
                    Some(java_path.as_str()),
                    &download_policy,
                    Some(progress_cb),
                )
                .map_err(|err| {
                    install_activity::clear_instance(instance_id.as_str());
                    err.to_string()
                })?;
                install_activity::clear_instance(instance_id.as_str());
                tracing::info!(
                    target: "vertexlauncher/library_runtime",
                    instance_id = %instance_id,
                    instance_root = %instance_root.display(),
                    downloaded_files = setup.downloaded_files,
                    "Library runtime launch completed ensure_game_files."
                );

                let launch_request = LaunchRequest {
                    instance_root: instance_root.clone(),
                    game_version: game_version.clone(),
                    modloader: modloader.clone(),
                    modloader_version: modloader_version.clone(),
                    account_key: launch_account_name.clone(),
                    java_executable: Some(java_path),
                    max_memory_mib,
                    extra_jvm_args: extra_jvm_args.clone(),
                    player_name: player_name.clone().or(launch_account_name.clone()),
                    player_uuid: player_uuid.clone(),
                    auth_access_token: access_token.clone(),
                    auth_xuid: xuid.clone(),
                    auth_user_type: user_type.clone(),
                    quick_play_singleplayer: quick_play_singleplayer.clone(),
                    quick_play_multiplayer: quick_play_multiplayer.clone(),
                    linux_set_opengl_driver,
                    linux_use_zink_driver,
                };
                tracing::info!(
                    target: "vertexlauncher/library_runtime",
                    instance_id = %instance_id,
                    instance_root = %instance_root.display(),
                    "Launching prepared library instance."
                );
                let launch = launch_instance(&launch_request).map_err(|err| err.to_string())?;
                Ok(RuntimeLaunchOutcome {
                    launch,
                    downloaded_files: setup.downloaded_files,
                    resolved_modloader_version: setup.resolved_modloader_version,
                    configured_java,
                })
            })();
            match &result {
                Ok(_) => tracing::info!(
                    target: "vertexlauncher/library_runtime",
                    instance_id = %instance_id,
                    instance_root = %instance_root.display(),
                    "Library runtime launch task finished successfully."
                ),
                Err(error) => tracing::warn!(
                    target: "vertexlauncher/library_runtime",
                    instance_id = %instance_id,
                    instance_root = %instance_root.display(),
                    error = %error,
                    "Library runtime launch task failed."
                ),
            }
            result
        })
        .await
        .map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/library_runtime",
                instance_id = %instance_id_for_join_log,
                instance_root = %instance_root_for_join_log.display(),
                error = %err,
                "Library runtime launch task join failed."
            );
            format!("{LIBRARY_RUNTIME_LAUNCH_TASK_KIND} failed: {err}")
        })
        .and_then(|result| result);

        if let Err(err) = tx.send(RuntimeLaunchResult {
            instance_id: instance_id_for_result,
            result,
        }) {
            tracing::error!(
                target: "vertexlauncher/library_runtime",
                instance_id = %instance_id_for_join_log,
                instance_root = %instance_root_for_join_log.display(),
                error = %err,
                "Failed to deliver library runtime launch result."
            );
        }
    });
    true
}

pub(super) fn poll_runtime_actions(
    state: &mut LibraryRuntimeState,
    config: &mut Config,
    instances: &mut InstanceStore,
) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/library",
                            pending = state.pending_launch_contexts.len(),
                            "Library runtime worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/library",
                    pending = state.pending_launch_contexts.len(),
                    "Library runtime receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        for context in state.pending_launch_contexts.values() {
            console::set_instance_tab_loading(
                context.instance_root_display.as_str(),
                context.tab_user_key.as_deref(),
                false,
            );
        }
        state.pending_launch_contexts.clear();
        state.results_tx = None;
        state.results_rx = None;
        notification::error!(
            "library/runtime",
            "Launch worker stopped unexpectedly before returning a result."
        );
    }

    for update in updates {
        state.pending_launches.remove(update.instance_id.as_str());
        let context = state
            .pending_launch_contexts
            .remove(update.instance_id.as_str());
        if let Some(context) = context.as_ref() {
            console::set_instance_tab_loading(
                context.instance_root_display.as_str(),
                context.tab_user_key.as_deref(),
                false,
            );
        }
        match update.result {
            Ok(outcome) => {
                let _ = record_instance_launch_usage(instances, update.instance_id.as_str());
                if let Some((runtime_major, path)) = outcome.configured_java
                    && let Some(runtime) = java_runtime_from_major(runtime_major)
                {
                    config.set_java_runtime_path_ref(runtime, Some(Path::new(path.as_str())));
                }
                if let Some(context) = context.as_ref() {
                    let tab_id = console::ensure_instance_tab(
                        context.instance_name.as_str(),
                        context.tab_username.as_str(),
                        context.instance_root_display.as_str(),
                        context.tab_user_key.as_deref(),
                    );
                    console::attach_launch_log(
                        tab_id.as_str(),
                        context.instance_root_display.as_str(),
                        outcome.launch.launch_log_path.as_path(),
                    );
                    console::push_line_to_tab(
                        tab_id.as_str(),
                        format!(
                            "Launched Minecraft (pid {}, profile {}).",
                            outcome.launch.pid, outcome.launch.profile_id
                        ),
                    );
                }
                state.status_by_instance.insert(
                    update.instance_id,
                    format!(
                        "Launched (pid {}, profile {}, {} file(s), loader {}).",
                        outcome.launch.pid,
                        outcome.launch.profile_id,
                        outcome.downloaded_files,
                        outcome
                            .resolved_modloader_version
                            .as_deref()
                            .unwrap_or("n/a"),
                    ),
                );
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/library",
                    instance_id = %update.instance_id,
                    error = %err,
                    "Library launch failed."
                );
                if let Some(context) = context.as_ref() {
                    let tab_id = console::ensure_instance_tab(
                        context.instance_name.as_str(),
                        context.tab_username.as_str(),
                        context.instance_root_display.as_str(),
                        context.tab_user_key.as_deref(),
                    );
                    console::push_line_to_tab(tab_id.as_str(), format!("Launch failed: {err}"));
                }
                state
                    .status_by_instance
                    .insert(update.instance_id, format!("Launch failed: {err}"));
            }
        }
    }
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn choose_java_executable(
    config: &Config,
    instance: &InstanceRecord,
    required_java_major: Option<u8>,
) -> Option<String> {
    if instance.java_override_enabled
        && let Some(override_major) = instance.java_override_runtime_major
        && let Some(runtime) = java_runtime_from_major(override_major)
        && let Some(path) = config.java_runtime_path_ref(runtime)
    {
        let trimmed = path.as_os_str().to_string_lossy().trim().to_owned();
        if !trimmed.is_empty() && path.exists() {
            return Some(trimmed);
        }
    }

    if let Some(runtime_major) = required_java_major
        && let Some(runtime) = java_runtime_from_major(runtime_major)
        && let Some(path) = config.java_runtime_path_ref(runtime)
    {
        let trimmed = path.as_os_str().to_string_lossy().trim().to_owned();
        if !trimmed.is_empty() && path.exists() {
            return Some(trimmed);
        }
    }
    None
}

fn required_java_major(game_version: &str) -> Option<u8> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        return major.checked_sub(1).and_then(|v| u8::try_from(v).ok());
    }
    if minor <= 16 {
        return Some(8);
    }
    if minor == 17 {
        return Some(16);
    }
    if minor >= 21 {
        return u8::try_from(minor).ok();
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        return Some(21);
    }
    Some(17)
}

fn effective_required_java_major(config: &Config, game_version: &str) -> Option<u8> {
    let required = required_java_major(game_version)?;
    if config.force_java_21_minimum() && required < 21 {
        Some(21)
    } else {
        Some(required)
    }
}

fn java_runtime_from_major(major: u8) -> Option<JavaRuntimeVersion> {
    match major {
        8 => Some(JavaRuntimeVersion::Java8),
        16 => Some(JavaRuntimeVersion::Java16),
        17 => Some(JavaRuntimeVersion::Java17),
        21 => Some(JavaRuntimeVersion::Java21),
        25 => Some(JavaRuntimeVersion::Java25),
        _ => None,
    }
}

fn ensure_delete_channel(state: &mut LibraryRuntimeState) {
    if state.delete_results_tx.is_some() && state.delete_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<InstanceRecord, String>>();
    state.delete_results_tx = Some(tx);
    state.delete_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn request_instance_delete(
    state: &mut LibraryRuntimeState,
    instance: InstanceRecord,
    installations_root: PathBuf,
) {
    if state.delete_in_flight {
        return;
    }

    ensure_delete_channel(state);
    let Some(tx) = state.delete_results_tx.as_ref().cloned() else {
        return;
    };

    state.delete_in_flight = true;
    state.delete_error = None;
    tokio_runtime::spawn_blocking_detached(move || {
        let instance_root = instance_root_path(installations_root.as_path(), &instance);
        let instance_for_result = instance.clone();
        let result = delete_instance_root_path(instance_root.as_path())
            .map(|()| instance_for_result)
            .map_err(|err| err.to_string());
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/library",
                error = %err,
                "Failed to deliver instance delete result."
            );
        }
    });
}

pub(super) fn poll_delete_instance_results(
    state: &mut LibraryRuntimeState,
    instances: &mut InstanceStore,
) {
    let Some(rx) = state.delete_results_rx.as_ref() else {
        return;
    };

    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/library",
                        target = ?state.delete_target_instance_id,
                        "Instance-delete worker disconnected unexpectedly."
                    );
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => {
            tracing::error!(
                target: "vertexlauncher/library",
                target = ?state.delete_target_instance_id,
                "Instance-delete receiver mutex was poisoned."
            );
            should_reset_channel = true;
        }
    }

    if should_reset_channel {
        state.delete_results_tx = None;
        state.delete_results_rx = None;
        state.delete_in_flight = false;
        state.delete_error = Some("Delete worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.delete_in_flight = false;
        match update {
            Ok(deleted) => {
                if let Err(err) = remove_instance_record(instances, deleted.id.as_str()) {
                    state.delete_error = Some(format!(
                        "Deleted the instance folder, but failed to remove launcher metadata: {err}"
                    ));
                    continue;
                }
                state.pending_launches.remove(deleted.id.as_str());
                state.pending_launch_contexts.remove(deleted.id.as_str());
                state.status_by_instance.remove(deleted.id.as_str());
                state.delete_target_instance_id = None;
                state.delete_error = None;
                notification::warn!(
                    "instance_store",
                    "Deleted instance '{}' and its folder.",
                    deleted.name
                );
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/library",
                    error = %err,
                    "Instance delete failed."
                );
                state.delete_error = Some(format!("Failed to delete instance: {err}"));
            }
        }
    }
}
