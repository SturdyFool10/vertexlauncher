use vertex_i18n::{tr, trn};
use super::*;



pub(super) fn request_vtmpack_export(
    state: &mut InstanceScreenState,
    instance: VtmpackInstanceMetadata,
    instance_root: PathBuf,
    output_path: PathBuf,
    options: vtmpack::VtmpackExportOptions,
) {
    if state.export_vtmpack_in_flight {
        state.show_export_vtmpack_modal = true;
        return;
    }

    let progress_tx = state.export_vtmpack_progress.sender();
    let results_tx = state.export_vtmpack_results.sender();

    state.export_vtmpack_in_flight = true;
    state.export_vtmpack_output_path = Some(output_path.clone());
    state.export_vtmpack_latest_progress = None;
    state.show_export_vtmpack_modal = true;
    state.status_message = Some(tr!("Exporting {name} to {output_path}...", name = instance.name, output_path = output_path.display()).into_owned());

    let instance_name = instance.name.clone();
    let export_path_for_task = output_path.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let instance_name_for_progress = instance_name.clone();
        let output_path_for_progress = output_path.clone();
        let result = export_instance_as_vtmpack_with_progress(
            &instance,
            instance_root.as_path(),
            export_path_for_task.as_path(),
            &options,
            |progress| {
                if let Err(err) = progress_tx.send(progress) {
                    tracing::error!(
                        target: "vertexlauncher/instance_export",
                        instance_name = %instance_name_for_progress,
                        output_path = %output_path_for_progress.display(),
                        error = %err,
                        "Failed to deliver vtmpack export progress update."
                    );
                }
            },
        );
        if let Err(err) = results_tx.send(VtmpackExportOutcome {
            instance_name,
            output_path,
            is_patch: false,
            result,
        }) {
            tracing::error!(
                target: "vertexlauncher/instance_export",
                error = %err,
                "Failed to deliver vtmpack export result."
            );
        }
    });
}

pub(super) fn request_vtmpatch_export(
    state: &mut InstanceScreenState,
    instance_name: String,
    instance_root: PathBuf,
    base_vtmpack_path: PathBuf,
    output_path: PathBuf,
    options: vtmpack::VtmpackExportOptions,
) {
    if state.export_vtmpack_in_flight {
        state.show_export_vtmpack_modal = true;
        return;
    }

    let progress_tx = state.export_vtmpack_progress.sender();
    let results_tx = state.export_vtmpack_results.sender();

    state.export_vtmpack_in_flight = true;
    state.export_vtmpack_output_path = Some(output_path.clone());
    state.export_vtmpack_latest_progress = None;
    state.show_export_vtmpack_modal = true;
    state.status_message = Some(tr!("Generating patch for {instance_name} at {output_path}...", instance_name = instance_name, output_path = output_path.display()).into_owned());

    let instance_name_for_task = instance_name.clone();
    let output_path_for_task = output_path.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let instance_name_for_progress = instance_name_for_task.clone();
        let output_path_for_progress = output_path.clone();
        let result = export_instance_as_vtmpatch_with_progress(
            instance_name_for_task.as_str(),
            instance_root.as_path(),
            base_vtmpack_path.as_path(),
            output_path_for_task.as_path(),
            &options,
            |progress| {
                if let Err(err) = progress_tx.send(progress) {
                    tracing::error!(
                        target: "vertexlauncher/instance_export",
                        instance_name = %instance_name_for_progress,
                        output_path = %output_path_for_progress.display(),
                        error = %err,
                        "Failed to deliver vtmpatch export progress update."
                    );
                }
            },
        );
        if let Err(err) = results_tx.send(VtmpackExportOutcome {
            instance_name,
            output_path,
            is_patch: true,
            result,
        }) {
            tracing::error!(
                target: "vertexlauncher/instance_export",
                error = %err,
                "Failed to deliver vtmpatch export result."
            );
        }
    });
}

pub(super) fn poll_vtmpack_export_progress(state: &mut InstanceScreenState) {
    let drained = state.export_vtmpack_progress.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_export", "export_vtmpack_progress worker channel stopped unexpectedly.");
    }
    let updates = drained.items;

    if let Some(update) = updates.into_iter().last() {
        state.export_vtmpack_latest_progress = Some(update);
    }
}

