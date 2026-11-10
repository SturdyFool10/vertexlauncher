use vertex_i18n::tr;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use config::{Config, JavaRuntimeVersion};
use installation::{
    DownloadPolicy, InstallProgressCallback, LaunchRequest, LaunchResult, display_user_path,
    ensure_game_files_async, ensure_openjdk_runtime_async, launch_instance,
};
use instances::{
    InstanceRecord, InstanceStore, delete_instance_root_path, instance_root_path,
    normalize_optional, record_instance_launch_usage, remove_instance_record,
};

use crate::launch_settings::InstanceLaunchSettings;
use crate::{app::tokio_runtime, console, install_activity, notification};

use super::LIBRARY_RUNTIME_LAUNCH_TASK_KIND;

#[path = "library_runtime/pending_launch_context.rs"]
mod pending_launch_context;
#[path = "library_runtime/runtime_launch_outcome.rs"]
mod runtime_launch_outcome;
#[path = "library_runtime/runtime_launch_result.rs"]
mod runtime_launch_result;

use self::pending_launch_context::PendingLaunchContext;
use self::runtime_launch_outcome::RuntimeLaunchOutcome;
use self::runtime_launch_result::RuntimeLaunchResult;

#[derive(Debug, Clone, Default)]
pub struct LibraryRuntimeState {
    results: launcher_runtime::WorkerChannel<RuntimeLaunchResult>,
    pub pending_launches: HashSet<String>,
    pending_launch_contexts: HashMap<String, PendingLaunchContext>,
    pub status_by_instance: HashMap<String, String>,
    pub(super) last_handled_launch_intent_nonce: Option<u64>,
    pub delete_target_instance_id: Option<String>,
    pub delete_error: Option<String>,
    pub delete_in_flight: bool,
    pub(super) delete_results: launcher_runtime::WorkerChannel<Result<InstanceRecord, String>>,
}



