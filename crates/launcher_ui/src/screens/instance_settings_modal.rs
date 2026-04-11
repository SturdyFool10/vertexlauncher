use super::*;
use std::sync::{Mutex, OnceLock, mpsc};

#[path = "instance_settings_modal/memory_slider_max_state.rs"]
mod memory_slider_max_state;

use self::memory_slider_max_state::MemorySliderMaxState;

pub(super) fn render_instance_settings_modal(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut InstanceScreenState,
    instances: &mut InstanceStore,
    config: &mut Config,
) -> bool {
    if !state.show_settings_modal {
        return false;
    }

    let mut instances_changed = false;
    let mut close_requested = false;
    let response = modal::show_window(
        ctx,
        "Instance Settings",
        modal::ModalOptions::new(
            egui::Id::new(("instance_settings_modal", instance_id)),
            modal::ModalLayout::centered(
                modal::AxisSizing::new(0.92, 1.0, f32::INFINITY),
                modal::AxisSizing::new(0.92, 1.0, f32::INFINITY),
            ),
        )
        .with_layer(modal::ModalLayer::Base)
        .with_dismiss_behavior(modal::DismissBehavior::EscapeAndScrim),
        |ui| {
            let muted_style = style::muted(ui);
            let section_style = style::subtitle(ui);
            let body_style = style::body(ui);
            let action_button_style = ButtonOptions {
                min_size: egui::vec2(220.0, 34.0),
                text_color: ui.visuals().widgets.active.fg_stroke.color,
                fill: ui.visuals().selection.bg_fill,
                fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
                fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().selection.stroke,
                ..ButtonOptions::default()
            };
            let refresh_style = style::neutral_button_with_min_size(ui, egui::vec2(190.0, 30.0));
            let reinstall_button_style =
                style::neutral_button_with_min_size(ui, egui::vec2(220.0, 34.0));
            egui::ScrollArea::vertical()
                .id_salt(("instance_settings_modal_scroll", instance_id))
                .scroll_source(ScrollSource {
                    scroll_bar: true,
                    drag: false,
                    mouse_wheel: true,
                })
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_heading", instance_id),
                        "Instance Settings",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_description", instance_id),
                        "Manage this profile's metadata, version stack, runtime overrides, and maintenance actions.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_versions_heading", instance_id),
                        "Metadata & Versions",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_versions_description", instance_id),
                        "Display info, Minecraft version, and modloader selection for this instance.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_name_input", instance_id),
                        "Name",
                        Some("Display name shown in the sidebar."),
                        &mut state.name_input,
                    );
                    ui.add_space(6.0);
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_description_input", instance_id),
                        "Description (optional)",
                        Some("Optional note shown in library instance tiles."),
                        &mut state.description_input,
                    );
                    ui.add_space(6.0);

                    let mut thumbnail_input =
                        state.thumbnail_input.as_os_str().to_string_lossy().into_owned();
                    let thumbnail_changed = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_thumbnail_input", instance_id),
                        "Thumbnail path (optional)",
                        Some("Local image path for this instance."),
                        &mut thumbnail_input,
                    )
                    .changed();
                    if thumbnail_changed {
                        let trimmed = thumbnail_input.trim();
                        state.thumbnail_input = if trimmed.is_empty() {
                            PathBuf::new()
                        } else {
                            PathBuf::from(trimmed)
                        };
                    }
                    ui.add_space(6.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_refresh_versions", instance_id),
                            "Refresh version list",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        sync_version_catalog(state, config.include_snapshots_and_betas(), true);
                        state.modloader_versions_cache.clear();
                        state.modloader_versions_status = None;
                        state.modloader_versions_status_key = None;
                    }
                    if state.version_catalog_in_flight {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let _ = text_ui.label(
                                ui,
                                ("instance_versions_loading", instance_id),
                                "Fetching version catalog...",
                                &muted_style,
                            );
                        });
                    }

                    if let Some(catalog_error) = state.version_catalog_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_version_catalog_error", instance_id),
                            catalog_error,
                            &style::error_text(ui),
                        );
                    }

                    let version_labels: Vec<String> = state
                        .available_game_versions
                        .iter()
                        .map(MinecraftVersionEntry::display_label)
                        .collect();
                    let version_refs: Vec<&str> =
                        version_labels.iter().map(String::as_str).collect();
                    if !version_refs.is_empty() {
                        let mut selected_index = state
                            .selected_game_version_index
                            .min(version_refs.len().saturating_sub(1));
                        let response = settings_widgets::dropdown_row(
                            text_ui,
                            ui,
                            ("instance_game_version_dropdown", instance_id),
                            "Minecraft game version",
                            Some("Pick from available Minecraft versions."),
                            &mut selected_index,
                            &version_refs,
                        );
                        if response.changed() {
                            state.selected_game_version_index = selected_index;
                            if let Some(version) = state.available_game_versions.get(selected_index)
                            {
                                state.game_version_input = version.id.clone();
                            }
                        }
                    } else {
                        let _ = text_ui.label(
                            ui,
                            ("instance_game_version_empty", instance_id),
                            "No game versions available yet.",
                            &muted_style,
                        );
                    }
                    ui.add_space(6.0);

                    let selected_game_version_for_loader = selected_game_version(state).to_owned();
                    ensure_selected_modloader_is_supported(
                        state,
                        selected_game_version_for_loader.as_str(),
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_modloader_label", instance_id),
                        "Modloader",
                        &body_style,
                    );
                    ui.add_space(4.0);
                    render_modloader_selector(
                        ui,
                        text_ui,
                        state,
                        instance_id,
                        selected_game_version_for_loader.as_str(),
                    );
                    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
                        ui.add_space(6.0);
                        let _ = settings_widgets::full_width_text_input_row(
                            text_ui,
                            ui,
                            ("instance_custom_modloader_input", instance_id),
                            "Custom modloader id",
                            Some("Use any custom modloader name."),
                            &mut state.custom_modloader,
                        );
                    }
                    ui.add_space(6.0);

                    let selected_modloader_label = selected_modloader_value(state);
                    let modloader_versions_key = modloader_versions_cache_key(
                        selected_modloader_label.as_str(),
                        selected_game_version_for_loader.as_str(),
                    );
                    let available_modloader_versions =
                        selected_modloader_versions(state, selected_game_version_for_loader.as_str())
                            .to_vec();
                    if state.selected_modloader == 0 {
                        state.modloader_version_input.clear();
                    } else {
                        let mut resolved_modloader_versions = available_modloader_versions;
                        let should_fetch_remote = state.selected_modloader != CUSTOM_MODLOADER_INDEX
                            && resolved_modloader_versions.is_empty();
                        if should_fetch_remote {
                            if let Some(cached) =
                                state.modloader_versions_cache.get(&modloader_versions_key)
                            {
                                resolved_modloader_versions = cached.clone();
                            } else {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version_for_loader.as_str(),
                                    false,
                                );
                            }
                        }

                        let in_flight = state
                            .modloader_versions_in_flight
                            .contains(&modloader_versions_key);
                        if in_flight {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                let _ = text_ui.label(
                                    ui,
                                    ("instance_modloader_versions_fetching", instance_id),
                                    "Fetching modloader versions...",
                                    &muted_style,
                                );
                            });
                        }

                        if state.modloader_versions_status_key.as_deref()
                            == Some(modloader_versions_key.as_str())
                            && let Some(status) = state.modloader_versions_status.as_deref()
                        {
                            let is_error = status.starts_with("Failed");
                            let _ = text_ui.label(
                                ui,
                                ("instance_modloader_versions_status", instance_id),
                                status,
                                &if is_error {
                                    style::error_text(ui)
                                } else {
                                    style::muted(ui)
                                },
                            );
                        }

                        let modloader_version_options: Vec<String> =
                            resolved_modloader_versions.clone();

                        if state.modloader_version_input.trim().is_empty()
                            && let Some(first) = modloader_version_options.first()
                        {
                            state.modloader_version_input = first.clone();
                        }

                        let option_refs: Vec<&str> =
                            modloader_version_options.iter().map(String::as_str).collect();
                        let current_modloader_version =
                            state.modloader_version_input.trim().to_owned();
                        let mut selected_index = modloader_version_options
                            .iter()
                            .position(|entry| entry == &current_modloader_version)
                            .unwrap_or(0);
                        if settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            ("instance_modloader_version_dropdown", instance_id),
                            "Modloader version",
                            Some("Cataloged by loader+Minecraft compatibility and cached once per day."),
                            &mut selected_index,
                            &option_refs,
                        )
                        .changed()
                        {
                            if let Some(selected) = modloader_version_options.get(selected_index) {
                                state.modloader_version_input = selected.clone();
                            }
                        }

                        if state.selected_modloader != CUSTOM_MODLOADER_INDEX {
                            let refresh_clicked = ui
                                .add_enabled_ui(!in_flight, |ui| {
                                    text_ui.button(
                                        ui,
                                        ("instance_modloader_versions_refresh", instance_id),
                                        "Refresh modloader versions",
                                        &refresh_style,
                                    )
                                })
                                .inner
                                .clicked();
                            if refresh_clicked {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version_for_loader.as_str(),
                                    true,
                                );
                            }
                        }

                        if resolved_modloader_versions.is_empty()
                            && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                        {
                            let _ = text_ui.label(
                                ui,
                                ("instance_modloader_versions_unavailable", instance_id),
                                "No cataloged modloader versions were found for this Minecraft version.",
                                &muted_style,
                            );
                        }
                    }

                    ui.add_space(8.0);

                    let trimmed_name = state.name_input.trim();
                    let requested_modloader = selected_modloader_value(state);
                    let requested_game_version = state.game_version_input.trim().to_owned();
                    let validation_error = if trimmed_name.is_empty() {
                        Some("Name cannot be empty.".to_owned())
                    } else if requested_game_version.is_empty() {
                        Some("Minecraft game version cannot be empty.".to_owned())
                    } else if requested_modloader.trim().is_empty() {
                        Some("Modloader cannot be empty.".to_owned())
                    } else if support_catalog_ready(state)
                        && !state.loader_support.supports_loader(
                            requested_modloader.as_str(),
                            requested_game_version.as_str(),
                        )
                        && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                    {
                        Some(format!(
                            "{} is not available for Minecraft {}.",
                            requested_modloader, requested_game_version
                        ))
                    } else {
                        resolve_modloader_version_for_settings(
                            state,
                            requested_modloader.as_str(),
                            requested_game_version.as_str(),
                        )
                        .err()
                    };
                    let can_save_versions = validation_error.is_none();
                    if let Some(error) = validation_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            ("instance_save_versions_validation_error", instance_id),
                            error,
                            &style::error_text(ui),
                        );
                        ui.add_space(6.0);
                    }

                    let save_versions_clicked = ui
                        .add_enabled_ui(can_save_versions, |ui| {
                            text_ui.button(
                                ui,
                                ("instance_save_versions", instance_id),
                                "Save metadata & versions",
                                &action_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    let reinstall_enabled =
                        can_save_versions && !state.runtime_prepare_in_flight && !state.running;
                    let reinstall_clicked = ui
                        .add_enabled_ui(reinstall_enabled, |ui| {
                            text_ui.button(
                                ui,
                                ("instance_reinstall_profile", instance_id),
                                "Reinstall Profile",
                                &reinstall_button_style,
                            )
                        })
                        .inner
                        .clicked();
                    if save_versions_clicked {
                        match save_instance_metadata_and_versions(state, instance_id, instances) {
                            Ok(()) => {
                                instances_changed = true;
                                if let Some(saved) = instances.find(instance_id) {
                                    tracing::info!(
                                        target: "vertexlauncher/ui/instance",
                                        instance_id = %instance_id,
                                        saved_modloader = %saved.modloader,
                                        saved_game_version = %saved.game_version,
                                        saved_modloader_version = %saved.modloader_version,
                                        "Saved instance metadata and versions."
                                    );
                                }
                                state.status_message =
                                    Some("Saved metadata and version settings.".to_owned());
                            }
                            Err(err) => {
                                tracing::warn!(
                                    target: "vertexlauncher/ui/instance",
                                    instance_id = %instance_id,
                                    error = %err,
                                    "Failed to save instance metadata and versions."
                                );
                                state.status_message = Some(err);
                            }
                        }
                    }
                    if reinstall_clicked {
                        match save_instance_metadata_and_versions(state, instance_id, instances) {
                            Ok(()) => {
                                instances_changed = true;
                                let game_version = state.game_version_input.trim().to_owned();
                                let modloader = selected_modloader_value(state);
                                if let Some(saved_instance) = instances.find(instance_id).cloned() {
                                    let modloader_version =
                                        normalize_optional(saved_instance.modloader_version.as_str());
                                    let installations_root =
                                        config.minecraft_installations_root_path().to_path_buf();
                                    let instance_root = instances::instance_root_path(
                                        &installations_root,
                                        &saved_instance,
                                    );
                                    let (linux_set_opengl_driver, linux_use_zink_driver) =
                                        effective_linux_graphics_settings_for_state(state, config);
                                    request_runtime_prepare(
                                        state,
                                        RuntimePrepareOperation::ReinstallProfile,
                                        instance_root,
                                        game_version.clone(),
                                        modloader.clone(),
                                        modloader_version,
                                        effective_required_java_major(
                                            config,
                                            game_version.as_str(),
                                        ),
                                        choose_java_executable(
                                            config,
                                            state.java_override_enabled,
                                            state.java_override_runtime_major,
                                            effective_required_java_major(
                                                config,
                                                game_version.as_str(),
                                            ),
                                        ),
                                        config.download_max_concurrent(),
                                        config.parsed_download_speed_limit_bps(),
                                        linux_set_opengl_driver,
                                        linux_use_zink_driver,
                                        config.default_instance_max_memory_mib(),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                    );
                                } else {
                                    state.status_message =
                                        Some("Instance was removed before reinstall.".to_owned());
                                }
                            }
                            Err(err) => {
                                state.status_message = Some(err);
                            }
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_heading", instance_id),
                        "Runtime Overrides",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_runtime_overrides_description", instance_id),
                        "Per-instance overrides for memory, JVM arguments, and Java runtime selection.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "Override max memory for this instance",
                        Some("When disabled, launcher instance default memory is used."),
                        &mut state.memory_override_enabled,
                    );
                    ui.add_space(6.0);

                    let (memory_slider_max, memory_slider_pending) = memory_slider_max_mib();
                    if memory_slider_pending {
                        ui.ctx().request_repaint_after(Duration::from_millis(50));
                    }
                    if state.memory_override_enabled {
                        let mut memory_mib = state
                            .memory_override_mib
                            .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max);
                        let response = settings_widgets::u128_slider_with_input_row(
                            text_ui,
                            ui,
                            ("instance_memory_override", instance_id),
                            "Max memory allocation (MiB)",
                            Some("Per-instance memory limit."),
                            &mut memory_mib,
                            INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
                            memory_slider_max,
                            INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
                        );
                        if response.changed() {
                            state.memory_override_mib = memory_mib;
                        }
                        ui.add_space(6.0);
                    }

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        ("instance_cli_args_override", instance_id),
                        "JVM args override (optional)",
                        Some("Leave blank to use launcher instance default JVM args."),
                        &mut state.cli_args_input,
                    );
                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "This modpack has a rich presence mod",
                        Some("When enabled, Vertex clears its own Discord Rich Presence for this instance after launch so the mod inside Minecraft can take over."),
                        &mut state.discord_rich_presence_mod_installed,
                    );
                    ui.add_space(8.0);

                    ui.add_space(8.0);

                    let _ = settings_widgets::toggle_row(
                        text_ui,
                        ui,
                        "Override Java runtime for this instance",
                        Some("When enabled, this instance will use the selected configured global Java path."),
                        &mut state.java_override_enabled,
                    );
                    ui.add_space(6.0);

                    let java_options = configured_java_path_options(config);
                    if state.java_override_enabled {
                        if java_options.is_empty() {
                            let _ = text_ui.label(
                                ui,
                                ("instance_java_override_no_options", instance_id),
                                "No configured global Java paths found. Add at least one Java path in Settings first.",
                                &style::error_text(ui),
                            );
                        } else {
                            if state.java_override_runtime_major.is_none_or(|major| {
                                !java_options.iter().any(|(m, _)| *m == major)
                            }) {
                                state.java_override_runtime_major =
                                    java_options.first().map(|(major, _)| *major);
                            }
                            let option_labels: Vec<&str> =
                                java_options.iter().map(|(_, label)| label.as_str()).collect();
                            let mut selected_index = java_options
                                .iter()
                                .position(|(major, _)| {
                                    Some(*major) == state.java_override_runtime_major
                                })
                                .unwrap_or(0);
                            if settings_widgets::full_width_dropdown_row(
                                text_ui,
                                ui,
                                ("instance_java_override_runtime", instance_id),
                                "Java path override",
                                Some("Select which configured Java path this instance should use."),
                                &mut selected_index,
                                &option_labels,
                            )
                            .changed()
                            {
                                state.java_override_runtime_major =
                                    java_options.get(selected_index).map(|(major, _)| *major);
                            }
                        }
                    }
                    ui.add_space(8.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_save_settings", instance_id),
                            "Save instance settings",
                            &action_button_style,
                        )
                        .clicked()
                    {
                        let java_override_runtime_major = if state.java_override_enabled {
                            if java_options.is_empty() {
                                state.status_message = Some(
                                    "Cannot save Java override: configure at least one global Java path in Settings."
                                        .to_owned(),
                                );
                                None
                            } else {
                                let selected = state.java_override_runtime_major.and_then(|major| {
                                    java_options.iter().find_map(|(candidate, _)| {
                                        (*candidate == major).then_some(major)
                                    })
                                });
                                selected.or_else(|| java_options.first().map(|(major, _)| *major))
                            }
                        } else {
                            None
                        };
                        if !state.java_override_enabled || java_override_runtime_major.is_some() {
                            let memory_override = if state.memory_override_enabled {
                                Some(
                                    state.memory_override_mib.clamp(
                                        INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN,
                                        memory_slider_max,
                                    ),
                                )
                            } else {
                                None
                            };
                            let cli_override = normalize_optional(state.cli_args_input.as_str());
                            let (linux_set_opengl_driver, linux_use_zink_driver) =
                                linux_instance_driver_settings_for_save(
                                    state,
                                    instances.find(instance_id),
                                );
                            match set_instance_settings(
                                instances,
                                instance_id,
                                memory_override,
                                cli_override,
                                state.java_override_enabled,
                                java_override_runtime_major,
                                linux_set_opengl_driver,
                                linux_use_zink_driver,
                                state.discord_rich_presence_mod_installed,
                            ) {
                                Ok(()) => {
                                    instances_changed = true;
                                    state.status_message =
                                        Some("Saved instance settings.".to_owned());
                                }
                                Err(err) => state.status_message = Some(err.to_string()),
                            }
                        }
                    }

                    render_platform_specific_instance_settings_section(
                        ui,
                        text_ui,
                        state,
                        instance_id,
                        &section_style,
                        &muted_style,
                    );

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(10.0);

                    let _ = text_ui.label(
                        ui,
                        ("instance_actions_heading", instance_id),
                        "Maintenance & Actions",
                        &section_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        ("instance_actions_description", instance_id),
                        "Open the instance folder and commit any metadata or runtime changes.",
                        &muted_style,
                    );
                    ui.add_space(8.0);

                    if text_ui
                        .button(
                            ui,
                            ("instance_open_folder", instance_id),
                            "Open Instance Folder",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        if let Some(instance) = instances.find(instance_id) {
                            let installations_root =
                                config.minecraft_installations_root_path().to_path_buf();
                            let instance_root =
                                instances::instance_root_path(&installations_root, instance);
                            match desktop::open_in_file_manager(instance_root.as_path()) {
                                Ok(()) => {
                                    state.status_message = Some(format!(
                                        "Opened instance folder: {}",
                                        instance_root.display()
                                    ));
                                }
                                Err(err) => {
                                    state.status_message =
                                        Some(format!("Failed to open instance folder: {err}"));
                                }
                            }
                        } else {
                            state.status_message =
                                Some("Instance was removed before opening its folder.".to_owned());
                        }
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_vtmpack", instance_id),
                            "Export .vtmpack...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        state.show_export_vtmpack_modal = true;
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_export_server_zip", instance_id),
                            "Auto-generate server zip...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        state.show_export_server_modal = true;
                    }
                    ui.add_space(6.0);
                    if text_ui
                        .button(
                            ui,
                            ("instance_move_instance", instance_id),
                            "Move Instance...",
                            &refresh_style,
                        )
                        .clicked()
                    {
                        if let Some(instance) = instances.find(instance_id) {
                            let installations_root =
                                config.minecraft_installations_root_path().to_path_buf();
                            let current_root =
                                instances::instance_root_path(&installations_root, instance);
                            state.move_instance_dest_input = current_root.display().to_string();
                        } else {
                            state.move_instance_dest_input = String::new();
                        }
                        state.move_instance_dest_valid = false;
                        state.move_instance_dest_error = None;
                        state.show_move_instance_modal = true;
                    }
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if text_ui
                            .button(
                                ui,
                                ("instance_settings_close", instance_id),
                                "Done",
                                &action_button_style,
                            )
                            .clicked()
                        {
                            close_requested = true;
                        }
                    });
                });
        },
    );

    if response.close_requested || close_requested {
        state.show_settings_modal = false;
    }
    instances_changed
}

