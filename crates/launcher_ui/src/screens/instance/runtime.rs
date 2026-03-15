use super::*;

pub(super) fn render_install_feedback(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    local_progress: Option<&InstallProgress>,
    external_activity: Option<&install_activity::InstallActivitySnapshot>,
    runtime_prepare_in_flight: bool,
) {
    if let Some(progress) = local_progress
        && matches!(progress.stage, InstallStage::Complete)
        && !runtime_prepare_in_flight
        && external_activity.is_none_or(|activity| matches!(activity.stage, InstallStage::Complete))
    {
        return;
    }

    if let Some(progress) = local_progress {
        ui.add_space(8.0);
        let fraction = progress_fraction(progress);
        let progress_label = if let Some(eta) = progress.eta_seconds {
            format!(
                "{} · {:.1} MiB/s · ETA {}s",
                stage_label(progress.stage),
                progress.bytes_per_second / (1024.0 * 1024.0),
                eta
            )
        } else {
            format!(
                "{} · {:.1} MiB/s",
                stage_label(progress.stage),
                progress.bytes_per_second / (1024.0 * 1024.0)
            )
        };
        ui.add(egui::ProgressBar::new(fraction));
        let _ = text_ui.label(
            ui,
            ("instance_runtime_progress_label", instance_id),
            &format!("{progress_label} · {:.0}%", fraction * 100.0),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        let _ = text_ui.label(
            ui,
            ("instance_runtime_progress_message", instance_id),
            progress.message.as_str(),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        let bytes_line = if let Some(total) = progress.total_bytes {
            format!(
                "{} / {}",
                format_bytes(progress.downloaded_bytes),
                format_bytes(total)
            )
        } else {
            format!("{} downloaded", format_bytes(progress.downloaded_bytes))
        };
        let _ = text_ui.label(
            ui,
            ("instance_runtime_bytes", instance_id),
            &format!(
                "Files: {}/{} · {}",
                progress.downloaded_files, progress.total_files, bytes_line
            ),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    }

    if runtime_prepare_in_flight {
        ui.add_space(8.0);
        ui.add(egui::ProgressBar::new(0.0).animate(true));
        let _ = text_ui.label(
            ui,
            ("instance_runtime_progress_starting", instance_id),
            "Starting installation...",
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    }

    if let Some(activity) = external_activity {
        if matches!(activity.stage, InstallStage::Complete) {
            return;
        }
        ui.add_space(8.0);
        let fraction = progress_fraction_from_activity(activity);
        let progress_label = if let Some(eta) = activity.eta_seconds {
            format!(
                "{} · {:.1} MiB/s · ETA {}s",
                stage_label(activity.stage),
                activity.bytes_per_second / (1024.0 * 1024.0),
                eta
            )
        } else {
            format!(
                "{} · {:.1} MiB/s",
                stage_label(activity.stage),
                activity.bytes_per_second / (1024.0 * 1024.0)
            )
        };
        ui.add(egui::ProgressBar::new(fraction));
        let _ = text_ui.label(
            ui,
            ("instance_runtime_progress_label_external", instance_id),
            &format!("{progress_label} · {:.0}%", fraction * 100.0),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        let _ = text_ui.label(
            ui,
            ("instance_runtime_progress_message_external", instance_id),
            activity.message.as_str(),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        let bytes_line = if let Some(total) = activity.total_bytes {
            format!(
                "{} / {}",
                format_bytes(activity.downloaded_bytes),
                format_bytes(total)
            )
        } else {
            format!("{} downloaded", format_bytes(activity.downloaded_bytes))
        };
        let _ = text_ui.label(
            ui,
            ("instance_runtime_bytes_external", instance_id),
            &format!(
                "Files: {}/{} · {}",
                activity.downloaded_files, activity.total_files, bytes_line
            ),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
    }
}

pub(super) fn render_runtime_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    id: &str,
    instance_root: &Path,
    game_version: &str,
    config: &Config,
    external_install_active: bool,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    streamer_mode: bool,
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
) {
    let button_style = ButtonOptions {
        min_size: egui::vec2(120.0, 34.0),
        text_color: ui.visuals().widgets.active.fg_stroke.color,
        fill: ui.visuals().selection.bg_fill,
        fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
        fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().selection.stroke,
        ..ButtonOptions::default()
    };
    let mut muted_style = LabelOptions::default();
    muted_style.color = ui.visuals().weak_text_color();
    muted_style.wrap = false;
    let instance_root_key = normalize_path_key(instance_root);
    let launch_account = active_launch_auth
        .map(|auth| auth.account_key.clone())
        .or_else(|| {
            active_username
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
    let launch_display_name = active_launch_auth
        .map(|auth| auth.player_name.clone())
        .or_else(|| {
            active_username
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
    let launch_player_uuid = active_launch_auth.map(|auth| auth.player_uuid.clone());
    let launch_access_token = active_launch_auth.and_then(|auth| auth.access_token.clone());
    let launch_xuid = active_launch_auth.and_then(|auth| auth.xuid.clone());
    let launch_user_type = active_launch_auth.map(|auth| auth.user_type.clone());
    let runtime_running_for_active_account = launch_account
        .as_deref()
        .is_some_and(|account| is_instance_running_for_account(instance_root, account));
    let account_running_root = launch_account
        .as_deref()
        .and_then(running_instance_for_account);
    let launch_disabled_for_account = !runtime_running_for_active_account
        && account_running_root
            .as_deref()
            .is_some_and(|running_root| running_root != instance_root_key.as_str());
    let launch_disabled_for_missing_ownership =
        !runtime_running_for_active_account && !active_account_owns_minecraft;
    let launch_disabled = launch_disabled_for_account || launch_disabled_for_missing_ownership;

    let running_account_key = if runtime_running_for_active_account {
        launch_player_uuid
            .clone()
            .or_else(|| launch_account.clone())
            .or_else(|| state.launch_user_key.clone())
            .map(|value| value.to_ascii_lowercase())
    } else {
        None
    };
    let running_avatar_png = running_account_key
        .as_deref()
        .and_then(|key| account_avatars_by_key.get(key))
        .map(Vec::as_slice);
    let runtime_running = runtime_running_for_active_account;
    state.running = runtime_running;

    ui.horizontal(|ui| {
        if !state.runtime_prepare_in_flight && !external_install_active {
            let response = ui
                .add_enabled_ui(!launch_disabled, |ui| {
                    if runtime_running {
                        render_stop_runtime_button(ui, id, &button_style, running_avatar_png)
                    } else {
                        text_ui.button(ui, ("instance_runtime_toggle", id), "Launch", &button_style)
                    }
                })
                .inner;
            let toggle_requested = response.clicked();
            if toggle_requested {
                if runtime_running {
                    let stopped = launch_account.as_deref().is_some_and(|account| {
                        stop_running_instance_for_account(instance_root, account)
                    });
                    if stopped {
                        state.running = false;
                        state.status_message = Some("Stopped instance runtime.".to_owned());
                    } else {
                        state.running = false;
                        state.status_message = Some("Instance runtime was not running.".to_owned());
                    }
                } else if game_version.trim().is_empty() {
                    state.status_message =
                        Some("Cannot launch: choose a Minecraft game version first.".to_owned());
                } else {
                    let max_memory_mib = if state.memory_override_enabled {
                        state.memory_override_mib
                    } else {
                        config.default_instance_max_memory_mib()
                    };
                    let extra_jvm_args = normalize_optional(state.cli_args_input.as_str());
                    state.launch_username = launch_display_name
                        .as_deref()
                        .map(|value| {
                            privacy::redact_account_label(streamer_mode, value).into_owned()
                        })
                        .or_else(|| {
                            launch_account.as_deref().map(|value| {
                                privacy::redact_account_label(streamer_mode, value).into_owned()
                            })
                        });
                    state.launch_user_key = launch_player_uuid
                        .clone()
                        .or_else(|| launch_account.clone())
                        .or_else(|| launch_display_name.clone())
                        .and_then(|value| {
                            let trimmed = value.trim();
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed.to_owned())
                            }
                        });
                    let (linux_set_opengl_driver, linux_use_zink_driver) =
                        super::effective_linux_graphics_settings_for_state(state, config);
                    request_runtime_prepare(
                        state,
                        RuntimePrepareOperation::Launch,
                        instance_root.to_path_buf(),
                        game_version.trim().to_owned(),
                        selected_modloader_value(state),
                        normalize_optional(state.modloader_version_input.as_str()),
                        effective_required_java_major(config, game_version),
                        choose_java_executable(
                            config,
                            state.java_override_enabled,
                            state.java_override_runtime_major,
                            effective_required_java_major(config, game_version),
                        ),
                        config.download_max_concurrent(),
                        config.parsed_download_speed_limit_bps(),
                        linux_set_opengl_driver,
                        linux_use_zink_driver,
                        max_memory_mib,
                        extra_jvm_args,
                        state.launch_username.clone(),
                        launch_display_name.clone(),
                        launch_player_uuid.clone(),
                        launch_access_token.clone(),
                        launch_xuid.clone(),
                        launch_user_type.clone(),
                        launch_account.clone(),
                    );
                }
            }
            ui.add_space(10.0);
        }

        let _ = text_ui.label(
            ui,
            ("instance_runtime_state", id),
            if state.runtime_prepare_in_flight || external_install_active {
                "Installing"
            } else if state.running {
                "Running"
            } else {
                "Stopped"
            },
            &muted_style,
        );
        if state.runtime_prepare_in_flight || external_install_active {
            ui.add_space(8.0);
            ui.spinner();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let settings_button_id = format!("instance_settings_open_{id}");
            let settings_button = icon_button::svg(
                ui,
                settings_button_id.as_str(),
                assets::SETTINGS_SVG,
                "Open instance settings",
                state.show_settings_modal,
                30.0,
            );
            if settings_button.clicked() {
                state.show_settings_modal = true;
            }
            let _ = text_ui.label(
                ui,
                ("instance_settings_hint", id),
                "Open instance settings",
                &muted_style,
            );
        });
    });

    if launch_disabled_for_account {
        let blocked_account = launch_display_name
            .as_deref()
            .map(|value| privacy::redact_account_label(streamer_mode, value))
            .unwrap_or_else(|| "this account".into());
        let _ = text_ui.label(
            ui,
            ("instance_runtime_account_locked", id),
            &format!("{blocked_account} is already running another instance."),
            &muted_style,
        );
    }
    if launch_disabled_for_missing_ownership {
        let _ = text_ui.label(
            ui,
            ("instance_runtime_account_ownership", id),
            "Sign in with a Minecraft account that owns Minecraft to launch.",
            &muted_style,
        );
    }

    if state.running
        && !launch_account
            .as_deref()
            .is_some_and(|account| is_instance_running_for_account(instance_root, account))
    {
        state.running = false;
        state.status_message = Some("Minecraft process exited.".to_owned());
    }
}

pub(super) fn render_modloader_selector(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    id: &str,
    game_version: &str,
) {
    let style = ButtonOptions {
        min_size: egui::vec2(88.0, 30.0),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for (index, option) in MODLOADER_OPTIONS.iter().enumerate() {
            let unavailable_reason = if index == CUSTOM_MODLOADER_INDEX {
                None
            } else {
                state
                    .loader_support
                    .unavailable_reason(option, game_version)
            };
            let available = unavailable_reason.is_none();

            let mut button_style = style.clone();
            if !available {
                button_style.text_color = ui.visuals().weak_text_color();
                button_style.fill = ui.visuals().widgets.noninteractive.bg_fill;
                button_style.fill_hovered = ui.visuals().widgets.noninteractive.bg_fill;
                button_style.fill_active = ui.visuals().widgets.noninteractive.bg_fill;
                button_style.fill_selected = ui.visuals().widgets.noninteractive.bg_fill;
            }

            let response = text_ui.selectable_button(
                ui,
                ("instance_modloader_option", id, index),
                option,
                state.selected_modloader == index,
                &button_style,
            );
            if let Some(reason) = unavailable_reason.as_deref() {
                let tooltip_options = TooltipOptions::default();
                text_ui.tooltip_for_response(
                    ui,
                    ("instance_modloader_unavailable_tooltip", id, index),
                    &response,
                    reason,
                    &tooltip_options,
                );
            }

            if available && response.clicked() {
                state.selected_modloader = index;
            }
        }
    });
}

pub(super) fn render_stop_runtime_button(
    ui: &mut Ui,
    id: &str,
    style: &ButtonOptions,
    avatar_png: Option<&[u8]>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(style.min_size, egui::Sense::click());
    let error_color = ui.visuals().error_fg_color;
    let fill_base = egui::Color32::from_rgba_premultiplied(
        error_color.r(),
        error_color.g(),
        error_color.b(),
        36,
    );
    let fill = if response.is_pointer_button_down_on() {
        fill_base.gamma_multiply(0.85)
    } else if response.hovered() {
        fill_base.gamma_multiply(1.25)
    } else {
        fill_base
    };
    let stroke = egui::Stroke::new(style.stroke.width.max(1.0), error_color);

    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(style.corner_radius), fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(style.corner_radius),
        stroke,
        egui::StrokeKind::Inside,
    );

    let inner_rect = rect.shrink2(style.padding);
    let avatar_size = (inner_rect.height() - 2.0).clamp(12.0, 20.0);
    let avatar_rect =
        egui::Rect::from_min_size(inner_rect.min, egui::vec2(avatar_size, avatar_size));
    render_runtime_avatar(ui, id, avatar_rect, avatar_png, error_color);

    let icon_lane = egui::Rect::from_min_max(
        egui::pos2(
            (avatar_rect.max.x + 8.0).min(inner_rect.max.x),
            inner_rect.min.y,
        ),
        inner_rect.max,
    );
    let icon_size = (icon_lane.height() - 4.0).clamp(12.0, 18.0);
    let stop_icon_rect =
        egui::Rect::from_center_size(icon_lane.center(), egui::vec2(icon_size, icon_size));
    let stop_icon_color = egui::Color32::WHITE;
    let stop_icon = egui::Image::from_bytes(
        format!(
            "bytes://instance/runtime-stop/{id}-{:02x}{:02x}{:02x}.svg",
            stop_icon_color.r(),
            stop_icon_color.g(),
            stop_icon_color.b()
        ),
        apply_color_to_svg(assets::STOP_SVG, stop_icon_color),
    )
    .fit_to_exact_size(egui::vec2(icon_size, icon_size));
    let _ = ui.put(stop_icon_rect, stop_icon);

    response
}

pub(super) fn render_runtime_avatar(
    ui: &mut Ui,
    id: &str,
    rect: egui::Rect,
    avatar_png: Option<&[u8]>,
    color: egui::Color32,
) {
    if let Some(bytes) = avatar_png {
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        bytes.hash(&mut hasher);
        let image = egui::Image::from_bytes(
            format!("bytes://instance/runtime-avatar/{}", hasher.finish()),
            bytes.to_vec(),
        )
        .fit_to_exact_size(rect.size());
        let _ = ui.put(rect, image);
        return;
    }

    let fallback = egui::Image::from_bytes(
        format!("bytes://instance/runtime-avatar-fallback/{id}.svg"),
        apply_color_to_svg(assets::USER_SVG, color),
    )
    .fit_to_exact_size(rect.size());
    let _ = ui.put(rect, fallback);
}

pub(super) fn split_modloader(modloader: &str) -> (usize, String) {
    for (index, option) in MODLOADER_OPTIONS.iter().enumerate() {
        if option.eq_ignore_ascii_case(modloader.trim()) {
            return (index, String::new());
        }
    }

    (CUSTOM_MODLOADER_INDEX, modloader.trim().to_owned())
}

pub(super) fn selected_modloader_value(state: &InstanceScreenState) -> String {
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        state.custom_modloader.trim().to_owned()
    } else {
        MODLOADER_OPTIONS
            .get(state.selected_modloader)
            .copied()
            .unwrap_or(MODLOADER_OPTIONS[0])
            .to_owned()
    }
}

pub(super) fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

pub(super) fn save_instance_metadata_and_versions(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
) -> Result<(), String> {
    let trimmed_name = state.name_input.trim();
    if trimmed_name.is_empty() {
        return Err("Name cannot be empty.".to_owned());
    }

    let modloader = selected_modloader_value(state);
    let game_version = state.game_version_input.trim().to_owned();
    if game_version.is_empty() {
        return Err("Minecraft game version cannot be empty.".to_owned());
    }
    if modloader.trim().is_empty() {
        return Err("Modloader cannot be empty.".to_owned());
    }
    let resolved_modloader_version =
        resolve_modloader_version_for_settings(state, modloader.as_str(), game_version.as_str())?;

    tracing::info!(
        target: "vertexlauncher/ui/instance",
        instance_id = %instance_id,
        requested_modloader = %modloader,
        requested_game_version = %game_version,
        requested_modloader_version = %resolved_modloader_version,
        "Saving instance metadata and versions from settings modal."
    );

    if let Some(instance) = instances.find_mut(instance_id) {
        instance.name = trimmed_name.to_owned();
        instance.description = normalize_optional(state.description_input.as_str());
        instance.thumbnail_path = normalize_optional(state.thumbnail_input.as_str());
    } else {
        return Err("Instance was removed before save.".to_owned());
    }

    set_instance_versions(
        instances,
        instance_id,
        modloader,
        game_version,
        resolved_modloader_version,
    )
    .map_err(|err| err.to_string())
}

pub(super) fn poll_background_tasks(
    state: &mut InstanceScreenState,
    config: &mut Config,
    instances: &mut InstanceStore,
    instance_id: &str,
) {
    poll_version_catalog(state);
    poll_modloader_versions(state);
    poll_content_lookup_results(state);
    poll_runtime_progress(state);
    poll_runtime_prepare(state, config, instances, instance_id);
}

pub(super) fn sync_version_catalog(
    state: &mut InstanceScreenState,
    include_snapshots_and_betas: bool,
    force_refresh: bool,
) {
    let should_refresh = force_refresh
        || state.version_catalog_include_snapshots != Some(include_snapshots_and_betas)
        || (state.available_game_versions.is_empty() && state.version_catalog_error.is_none());
    if !should_refresh || state.version_catalog_in_flight {
        return;
    }

    ensure_version_catalog_channel(state);
    let Some(tx) = state.version_catalog_results_tx.as_ref().cloned() else {
        return;
    };

    state.version_catalog_in_flight = true;
    state.version_catalog_error = None;
    state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);

    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            fetch_version_catalog_with_refresh(include_snapshots_and_betas, force_refresh)
                .map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| format!("version catalog task join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send((include_snapshots_and_betas, result));
    });
}

pub(super) fn ensure_version_catalog_channel(state: &mut InstanceScreenState) {
    if state.version_catalog_results_tx.is_some() && state.version_catalog_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(bool, Result<VersionCatalog, String>)>();
    state.version_catalog_results_tx = Some(tx);
    state.version_catalog_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn apply_version_catalog(
    state: &mut InstanceScreenState,
    include_snapshots_and_betas: bool,
    catalog: VersionCatalog,
) {
    state.available_game_versions = catalog.game_versions;
    state.loader_support = catalog.loader_support;
    state.loader_versions = catalog.loader_versions;
    state.version_catalog_error = None;
    state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);

    if state.available_game_versions.is_empty() {
        state.selected_game_version_index = 0;
        state.game_version_input.clear();
        return;
    }

    let preferred_index = if state.game_version_input.trim().is_empty() {
        0
    } else {
        state
            .available_game_versions
            .iter()
            .position(|entry| entry.id == state.game_version_input)
            .unwrap_or(0)
    };
    state.selected_game_version_index = preferred_index;
    if let Some(selected) = state.available_game_versions.get(preferred_index) {
        state.game_version_input = selected.id.clone();
    }
}

pub(super) fn apply_version_catalog_error(
    state: &mut InstanceScreenState,
    include_snapshots_and_betas: bool,
    error: &str,
) {
    state.version_catalog_error = Some(format!("Failed to fetch version catalog: {error}"));
    state.available_game_versions.clear();
    state.loader_support = LoaderSupportIndex::default();
    state.loader_versions = LoaderVersionIndex::default();
    state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);
    state.selected_game_version_index = 0;
    state.game_version_input.clear();
}

pub(super) fn poll_version_catalog(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.version_catalog_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => should_reset_channel = true,
        }
    }

    if should_reset_channel {
        state.version_catalog_results_tx = None;
        state.version_catalog_results_rx = None;
        state.version_catalog_in_flight = false;
    }

    for (include_snapshots_and_betas, result) in updates {
        state.version_catalog_in_flight = false;
        match result {
            Ok(catalog) => apply_version_catalog(state, include_snapshots_and_betas, catalog),
            Err(err) => apply_version_catalog_error(state, include_snapshots_and_betas, &err),
        }
    }
}