pub fn request_runtime_launch(
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
            tr!("Cannot launch: choose a Minecraft game version first.").into_owned(),
        );
        return false;
    }

    let tx = state.results.sender();

    let instance_id = instance.id.clone();
    let instance_name = instance.name.clone();
    state.pending_launches.insert(instance_id.clone());
    state.status_by_instance.insert(
        instance_id.clone(),
        tr!("Preparing Minecraft {game_version}...", game_version = game_version).into_owned(),
    );

    let modloader = instance.modloader.trim().to_owned();
    let modloader_version = normalize_optional(instance.modloader_version.as_str());
    let modloader_version_display = modloader_version
        .as_deref()
        .map(|value| tr!(" {value}", value = value).into_owned())
        .unwrap_or_default();
    let required_java_major = config.effective_required_java_major(game_version.as_str());
    let java_executable = config.choose_java_executable(
        instance.java_override_enabled,
        instance.java_override_runtime_major,
        required_java_major,
    );
    let java_launch_mode = if let Some(path) = java_executable
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tr!("configured Java at {path}", path = path).into_owned()
    } else if let Some(runtime_major) = required_java_major {
        tr!("auto-provisioned OpenJDK {runtime_major}", runtime_major = runtime_major).into_owned()
    } else {
        tr!("java from PATH").into_owned()
    };
    let download_max_concurrent = config.download_max_concurrent().max(1);
    let download_speed_limit_bps = config.parsed_download_speed_limit_bps();
    let sync_run = crate::sync_runner::SyncRunConfig::from_config(config);
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: download_max_concurrent,
        max_download_bps: download_speed_limit_bps,
    };
    let InstanceLaunchSettings {
        max_memory_mib,
        extra_jvm_args,
        extra_env_vars,
        linux_set_opengl_driver,
        linux_use_zink_driver,
    } = InstanceLaunchSettings::resolve(config, instance);
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
    console::request_focus(tab_id.as_str());
    console::set_instance_tab_loading(
        instance_root_display.as_str(),
        tab_user_key.as_deref(),
        true,
    );
    console::push_log_to_tab(
        tab_id.as_str(),
        crate::log_format::LogLevel::Info,
        tr!("Launch request: root={instance_root_display} | Minecraft {game_version} | {modloader}{modloader_version_display} | max memory={max_memory} MiB | {java_launch_mode}", instance_root_display = instance_root_display, game_version = game_version, modloader = modloader, modloader_version_display = modloader_version_display, max_memory = max_memory_mib.max(512), java_launch_mode = java_launch_mode).into_owned(),
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
        let mut configured_java = None;
        let java_path_result = if let Some(path) = java_executable
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
                            tr!("failed to auto-install OpenJDK {runtime_major}: {err}", runtime_major = runtime_major, err = err).into_owned()
                        })
                }
                Err(err) => Err(tr!("configured Java path is not readable ({path}): {err}", path = path, err = err).into_owned()),
            }
        } else if let Some(runtime_major) = required_java_major {
            ensure_openjdk_runtime_async(runtime_major)
                .await
                .map(|installed| {
                    let installed = display_user_path(installed.as_path());
                    configured_java = Some((runtime_major, installed.clone()));
                    installed
                })
                .map_err(|err| tr!("failed to auto-install OpenJDK {runtime_major}: {err}", runtime_major = runtime_major, err = err).into_owned())
        } else {
            Ok("java".to_owned())
        };

        let result = if let Ok(java_path) = java_path_result {
            'launch_result: {
                tracing::info!(
                    target: "vertexlauncher/library_runtime",
                    instance_id = %instance_id,
                    instance_root = %instance_root.display(),
                    game_version = %game_version,
                    modloader = %modloader,
                    java_executable = %java_path,
                    required_java_major = ?required_java_major,
                    "Starting library runtime launch task."
                );
                let progress_instance_id = instance_id.clone();
                install_activity::set_status(
                    instance_id.as_str(),
                    installation::InstallStage::ResolvingMetadata,
                    tr!("Preparing Minecraft {game_version}...", game_version = game_version).into_owned(),
                );
                let progress_cb: InstallProgressCallback =
                    Arc::new(move |progress: installation::InstallProgress| {
                        install_activity::set_progress(progress_instance_id.as_str(), &progress);
                    });
                let setup = ensure_game_files_async(
                    instance_root.clone(),
                    game_version.clone(),
                    modloader.clone(),
                    modloader_version.clone(),
                    Some(java_path.clone()),
                    download_policy,
                    Some(progress_cb),
                )
                .await
                .map_err(|err| {
                    install_activity::clear_instance(instance_id.as_str());
                    err.to_string()
                });
                let setup = match setup {
                    Ok(setup) => setup,
                    Err(err) => {
                        tracing::warn!(
                            target: "vertexlauncher/library_runtime",
                            instance_id = %instance_id,
                            instance_root = %instance_root.display(),
                            error = %err,
                            "Library runtime launch task failed."
                        );
                        break 'launch_result Err(err);
                    }
                };
                install_activity::clear_instance(instance_id.as_str());
                tracing::info!(
                    target: "vertexlauncher/library_runtime",
                    instance_id = %instance_id,
                    instance_root = %instance_root.display(),
                    downloaded_files = setup.downloaded_files,
                    "Library runtime launch completed ensure_game_files."
                );
                let downloaded_files = setup.downloaded_files;
                let resolved_modloader_version = setup.resolved_modloader_version;
                // Pull servers, history and hotbars into this instance before the game reads them.
                let _ = tokio_runtime::spawn_blocking(move || {
                    crate::sync_runner::run_from_saved_store_blocking(&sync_run)
                })
                .await;
                let launch_request = LaunchRequest {
                    instance_root: instance_root.clone(),
                    game_version: game_version.clone(),
                    modloader: modloader.clone(),
                    modloader_version: modloader_version.clone(),
                    account_key: launch_account_name.clone(),
                    java_executable: Some(java_path),
                    max_memory_mib,
                    extra_jvm_args: extra_jvm_args.clone(),
                    extra_env_vars: extra_env_vars.clone(),
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
                let launch =
                    tokio_runtime::spawn_blocking(move || launch_instance(&launch_request))
                        .await
                        .map_err(|err| format!("{LIBRARY_RUNTIME_LAUNCH_TASK_KIND} failed: {err}"))
                        .and_then(|result| result.map_err(|err| err.to_string()));
                match launch {
                    Ok(launch) => {
                        tracing::info!(
                            target: "vertexlauncher/library_runtime",
                            instance_id = %instance_id,
                            instance_root = %instance_root.display(),
                            "Library runtime launch task finished successfully."
                        );
                        Ok(RuntimeLaunchOutcome {
                            launch,
                            downloaded_files,
                            resolved_modloader_version,
                            configured_java,
                        })
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "vertexlauncher/library_runtime",
                            instance_id = %instance_id,
                            instance_root = %instance_root.display(),
                            error = %err,
                            "Library runtime launch task failed."
                        );
                        Err(err)
                    }
                }
            }
        } else {
            Err(java_path_result
                .err()
                .unwrap_or_else(|| tr!("failed to select Java executable").into_owned()))
        };

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

