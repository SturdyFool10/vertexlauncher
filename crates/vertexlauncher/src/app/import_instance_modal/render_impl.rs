use super::*;

pub fn render(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut ImportInstanceState,
    curseforge_api_key_configured: bool,
) -> ModalAction {
    poll_preview_results(state);
    let mut action = ModalAction::None;
    if state.preview_in_flight || state.import_in_flight {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let response = show_dialog(
        ctx,
        dialog_options("import_instance_modal_window", DialogPreset::Form),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(MODAL_GAP_MD, MODAL_GAP_MD);
            let text_color = ui.visuals().text_color();
            let heading_style = LabelOptions {
                font_size: 34.0,
                line_height: 38.0,
                weight: 700,
                color: text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            let body_style = LabelOptions {
                font_size: 18.0,
                line_height: 24.0,
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            };

            let _ = text_ui.label(
                ui,
                "instance_import_heading",
                "Import Profile",
                &heading_style,
            );
            let _ = text_ui.label(
                ui,
                "instance_import_subheading",
                "Import from a pack manifest or copy an instance out of another launcher.",
                &body_style,
            );

            let previous_mode = state.source_mode_index;
            let _ = settings_widgets::full_width_dropdown_row(
                text_ui,
                ui,
                "instance_import_mode",
                "Import source",
                Some(
                    "Choose whether to import from a pack manifest or an existing launcher instance folder.",
                ),
                &mut state.source_mode_index,
                &ImportMode::options(),
            );
            if state.source_mode_index != previous_mode {
                state.preview = None;
                state.error = None;
            }
            ui.add_space(MODAL_GAP_SM);

            match selected_import_mode(state) {
                ImportMode::ManifestFile => {
                    let previous_path = state.package_path.clone();
                    let mut package_path_input = path_input_string(state.package_path.as_path());
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_import_package_path",
                        "Manifest file",
                        Some("Select a .vtmpack, .mrpack, or CurseForge modpack .zip file."),
                        &mut package_path_input,
                    );
                    if update_path_from_input(&mut state.package_path, &package_path_input)
                        || state.package_path != previous_path
                    {
                        state.preview = None;
                        state.error = None;
                    }

                    ui.horizontal(|ui| {
                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_choose_file",
                            "Choose manifest",
                            (ui.available_width() * 0.5).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            if let Some(path) = pick_import_file() {
                                state.package_path = path;
                                load_preview_from_state(state);
                            }
                        }

                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_inspect_file",
                            "Inspect manifest",
                            (ui.available_width()).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            load_preview_from_state(state);
                        }
                    });

                    let highlight_curseforge_notice = !curseforge_api_key_configured
                        && (matches!(
                            state.preview.as_ref().map(|preview| preview.kind),
                            Some(ImportPreviewKind::Manifest(
                                ImportPackageKind::CurseForgePack
                            ))
                        ) || state
                            .package_path
                            .as_os_str()
                            .to_string_lossy()
                            .trim()
                            .to_ascii_lowercase()
                            .ends_with(".zip"));
                    let _ = text_ui.label(
                        ui,
                        "instance_import_curseforge_notice",
                        "CurseForge modpack zips are supported, but they only work if you have a CurseForge API key in Settings. Vertex will fall back to Modrinth downloads when it can resolve an exact compatible match.",
                        &LabelOptions {
                            color: if highlight_curseforge_notice {
                                ui.visuals().error_fg_color
                            } else {
                                ui.visuals().weak_text_color()
                            },
                            wrap: true,
                            ..LabelOptions::default()
                        },
                    );
                }
                ImportMode::LauncherDirectory => {
                    let previous_path = state.launcher_path.clone();
                    let previous_launcher_kind = state.launcher_kind_index;
                    let mut launcher_path_input = path_input_string(state.launcher_path.as_path());
                    let _ = settings_widgets::full_width_dropdown_row(
                        text_ui,
                        ui,
                        "instance_import_launcher_kind",
                        "Launcher",
                        Some(
                            "Use Auto-detect unless you know which launcher produced the instance.",
                        ),
                        &mut state.launcher_kind_index,
                        &LAUNCHER_KIND_OPTIONS,
                    );
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_import_launcher_path",
                        "Instance folder",
                        Some(
                            "Choose the instance directory from Modrinth, CurseForge, Prism, ATLauncher, or another launcher.",
                        ),
                        &mut launcher_path_input,
                    );
                    if update_path_from_input(&mut state.launcher_path, &launcher_path_input)
                        || state.launcher_path != previous_path
                        || state.launcher_kind_index != previous_launcher_kind
                    {
                        state.preview = None;
                        state.error = None;
                    }

                    ui.horizontal(|ui| {
                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_choose_folder",
                            "Choose folder",
                            (ui.available_width() * 0.5).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            if let Some(path) = pick_import_directory() {
                                state.launcher_path = path;
                                load_preview_from_state(state);
                            }
                        }

                        if settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_import_inspect_launcher",
                            "Inspect folder",
                            (ui.available_width()).clamp(120.0, ACTION_BUTTON_MAX_WIDTH),
                            false,
                        )
                        .clicked()
                        {
                            load_preview_from_state(state);
                        }
                    });
                }
            }

            ui.add_space(MODAL_GAP_SM);
            let _ = settings_widgets::full_width_text_input_row(
                text_ui,
                ui,
                "instance_import_name",
                "Imported profile name",
                Some("Defaults to the package name, but you can override it."),
                &mut state.instance_name,
            );

            if let Some(preview) = state.preview.as_ref() {
                ui.add_space(MODAL_GAP_SM);
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_title",
                        "Detected package",
                        &LabelOptions {
                            font_size: 20.0,
                            line_height: 24.0,
                            weight: 600,
                            color: ui.visuals().text_color(),
                            wrap: false,
                            ..LabelOptions::default()
                        },
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_kind",
                        preview.kind.label(),
                        &body_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_versions",
                        format!(
                            "Minecraft {} • {}",
                            preview.game_version,
                            format_loader_label(
                                preview.modloader.as_str(),
                                preview.modloader_version.as_str()
                            )
                        )
                        .as_str(),
                        &body_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_import_preview_summary",
                        preview.summary.as_str(),
                        &body_style,
                    );
                });
            }

            if let Some(error) = state.error.as_deref() {
                let _ = text_ui.label(
                    ui,
                    "instance_import_error",
                    error,
                    &LabelOptions {
                        color: ui.visuals().error_fg_color,
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
            }

            if state.preview_in_flight {
                let _ = text_ui.label(
                    ui,
                    "instance_import_preview_in_flight",
                    "Inspecting import source in the background...",
                    &body_style,
                );
            }

            if state.import_in_flight {
                let progress = state.import_latest_progress.as_ref();
                let progress_fraction = progress
                    .and_then(|progress| {
                        (progress.total_steps > 0).then_some(
                            progress.completed_steps as f32 / progress.total_steps as f32,
                        )
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0);
                let progress_message = progress
                    .map(|progress| progress.message.as_str())
                    .unwrap_or("Importing profile in the background...");
                let progress_counts = progress
                    .map(|progress| {
                        format!(
                            "{} of {} steps complete",
                            progress.completed_steps.min(progress.total_steps),
                            progress.total_steps
                        )
                    })
                    .unwrap_or_else(|| "Preparing import task...".to_owned());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        "instance_import_in_flight",
                        progress_message,
                        &body_style,
                    );
                });
                let _ = text_ui.label(
                    ui,
                    "instance_import_in_flight_counts",
                    progress_counts.as_str(),
                    &body_style,
                );
                ui.add(
                    egui::ProgressBar::new(progress_fraction)
                        .desired_width(ui.available_width())
                        .show_percentage(),
                );
            }

            ui.add_space(MODAL_GAP_LG);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled_ui(!state.import_in_flight, |ui| {
                        text_ui.button(
                            ui,
                            "instance_import_cancel",
                            "Cancel",
                            &secondary_button(ui, egui::vec2(160.0, style::CONTROL_HEIGHT)),
                        )
                    })
                    .inner
                    .clicked()
                {
                    action = ModalAction::Cancel;
                }

                let import_disabled = state.import_in_flight
                    || match selected_import_mode(state) {
                        ImportMode::ManifestFile => state.package_path.as_os_str().is_empty(),
                        ImportMode::LauncherDirectory => state.launcher_path.as_os_str().is_empty(),
                    };
                if ui
                    .add_enabled_ui(!import_disabled, |ui| {
                        text_ui.button(
                            ui,
                            "instance_import_confirm",
                            "Import profile",
                            &primary_button(ui, egui::vec2(160.0, style::CONTROL_HEIGHT)),
                        )
                    })
                    .inner
                    .clicked()
                {
                    if state.preview.is_none() {
                        load_preview_from_state(state);
                    }
                    if let Some(preview) = state.preview.as_ref() {
                        let instance_name = non_empty(state.instance_name.as_str())
                            .unwrap_or_else(|| preview.detected_name.clone());
                        action = ModalAction::Import(ImportRequest {
                            source: match selected_import_mode(state) {
                                ImportMode::ManifestFile => {
                                    ImportSource::ManifestFile(state.package_path.clone())
                                }
                                ImportMode::LauncherDirectory => ImportSource::LauncherDirectory {
                                    path: state.launcher_path.clone(),
                                    launcher: selected_launcher_hint(state),
                                },
                            },
                            instance_name,
                            manual_curseforge_files: HashMap::new(),
                            manual_curseforge_staging_dir: None,
                            max_concurrent_downloads: 4,
                        });
                    }
                }
            });
        },
    );

    if response.close_requested && !state.import_in_flight && matches!(action, ModalAction::None) {
        action = ModalAction::Cancel;
    }

    action
}