pub(super) fn selected_modloader_versions<'a>(
    state: &'a InstanceScreenState,
    game_version: &str,
) -> &'a [String] {
    if game_version.trim().is_empty() {
        return &[];
    }
    let selected_label = MODLOADER_OPTIONS
        .get(state.selected_modloader)
        .copied()
        .unwrap_or(MODLOADER_OPTIONS[0]);
    state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
        .unwrap_or(&[])
}

pub(super) fn modloader_versions_cache_key(loader_label: &str, game_version: &str) -> String {
    format!(
        "{}|{}",
        loader_label.trim().to_ascii_lowercase(),
        game_version.trim()
    )
}

pub(super) fn ensure_modloader_versions_channel(state: &mut InstanceScreenState) {
    if state.modloader_versions_results_tx.is_some()
        && state.modloader_versions_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Result<Vec<String>, String>)>();
    state.modloader_versions_results_tx = Some(tx);
    state.modloader_versions_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn request_modloader_versions(
    state: &mut InstanceScreenState,
    loader_label: &str,
    game_version: &str,
    force_refresh: bool,
) {
    let loader_label = loader_label.trim();
    let game_version = game_version.trim();
    if loader_label.is_empty() || game_version.is_empty() {
        return;
    }

    let key = modloader_versions_cache_key(loader_label, game_version);
    if force_refresh {
        state.modloader_versions_cache.remove(&key);
    } else if state.modloader_versions_cache.contains_key(&key)
        || state.modloader_versions_in_flight.contains(&key)
    {
        return;
    }

    ensure_modloader_versions_channel(state);
    let Some(tx) = state.modloader_versions_results_tx.as_ref().cloned() else {
        return;
    };

    state.modloader_versions_in_flight.insert(key.clone());
    state.modloader_versions_status_key = Some(key.clone());
    state.modloader_versions_status = Some(format!(
        "Fetching {loader_label} versions for Minecraft {game_version}..."
    ));

    let loader = loader_label.to_owned();
    let game = game_version.to_owned();
    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            fetch_loader_versions_for_game(loader.as_str(), game.as_str(), force_refresh)
                .map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| format!("background task join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send((key, result));
    });
}