pub fn poll_runtime_actions(
    state: &mut LibraryRuntimeState,
    config: &mut Config,
    instances: &mut InstanceStore,
) {
    let drained = state.results.drain();
    let updates = drained.items;

    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/library", "results worker channel stopped unexpectedly.");
        for context in state.pending_launch_contexts.values() {
            console::set_instance_tab_loading(
                context.instance_root_display.as_str(),
                context.tab_user_key.as_deref(),
                false,
            );
        }
        state.pending_launch_contexts.clear();
        notification::error!("library/runtime", "Launch worker stopped unexpectedly before returning a result.");
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
                    && let Some(runtime) = JavaRuntimeVersion::from_major(runtime_major)
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
                    console::push_log_to_tab(
                        tab_id.as_str(),
                        crate::log_format::LogLevel::Info,
                        tr!("Launched Minecraft (pid {pid}, profile {profile_id}).", pid = outcome.launch.pid, profile_id = outcome.launch.profile_id).into_owned(),
                    );
                }
                state.status_by_instance.insert(
                    update.instance_id,
                    tr!("Launched (pid {pid}, profile {profile_id}, {files}, loader {loader}).", pid = outcome.launch.pid, profile_id = outcome.launch.profile_id, files = vertex_i18n::trn!(outcome.downloaded_files, "{count} file downloaded", "{count} files downloaded"), loader = outcome
                            .resolved_modloader_version
                            .as_deref()
                            .unwrap_or("n/a")).into_owned(),
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
                    console::push_log_to_tab(
                        tab_id.as_str(),
                        crate::log_format::LogLevel::Error,
                        tr!("Launch failed: {err}", err = err).into_owned(),
                    );
                }
                state
                    .status_by_instance
                    .insert(update.instance_id, tr!("Launch failed: {err}", err = err).into_owned());
            }
        }
    }
}

pub fn request_instance_delete(
    state: &mut LibraryRuntimeState,
    instance: InstanceRecord,
    installations_root: PathBuf,
) {
    if state.delete_in_flight {
        return;
    }

    let tx = state.delete_results.sender();

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

pub fn poll_delete_instance_results(
    state: &mut LibraryRuntimeState,
    instances: &mut InstanceStore,
) {
    let drained = state.delete_results.drain();
    let updates = drained.items;

    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/library", "delete_results worker channel stopped unexpectedly.");
        state.delete_in_flight = false;
        state.delete_error = Some(tr!("Delete worker stopped unexpectedly.").into_owned());
    }

    for update in updates {
        state.delete_in_flight = false;
        match update {
            Ok(deleted) => {
                if let Err(err) = remove_instance_record(instances, deleted.id.as_str()) {
                    state.delete_error = Some(tr!("Deleted the instance folder, but failed to remove launcher metadata: {err}", err = err).into_owned());
                    continue;
                }
                state.pending_launches.remove(deleted.id.as_str());
                state.pending_launch_contexts.remove(deleted.id.as_str());
                state.status_by_instance.remove(deleted.id.as_str());
                state.delete_target_instance_id = None;
                state.delete_error = None;
                notification::warn!("instance_store", "Deleted instance '{name}' and its folder.", name = deleted.name);
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/library",
                    error = %err,
                    "Instance delete failed."
                );
                state.delete_error = Some(tr!("Failed to delete instance: {err}", err = err).into_owned());
            }
        }
    }
}
