use super::*;

pub(super) fn render_export_vtmpack_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
) {
    if !state.show_export_vtmpack_modal {
        return;
    }

    let mut close_requested = false;
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root = instances
        .find(instance_id)
        .map(|instance| instances::instance_root_path(&installations_root, instance));
    if let Some(instance_root) = instance_root.as_deref() {
        sync_vtmpack_export_options(instance_root, &mut state.export_vtmpack_options);
    }
    let mut export_requested = false;
    let response = show_dialog(
        ctx,
        dialog_options(
            ("instance_export_vtmpack_modal", instance_id),
            DialogPreset::Form,
        ),
        |ui| {
            let title_style = style::modal_title(ui);
            let body_style = style::muted(ui);
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_title", instance_id),
                "Export .vtmpack",
                &title_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_vtmpack_body", instance_id),
                "Choose whether the exported pack may reference CurseForge metadata directly, then select which top-level files and folders from the Minecraft root should be bundled into the pack.",
                &body_style,
            );
            ui.add_space(12.0);

            if state.export_vtmpack_in_flight {
                let progress = state.export_vtmpack_latest_progress.as_ref();
                let progress_fraction = progress
                    .and_then(|progress| {
                        (progress.total_steps > 0).then_some(
                            progress.completed_steps as f32 / progress.total_steps as f32,
                        )
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let progress_label = progress
                    .map(|progress| progress.message.as_str())
                    .unwrap_or("Starting export...");
                let progress_counts = progress
                    .map(|progress| {
                        format!(
                            "{} of {} steps complete",
                            progress.completed_steps.min(progress.total_steps),
                            progress.total_steps
                        )
                    })
                    .unwrap_or_else(|| "Preparing export task...".to_owned());

                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_vtmpack_progress_title", instance_id),
                        "Export in progress",
                        &style::stat_label(ui),
                    );
                });
                ui.add_space(12.0);
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_progress_message", instance_id),
                    progress_label,
                    &body_style,
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_progress_counts", instance_id),
                    progress_counts.as_str(),
                    &body_style,
                );
                if let Some(path) = state.export_vtmpack_output_path.as_ref() {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_vtmpack_progress_path", instance_id),
                        &format!("Destination: {}", path.display()),
                        &style::muted(ui),
                    );
                }
                ui.add_space(10.0);
                ui.add(
                    egui::ProgressBar::new(progress_fraction)
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
            } else {
                for provider_mode in [
                    VtmpackProviderMode::IncludeCurseForge,
                    VtmpackProviderMode::ExcludeCurseForge,
                ] {
                    let selected = state.export_vtmpack_options.provider_mode == provider_mode;
                    if ui.radio(selected, provider_mode.label()).clicked() {
                        state.export_vtmpack_options.provider_mode = provider_mode;
                    }
                }

                ui.add_space(12.0);
                let explanation = match state.export_vtmpack_options.provider_mode {
                    VtmpackProviderMode::IncludeCurseForge => {
                        "Managed CurseForge entries stay downloadable in the pack manifest."
                    }
                    VtmpackProviderMode::ExcludeCurseForge => {
                        "CurseForge metadata is removed from the export. CurseForge-managed files are bundled into the pack unless they already use Modrinth as the selected source."
                    }
                };
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_explanation", instance_id),
                    explanation,
                    &body_style,
                );

                ui.add_space(16.0);
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_include_label", instance_id),
                    "Include top-level entries from the Minecraft root",
                    &style::stat_label(ui),
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_vtmpack_include_help", instance_id),
                    "Defaults to mods, resourcepacks, shaderpacks, and config. You can also include any other top-level files or folders found in the instance root.",
                    &body_style,
                );
                ui.add_space(8.0);

                if let Some(instance_root) = instance_root.as_deref() {
                    if !instance_root.is_dir() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_export_vtmpack_missing_root", instance_id),
                            &format!(
                                "Instance root directory not found: {}",
                                instance_root.display()
                            ),
                            &body_style,
                        );
                    } else {
                        let entries = list_exportable_root_entries(instance_root);
                        ui.set_width(ui.available_width());
                        if entries.is_empty() {
                            let _ = text_ui.label(
                                ui,
                                ("instance_export_vtmpack_empty_root", instance_id),
                                "No files or folders found in the instance root.",
                                &body_style,
                            );
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt(("instance_export_vtmpack_entries_scroll", instance_id))
                                .max_height(360.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    for entry in entries {
                                        let checked = state
                                            .export_vtmpack_options
                                            .included_root_entries
                                            .entry(entry.clone())
                                            .or_insert_with(|| {
                                                default_vtmpack_root_entry_selected(&entry)
                                            });
                                        let label = if instance_root.join(entry.as_str()).is_dir() {
                                            format!("{entry}/")
                                        } else {
                                            entry.clone()
                                        };
                                        ui.checkbox(checked, label);
                                    }
                                });
                        }
                    }
                } else {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_vtmpack_missing_instance", instance_id),
                        "Instance root is unavailable, so folder selection cannot be shown.",
                        &body_style,
                    );
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack_cancel", instance_id),
                            "Cancel",
                            &secondary_button(ui, egui::vec2(120.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack_confirm", instance_id),
                            "Choose file",
                            &primary_button(ui, egui::vec2(140.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        export_requested = true;
                    }
                });
            }
        },
    );
    close_requested |= response.close_requested;

    if close_requested && !state.export_vtmpack_in_flight {
        state.show_export_vtmpack_modal = false;
    }

    if export_requested {
        if let Some(instance) = instances.find(instance_id) {
            let instance_root = instances::instance_root_path(&installations_root, instance);
            let default_file_name = default_vtmpack_file_name(instance.name.as_str());
            let selected_output = rfd::FileDialog::new()
                .set_title("Export Modpack")
                .set_file_name(default_file_name.as_str())
                .add_filter("Vertex Modpack", &[VTMPACK_EXTENSION])
                .save_file();

            if let Some(selected_path) = selected_output {
                let output_path = enforce_vtmpack_extension(selected_path);
                let pack_instance = VtmpackInstanceMetadata {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    game_version: instance.game_version.clone(),
                    modloader: instance.modloader.clone(),
                    modloader_version: instance.modloader_version.clone(),
                };
                request_vtmpack_export(
                    state,
                    pack_instance,
                    instance_root,
                    output_path,
                    state.export_vtmpack_options.clone(),
                );
                state.show_export_vtmpack_modal = true;
            }
        } else {
            state.status_message = Some("Instance was removed before export.".to_owned());
            state.show_export_vtmpack_modal = false;
        }
    }

    state.show_export_vtmpack_modal =
        state.show_export_vtmpack_modal || state.export_vtmpack_in_flight;
}