pub(super) fn poll_modloader_versions(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.modloader_versions_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => should_reset_channel = true,
        }
    }

    if should_reset_channel {
        state.modloader_versions_results_tx = None;
        state.modloader_versions_results_rx = None;
        state.modloader_versions_in_flight.clear();
    }

    for (key, result) in updates {
        state.modloader_versions_in_flight.remove(&key);
        state.modloader_versions_status_key = Some(key.clone());
        match result {
            Ok(versions) => {
                state.modloader_versions_cache.insert(key, versions.clone());
                state.modloader_versions_status = if versions.is_empty() {
                    Some("No modloader versions found for this Minecraft version.".to_owned())
                } else {
                    Some(format!("Loaded {} modloader versions.", versions.len()))
                };
            }
            Err(err) => {
                state.modloader_versions_cache.insert(key, Vec::new());
                state.modloader_versions_status =
                    Some(format!("Failed to fetch modloader versions: {err}"));
            }
        }
    }
}

pub(super) fn is_latest_modloader_version_alias(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "latest" | "latest available" | "use latest version" | "auto" | "default"
    )
}

pub(super) fn resolve_latest_modloader_version_from_state(
    state: &InstanceScreenState,
    modloader_label: &str,
    game_version: &str,
) -> Option<String> {
    if game_version.trim().is_empty() {
        return None;
    }

    if let Some(version) = state
        .loader_versions
        .versions_for_loader(modloader_label, game_version)
        .and_then(|versions| versions.first())
    {
        return Some(version.clone());
    }

    let key = modloader_versions_cache_key(modloader_label, game_version);
    state
        .modloader_versions_cache
        .get(&key)
        .and_then(|versions| versions.first().cloned())
}