pub fn import_package_with_progress<F>(
    store: &mut InstanceStore,
    installations_root: &Path,
    request: ImportRequest,
    mut progress: F,
) -> Result<InstanceRecord, ImportPackageError>
where
    F: FnMut(ImportProgress),
{
    match &request.source {
        ImportSource::ManifestFile(path) => {
            let preview = inspect_package(path.as_path()).map_err(ImportPackageError::message)?;
            match preview.kind {
                ImportPreviewKind::Manifest(ImportPackageKind::VertexPack) => {
                    import_vtmpack(store, installations_root, &request, &mut progress)
                        .map_err(ImportPackageError::message)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::ModrinthPack) => {
                    import_mrpack(store, installations_root, &request, &mut progress)
                        .map_err(ImportPackageError::message)
                }
                ImportPreviewKind::Manifest(ImportPackageKind::CurseForgePack) => {
                    import_curseforge_pack(store, installations_root, &request, &mut progress)
                }
                ImportPreviewKind::Launcher(_) => Err(ImportPackageError::message(
                    "Launcher previews are not valid for manifest imports.",
                )),
            }
        }
        ImportSource::LauncherDirectory { .. } => {
            progress(import_progress("Copying launcher instance files...", 0, 0));
            import_launcher_instance(store, installations_root, &request)
                .map_err(ImportPackageError::message)
        }
    }
}

