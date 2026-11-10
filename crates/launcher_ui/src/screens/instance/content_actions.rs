use vertex_i18n::{tr, trn};
use super::content_updates::update_all_installed_content;
use super::*;

pub(super) fn request_content_update(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    lookup_key: &str,
    entry: modprovider::UnifiedContentEntry,
    installed_file_path: &Path,
    version_id: &str,
    game_version: &str,
    loader_label: &str,
) {
    let lookup_key = lookup_key.trim();
    let version_id = version_id.trim();
    if lookup_key.is_empty() || version_id.is_empty() || state.content_apply_in_flight {
        return;
    }

    let tx = state.content_apply_results.sender();

    let lookup_key = lookup_key.to_owned();
    let version_id = version_id.to_owned();
    let installed_file_path = installed_file_path.to_path_buf();
    let game_version = game_version.trim().to_owned();
    let loader_label = loader_label.trim().to_owned();
    let project_name = if entry.name.trim().is_empty() {
        "content".to_owned()
    } else {
        entry.name.clone()
    };
    let instance_name = state.name_input.clone();
    let instance_root = instance_root.to_path_buf();
    let kind = state.selected_content_tab;

    state.content_apply_in_flight = true;
    state.status_message = Some(tr!("Updating {project_name}...", project_name = project_name).into_owned());
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        lookup_key = %lookup_key,
        project = %project_name,
        version_id = %version_id,
        installed_path = %installed_file_path.display(),
        game_version = %game_version,
        loader = %loader_label,
        "starting individual content update"
    );
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        tr!("Updating {project_name}...", project_name = project_name).into_owned(),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking(move || {
            crate::screens::content_browser::update_installed_content_to_version(
                instance_root.as_path(),
                &entry,
                installed_file_path.as_path(),
                version_id.as_str(),
                game_version.as_str(),
                loader_label.as_str(),
            )
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => {
                tracing::error!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    instance = %instance_name,
                    lookup_key = %lookup_key,
                    project = %project_name,
                    "content update worker panicked: {err}"
                );
                return;
            }
        };
        match &result {
            Ok(message) => tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                lookup_key = %lookup_key,
                project = %project_name,
                "individual content update completed: {message}"
            ),
            Err(err) => tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                lookup_key = %lookup_key,
                project = %project_name,
                "individual content update failed: {err}"
            ),
        }
        let focus_lookup_keys = vec![lookup_key.clone()];
        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys,
            refresh_all_content: false,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                lookup_key = %lookup_key,
                project = %project_name,
                error = %err,
                "Failed to deliver individual content update result."
            );
        }
    });
}



/// Forgets everything resolved in memory so the lists rescan and re-resolve. The hash cache is kept: it is keyed by file
/// hash, so it stays right however the files changed, and keeping it makes the re-resolve cheap.
pub(super) fn invalidate_installed_content_state(state: &mut InstanceScreenState) {
    state.installed_content_cache.mark_stale();
    state.installed_content_entry_ui_cache.clear();
    state.content_metadata_cache.clear();
    state.content_lookup_in_flight.clear();
    state.content_lookup_latest_serial_by_key.clear();
    state.content_lookup_retry_after_by_key.clear();
    state.content_lookup_failure_count_by_key.clear();
}

/// The explicit "refresh" action: [`invalidate_installed_content_state`] and also throws the hash cache away.
pub(super) fn refresh_installed_content_state(
    state: &mut InstanceScreenState,
    instance_root: &Path,
) {
    invalidate_installed_content_state(state);
    super::clear_content_hash_cache(state, instance_root);
}

pub(super) fn request_vtmpatch_apply(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    patch_path: PathBuf,
) {
    if state.content_apply_in_flight {
        return;
    }

    let tx = state.content_apply_results.sender();

    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    state.content_apply_in_flight = true;
    state.status_message = Some(tr!("Applying patch {patch_path}...", patch_path = patch_path.display()).into_owned());
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        tr!("Applying patch...").into_owned(),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let patch_path_for_task = patch_path.clone();
        let instance_root_for_task = instance_root.clone();
        let join = tokio_runtime::spawn_blocking(move || {
            apply_vtmpatch_to_instance(
                patch_path_for_task.as_path(),
                instance_root_for_task.as_path(),
            )
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(tr!("patch apply worker failed: {err}", err = err).into_owned()),
        };

        let patch_file_name = patch_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| patch_path.display().to_string());

        match &result {
            Ok(message) => {
                tracing::info!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    instance = %instance_name,
                    patch = %patch_path.display(),
                    "vtmpatch apply succeeded: {message}"
                );
                notification::info!("vtmpatch/apply", "Patch applied — {patch_file_name} — {message}", patch_file_name = patch_file_name, message = message);
            }
            Err(err) => {
                tracing::error!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    instance = %instance_name,
                    patch = %patch_path.display(),
                    "vtmpatch apply failed: {err}"
                );
                notification::error!("vtmpatch/apply", "Patch apply failed — {patch_file_name} — {err}", patch_file_name = patch_file_name, err = err);
            }
        }

        if let Err(err) = tx.send(ContentApplyResult {
            kind: InstalledContentKind::Mods,
            focus_lookup_keys: Vec::new(),
            refresh_all_content: true,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                patch = %patch_path.display(),
                error = %err,
                "Failed to deliver vtmpatch apply result."
            );
        }
    });
}