pub(super) fn resolve_modloader_version_for_settings(
    state: &InstanceScreenState,
    modloader_label: &str,
    game_version: &str,
) -> Result<String, String> {
    let raw_modloader_version = state.modloader_version_input.trim();
    let normalized_loader = modloader_label.trim().to_ascii_lowercase();
    let catalog_loader = matches!(
        normalized_loader.as_str(),
        "fabric" | "forge" | "neoforge" | "quilt"
    );

    if !catalog_loader {
        return Ok(raw_modloader_version.to_owned());
    }

    if raw_modloader_version.is_empty() || is_latest_modloader_version_alias(raw_modloader_version)
    {
        return resolve_latest_modloader_version_from_state(state, modloader_label, game_version)
            .ok_or_else(|| {
                format!(
                    "Could not resolve latest {modloader_label} version for Minecraft {game_version}. Refresh modloader versions and try again."
                )
            });
    }

    let matches_catalog = state
        .loader_versions
        .versions_for_loader(modloader_label, game_version)
        .is_some_and(|versions| {
            versions
                .iter()
                .any(|version| version.eq_ignore_ascii_case(raw_modloader_version))
        });
    let matches_cache = state
        .modloader_versions_cache
        .get(&modloader_versions_cache_key(modloader_label, game_version))
        .is_some_and(|versions| {
            versions
                .iter()
                .any(|version| version.eq_ignore_ascii_case(raw_modloader_version))
        });
    if matches_catalog || matches_cache {
        Ok(raw_modloader_version.to_owned())
    } else {
        Err(format!(
            "{modloader_label} {raw_modloader_version} is not available for Minecraft {game_version}."
        ))
    }
}

