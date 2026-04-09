use super::*;

#[path = "discover_detail/discover_version_entry.rs"]
mod discover_version_entry;
#[path = "discover_detail/discover_versions_result.rs"]
mod discover_versions_result;

pub(super) use self::discover_version_entry::DiscoverVersionEntry;
pub(super) use self::discover_versions_result::DiscoverVersionsResult;

pub(super) fn render_discover_detail_content(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut DiscoverState,
) -> DiscoverOutput {
    let mut output = DiscoverOutput::default();
    poll_detail_versions(state);
    request_detail_versions(state);
    if state.detail_versions_in_flight || state.install_in_flight {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    let Some(entry) = state.detail_entry.clone() else {
        let _ = text_ui.label(
            ui,
            "discover_detail_missing",
            "No modpack selected.",
            &style::muted(ui),
        );
        return output;
    };

    let muted_style = style::muted(ui);
    let heading_style = LabelOptions {
        wrap: true,
        ..style::subtitle(ui)
    };
    let body_style = style::body(ui);
    let selected_source = selected_detail_source(state, &entry);
    let previous_selected_source = state.detail_selected_source;

    ui.horizontal(|ui| {
        if text_ui
            .button(
                ui,
                "discover_detail_back",
                "Back to Discover",
                &style::neutral_button(ui),
            )
            .clicked()
        {
            output.requested_screen = Some(AppScreen::Discover);
        }

        if entry.provider_refs.len() > 1 {
            ui.add_space(style::SPACE_SM);
            sized_combo_box(
                ui,
                "discover_detail_source",
                180.0,
                selected_source.label(),
                |ui| {
                    for provider in &entry.provider_refs {
                        ui.selectable_value(
                            &mut state.detail_selected_source,
                            Some(provider.source),
                            provider.source.label(),
                        );
                    }
                },
            );
        }
    });
    if state.detail_selected_source != previous_selected_source {
        state.detail_versions.clear();
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
    }
    ui.add_space(style::SPACE_MD);

    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
        .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
        .show(ui, |ui| {
            let preview_height = 180.0;
            if let Some(icon_url) = entry.icon_url.as_deref() {
                remote_tiled_image::show(
                    ui,
                    icon_url,
                    egui::vec2(ui.available_width(), preview_height),
                    ("discover_detail_image", entry.dedupe_key.as_str()),
                    assets::DISCOVER_SVG,
                );
            }
            ui.add_space(style::SPACE_MD);
            let _ = text_ui.label(
                ui,
                ("discover_detail_title", entry.dedupe_key.as_str()),
                entry.name.as_str(),
                &heading_style,
            );
            if let Some(author) = entry.author.as_deref() {
                let _ = text_ui.label(
                    ui,
                    ("discover_detail_author", entry.dedupe_key.as_str()),
                    &format!("by {author}"),
                    &muted_style,
                );
            }
            ui.add_space(style::SPACE_SM);
            let _ = text_ui.label(
                ui,
                ("discover_detail_summary", entry.dedupe_key.as_str()),
                entry.summary.as_str(),
                &body_style,
            );
            if let Some(url) = selected_detail_provider_ref(&entry, selected_source)
                .and_then(|provider| provider.primary_url.as_deref())
            {
                ui.add_space(style::SPACE_SM);
                ui.hyperlink_to("Open project page", url);
            }
        });

    if let Some(error) = state.install_error.as_deref() {
        ui.add_space(style::SPACE_SM);
        let _ = text_ui.label(
            ui,
            "discover_detail_install_error",
            error,
            &style::error_text(ui),
        );
    }
    if state.install_in_flight {
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            ui.spinner();
            let _ = text_ui.label(
                ui,
                "discover_detail_install_progress",
                state
                    .install_message
                    .as_deref()
                    .unwrap_or("Installing modpack..."),
                &muted_style,
            );
        });
        if state.install_total_steps > 0 {
            ui.add(
                egui::ProgressBar::new(
                    state.install_completed_steps as f32 / state.install_total_steps as f32,
                )
                .show_percentage(),
            );
        }
    }

    ui.add_space(style::SPACE_MD);
    let versions_height = ui.available_height().max(1.0);
    egui::ScrollArea::vertical()
        .id_salt("discover_detail_versions_scroll")
        .auto_shrink([false, false])
        .max_height(versions_height)
        .show(ui, |ui| {
            ui.set_width(ui.available_width().max(1.0));
            if state.detail_versions_in_flight {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let _ = text_ui.label(
                        ui,
                        "discover_detail_versions_loading",
                        "Loading modpack versions...",
                        &muted_style,
                    );
                });
                return;
            }
            if let Some(error) = state.detail_versions_error.as_deref() {
                let _ = text_ui.label(
                    ui,
                    "discover_detail_versions_error",
                    error,
                    &style::error_text(ui),
                );
                return;
            }

            for version in &state.detail_versions {
                let row_width = ui.available_width().max(1.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(row_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::new()
                            .fill(ui.visuals().window_fill)
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
                            .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
                            .show(ui, |ui| {
                                let row_width = ui.available_width().max(1.0);
                                let action_width = 150.0;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(
                                            (row_width - action_width - style::SPACE_MD).max(1.0),
                                            0.0,
                                        ),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            let _ = text_ui.label(
                                                ui,
                                                (
                                                    "discover_detail_version_name",
                                                    version.version_id.as_str(),
                                                ),
                                                version.version_name.as_str(),
                                                &LabelOptions {
                                                    font_size: 18.0,
                                                    line_height: 22.0,
                                                    wrap: true,
                                                    ..style::stat_label(ui)
                                                },
                                            );
                                            if let Some(published_at) =
                                                version.published_at.as_deref()
                                            {
                                                let _ = text_ui.label(
                                                    ui,
                                                    (
                                                        "discover_detail_version_date",
                                                        version.version_id.as_str(),
                                                    ),
                                                    &format!(
                                                        "Published: {}",
                                                        format_short_date(published_at)
                                                    ),
                                                    &muted_style,
                                                );
                                            }
                                            if !version.loaders.is_empty() {
                                                let _ = text_ui.label(
                                                    ui,
                                                    (
                                                        "discover_detail_version_loaders",
                                                        version.version_id.as_str(),
                                                    ),
                                                    &format!(
                                                        "Loaders: {}",
                                                        version.loaders.join(", ")
                                                    ),
                                                    &muted_style,
                                                );
                                            }
                                            if !version.game_versions.is_empty() {
                                                let preview = version
                                                    .game_versions
                                                    .iter()
                                                    .take(4)
                                                    .cloned()
                                                    .collect::<Vec<_>>()
                                                    .join(", ");
                                                let _ = text_ui.label(
                                                    ui,
                                                    (
                                                        "discover_detail_version_game_versions",
                                                        version.version_id.as_str(),
                                                    ),
                                                    &format!("Game versions: {preview}"),
                                                    &muted_style,
                                                );
                                            }
                                            if let Some(download_count) = version.download_count {
                                                let _ = text_ui.label(
                                                    ui,
                                                    (
                                                        "discover_detail_version_downloads",
                                                        version.version_id.as_str(),
                                                    ),
                                                    &format!(
                                                        "Downloads: {}",
                                                        format_compact_number(download_count)
                                                    ),
                                                    &muted_style,
                                                );
                                            }
                                        },
                                    );
                                    ui.add_space(style::SPACE_MD);
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(action_width, 0.0),
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let install_enabled = !state.install_in_flight;
                                            let response = ui
                                                .add_enabled_ui(install_enabled, |ui| {
                                                    text_ui.button(
                                                        ui,
                                                        (
                                                            "discover_create_instance",
                                                            entry.dedupe_key.as_str(),
                                                            version.version_id.as_str(),
                                                        ),
                                                        "Create Instance",
                                                        &style::neutral_button_with_min_size(
                                                            ui,
                                                            egui::vec2(
                                                                action_width,
                                                                style::CONTROL_HEIGHT,
                                                            ),
                                                        ),
                                                    )
                                                })
                                                .inner;
                                            if response.clicked()
                                                && let Some(request) =
                                                    build_install_request(&entry, version)
                                            {
                                                state.install_in_flight = true;
                                                state.install_error = None;
                                                state.install_message = Some(format!(
                                                    "Preparing {}...",
                                                    version.version_name
                                                ));
                                                state.install_completed_steps = 0;
                                                state.install_total_steps = 0;
                                                output.install_requested = Some(request);
                                            }
                                        },
                                    );
                                });
                            });
                    },
                );
                ui.add_space(style::SPACE_SM);
            }
        });

    output
}