fn ensure_vtmpack_export_channels(state: &mut InstanceScreenState) {
    if state.export_vtmpack_progress_tx.is_none() || state.export_vtmpack_progress_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_vtmpack_progress_tx = Some(tx);
        state.export_vtmpack_progress_rx = Some(Arc::new(Mutex::new(rx)));
    }
    if state.export_vtmpack_results_tx.is_none() || state.export_vtmpack_results_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_vtmpack_results_tx = Some(tx);
        state.export_vtmpack_results_rx = Some(Arc::new(Mutex::new(rx)));
    }
}

fn request_vtmpack_export(
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

    ensure_vtmpack_export_channels(state);
    let Some(progress_tx) = state.export_vtmpack_progress_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start .vtmpack export progress channel.".to_owned());
        return;
    };
    let Some(results_tx) = state.export_vtmpack_results_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start .vtmpack export result channel.".to_owned());
        return;
    };

    state.export_vtmpack_in_flight = true;
    state.export_vtmpack_output_path = Some(output_path.clone());
    state.export_vtmpack_latest_progress = None;
    state.show_export_vtmpack_modal = true;
    state.status_message = Some(format!(
        "Exporting {} to {}...",
        instance.name,
        output_path.display()
    ));

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

pub(super) fn poll_vtmpack_export_progress(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_vtmpack_progress_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "vtmpack export progress worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "vtmpack export progress receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel && !state.export_vtmpack_in_flight {
        state.export_vtmpack_progress_tx = None;
        state.export_vtmpack_progress_rx = None;
    }

    if let Some(update) = updates.into_iter().last() {
        state.export_vtmpack_latest_progress = Some(update);
    }
}