#[allow(dead_code)]
pub fn update_mrpack_instance_with_progress<F>(
    store: &mut InstanceStore,
    installations_root: &Path,
    instance_id: &str,
    package_path: &Path,
    mut progress: F,
) -> Result<InstanceRecord, String>
where
    F: FnMut(ImportProgress),
{
    let existing_instance = store
        .find(instance_id)
        .cloned()
        .ok_or_else(|| format!("instance {instance_id} was not found"))?;
    let existing_root = instance_root_path(installations_root, &existing_instance);
    let current_modpack_state = load_modpack_install_state(existing_root.as_path())
        .ok_or_else(|| "This instance is not tied to an updatable modpack yet.".to_owned())?;

    progress(import_progress("Reading updated .mrpack manifest...", 0, 1));
    let manifest = read_mrpack_manifest(package_path)?;
    let dependency_info = resolve_mrpack_dependencies(&manifest.dependencies)?;
    let override_steps = count_mrpack_override_entries(package_path)?;
    let total_steps = 6 + override_steps + manifest.files.len();
    let temp_root = unique_temp_instance_root(installations_root, existing_instance.id.as_str());

    fs_create_dir_all_logged(temp_root.as_path()).map_err(|err| {
        format!(
            "failed to create temp instance root {}: {err}",
            temp_root.display()
        )
    })?;
    fs_create_dir_all_logged(temp_root.join("mods").as_path())
        .map_err(|err| format!("failed to create temp mods directory: {err}"))?;

    progress(import_progress(
        "Building updated modpack files...",
        1,
        total_steps,
    ));
    if let Err(err) = populate_mrpack_instance(
        package_path,
        manifest.clone(),
        temp_root.as_path(),
        total_steps,
        &mut progress,
    ) {
        let _ = fs_remove_dir_all_logged(temp_root.as_path());
        return Err(err);
    }

    let new_base_manifest = build_mrpack_base_manifest(temp_root.as_path(), &manifest)?;
    let new_modpack_state = build_mrpack_install_state(package_path, &manifest, new_base_manifest);
    let current_manifest = load_content_manifest(existing_root.as_path());
    let mut final_manifest = new_modpack_state.base_manifest.clone();
    let pack_managed_paths =
        pack_managed_path_keys(&current_manifest, &current_modpack_state.base_manifest);

    progress(import_progress(
        "Preserving user-added content...",
        total_steps.saturating_sub(4),
        total_steps,
    ));
    preserve_non_pack_managed_content(
        existing_root.as_path(),
        temp_root.as_path(),
        &pack_managed_paths,
    )?;
    for (project_key, project) in current_manifest.projects {
        if !project.pack_managed {
            final_manifest.projects.insert(project_key, project);
        }
    }

    progress(import_progress(
        "Preserving worlds and servers...",
        total_steps.saturating_sub(3),
        total_steps,
    ));
    preserve_instance_user_state(existing_root.as_path(), temp_root.as_path())?;
    save_content_manifest(temp_root.as_path(), &final_manifest)?;
    save_modpack_install_state(temp_root.as_path(), &new_modpack_state)?;

    progress(import_progress(
        "Finalizing updated instance...",
        total_steps.saturating_sub(2),
        total_steps,
    ));
    swap_instance_root(existing_root.as_path(), temp_root.as_path())?;

    let instance = store
        .find_mut(instance_id)
        .ok_or_else(|| format!("instance {instance_id} disappeared during update"))?;
    instance.game_version = dependency_info.game_version;
    instance.modloader = dependency_info.modloader;
    instance.modloader_version = dependency_info.modloader_version;

    progress(import_progress(
        "Update complete.",
        total_steps,
        total_steps,
    ));
    Ok(instance.clone())
}