pub(super) fn ensure_runtime_prepare_channel(state: &mut InstanceScreenState) {
    if state.runtime_prepare_results_tx.is_some() && state.runtime_prepare_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, String, Result<RuntimePrepareOutcome, String>)>();
    state.runtime_prepare_results_tx = Some(tx);
    state.runtime_prepare_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn ensure_runtime_progress_channel(state: &mut InstanceScreenState) {
    if state.runtime_progress_tx.is_some() && state.runtime_progress_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<InstallProgress>();
    state.runtime_progress_tx = Some(tx);
    state.runtime_progress_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn reinstall_instance_profile_files(instance_root: &Path) -> Result<(), std::io::Error> {
    const REINSTALL_PATHS: [&str; 5] = ["versions", "libraries", "assets", "natives", "loaders"];
    for relative in REINSTALL_PATHS {
        let path = instance_root.join(relative);
        match std::fs::remove_dir_all(path.as_path()) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

pub(super) fn request_runtime_prepare(
    state: &mut InstanceScreenState,
    operation: RuntimePrepareOperation,
    instance_root: PathBuf,
    game_version: String,
    modloader: String,
    modloader_version: Option<String>,
    required_java_major: Option<u8>,
    java_executable: Option<String>,
    download_max_concurrent: u32,
    download_speed_limit_bps: Option<u64>,
    linux_set_opengl_driver: bool,
    linux_use_zink_driver: bool,
    max_memory_mib: u128,
    extra_jvm_args: Option<String>,
    visible_username: Option<String>,
    player_name: Option<String>,
    player_uuid: Option<String>,
    access_token: Option<String>,
    xuid: Option<String>,
    user_type: Option<String>,
    launch_account_name: Option<String>,
) {
    let game_version = game_version.trim().to_owned();
    if game_version.is_empty() || state.runtime_prepare_in_flight {
        return;
    }

    ensure_runtime_prepare_channel(state);
    ensure_runtime_progress_channel(state);
    let Some(tx) = state.runtime_prepare_results_tx.as_ref().cloned() else {
        return;
    };
    let Some(progress_tx) = state.runtime_progress_tx.as_ref().cloned() else {
        return;
    };

    state.runtime_prepare_in_flight = true;
    state.runtime_latest_progress = None;
    if operation == RuntimePrepareOperation::ReinstallProfile {
        state.launch_username = None;
        state.launch_user_key = None;
    }
    state.status_message = Some(match operation {
        RuntimePrepareOperation::Launch => format!("Preparing Minecraft {game_version}..."),
        RuntimePrepareOperation::ReinstallProfile => {
            format!("Reinstalling Minecraft {game_version} profile...")
        }
    });
    let instance_root_display = display_user_path(instance_root.as_path());
    state.runtime_prepare_instance_root = Some(instance_root_display.clone());
    let game_version_for_task = game_version.clone();
    let game_version_for_result = game_version.clone();
    let modloader_for_task = modloader.trim().to_owned();
    let modloader_version_for_task = modloader_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let java_executable_for_task = java_executable;
    let extra_jvm_args_for_task = extra_jvm_args;
    let visible_username_for_task = visible_username;
    let player_name_for_task = player_name;
    let player_uuid_for_task = player_uuid;
    let access_token_for_task = access_token;
    let xuid_for_task = xuid;
    let user_type_for_task = user_type;
    let launch_account_name_for_task = launch_account_name;
    let tab_user_key = player_uuid_for_task
        .as_deref()
        .or(launch_account_name_for_task.as_deref())
        .or(player_name_for_task.as_deref())
        .and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        });
    state.runtime_prepare_user_key = tab_user_key.clone();
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: download_max_concurrent.max(1),
        max_download_bps: download_speed_limit_bps,
    };
    let modloader_version_display = modloader_version_for_task
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" {value}"))
        .unwrap_or_default();
    let java_launch_mode = if let Some(path) = java_executable_for_task
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        format!("configured Java at {path}")
    } else if let Some(runtime_major) = required_java_major {
        format!("auto-provisioned OpenJDK {runtime_major}")
    } else {
        "java from PATH".to_owned()
    };
    let username = visible_username_for_task
        .as_deref()
        .or(player_name_for_task.as_deref())
        .or(launch_account_name_for_task.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Player");
    let tab_id = console::ensure_instance_tab(
        state.name_input.as_str(),
        username,
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
        match operation {
            RuntimePrepareOperation::Launch => format!(
                "Launch request: root={} | Minecraft {} | {}{} | max memory={} MiB | {}",
                instance_root_display,
                game_version_for_task,
                modloader_for_task,
                modloader_version_display,
                max_memory_mib.max(512),
                java_launch_mode
            ),
            RuntimePrepareOperation::ReinstallProfile => format!(
                "Reinstall request: root={} | Minecraft {} | {}{} | {}",
                instance_root_display,
                game_version_for_task,
                modloader_for_task,
                modloader_version_display,
                java_launch_mode
            ),
        },
    );
    let instance_id_for_notifications = state.name_input.clone();
    let _ = tokio_runtime::spawn(async move {
        let progress_tx_done = progress_tx.clone();
        let progress_callback: InstallProgressCallback = Arc::new(move |event| {
            let _ = progress_tx.send(event);
        });
        let result = tokio_runtime::spawn_blocking(move || {
            let mut configured_java = None;
            let java_path = if let Some(path) = java_executable_for_task
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
            if operation == RuntimePrepareOperation::ReinstallProfile {
                reinstall_instance_profile_files(instance_root.as_path()).map_err(|err| {
                    format!("failed to clear install artifacts before reinstall: {err}")
                })?;
            }
            let setup = ensure_game_files(
                instance_root.as_path(),
                game_version_for_task.as_str(),
                modloader_for_task.as_str(),
                modloader_version_for_task.as_deref(),
                Some(java_path.as_str()),
                &download_policy,
                Some(progress_callback),
            )
            .map_err(|err| err.to_string())?;
            let launch = if operation == RuntimePrepareOperation::Launch {
                let launch_request = LaunchRequest {
                    instance_root: instance_root.clone(),
                    game_version: game_version_for_task.clone(),
                    modloader: modloader_for_task.clone(),
                    modloader_version: modloader_version_for_task.clone(),
                    account_key: launch_account_name_for_task.clone(),
                    java_executable: Some(java_path.clone()),
                    max_memory_mib,
                    extra_jvm_args: extra_jvm_args_for_task.clone(),
                    player_name: player_name_for_task
                        .clone()
                        .or(launch_account_name_for_task.clone()),
                    player_uuid: player_uuid_for_task.clone(),
                    auth_access_token: access_token_for_task.clone(),
                    auth_xuid: xuid_for_task.clone(),
                    auth_user_type: user_type_for_task.clone(),
                    quick_play_singleplayer: None,
                    quick_play_multiplayer: None,
                    linux_set_opengl_driver,
                    linux_use_zink_driver,
                };
                Some(launch_instance(&launch_request).map_err(|err| err.to_string())?)
            } else {
                None
            };
            Ok(RuntimePrepareOutcome {
                operation,
                setup,
                configured_java,
                launch,
            })
        })
        .await
        .map_err(|err| format!("runtime prepare task join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send((game_version_for_result, instance_root_display, result));
        let _ = progress_tx_done.send(InstallProgress {
            stage: InstallStage::Complete,
            message: format!("Install task ended for {instance_id_for_notifications}."),
            downloaded_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            bytes_per_second: 0.0,
            eta_seconds: Some(0),
        });
    });
}

pub(super) fn poll_runtime_progress(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.runtime_progress_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => should_reset_channel = true,
        }
    }

    if should_reset_channel {
        state.runtime_progress_tx = None;
        state.runtime_progress_rx = None;
    }

    for progress in updates {
        state.runtime_latest_progress = Some(progress.clone());
        install_activity::set_progress(state.name_input.as_str(), &progress);
        if should_emit_progress_notification(state, &progress) {
            let source = format!("installation/{}", state.name_input);
            let fraction = progress_fraction(&progress);
            notification::progress!(
                notification::Severity::Info,
                source,
                fraction,
                "{} · {:.1} MiB/s{}",
                stage_label(progress.stage),
                progress.bytes_per_second / (1024.0 * 1024.0),
                progress
                    .eta_seconds
                    .map(|eta| format!(" · ETA {}s", eta))
                    .unwrap_or_default()
            );
            state.runtime_last_notification_at = Some(Instant::now());
        }
    }
}