pub(super) fn request_local_content_import(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    selected_paths: Vec<PathBuf>,
) {
    if state.content_apply_in_flight || selected_paths.is_empty() {
        return;
    }

    let tx = state.content_apply_results.sender();

    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    state.content_apply_in_flight = true;
    state.status_message = Some(trn!(selected_paths.len(), "Adding {count} local content file...", "Adding {count} local content files..."));
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        tr!("Adding local content files...").into_owned(),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking({
            let instance_root = instance_root.clone();
            let selected_paths = selected_paths.clone();
            move || import_local_content_files(instance_root.as_path(), selected_paths.as_slice())
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => {
                tracing::error!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    instance = %instance_name,
                    error = %err,
                    "import_local_content_files task panicked."
                );
                return;
            }
        };
        let focus_lookup_keys = selected_paths
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|value| value.to_string_lossy().to_string())
            })
            .collect();
        if let Err(err) = tx.send(ContentApplyResult {
            kind: InstalledContentKind::Mods,
            focus_lookup_keys,
            refresh_all_content: true,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                error = %err,
                "Failed to deliver local content import result."
            );
        }
    });
}

fn import_local_content_files(
    instance_root: &Path,
    selected_paths: &[PathBuf],
) -> Result<String, String> {
    let mut imported_total = 0usize;
    let mut counts_by_kind = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    for source_path in selected_paths {
        let Some(file_name) = source_path.file_name() else {
            skipped.push(source_path.display().to_string());
            continue;
        };

        let Some(kind) = detect_installed_content_kind(source_path.as_path()) else {
            skipped.push(source_path.display().to_string());
            continue;
        };

        let destination_dir = instance_root.join(kind.folder_name());
        if let Err(err) = fs::create_dir_all(destination_dir.as_path()) {
            failures.push(tr!("{source_path} -> {destination_dir} ({err})", source_path = source_path.display(), destination_dir = destination_dir.display(), err = err).into_owned());
            continue;
        }

        let destination_path = destination_dir.join(file_name);
        if let Err(err) = fs::copy(source_path.as_path(), destination_path.as_path()) {
            failures.push(tr!("{source_path} -> {destination_path} ({err})", source_path = source_path.display(), destination_path = destination_path.display(), err = err).into_owned());
            continue;
        }

        imported_total += 1;
        *counts_by_kind.entry(kind.label()).or_insert(0usize) += 1;
    }

    let mut summary = counts_by_kind
        .into_iter()
        .map(|(label, count)| tr!("{count} {label}", count = count, label = label).into_owned())
        .collect::<Vec<_>>();
    if summary.is_empty() {
        summary.push(tr!("no files").into_owned());
    }

    if imported_total == 0 {
        if !failures.is_empty() {
            return Err(tr!("Failed to add local content: {failures}.", failures = failures.join("; ")).into_owned());
        }
        if !skipped.is_empty() {
            return Err(tr!("Could not determine where to place: {skipped}.", skipped = skipped.join(", ")).into_owned());
        }
    }

    let mut message = tr!("Added {summary}.", summary = summary.join(", ")).into_owned();
    if !failures.is_empty() {
        message.push_str(" Failed to copy: ");
        message.push_str(failures.join("; ").as_str());
        message.push('.');
    }
    if !skipped.is_empty() {
        message.push_str(" Skipped unrecognized files: ");
        message.push_str(skipped.join(", ").as_str());
        message.push('.');
    }
    Ok(message)
}

pub(super) fn request_content_delete(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    kind: InstalledContentKind,
    lookup_key: &str,
    path: &Path,
) {
    let lookup_key = lookup_key.trim();
    if lookup_key.is_empty() || state.content_apply_in_flight {
        return;
    }

    let tx = state.content_apply_results.sender();

    let lookup_key = lookup_key.to_owned();
    let path = path.to_path_buf();
    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    let path_display = path.display().to_string();

    state.content_apply_in_flight = true;
    state.status_message = Some(tr!("Removing installed content...").into_owned());
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        lookup_key = %lookup_key,
        path = %path_display,
        "starting installed content delete"
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking({
            let instance_root = instance_root.clone();
            let path = path.clone();
            move || -> Result<String, String> {
                let delete_result = if path.is_dir() {
                    std::fs::remove_dir_all(path.as_path())
                } else {
                    std::fs::remove_file(path.as_path())
                };
                delete_result.map_err(|err| tr!("failed to remove {path}: {err}", path = path.display(), err = err).into_owned())?;

                managed_content::remove_content_manifest_entries_for_path(
                    instance_root.as_path(),
                    path.as_path(),
                )
                .map(|_| tr!("Removed installed content.").into_owned())
                .map_err(|err| {
                    tr!("removed installed content, but failed to update the content manifest: {err}", err = err).into_owned()
                })
            }
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);

        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys: vec![lookup_key],
            refresh_all_content: false,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                path = %path_display,
                error = %err,
                "Failed to deliver installed content delete result."
            );
        }
    });
}