pub(super) fn poll_vtmpack_export_results(state: &mut InstanceScreenState) {
    let drained = state.export_vtmpack_results.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_export", "export_vtmpack_results worker channel stopped unexpectedly.");
    }
    let updates = drained.items;

    for update in updates {
        state.export_vtmpack_in_flight = false;
        state.export_vtmpack_latest_progress = None;
        state.export_vtmpack_output_path = None;
        state.show_export_vtmpack_modal = false;
        let output_file_name = update
            .output_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| update.output_path.display().to_string());
        match update.result {
            Ok(stats) => {
                if update.is_patch {
                    let msg = tr!(
                        "Generated patch for {instance} — {counts} — saved to {path}",
                        instance = update.instance_name,
                        counts = export_counts(&stats, true),
                        path = update.output_path.display()
                    )
                    .into_owned();
                    tracing::info!(
                        target: "vertexlauncher/instance_export",
                        instance_name = %update.instance_name,
                        output = %update.output_path.display(),
                        downloadable_mods = stats.downloadable_mod_files,
                        bundled_mods = stats.bundled_mod_files,
                        config_files = stats.config_files,
                        additional_files = stats.additional_files,
                        "vtmpatch export succeeded"
                    );
                    notification::info!("vtmpatch/export", "Patch generated — {output_file_name} — {counts}", output_file_name = output_file_name, counts = export_counts(&stats, true));
                    state.status_message = Some(msg);
                } else {
                    let msg = tr!(
                        "Exported {instance} — {counts} — saved to {path}",
                        instance = update.instance_name,
                        counts = export_counts(&stats, false),
                        path = update.output_path.display()
                    )
                    .into_owned();
                    tracing::info!(
                        target: "vertexlauncher/instance_export",
                        instance_name = %update.instance_name,
                        output = %update.output_path.display(),
                        downloadable_mods = stats.downloadable_mod_files,
                        bundled_mods = stats.bundled_mod_files,
                        config_files = stats.config_files,
                        additional_files = stats.additional_files,
                        "vtmpack export succeeded"
                    );
                    notification::info!("vtmpack/export", "Pack exported — {output_file_name} — {counts}", output_file_name = output_file_name, counts = export_counts(&stats, false));
                    state.status_message = Some(msg);
                }
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    instance_name = %update.instance_name,
                    output_path = %update.output_path.display(),
                    error = %err,
                    "{} export failed",
                    if update.is_patch { "vtmpatch" } else { "vtmpack" }
                );
                let ext = if update.is_patch {
                    "vtmpatch"
                } else {
                    "vtmpack"
                };
                notification::error!("vtmpack/export", "{output_file_name} export failed — {err}", output_file_name = output_file_name, err = err);
                state.status_message = Some(tr!("Failed to export .{ext}: {err}", ext = ext, err = err).into_owned());
            }
        }
    }

    if drained.disconnected && state.export_vtmpack_in_flight {
        state.export_vtmpack_in_flight = false;
        state.export_vtmpack_latest_progress = None;
        state.export_vtmpack_output_path = None;
        state.show_export_vtmpack_modal = false;
        state.status_message =
            Some(tr!("Failed to export: export task stopped unexpectedly.").into_owned());
    }

    if drained.disconnected || !state.export_vtmpack_in_flight {
        state.export_vtmpack_progress.reset();
        state.export_vtmpack_results.reset();
    }
}

pub(super) fn default_server_root_entry_selected(entry: &str) -> bool {
    matches!(
        entry,
        "mods"
            | "config"
            | "defaultconfigs"
            | "kubejs"
            | "tacz"
            | "scripts"
            | "serverconfig"
            | "libraries"
            | "versions"
    )
}

pub(super) fn sync_server_export_options(
    instance_root: &Path,
    included_root_entries: &mut BTreeMap<String, bool>,
) {
    let available_entries = list_exportable_root_entries(instance_root);
    let available_set = available_entries.iter().cloned().collect::<HashSet<_>>();
    included_root_entries.retain(|entry, _| available_set.contains(entry));
    for entry in available_entries {
        included_root_entries
            .entry(entry.clone())
            .or_insert_with(|| default_server_root_entry_selected(entry.as_str()));
    }
}