pub(super) fn poll_runtime_prepare(
    state: &mut InstanceScreenState,
    config: &mut Config,
    instances: &mut InstanceStore,
    instance_id: &str,
) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.runtime_prepare_results_rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => should_reset_channel = true,
        }
    }

    if should_reset_channel {
        let prepare_user_key = state.runtime_prepare_user_key.take();
        if let Some(root) = state.runtime_prepare_instance_root.take() {
            console::set_instance_tab_loading(root.as_str(), prepare_user_key.as_deref(), false);
        }
        state.runtime_prepare_results_tx = None;
        state.runtime_prepare_results_rx = None;
        state.runtime_prepare_in_flight = false;
        state.runtime_progress_tx = None;
        state.runtime_progress_rx = None;
    }

    for (game_version, instance_root_display, result) in updates {
        let prepare_user_key = state.runtime_prepare_user_key.take();
        state.runtime_prepare_instance_root = None;
        console::set_instance_tab_loading(
            instance_root_display.as_str(),
            prepare_user_key.as_deref(),
            false,
        );
        state.runtime_prepare_in_flight = false;
        match result {
            Ok(outcome) => {
                let operation = outcome.operation;
                if let Some((runtime_major, path)) = outcome.configured_java
                    && let Some(runtime) = java_runtime_from_major(runtime_major)
                {
                    config.set_java_runtime_path(runtime, Some(path));
                }
                let setup = outcome.setup;
                if let Some(launch) = outcome.launch {
                    let _ = record_instance_launch_usage(instances, instance_id);
                    let username = state
                        .launch_username
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or("Player");
                    let tab_id = console::ensure_instance_tab(
                        state.name_input.as_str(),
                        username,
                        instance_root_display.as_str(),
                        state.launch_user_key.as_deref(),
                    );
                    console::attach_launch_log(
                        tab_id.as_str(),
                        instance_root_display.as_str(),
                        launch.launch_log_path.as_path(),
                    );
                    console::push_line_to_tab(
                        tab_id.as_str(),
                        format!(
                            "Launched Minecraft {} (pid {}, profile {}).",
                            game_version, launch.pid, launch.profile_id
                        ),
                    );
                    state.running = true;
                    install_activity::clear_instance(state.name_input.as_str());
                    state.status_message = Some(format!(
                        "Launched Minecraft {} in {} (pid {}, profile {}, {} file(s) downloaded, loader: {}).",
                        game_version,
                        instance_root_display,
                        launch.pid,
                        launch.profile_id,
                        setup.downloaded_files,
                        setup.resolved_modloader_version.as_deref().unwrap_or("n/a")
                    ));
                    notification::progress!(
                        notification::Severity::Info,
                        format!("installation/{}", state.name_input),
                        1.0f32,
                        "Launched Minecraft {} (pid {}, {} files).",
                        game_version,
                        launch.pid,
                        setup.downloaded_files
                    );
                } else {
                    state.running = false;
                    install_activity::clear_instance(state.name_input.as_str());
                    let source = format!("installation/{}", state.name_input);
                    match operation {
                        RuntimePrepareOperation::ReinstallProfile => {
                            state.status_message = Some(format!(
                                "Reinstalled Minecraft {} in {} ({} file(s) downloaded, loader: {}).",
                                game_version,
                                instance_root_display,
                                setup.downloaded_files,
                                setup.resolved_modloader_version.as_deref().unwrap_or("n/a")
                            ));
                            notification::progress!(
                                notification::Severity::Info,
                                source,
                                1.0f32,
                                "Reinstalled Minecraft {} ({} files).",
                                game_version,
                                setup.downloaded_files
                            );
                        }
                        RuntimePrepareOperation::Launch => {
                            state.status_message = Some(format!(
                                "Installed Minecraft {} in {} ({} file(s) downloaded).",
                                game_version, instance_root_display, setup.downloaded_files
                            ));
                            notification::progress!(
                                notification::Severity::Info,
                                source,
                                1.0f32,
                                "Installed Minecraft {} ({} files).",
                                game_version,
                                setup.downloaded_files
                            );
                        }
                    }
                }
            }
            Err(err) => {
                state.running = false;
                install_activity::clear_instance(state.name_input.as_str());
                state.status_message = Some(format!("Failed to prepare game files: {err}"));
                notification::error!(
                    format!("installation/{}", state.name_input),
                    "{} installation failed: {}",
                    state.name_input,
                    err
                );
            }
        }
    }
}