pub(super) fn request_content_toggle(
    state: &mut InstanceScreenState,
    kind: InstalledContentKind,
    lookup_key: &str,
    path: &Path,
    currently_disabled: bool,
) {
    let lookup_key = lookup_key.trim();
    if lookup_key.is_empty() || state.content_apply_in_flight {
        return;
    }

    let tx = state.content_apply_results.sender();

    let lookup_key = lookup_key.to_owned();
    let path = path.to_path_buf();
    let instance_name = state.name_input.clone();

    let new_path = if currently_disabled {
        path.with_extension("")
    } else {
        let disabled_name = format!(
            "{}.DISABLED",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        path.with_file_name(disabled_name)
    };

    // Don't set content_apply_in_flight — a rename is near-instantaneous and
    // setting that flag would trigger the install spinner for a single frame,
    // causing visible flicker with no useful progress indication.

    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking({
            let path = path.clone();
            let new_path = new_path.clone();
            move || {
                std::fs::rename(&path, &new_path)
                    .map(|_| {
                        if currently_disabled {
                            tr!("Mod enabled.").into_owned()
                        } else {
                            tr!("Mod disabled.").into_owned()
                        }
                    })
                    .map_err(|err| tr!("failed to rename {path}: {err}", path = path.display(), err = err).into_owned())
            }
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);

        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys: vec![lookup_key],
            refresh_all_content: false,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                error = %err,
                "Failed to deliver mod toggle result."
            );
        }
    });
}

pub(super) fn request_bulk_content_update(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    kind: InstalledContentKind,
    game_version: &str,
    loader_label: &str,
    download_policy: &DownloadPolicy,
) {
    if state.content_apply_in_flight {
        return;
    }

    let tx = state.content_apply_results.sender();

    let instance_name = state.name_input.clone();
    let instance_root = instance_root.to_path_buf();
    let game_version = game_version.trim().to_owned();
    let loader_label = loader_label.trim().to_owned();
    let download_policy = download_policy.clone();
    let progress_instance_name = instance_name.clone();
    let progress: InstallProgressCallback = Arc::new(move |progress| {
        install_activity::set_progress(progress_instance_name.as_str(), &progress);
    });
    let operation_label = super::bulk_update_button_label(kind);
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        kind = %kind.folder_name(),
        operation = %operation_label,
        game_version = %game_version,
        loader = %loader_label,
        "starting bulk content update"
    );

    state.content_apply_in_flight = true;
    state.status_message = Some(format!("{operation_label}..."));
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        format!("{operation_label}..."),
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking(move || {
            update_all_installed_content(
                instance_root.as_path(),
                kind,
                game_version.as_str(),
                loader_label.as_str(),
                &download_policy,
                Some(&progress),
            )
        });
        let result = match join.await {
            Ok(result) => result,
            Err(err) => Err(err.to_string()),
        };
        if result.is_ok() {
            install_activity::set_status(
                instance_name.as_str(),
                InstallStage::Complete,
                tr!("{operation_label} complete.", operation_label = operation_label).into_owned(),
            );
        }
        match &result {
            Ok(message) => tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                kind = %kind.folder_name(),
                operation = %operation_label,
                "bulk content update completed: {message}"
            ),
            Err(err) => tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                kind = %kind.folder_name(),
                operation = %operation_label,
                "bulk content update failed: {err}"
            ),
        }
        if let Err(err) = tx.send(ContentApplyResult {
            kind,
            focus_lookup_keys: Vec::new(),
            refresh_all_content: true,
            status_message: result,
        }) {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance = %instance_name,
                kind = %kind.folder_name(),
                operation = %operation_label,
                error = %err,
                "Failed to deliver bulk content update result."
            );
        }
    });
}

pub(super) fn poll_content_apply_results(state: &mut InstanceScreenState, instance_root: &Path) {
    let drained = state.content_apply_results.drain();
    let updates = drained.items;

    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_content", "content_apply_results worker channel stopped unexpectedly.");
        state.content_apply_in_flight = false;
        install_activity::clear_instance(state.name_input.as_str());
        state.status_message = Some(tr!("Content apply worker stopped unexpectedly.").into_owned());
    }

    for result in updates {
        state.content_apply_in_flight = false;
        install_activity::clear_instance(state.name_input.as_str());
        match result.status_message {
            Ok(message) => {
                if result.refresh_all_content {
                    invalidate_installed_content_state(state);
                } else {
                    // Only the file lists and managed identities are rescanned; the resolved metadata of every file
                    // that did not change is kept.
                    state.installed_content_cache.mark_stale();
                    super::refresh_cached_metadata_after_apply(
                        state,
                        instance_root,
                        result.kind,
                        result.focus_lookup_keys.as_slice(),
                    );
                }
                state.status_message = Some(message);
            }
            Err(err) => {
                if result.refresh_all_content {
                    invalidate_installed_content_state(state);
                }
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    error = %err,
                    "Applying content changes failed."
                );
                state.status_message = Some(tr!("Failed to apply content changes: {err}", err = err).into_owned());
            }
        }
    }
}
