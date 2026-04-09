use super::content_updates::update_all_installed_content;
use super::*;

fn ensure_content_apply_channel(state: &mut InstanceScreenState) {
    if state.content_apply_results_tx.is_some() && state.content_apply_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<ContentApplyResult>();
    state.content_apply_results_tx = Some(tx);
    state.content_apply_results_rx = Some(Arc::new(Mutex::new(rx)));
}

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

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

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
    state.status_message = Some(format!("Updating {}...", project_name));
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
        format!("Updating {}...", project_name),
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

pub(super) fn render_joined_content_browser_controls(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    open_browser_enabled: bool,
) -> (egui::Response, egui::Response) {
    let total_width = ui.available_width().max(1.0);
    let control_height = 34.0;
    let icon_button_width = control_height;
    let label_width = (total_width - icon_button_width).max(120.0);
    let button_style =
        style::neutral_button_with_min_size(ui, egui::vec2(label_width, control_height));
    let cr = style::CORNER_RADIUS_SM;
    let label_radius = egui::CornerRadius {
        nw: cr,
        sw: cr,
        ne: 0,
        se: 0,
    };
    let icon_radius = egui::CornerRadius {
        nw: 0,
        sw: 0,
        ne: cr,
        se: cr,
    };
    let (outer_rect, _) = ui.allocate_exact_size(
        egui::vec2(total_width, control_height),
        egui::Sense::hover(),
    );
    let label_rect =
        egui::Rect::from_min_size(outer_rect.min, egui::vec2(label_width, control_height));
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(label_rect.max.x, outer_rect.min.y),
        egui::vec2(icon_button_width, control_height),
    );

    let label_response = ui.interact(
        label_rect,
        ui.id().with(("instance_add_content_label", instance_id)),
        if open_browser_enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let label_visuals = if open_browser_enabled {
        ui.style().interact(&label_response)
    } else {
        &ui.visuals().widgets.inactive
    };
    ui.painter().rect(
        label_rect,
        label_radius,
        label_visuals.bg_fill,
        label_visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let label_inner = label_rect.shrink2(egui::vec2(button_style.padding.x, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_inner), |ui| {
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let label_options = LabelOptions {
                    font_size: button_style.font_size,
                    line_height: button_style.line_height,
                    weight: 700,
                    color: if open_browser_enabled {
                        button_style.text_color
                    } else {
                        ui.visuals().weak_text_color()
                    },
                    wrap: false,
                    ..LabelOptions::default()
                };
                let _ = text_ui.label(
                    ui,
                    ("instance_add_content_label", instance_id),
                    "Open Content Browser",
                    &label_options,
                );
            },
        );
    });

    let icon_response = ui.interact(
        icon_rect,
        ui.id().with(("instance_add_content_plus", instance_id)),
        egui::Sense::click(),
    );
    let icon_visuals = ui.style().interact(&icon_response);
    ui.painter().rect(
        icon_rect,
        icon_radius,
        icon_visuals.bg_fill,
        icon_visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon_color = ui.visuals().text_color();
    let themed_svg = apply_color_to_svg(assets::PLUS_SVG, icon_color);
    let uri = format!(
        "bytes://instance/content-plus/{instance_id}-{:02x}{:02x}{:02x}.svg",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let icon_size = (control_height - style::SPACE_MD * 2.0).clamp(12.0, 18.0);
    let icon_draw_rect =
        egui::Rect::from_center_size(icon_rect.center(), egui::vec2(icon_size, icon_size));
    egui::Image::from_bytes(uri, themed_svg).paint_at(ui, icon_draw_rect);

    (label_response, icon_response)
}

pub(super) fn refresh_installed_content_state(
    state: &mut InstanceScreenState,
    instance_root: &Path,
) {
    state.invalidate_installed_content_cache();
    state.content_metadata_cache.clear();
    state.content_lookup_in_flight.clear();
    state.content_lookup_latest_serial_by_key.clear();
    state.content_lookup_retry_after_by_key.clear();
    state.content_lookup_failure_count_by_key.clear();
    super::clear_content_hash_cache(state, instance_root);
}

pub(super) fn request_local_content_import(
    state: &mut InstanceScreenState,
    instance_root: &Path,
    selected_paths: Vec<PathBuf>,
) {
    if state.content_apply_in_flight || selected_paths.is_empty() {
        return;
    }

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    state.content_apply_in_flight = true;
    state.status_message = Some(format!(
        "Adding {} local content file{}...",
        selected_paths.len(),
        if selected_paths.len() == 1 { "" } else { "s" }
    ));
    install_activity::set_status(
        instance_name.as_str(),
        InstallStage::DownloadingCore,
        "Adding local content files...".to_owned(),
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
            failures.push(format!(
                "{} -> {} ({err})",
                source_path.display(),
                destination_dir.display()
            ));
            continue;
        }

        let destination_path = destination_dir.join(file_name);
        if let Err(err) = fs::copy(source_path.as_path(), destination_path.as_path()) {
            failures.push(format!(
                "{} -> {} ({err})",
                source_path.display(),
                destination_path.display()
            ));
            continue;
        }

        imported_total += 1;
        *counts_by_kind.entry(kind.label()).or_insert(0usize) += 1;
    }

    let mut summary = counts_by_kind
        .into_iter()
        .map(|(label, count)| format!("{count} {label}"))
        .collect::<Vec<_>>();
    if summary.is_empty() {
        summary.push("no files".to_owned());
    }

    if imported_total == 0 {
        if !failures.is_empty() {
            return Err(format!(
                "Failed to add local content: {}.",
                failures.join("; ")
            ));
        }
        if !skipped.is_empty() {
            return Err(format!(
                "Could not determine where to place: {}.",
                skipped.join(", ")
            ));
        }
    }

    let mut message = format!("Added {}.", summary.join(", "));
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

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

    let lookup_key = lookup_key.to_owned();
    let path = path.to_path_buf();
    let instance_root = instance_root.to_path_buf();
    let instance_name = state.name_input.clone();
    let path_display = path.display().to_string();

    state.content_apply_in_flight = true;
    state.status_message = Some("Removing installed content...".to_owned());
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance = %instance_name,
        lookup_key = %lookup_key,
        path = %path_display,
        "starting installed content delete"
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = (|| -> Result<String, String> {
            let delete_result = if path.is_dir() {
                std::fs::remove_dir_all(path.as_path())
            } else {
                std::fs::remove_file(path.as_path())
            };
            delete_result.map_err(|err| format!("failed to remove {}: {err}", path.display()))?;

            managed_content::remove_content_manifest_entries_for_path(
                instance_root.as_path(),
                path.as_path(),
            )
            .map(|_| "Removed installed content.".to_owned())
            .map_err(|err| {
                format!(
                    "removed installed content, but failed to update the content manifest: {err}"
                )
            })
        })();

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

    ensure_content_apply_channel(state);
    let Some(tx) = state.content_apply_results_tx.as_ref().cloned() else {
        return;
    };

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
                format!("{operation_label} complete."),
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
            refresh_all_content: false,
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
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.content_apply_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        tracing::error!(
                            target: "vertexlauncher/instance_content",
                            "Content-apply worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    "Content-apply receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.content_apply_results_tx = None;
        state.content_apply_results_rx = None;
        state.content_apply_in_flight = false;
        install_activity::clear_instance(state.name_input.as_str());
        state.status_message = Some("Content apply worker stopped unexpectedly.".to_owned());
    }

    for result in updates {
        state.content_apply_in_flight = false;
        install_activity::clear_instance(state.name_input.as_str());
        match result.status_message {
            Ok(message) => {
                if result.refresh_all_content {
                    refresh_installed_content_state(state, instance_root);
                } else {
                    state.invalidate_installed_content_cache();
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
                tracing::error!(
                    target: "vertexlauncher/instance_content",
                    error = %err,
                    "Applying content changes failed."
                );
                state.status_message = Some(format!("Failed to apply content changes: {err}"));
            }
        }
    }
}