pub(super) fn load_preview_from_state(state: &mut ImportInstanceState) {
    ensure_preview_channel(state);
    let Some(tx) = state.preview_results_tx.as_ref().cloned() else {
        return;
    };
    let request = match selected_import_mode(state) {
        ImportMode::ManifestFile => {
            let path = state.package_path.clone();
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some(
                    "Choose a .vtmpack, .mrpack, or CurseForge modpack .zip file first.".to_owned(),
                );
                return;
            }
            (path, selected_launcher_hint(state), true)
        }
        ImportMode::LauncherDirectory => {
            let path = state.launcher_path.clone();
            if path.as_os_str().is_empty() {
                state.preview = None;
                state.error = Some("Choose an instance folder first.".to_owned());
                return;
            }
            (path, selected_launcher_hint(state), false)
        }
    };

    state.preview_request_serial = state.preview_request_serial.saturating_add(1);
    let request_serial = state.preview_request_serial;
    state.preview_in_flight = true;
    state.error = None;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            let (path, launcher_hint, manifest_mode) = request;
            if manifest_mode {
                inspect_package(path.as_path())
            } else {
                inspect_launcher_instance(path.as_path(), launcher_hint)
            }
        })
        .await
        .map_err(|err| err.to_string())
        .and_then(|result| result);
        if let Err(err) = tx.send((request_serial, result)) {
            tracing::error!(
                target: "vertexlauncher/import_instance",
                request_serial,
                error = %err,
                "Failed to deliver import preview result."
            );
        }
    });
}

pub(super) fn pick_import_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Launcher profiles", &["vtmpack", "mrpack", "zip"])
        .add_filter("Vertex packs", &["vtmpack"])
        .add_filter("Modrinth packs", &["mrpack"])
        .add_filter("CurseForge packs", &["zip"])
        .pick_file()
}

pub(super) fn pick_import_directory() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}