pub(super) fn open_detail_page(state: &mut DiscoverState, entry: &DiscoverEntry) {
    let same_entry = state
        .detail_entry
        .as_ref()
        .is_some_and(|current| current.dedupe_key == entry.dedupe_key);
    if !same_entry {
        state.detail_entry = Some(entry.clone());
        state.detail_selected_source = entry.provider_refs.first().map(|provider| provider.source);
        state.detail_versions.clear();
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
        state.detail_version_request_serial = 0;
        state.install_in_flight = false;
        state.install_message = None;
        state.install_error = None;
        state.install_completed_steps = 0;
        state.install_total_steps = 0;
    }
}

fn selected_detail_source(state: &DiscoverState, entry: &DiscoverEntry) -> DiscoverSource {
    state
        .detail_selected_source
        .filter(|source| {
            entry
                .provider_refs
                .iter()
                .any(|provider| provider.source == *source)
        })
        .or_else(|| entry.provider_refs.first().map(|provider| provider.source))
        .unwrap_or(DiscoverSource::Modrinth)
}

fn selected_detail_provider_ref<'a>(
    entry: &'a DiscoverEntry,
    selected_source: DiscoverSource,
) -> Option<&'a DiscoverProviderRef> {
    entry
        .provider_refs
        .iter()
        .find(|provider| provider.source == selected_source)
}