pub(super) fn should_emit_progress_notification(
    state: &InstanceScreenState,
    _progress: &InstallProgress,
) -> bool {
    match state.runtime_last_notification_at {
        Some(last) => last.elapsed() >= Duration::from_millis(250),
        None => true,
    }
}

pub(super) fn progress_fraction(progress: &InstallProgress) -> f32 {
    if let Some(total_bytes) = progress.total_bytes
        && total_bytes > 0
    {
        return (progress.downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
    }
    if progress.total_files > 0 {
        return (progress.downloaded_files as f32 / progress.total_files as f32).clamp(0.0, 1.0);
    }
    if matches!(progress.stage, InstallStage::Complete) {
        1.0
    } else {
        0.0
    }
}

pub(super) fn progress_fraction_from_activity(
    progress: &install_activity::InstallActivitySnapshot,
) -> f32 {
    if let Some(total_bytes) = progress.total_bytes
        && total_bytes > 0
    {
        return (progress.downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
    }
    if progress.total_files > 0 {
        return (progress.downloaded_files as f32 / progress.total_files as f32).clamp(0.0, 1.0);
    }
    if matches!(progress.stage, InstallStage::Complete) {
        1.0
    } else {
        0.0
    }
}

pub(super) fn stage_label(stage: InstallStage) -> &'static str {
    match stage {
        InstallStage::PreparingFolders => "Preparing folders",
        InstallStage::ResolvingMetadata => "Resolving metadata",
        InstallStage::DownloadingCore => "Downloading core files",
        InstallStage::InstallingModloader => "Installing modloader",
        InstallStage::Complete => "Complete",
    }
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.2} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.2} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub(super) fn selected_game_version(state: &InstanceScreenState) -> &str {
    state
        .available_game_versions
        .get(state.selected_game_version_index)
        .map(|entry| entry.id.as_str())
        .unwrap_or_else(|| state.game_version_input.as_str())
}