pub(super) fn poll_vtmpack_export_results(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_vtmpack_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "vtmpack export result worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "vtmpack export result receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    for update in updates {
        state.export_vtmpack_in_flight = false;
        state.export_vtmpack_latest_progress = None;
        state.export_vtmpack_output_path = None;
        state.show_export_vtmpack_modal = false;
        match update.result {
            Ok(stats) => {
                state.status_message = Some(format!(
                    "Exported {} ({} bundled mods, {} config files, {} additional files) to {}",
                    update.instance_name,
                    stats.bundled_mod_files,
                    stats.config_files,
                    stats.additional_files,
                    update.output_path.display()
                ));
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    instance_name = %update.instance_name,
                    output_path = %update.output_path.display(),
                    error = %err,
                    "vtmpack export failed."
                );
                state.status_message = Some(format!("Failed to export .vtmpack: {err}"));
            }
        }
    }

    if should_reset_channel && state.export_vtmpack_in_flight {
        state.export_vtmpack_in_flight = false;
        state.export_vtmpack_latest_progress = None;
        state.export_vtmpack_output_path = None;
        state.show_export_vtmpack_modal = false;
        state.status_message =
            Some("Failed to export .vtmpack: export task stopped unexpectedly.".to_owned());
    }

    if should_reset_channel || !state.export_vtmpack_in_flight {
        state.export_vtmpack_progress_tx = None;
        state.export_vtmpack_progress_rx = None;
        state.export_vtmpack_results_tx = None;
        state.export_vtmpack_results_rx = None;
    }
}

fn default_server_root_entry_selected(entry: &str) -> bool {
    matches!(
        entry,
        "mods"
            | "config"
            | "defaultconfigs"
            | "kubejs"
            | "scripts"
            | "serverconfig"
            | "libraries"
            | "versions"
    )
}

fn sync_server_export_options(
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

pub(super) fn render_export_server_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &InstanceStore,
    config: &Config,
) {
    if !state.show_export_server_modal {
        return;
    }

    let mut close_requested = false;
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root = instances
        .find(instance_id)
        .map(|instance| instances::instance_root_path(&installations_root, instance));
    if let Some(instance_root) = instance_root.as_deref() {
        sync_server_export_options(
            instance_root,
            &mut state.export_server_included_root_entries,
        );
    }
    let mut export_requested = false;
    let response = show_dialog(
        ctx,
        dialog_options(
            ("instance_export_server_modal", instance_id),
            DialogPreset::Form,
        ),
        |ui| {
            let title_style = style::modal_title(ui);
            let body_style = style::muted(ui);
            let _ = text_ui.label(
                ui,
                ("instance_export_server_title", instance_id),
                "Auto-generate server zip",
                &title_style,
            );
            let _ = text_ui.label(
                ui,
                ("instance_export_server_body", instance_id),
                "Builds a portable server package in your Downloads folder using this instance's files. CurseForge-managed mods that cannot be resolved on Modrinth by hash are listed as unknowns in the report.",
                &body_style,
            );
            ui.add_space(12.0);

            if state.export_server_in_flight {
                let progress = state.export_server_latest_progress.as_ref();
                let progress_fraction = progress
                    .and_then(|progress| {
                        (progress.total_steps > 0).then_some(
                            progress.completed_steps as f32 / progress.total_steps as f32,
                        )
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let progress_label = progress
                    .map(|progress| progress.message.as_str())
                    .unwrap_or("Starting export...");
                let progress_counts = progress
                    .map(|progress| {
                        format!(
                            "{} of {} steps complete",
                            progress.completed_steps.min(progress.total_steps),
                            progress.total_steps
                        )
                    })
                    .unwrap_or_else(|| "Preparing export task...".to_owned());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_server_progress_title", instance_id),
                        "Server export in progress",
                        &style::stat_label(ui),
                    );
                });
                ui.add_space(12.0);
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_progress_message", instance_id),
                    progress_label,
                    &body_style,
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_progress_counts", instance_id),
                    progress_counts.as_str(),
                    &body_style,
                );
                if let Some(path) = state.export_server_output_path.as_ref() {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_server_progress_path", instance_id),
                        &format!("Destination: {}", path.display()),
                        &style::muted(ui),
                    );
                }
                ui.add_space(10.0);
                ui.add(
                    egui::ProgressBar::new(progress_fraction)
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
            } else {
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_include_label", instance_id),
                    "Include top-level entries from the Minecraft root",
                    &style::stat_label(ui),
                );
                let _ = text_ui.label(
                    ui,
                    ("instance_export_server_include_help", instance_id),
                    "Defaults to common server directories. You can enable or disable any top-level file or folder before export.",
                    &body_style,
                );
                ui.add_space(8.0);

                if let Some(instance_root) = instance_root.as_deref() {
                    let entries = list_exportable_root_entries(instance_root);
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt(("instance_export_server_entries_scroll", instance_id))
                        .max_height(360.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for entry in entries {
                                let checked = state
                                    .export_server_included_root_entries
                                    .entry(entry.clone())
                                    .or_insert_with(|| {
                                        default_server_root_entry_selected(entry.as_str())
                                    });
                                let label = if instance_root.join(entry.as_str()).is_dir() {
                                    format!("{entry}/")
                                } else {
                                    entry.clone()
                                };
                                ui.checkbox(checked, label);
                            }
                        });
                } else {
                    let _ = text_ui.label(
                        ui,
                        ("instance_export_server_missing_instance", instance_id),
                        "Instance root is unavailable, so folder selection cannot be shown.",
                        &body_style,
                    );
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_server_cancel", instance_id),
                            "Cancel",
                            &secondary_button(ui, egui::vec2(120.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_server_confirm", instance_id),
                            "Build zip in Downloads",
                            &primary_button(ui, egui::vec2(196.0, style::CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        export_requested = true;
                    }
                });
            }
        },
    );
    close_requested |= response.close_requested;

    if close_requested && !state.export_server_in_flight {
        state.show_export_server_modal = false;
    }

    if export_requested {
        if let Some(instance) = instances.find(instance_id) {
            let instance_root = instances::instance_root_path(&installations_root, instance);
            let output_path = default_server_export_output_path(instance, config);
            request_server_export(
                state,
                instance.clone(),
                instance_root,
                output_path,
                state.export_server_included_root_entries.clone(),
                config.force_java_21_minimum(),
            );
            state.show_export_server_modal = true;
        } else {
            state.status_message = Some("Instance was removed before export.".to_owned());
            state.show_export_server_modal = false;
        }
    }

    state.show_export_server_modal =
        state.show_export_server_modal || state.export_server_in_flight;
}