fn build_install_request(
    entry: &DiscoverEntry,
    version: &DiscoverVersionEntry,
) -> Option<DiscoverInstallRequest> {
    let provider = selected_provider_for_version(entry, version)?;
    let source = match (&provider.project_ref, version.source) {
        (DiscoverProjectRef::Modrinth { project_id }, DiscoverSource::Modrinth) => {
            DiscoverInstallSource::Modrinth {
                project_id: project_id.clone(),
                version_id: version.version_id.clone(),
                file_url: version.file_url.clone()?,
                file_name: version.file_name.clone(),
            }
        }
        (DiscoverProjectRef::CurseForge { project_id }, DiscoverSource::CurseForge) => {
            DiscoverInstallSource::CurseForge {
                project_id: *project_id,
                file_id: version.version_id.parse().ok()?,
                file_name: version.file_name.clone(),
                download_url: version.file_url.clone(),
                manual_download_path: None,
            }
        }
        _ => return None,
    };
    Some(DiscoverInstallRequest {
        instance_name: entry.name.clone(),
        project_summary: non_empty(entry.summary.as_str()),
        icon_url: entry.icon_url.clone(),
        version_name: version.version_name.clone(),
        source,
    })
}

fn selected_provider_for_version<'a>(
    entry: &'a DiscoverEntry,
    version: &DiscoverVersionEntry,
) -> Option<&'a DiscoverProviderRef> {
    entry
        .provider_refs
        .iter()
        .find(|provider| provider.source == version.source)
}

pub(super) fn request_detail_versions(state: &mut DiscoverState) {
    if state.detail_versions_in_flight
        || !state.detail_versions.is_empty()
        || state.detail_versions_error.is_some()
    {
        return;
    }
    let Some(entry) = state.detail_entry.as_ref().cloned() else {
        return;
    };
    let selected_source = selected_detail_source(state, &entry);
    let Some(provider_ref) = selected_detail_provider_ref(&entry, selected_source).cloned() else {
        return;
    };

    ensure_detail_versions_channel(state);
    let Some(tx) = state.detail_version_results_tx.as_ref().cloned() else {
        return;
    };

    state.detail_versions_in_flight = true;
    state.detail_version_request_serial = state.detail_version_request_serial.saturating_add(1);
    let request_serial = state.detail_version_request_serial;
    let loader_filter = state.loader_filter;
    let game_version_filter = non_empty(state.game_version_filter.as_str());
    let _ = tokio_runtime::spawn_detached(async move {
        let versions: Result<Vec<DiscoverVersionEntry>, String> = match tokio::time::timeout(
            DETAIL_VERSIONS_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                load_detail_versions(
                    &provider_ref,
                    selected_source,
                    loader_filter,
                    game_version_filter.as_deref(),
                )
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(format!(
                "detail version request timed out after {}s",
                DETAIL_VERSIONS_FETCH_TIMEOUT.as_secs()
            )),
        };
        if let Err(err) = tx.send(DiscoverVersionsResult {
            request_serial,
            versions,
        }) {
            tracing::error!(
                target: "vertexlauncher/discover",
                request_serial,
                error = %err,
                "Failed to deliver discover detail versions result."
            );
        }
    });
}