pub(super) fn choose_java_executable(
    config: &Config,
    java_override_enabled: bool,
    java_override_runtime_major: Option<u8>,
    required_java_major: Option<u8>,
) -> Option<String> {
    if java_override_enabled
        && let Some(override_major) = java_override_runtime_major
        && let Some(runtime) = java_runtime_from_major(override_major)
        && let Some(path) = config.java_runtime_path(runtime)
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() && Path::new(trimmed).exists() {
            return Some(trimmed.to_owned());
        }
    }

    if let Some(runtime_major) = required_java_major
        && let Some(runtime) = java_runtime_from_major(runtime_major)
        && let Some(path) = config.java_runtime_path(runtime)
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() && Path::new(trimmed).exists() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

pub(super) fn required_java_major(game_version: &str) -> Option<u8> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        return Some(21);
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

pub(super) fn effective_required_java_major(config: &Config, game_version: &str) -> Option<u8> {
    let required = required_java_major(game_version)?;
    if config.force_java_21_minimum() && required < 21 {
        Some(21)
    } else {
        Some(required)
    }
}

pub(super) fn java_runtime_from_major(major: u8) -> Option<JavaRuntimeVersion> {
    match major {
        8 => Some(JavaRuntimeVersion::Java8),
        16 => Some(JavaRuntimeVersion::Java16),
        17 => Some(JavaRuntimeVersion::Java17),
        21 => Some(JavaRuntimeVersion::Java21),
        _ => None,
    }
}

pub(super) fn configured_java_path_options(config: &Config) -> Vec<(u8, String)> {
    let mut options = Vec::new();
    for runtime in JavaRuntimeVersion::ALL {
        let Some(path) = config.java_runtime_path(runtime) else {
            continue;
        };
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        options.push((
            runtime.major(),
            format!("Java {} ({trimmed})", runtime.major()),
        ));
    }
    options
}