fn ensure_server_export_channels(state: &mut InstanceScreenState) {
    if state.export_server_progress_tx.is_none() || state.export_server_progress_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_server_progress_tx = Some(tx);
        state.export_server_progress_rx = Some(Arc::new(Mutex::new(rx)));
    }
    if state.export_server_results_tx.is_none() || state.export_server_results_rx.is_none() {
        let (tx, rx) = mpsc::channel();
        state.export_server_results_tx = Some(tx);
        state.export_server_results_rx = Some(Arc::new(Mutex::new(rx)));
    }
}

fn request_server_export(
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

    ensure_server_export_channels(state);
    let Some(progress_tx) = state.export_server_progress_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start server export progress channel.".to_owned());
        return;
    };
    let Some(results_tx) = state.export_server_results_tx.as_ref().cloned() else {
        state.status_message = Some("Failed to start server export result channel.".to_owned());
        return;
    };

    state.export_server_in_flight = true;
    state.export_server_output_path = Some(output_path.clone());
    state.export_server_latest_progress = None;
    state.show_export_server_modal = true;
    state.status_message = Some(format!(
        "Building server zip for {} at {}...",
        instance.name,
        output_path.display()
    ));

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
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_server_progress_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "Server export progress worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "Server export progress receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel && !state.export_server_in_flight {
        state.export_server_progress_tx = None;
        state.export_server_progress_rx = None;
    }

    if let Some(update) = updates.into_iter().last() {
        state.export_server_latest_progress = Some(update);
    }
}

pub(super) fn poll_server_export_results(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.export_server_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_export",
                            "Server export result worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    "Server export result receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    for update in updates {
        state.export_server_in_flight = false;
        state.export_server_latest_progress = None;
        state.export_server_output_path = None;
        state.show_export_server_modal = false;
        match update.result {
            Ok(summary) => {
                state.status_message = Some(format!(
                    "Server zip ready for {} at {}. {summary}",
                    update.instance_name,
                    update.output_path.display()
                ));
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/instance_export",
                    instance_name = %update.instance_name,
                    output_path = %update.output_path.display(),
                    error = %err,
                    "Server export failed."
                );
                state.status_message = Some(format!("Failed to export server zip: {err}"));
            }
        }
    }

    if should_reset_channel && state.export_server_in_flight {
        state.export_server_in_flight = false;
        state.export_server_latest_progress = None;
        state.export_server_output_path = None;
        state.show_export_server_modal = false;
        state.status_message =
            Some("Failed to export server zip: export task stopped unexpectedly.".to_owned());
    }

    if should_reset_channel || !state.export_server_in_flight {
        state.export_server_progress_tx = None;
        state.export_server_progress_rx = None;
        state.export_server_results_tx = None;
        state.export_server_results_rx = None;
    }
}

fn default_server_export_output_path(
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