fn ensure_detail_versions_channel(state: &mut DiscoverState) {
    if state.detail_version_results_tx.is_some() && state.detail_version_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<DiscoverVersionsResult>();
    state.detail_version_results_tx = Some(tx);
    state.detail_version_results_rx = Some(Arc::new(Mutex::new(rx)));
}

pub(super) fn poll_detail_versions(state: &mut DiscoverState) {
    let Some(rx) = state.detail_version_results_rx.as_ref().cloned() else {
        return;
    };
    let Ok(receiver) = rx.lock() else {
        tracing::error!(
            target: "vertexlauncher/discover",
            request_serial = state.detail_version_request_serial,
            "Discover detail-version receiver mutex was poisoned."
        );
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok(result) => {
                if result.request_serial != state.detail_version_request_serial {
                    tracing::debug!(
                        target: "vertexlauncher/discover",
                        request_serial = result.request_serial,
                        active_request_serial = state.detail_version_request_serial,
                        "Ignoring stale discover detail-version result."
                    );
                    continue;
                }
                state.detail_versions_in_flight = false;
                match result.versions {
                    Ok(versions) => {
                        state.detail_versions = versions;
                        state.detail_versions_error = None;
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "vertexlauncher/discover",
                            request_serial = result.request_serial,
                            error = %error,
                            "Discover detail-version fetch failed."
                        );
                        state.detail_versions.clear();
                        state.detail_versions_error = Some(error);
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/discover",
                    request_serial = state.detail_version_request_serial,
                    "Discover detail-version worker disconnected unexpectedly."
                );
                state.detail_versions_in_flight = false;
                state.detail_versions.clear();
                state.detail_versions_error =
                    Some("Version detail worker stopped unexpectedly.".to_owned());
                break;
            }
        }
    }
}

fn load_detail_versions(
    provider_ref: &DiscoverProviderRef,
    source: DiscoverSource,
    loader_filter: DiscoverLoaderFilter,
    game_version_filter: Option<&str>,
) -> Result<Vec<DiscoverVersionEntry>, String> {
    match (&provider_ref.project_ref, source) {
        (DiscoverProjectRef::Modrinth { project_id }, DiscoverSource::Modrinth) => {
            let loaders = loader_filter
                .modrinth_slug()
                .map(|loader| vec![loader.to_owned()])
                .unwrap_or_default();
            let game_versions = game_version_filter
                .map(|version| vec![version.to_owned()])
                .unwrap_or_default();
            ModrinthClient::default()
                .list_project_versions(
                    project_id.as_str(),
                    loaders.as_slice(),
                    game_versions.as_slice(),
                )
                .map_err(|err| format!("failed to load Modrinth versions: {err}"))
                .map(|versions| {
                    versions
                        .into_iter()
                        .filter_map(|version| {
                            let file = version
                                .files
                                .iter()
                                .find(|file| file.primary)
                                .or_else(|| version.files.first())?;
                            Some(DiscoverVersionEntry {
                                source: DiscoverSource::Modrinth,
                                version_id: version.id,
                                version_name: version.version_number,
                                published_at: non_empty(version.date_published.as_str()),
                                file_name: file.filename.clone(),
                                file_url: Some(file.url.clone()),
                                game_versions: version.game_versions,
                                loaders: version.loaders,
                                download_count: Some(version.downloads),
                            })
                        })
                        .collect()
                })
        }
        (DiscoverProjectRef::CurseForge { project_id }, DiscoverSource::CurseForge) => {
            let client = CurseForgeClient::from_env()
                .ok_or_else(|| "CurseForge API key missing in settings.".to_owned())?;
            client
                .list_mod_files(
                    *project_id,
                    game_version_filter,
                    loader_filter.curseforge_mod_loader_type(),
                    0,
                    50,
                )
                .map_err(|err| format!("failed to load CurseForge files: {err}"))
                .map(|files| {
                    files
                        .into_iter()
                        .map(|file| DiscoverVersionEntry {
                            source: DiscoverSource::CurseForge,
                            version_id: file.id.to_string(),
                            version_name: file.display_name,
                            published_at: non_empty(file.file_date.as_str()),
                            file_name: file.file_name,
                            file_url: file.download_url,
                            game_versions: file.game_versions,
                            loaders: Vec::new(),
                            download_count: Some(file.download_count),
                        })
                        .collect()
                })
        }
        _ => Ok(Vec::new()),
    }
}

pub(super) fn format_compact_number(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

pub(super) fn format_short_date(value: &str) -> String {
    value.get(0..10).unwrap_or(value).to_owned()
}