fn memory_slider_max_mib() -> (u128, bool) {
    static CACHED: OnceLock<Mutex<MemorySliderMaxState>> = OnceLock::new();
    let cache = CACHED.get_or_init(|| Mutex::new(MemorySliderMaxState::default()));
    let mut total_mib = None;
    let mut pending = false;

    if let Ok(mut state) = cache.lock() {
        if !state.load_complete {
            if let Some(rx) = state.rx.as_ref() {
                match rx.try_recv() {
                    Ok(result) => {
                        state.detected_total_mib = result;
                        state.load_complete = true;
                        state.rx = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        pending = true;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        state.load_complete = true;
                        state.rx = None;
                    }
                }
            }

            if !state.load_complete && state.rx.is_none() {
                let (tx, rx) = mpsc::channel::<Option<u128>>();
                state.rx = Some(rx);
                pending = true;
                let _ = tokio_runtime::spawn_blocking_detached(move || {
                    let result = screen_platform::detect_total_memory_mib();
                    if let Err(err) = tx.send(result) {
                        tracing::error!(
                            target: "vertexlauncher/instance",
                            error = %err,
                            "Failed to deliver server-export memory probe result."
                        );
                    }
                });
            }
        }
        total_mib = state.detected_total_mib;
        pending |= !state.load_complete;
    }

    let max_mib = total_mib
        .unwrap_or(FALLBACK_TOTAL_MEMORY_MIB)
        .saturating_sub(RESERVED_SYSTEM_MEMORY_MIB)
        .max(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN);
    (max_mib, pending)
}