pub(super) fn request_server_export(
    state: &mut InstanceScreenState,
    instance: instances::InstanceRecord,
    instance_root: PathBuf,
    output_path: PathBuf,
    included_root_entries: BTreeMap<String, bool>,
    force_java_21_minimum: bool,
) {
    if state.export_server_in_flight {
        state.show_export_server_modal = true;
        return;
    }

    let progress_tx = state.export_server_progress.sender();
    let results_tx = state.export_server_results.sender();

    state.export_server_in_flight = true;
    state.export_server_output_path = Some(output_path.clone());
    state.export_server_latest_progress = None;
    state.show_export_server_modal = true;
    state.status_message = Some(tr!("Building server zip for {name} at {output_path}...", name = instance.name, output_path = output_path.display()).into_owned());

    let instance_name = instance.name.clone();
    let output_path_for_task = output_path.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let instance_name_for_progress = instance_name.clone();
        let output_path_for_progress = output_path.clone();
        let result = tokio_runtime::spawn_blocking(move || {
            export_instance_as_server_zip_with_progress(
                &instance,
                instance_root.as_path(),
                output_path_for_task.as_path(),
                &included_root_entries,
                force_java_21_minimum,
                |progress| {
                    if let Err(err) = progress_tx.send(progress) {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            instance_name = %instance_name_for_progress,
                            output_path = %output_path_for_progress.display(),
                            error = %err,
                            "Failed to deliver server export progress update."
                        );
                    }
                },
            )
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if let Err(err) = results_tx.send(ServerExportOutcome {
            instance_name,
            output_path,
            result,
        }) {
            tracing::error!(
                target: "vertexlauncher/instance_export",
                error = %err,
                "Failed to deliver server export result."
            );
        }
    });
}

pub(super) fn poll_server_export_progress(state: &mut InstanceScreenState) {
    let drained = state.export_server_progress.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_export", "export_server_progress worker channel stopped unexpectedly.");
    }
    let updates = drained.items;

    if let Some(update) = updates.into_iter().last() {
        state.export_server_latest_progress = Some(update);
    }
}

pub(super) fn poll_server_export_results(state: &mut InstanceScreenState) {
    let drained = state.export_server_results.drain();
    if drained.disconnected {
        tracing::error!(target: "vertexlauncher/instance_export", "export_server_results worker channel stopped unexpectedly.");
    }
    let updates = drained.items;

    for update in updates {
        state.export_server_in_flight = false;
        state.export_server_latest_progress = None;
        state.export_server_output_path = None;
        state.show_export_server_modal = false;
        match update.result {
            Ok(summary) => {
                state.status_message = Some(tr!("Server zip ready for {instance_name} at {output_path}. {summary}", instance_name = update.instance_name, output_path = update.output_path.display(), summary = summary).into_owned());
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    instance_name = %update.instance_name,
                    output_path = %update.output_path.display(),
                    error = %err,
                    "Server export failed."
                );
                state.status_message = Some(tr!("Failed to export server zip: {err}", err = err).into_owned());
            }
        }
    }

    if drained.disconnected && state.export_server_in_flight {
        state.export_server_in_flight = false;
        state.export_server_latest_progress = None;
        state.export_server_output_path = None;
        state.show_export_server_modal = false;
        state.status_message =
            Some(tr!("Failed to export server zip: export task stopped unexpectedly.").into_owned());
    }

    if drained.disconnected || !state.export_server_in_flight {
        state.export_server_progress.reset();
        state.export_server_results.reset();
    }
}

pub(super) fn default_server_export_output_path(
    instance: &instances::InstanceRecord,
    config: &Config,
) -> PathBuf {
    let downloads_dir = UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(PathBuf::from))
        .unwrap_or_else(|| config.minecraft_installations_root_path().to_path_buf());
    let base_name = format!(
        "{}-server-{}-{}",
        sanitize_file_stem(instance.name.as_str()),
        sanitize_file_stem(instance.game_version.as_str()),
        sanitize_file_stem(instance.modloader.as_str()),
    );
    unique_file_path(downloads_dir.as_path(), base_name.as_str(), "zip")
}

fn sanitize_file_stem(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        let keep = lower.is_ascii_alphanumeric();
        if keep {
            out.push(lower);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "instance".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn unique_file_path(parent: &Path, stem: &str, extension: &str) -> PathBuf {
    let mut attempt = 0u32;
    loop {
        let file_name = if attempt == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{attempt}.{extension}")
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
        attempt = attempt.saturating_add(1);
    }
}

/// "1 downloadable mod, 2 bundled mods, ..." for an export summary, each count with its noun in the right plural form.
fn export_counts(stats: &vtmpack::VtmpackExportStats, changed_only: bool) -> String {
    let (config, additional) = if changed_only {
        (
            trn!(stats.config_files, "{count} changed config file", "{count} changed config files"),
            trn!(stats.additional_files, "{count} changed additional file", "{count} changed additional files"),
        )
    } else {
        (
            trn!(stats.config_files, "{count} config file", "{count} config files"),
            trn!(stats.additional_files, "{count} additional file", "{count} additional files"),
        )
    };
    notification::natural_language::join_natural(&[
        trn!(stats.downloadable_mod_files, "{count} downloadable mod", "{count} downloadable mods"),
        trn!(stats.bundled_mod_files, "{count} bundled mod", "{count} bundled mods"),
        config,
        additional,
    ])
}
