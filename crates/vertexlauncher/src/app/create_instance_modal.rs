use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use installation::{
    LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, VersionCatalog,
    VersionCatalogFilter, fetch_version_catalog_with_refresh,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::{
    assets,
    ui::components::{icon_button, settings_widgets},
};
use textui::TextUi;
use textui_egui::prelude::*;
use ui_foundation::{DialogPreset, dialog_options, show_dialog};
use url::Url;

const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;
const ACTION_BUTTON_MAX_WIDTH: f32 = 260.0;
const MODAL_GAP_SM: f32 = 6.0;
const MODAL_GAP_MD: f32 = 8.0;
const MODAL_GAP_LG: f32 = 10.0;
const VERSION_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(75);
const MODLOADER_VERSIONS_FETCH_TIMEOUT: Duration = Duration::from_secs(45);
const PRIMARY_ACTION_WIDTH: f32 = 160.0;
const SECONDARY_ACTION_WIDTH: f32 = 100.0;

#[derive(Debug)]
pub struct CreateInstanceState {
    pub name: String,
    pub description: String,
    pub thumbnail_path: PathBuf,
    pub game_version: String,
    pub modloader_version: String,
    pub selected_modloader: usize,
    pub custom_modloader: String,
    pub error: Option<String>,
    pub create_in_flight: bool,
    pub create_results_tx: Option<mpsc::Sender<CreateInstanceTaskResult>>,
    pub create_results_rx: Option<mpsc::Receiver<CreateInstanceTaskResult>>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    selected_game_version_index: usize,
    loader_support: LoaderSupportIndex,
    loader_versions: LoaderVersionIndex,
    version_catalog_filter: Option<VersionCatalogFilter>,
    version_catalog_error: Option<String>,
    version_catalog_in_flight: bool,
    version_catalog_results_tx:
        Option<mpsc::Sender<(VersionCatalogFilter, Result<VersionCatalog, String>)>>,
    version_catalog_results_rx:
        Option<mpsc::Receiver<(VersionCatalogFilter, Result<VersionCatalog, String>)>>,
    modloader_versions_cache: BTreeMap<String, Vec<String>>,
    modloader_versions_in_flight: HashSet<String>,
    modloader_versions_results_tx: Option<mpsc::Sender<(String, Result<Vec<String>, String>)>>,
    modloader_versions_results_rx: Option<mpsc::Receiver<(String, Result<Vec<String>, String>)>>,
    modloader_versions_status_key: Option<String>,
    modloader_versions_status: Option<String>,
}

pub type CreateInstanceTaskResult =
    Result<(instances::InstanceStore, instances::InstanceRecord), String>;

impl Default for CreateInstanceState {
    fn default() -> Self {
        Self {
            name: "New Instance".to_owned(),
            description: String::new(),
            thumbnail_path: PathBuf::new(),
            game_version: String::new(),
            modloader_version: String::new(),
            selected_modloader: 0,
            custom_modloader: String::new(),
            error: None,
            create_in_flight: false,
            create_results_tx: None,
            create_results_rx: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            loader_versions: LoaderVersionIndex::default(),
            version_catalog_filter: None,
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_results_tx: None,
            version_catalog_results_rx: None,
            modloader_versions_cache: BTreeMap::new(),
            modloader_versions_in_flight: HashSet::new(),
            modloader_versions_results_tx: None,
            modloader_versions_results_rx: None,
            modloader_versions_status_key: None,
            modloader_versions_status: None,
        }
    }
}

impl CreateInstanceState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug)]
pub struct CreateInstanceDraft {
    pub name: String,
    pub description: Option<String>,
    pub thumbnail_path: Option<PathBuf>,
    pub modloader: String,
    pub game_version: String,
    pub modloader_version: String,
}

impl CreateInstanceDraft {
    pub fn into_new_instance_spec(self) -> instances::NewInstanceSpec {
        instances::NewInstanceSpec {
            name: self.name,
            description: self.description,
            thumbnail_path: self.thumbnail_path,
            modloader: self.modloader,
            game_version: self.game_version,
            modloader_version: self.modloader_version,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ModalAction {
    None,
    Cancel,
    Create(CreateInstanceDraft),
}

pub fn render(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    state: &mut CreateInstanceState,
    version_catalog_filter: VersionCatalogFilter,
) -> ModalAction {
    let mut action = ModalAction::None;
    poll_version_catalog(state);
    sync_version_catalog(state, version_catalog_filter, false);
    poll_modloader_versions(state);
    if state.version_catalog_in_flight || !state.modloader_versions_in_flight.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    let response = show_dialog(
        ctx,
        dialog_options("create_instance_modal_window", DialogPreset::Form),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(MODAL_GAP_MD, MODAL_GAP_MD);
            let modal_max_height = ui.max_rect().height();
            let action_width = ui.available_width();
            let compact_actions = action_width < 320.0;
            let footer_reserve = if compact_actions { 152.0 } else { 110.0 };
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

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height((modal_max_height - footer_reserve).max(220.0))
                .show(ui, |ui| {
                    let _ = text_ui.label(
                        ui,
                        "instance_create_heading",
                        "Create Instance",
                        &heading_style,
                    );
                    let _ = text_ui.label(
                        ui,
                        "instance_create_subheading",
                        "Choose name, thumbnail, modloader, and versions.",
                        &body_style,
                    );
                    render_thumbnail_picker(text_ui, ui, state);

                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_create_name",
                        "Instance name",
                        Some("Display name shown in the sidebar."),
                        &mut state.name,
                    );
                    ui.add_space(MODAL_GAP_SM);
                    let _ = settings_widgets::full_width_text_input_row(
                        text_ui,
                        ui,
                        "instance_create_description",
                        "Description (optional)",
                        Some("Optional note shown in the library tile."),
                        &mut state.description,
                    );
                    ui.add_space(MODAL_GAP_SM);

                    let refresh_versions_clicked = ui
                        .add_enabled_ui(!state.version_catalog_in_flight, |ui| {
                            settings_widgets::full_width_button(
                                text_ui,
                                ui,
                                "instance_create_refresh_versions",
                                "Refresh version list",
                                ui.available_width().clamp(1.0, ACTION_BUTTON_MAX_WIDTH),
                                false,
                            )
                        })
                        .inner
                        .clicked();
                    if refresh_versions_clicked {
                        sync_version_catalog(state, version_catalog_filter, true);
                        state.modloader_versions_cache.clear();
                        state.modloader_versions_status = None;
                        state.modloader_versions_status_key = None;
                    }
                    if state.version_catalog_in_flight {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let _ = text_ui.label(
                                ui,
                                "instance_create_catalog_fetching",
                                "Fetching version catalog...",
                                &LabelOptions {
                                    color: ui.visuals().weak_text_color(),
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        });
                    }

                    if let Some(catalog_error) = state.version_catalog_error.as_deref() {
                        let _ = text_ui.label(
                            ui,
                            "instance_create_version_catalog_error",
                            catalog_error,
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
                        );
                    }

                    let version_labels: Vec<String> = state
                        .available_game_versions
                        .iter()
                        .map(MinecraftVersionEntry::display_label)
                        .collect();
                    let version_refs: Vec<&str> = version_labels.iter().map(String::as_str).collect();
                    if !version_refs.is_empty() {
                        let mut selected_index = state
                            .selected_game_version_index
                            .min(version_refs.len().saturating_sub(1));
                        let changed = settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            "instance_create_game_version_dropdown",
                            "Minecraft game version",
                            Some("Choose from fetched Minecraft versions."),
                            &mut selected_index,
                            &version_refs,
                        )
                        .changed();
                        if changed {
                            state.selected_game_version_index = selected_index;
                            if let Some(version) = state.available_game_versions.get(selected_index) {
                                state.game_version = version.id.clone();
                            }
                        }
                    } else {
                        let _ = text_ui.label(
                            ui,
                            "instance_create_no_game_versions",
                            "No game versions available yet.",
                            &body_style,
                        );
                    }

                    ui.add_space(MODAL_GAP_SM);

                    let _ = text_ui.label(
                        ui,
                        "instance_create_modloader_label",
                        "Modloader",
                        &LabelOptions {
                            font_size: 18.0,
                            line_height: 24.0,
                            color: text_color,
                            wrap: false,
                            ..LabelOptions::default()
                        },
                    );
                    ui.add_space(4.0);

                    let selected_game_version = state
                        .available_game_versions
                        .get(state.selected_game_version_index)
                        .map(|entry| entry.id.as_str())
                        .unwrap_or_else(|| state.game_version.as_str())
                        .to_owned();
                    ensure_selected_modloader_is_supported(state, selected_game_version.as_str());

                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for (index, option) in MODLOADER_OPTIONS.iter().enumerate() {
                            let unavailable_reason = if index == CUSTOM_MODLOADER_INDEX {
                                None
                            } else {
                                state
                                    .loader_support
                                    .unavailable_reason(option, selected_game_version.as_str())
                            };
                            let available = unavailable_reason.is_none();

                            let mut response = settings_widgets::selectable_chip_button(
                                text_ui,
                                ui,
                                ("instance_create_modloader", index),
                                option,
                                state.selected_modloader == index,
                                88.0,
                                available,
                            );
                            if let Some(reason) = unavailable_reason.as_deref() {
                                response = response.on_hover_text(reason);
                            }

                            if available && response.clicked() && state.selected_modloader != index {
                                state.selected_modloader = index;
                                state.modloader_version.clear();
                            }
                        }
                    });

                    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
                        ui.add_space(MODAL_GAP_SM);
                        let _ = settings_widgets::full_width_text_input_row(
                            text_ui,
                            ui,
                            "instance_create_custom_modloader",
                            "Custom modloader id",
                            Some("Use any custom loader name."),
                            &mut state.custom_modloader,
                        );
                    }

                    ui.add_space(MODAL_GAP_SM);
                    let selected_modloader_label = selected_modloader_label(state);
                    let modloader_versions_key = modloader_versions_cache_key(
                        selected_modloader_label.as_str(),
                        selected_game_version.as_str(),
                    );
                    let available_modloader_versions =
                        selected_modloader_versions(state, selected_game_version.as_str()).to_vec();
                    if state.selected_modloader == 0 {
                        state.modloader_version.clear();
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
                                    selected_game_version.as_str(),
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
                                    "instance_create_modloader_versions_fetching",
                                    "Fetching modloader versions...",
                                    &LabelOptions {
                                        color: ui.visuals().weak_text_color(),
                                        wrap: true,
                                        ..LabelOptions::default()
                                    },
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
                                "instance_create_modloader_versions_status",
                                status,
                                &LabelOptions {
                                    color: if is_error {
                                        ui.visuals().error_fg_color
                                    } else {
                                        ui.visuals().weak_text_color()
                                    },
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        }

                        let modloader_version_options: Vec<String> = resolved_modloader_versions.clone();

                        if state.modloader_version.trim().is_empty() {
                            if let Some(first) = modloader_version_options.first() {
                                state.modloader_version = first.clone();
                            }
                        }

                        let option_refs: Vec<&str> =
                            modloader_version_options.iter().map(String::as_str).collect();
                        let current_modloader_version = state.modloader_version.trim().to_owned();
                        let mut selected_index = modloader_version_options
                            .iter()
                            .position(|entry| entry == &current_modloader_version)
                            .unwrap_or(0);

                        let changed = settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            "instance_create_modloader_version_dropdown",
                            "Modloader version",
                            Some("Cataloged by loader+Minecraft compatibility and cached once per day."),
                            &mut selected_index,
                            &option_refs,
                        )
                        .changed();
                        if changed {
                            if let Some(selected) = modloader_version_options.get(selected_index) {
                                state.modloader_version = selected.clone();
                            }
                        }

                        if state.selected_modloader != CUSTOM_MODLOADER_INDEX {
                            let refresh_clicked = ui
                                .add_enabled_ui(!in_flight, |ui| {
                                    settings_widgets::full_width_button(
                                        text_ui,
                                        ui,
                                        "instance_create_modloader_versions_refresh",
                                        "Refresh modloader versions",
                                        ui.available_width().clamp(1.0, ACTION_BUTTON_MAX_WIDTH),
                                        false,
                                    )
                                })
                                .inner
                                .clicked();
                            if refresh_clicked {
                                request_modloader_versions(
                                    state,
                                    selected_modloader_label.as_str(),
                                    selected_game_version.as_str(),
                                    true,
                                );
                            }
                        }

                        if resolved_modloader_versions.is_empty()
                            && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                        {
                            let _ = text_ui.label(
                                ui,
                                "instance_create_modloader_versions_unavailable",
                                "No cataloged modloader versions were found for this Minecraft version.",
                                &LabelOptions {
                                    color: ui.visuals().weak_text_color(),
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        }
                    }

                    if let Some(error) = state.error.as_deref() {
                        ui.add_space(MODAL_GAP_MD);
                        let _ = text_ui.label(
                            ui,
                            "instance_create_error",
                            error,
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
                        );
                    }

                    if state.create_in_flight {
                        ui.add_space(MODAL_GAP_MD);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            let _ = text_ui.label(
                                ui,
                                "instance_create_in_progress",
                                "Creating instance in the background...",
                                &LabelOptions {
                                    color: ui.visuals().weak_text_color(),
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        });
                    }
                });

            ui.add_space(MODAL_GAP_LG);
            ui.separator();
            ui.add_space(MODAL_GAP_LG);

            let mut create_clicked = false;
            let mut cancel_clicked = false;

            if compact_actions {
                create_clicked = ui
                    .add_enabled_ui(!state.create_in_flight, |ui| {
                        settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_create_confirm",
                            "Create instance",
                            action_width,
                            true,
                        )
                    })
                    .inner
                    .clicked();
                ui.add_space(MODAL_GAP_SM);
                cancel_clicked = ui
                    .add_enabled_ui(!state.create_in_flight, |ui| {
                        settings_widgets::full_width_button(
                            text_ui,
                            ui,
                            "instance_create_cancel",
                            "Cancel",
                            action_width,
                            false,
                        )
                    })
                    .inner
                    .clicked();
            } else {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    create_clicked = ui
                        .add_enabled_ui(!state.create_in_flight, |ui| {
                            settings_widgets::full_width_button(
                                text_ui,
                                ui,
                                "instance_create_confirm",
                                "Create instance",
                                PRIMARY_ACTION_WIDTH,
                                true,
                            )
                        })
                        .inner
                        .clicked();
                    cancel_clicked = ui
                        .add_enabled_ui(!state.create_in_flight, |ui| {
                            settings_widgets::full_width_button(
                                text_ui,
                                ui,
                                "instance_create_cancel",
                                "Cancel",
                                SECONDARY_ACTION_WIDTH,
                                false,
                            )
                        })
                        .inner
                        .clicked();
                });
            }

            if cancel_clicked {
                state.error = None;
                action = ModalAction::Cancel;
            } else if create_clicked {
                match build_draft(state) {
                    Ok(draft) => {
                        state.error = None;
                        action = ModalAction::Create(draft);
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "vertexlauncher/create_instance",
                            name_present = !state.name.trim().is_empty(),
                            selected_modloader = %MODLOADER_OPTIONS
                                .get(state.selected_modloader)
                                .copied()
                                .unwrap_or(MODLOADER_OPTIONS[0]),
                            game_version = %state.game_version.trim(),
                            "Create-instance validation failed before submit."
                        );
                        state.error = Some(error);
                    }
                }
            }
        },
    );

    if response.close_requested && !state.create_in_flight && matches!(action, ModalAction::None) {
        state.error = None;
        action = ModalAction::Cancel;
    }

    action
}

fn sync_version_catalog(
    state: &mut CreateInstanceState,
    filter: VersionCatalogFilter,
    force_refresh: bool,
) {
    let should_refresh = force_refresh
        || state.version_catalog_filter != Some(filter)
        || (state.available_game_versions.is_empty() && state.version_catalog_error.is_none());
    if !should_refresh || state.version_catalog_in_flight {
        return;
    }

    ensure_version_catalog_channel(state);
    let Some(tx) = state.version_catalog_results_tx.as_ref().cloned() else {
        return;
    };

    state.version_catalog_in_flight = true;
    state.version_catalog_filter = Some(filter);
    state.version_catalog_error = None;
    tracing::info!(
        target: "vertexlauncher/create_instance",
        ?filter,
        force_refresh,
        "Starting version catalog fetch for create-instance modal."
    );

    let _ = tokio_runtime::spawn_detached(async move {
        let result = match tokio::time::timeout(
            VERSION_CATALOG_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_version_catalog_with_refresh(filter, force_refresh)
                    .map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(format!(
                "version catalog request timed out after {}s",
                VERSION_CATALOG_FETCH_TIMEOUT.as_secs()
            )),
        };
        match &result {
            Ok(catalog) => tracing::info!(
                target: "vertexlauncher/create_instance",
                ?filter,
                force_refresh,
                game_versions = catalog.game_versions.len(),
                "Create-instance version catalog fetch completed."
            ),
            Err(error) => tracing::warn!(
                target: "vertexlauncher/create_instance",
                ?filter,
                force_refresh,
                error = %error,
                "Create-instance version catalog fetch failed."
            ),
        }
        if let Err(err) = tx.send((filter, result)) {
            tracing::error!(
                target: "vertexlauncher/create_instance",
                ?filter,
                force_refresh,
                error = %err,
                "Failed to deliver create-instance version catalog result."
            );
        }
    });
}

fn ensure_version_catalog_channel(state: &mut CreateInstanceState) {
    if state.version_catalog_results_tx.is_some() && state.version_catalog_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(VersionCatalogFilter, Result<VersionCatalog, String>)>();
    state.version_catalog_results_tx = Some(tx);
    state.version_catalog_results_rx = Some(rx);
}

fn apply_version_catalog(
    state: &mut CreateInstanceState,
    filter: VersionCatalogFilter,
    catalog: VersionCatalog,
) {
    state.available_game_versions = catalog.game_versions;
    state.loader_support = catalog.loader_support;
    state.loader_versions = catalog.loader_versions;
    state.version_catalog_filter = Some(filter);
    state.version_catalog_error = None;

    if state.available_game_versions.is_empty() {
        state.selected_game_version_index = 0;
        state.game_version.clear();
    } else {
        let preferred_index = if state.game_version.trim().is_empty() {
            0
        } else {
            state
                .available_game_versions
                .iter()
                .position(|entry| entry.id == state.game_version)
                .unwrap_or(0)
        };
        state.selected_game_version_index = preferred_index;
        if let Some(selected) = state.available_game_versions.get(preferred_index) {
            state.game_version = selected.id.clone();
        }
    }
}

fn apply_version_catalog_error(
    state: &mut CreateInstanceState,
    filter: VersionCatalogFilter,
    error: &str,
) {
    tracing::warn!(
        target: "vertexlauncher/create_instance",
        ?filter,
        error = %error,
        "Applying version catalog error to create-instance modal."
    );
    state.version_catalog_error = Some(format!("Failed to fetch version catalog: {error}"));
    state.available_game_versions.clear();
    state.loader_support = LoaderSupportIndex::default();
    state.loader_versions = LoaderVersionIndex::default();
    state.version_catalog_filter = Some(filter);
    state.selected_game_version_index = 0;
    state.game_version.clear();
}

fn poll_version_catalog(state: &mut CreateInstanceState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.version_catalog_results_rx.as_ref() {
        loop {
            match rx.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/create_instance",
                        version_catalog_filter = ?state.version_catalog_filter,
                        "Create-instance version catalog worker channel disconnected unexpectedly."
                    );
                    should_reset_channel = true;
                    break;
                }
            }
        }
    }

    if should_reset_channel {
        state.version_catalog_results_tx = None;
        state.version_catalog_results_rx = None;
        state.version_catalog_in_flight = false;
        state.version_catalog_error =
            Some("Version catalog worker stopped unexpectedly.".to_owned());
    }

    for (filter, result) in updates {
        state.version_catalog_in_flight = false;
        match result {
            Ok(catalog) => apply_version_catalog(state, filter, catalog),
            Err(err) => apply_version_catalog_error(state, filter, &err),
        }
    }
}

fn render_thumbnail_picker(
    text_ui: &mut TextUi,
    ui: &mut egui::Ui,
    state: &mut CreateInstanceState,
) {
    const THUMBNAIL_PREVIEW_SIZE: f32 = 150.0;
    const PREVIEW_FRAME_PADDING: f32 = 0.0;
    let preview_inner_width = ui.available_width().clamp(64.0, THUMBNAIL_PREVIEW_SIZE);
    let preview_height = preview_inner_width;
    ui.horizontal(|ui| {
        let frame_outer_size = preview_inner_width + PREVIEW_FRAME_PADDING * 2.0;
        let left_inset = ((ui.available_width() - frame_outer_size) * 0.5).max(0.0);
        ui.add_space(left_inset);
        let frame_response = egui::Frame::new()
            .fill(ui.visuals().widgets.inactive.bg_fill)
            .stroke(ui.visuals().widgets.inactive.bg_stroke)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(PREVIEW_FRAME_PADDING.round() as i8))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(preview_inner_width, preview_height),
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        let path = state.thumbnail_path.as_path();
                        if path.as_os_str().is_empty() {
                            let _ = text_ui.label(
                                ui,
                                "instance_create_thumbnail_empty",
                                "No thumbnail selected",
                                &LabelOptions {
                                    color: ui.visuals().weak_text_color(),
                                    wrap: false,
                                    ..LabelOptions::default()
                                },
                            );
                            return;
                        }

                        if !path.is_file() {
                            let _ = text_ui.label(
                                ui,
                                "instance_create_thumbnail_missing",
                                "Thumbnail file was not found",
                                &LabelOptions {
                                    color: ui.visuals().weak_text_color(),
                                    wrap: false,
                                    ..LabelOptions::default()
                                },
                            );
                            return;
                        }

                        let image_uri = file_uri_from_path(path);
                        let image = egui::Image::from_uri(image_uri)
                            .maintain_aspect_ratio(true)
                            .max_size(egui::vec2(preview_inner_width, preview_height));
                        let _ = ui.add(image);
                    },
                );
            })
            .response
            .interact(egui::Sense::click());

        let pointer_in_preview = ui.input(|i| {
            i.pointer
                .hover_pos()
                .is_some_and(|pos| frame_response.rect.contains(pos))
        });
        let mut should_open_picker = frame_response.clicked();
        if pointer_in_preview {
            let overlay_size = egui::vec2(52.0, 52.0);
            let overlay_rect =
                egui::Rect::from_center_size(frame_response.rect.center(), overlay_size);
            let overlay_response = ui
                .scope_builder(egui::UiBuilder::new().max_rect(overlay_rect), |ui| {
                    icon_button::svg(
                        ui,
                        "instance_create_thumbnail_edit_overlay",
                        assets::EDIT_SVG,
                        "Edit thumbnail",
                        false,
                        overlay_size.x,
                    )
                })
                .inner;
            if overlay_response.clicked() {
                should_open_picker = true;
            }
        }

        if should_open_picker
            && let Some(path) = pick_thumbnail_path(state.thumbnail_path.as_path())
        {
            state.thumbnail_path = path;
        }
    });
}

fn file_uri_from_path(path: &Path) -> String {
    Url::from_file_path(path)
        .map(|url| url.into())
        .unwrap_or_else(|_| "file:///".to_owned())
}

fn pick_thumbnail_path(current_path: &Path) -> Option<PathBuf> {
    let mut dialog =
        rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]);
    if current_path.is_file() {
        if let Some(parent) = current_path.parent() {
            dialog = dialog.set_directory(parent);
        }
    } else if current_path.is_dir() {
        dialog = dialog.set_directory(current_path);
    }
    dialog.pick_file()
}

fn selected_modloader_versions<'a>(
    state: &'a CreateInstanceState,
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

fn selected_modloader_label(state: &CreateInstanceState) -> String {
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

fn modloader_versions_cache_key(loader_label: &str, game_version: &str) -> String {
    format!(
        "{}|{}",
        loader_label.trim().to_ascii_lowercase(),
        game_version.trim()
    )
}

fn ensure_modloader_versions_channel(state: &mut CreateInstanceState) {
    if state.modloader_versions_results_tx.is_some()
        && state.modloader_versions_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Result<Vec<String>, String>)>();
    state.modloader_versions_results_tx = Some(tx);
    state.modloader_versions_results_rx = Some(rx);
}

fn request_modloader_versions(
    state: &mut CreateInstanceState,
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
    tracing::info!(
        target: "vertexlauncher/create_instance",
        loader = loader_label,
        game_version,
        force_refresh,
        "Starting create-instance modloader version fetch."
    );

    let loader = loader_label.to_owned();
    let game = game_version.to_owned();
    let _ = tokio_runtime::spawn_detached(async move {
        let result = match tokio::time::timeout(
            MODLOADER_VERSIONS_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                installation::fetch_loader_versions_for_game(
                    loader.as_str(),
                    game.as_str(),
                    force_refresh,
                )
                .map_err(|err| err.to_string())
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            Err(_) => Err(format!(
                "modloader version request timed out after {}s",
                MODLOADER_VERSIONS_FETCH_TIMEOUT.as_secs()
            )),
        };
        match &result {
            Ok(versions) => tracing::info!(
                target: "vertexlauncher/create_instance",
                versions = versions.len(),
                "Create-instance modloader version fetch completed."
            ),
            Err(error) => tracing::warn!(
                target: "vertexlauncher/create_instance",
                error = %error,
                "Create-instance modloader version fetch failed."
            ),
        }
        if let Err(err) = tx.send((key.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/create_instance",
                key = %key,
                force_refresh,
                error = %err,
                "Failed to deliver create-instance modloader version result."
            );
        }
    });
}

fn poll_modloader_versions(state: &mut CreateInstanceState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.modloader_versions_results_rx.as_ref() {
        loop {
            match rx.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/create_instance",
                        "Create-instance modloader version worker channel disconnected unexpectedly."
                    );
                    should_reset_channel = true;
                    break;
                }
            }
        }
    }

    if should_reset_channel {
        state.modloader_versions_results_tx = None;
        state.modloader_versions_results_rx = None;
        state.modloader_versions_status =
            Some("Modloader version worker stopped unexpectedly.".to_owned());
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
                tracing::warn!(
                    target: "vertexlauncher/create_instance",
                    cache_key = %key,
                    error = %err,
                    "Applying modloader-version fetch failure to create-instance modal."
                );
                state.modloader_versions_cache.insert(key, Vec::new());
                state.modloader_versions_status =
                    Some(format!("Failed to fetch modloader versions: {err}"));
            }
        }
    }
}

fn ensure_selected_modloader_is_supported(state: &mut CreateInstanceState, game_version: &str) {
    if !support_catalog_ready(state) {
        return;
    }
    if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        return;
    }

    let selected_label = MODLOADER_OPTIONS
        .get(state.selected_modloader)
        .copied()
        .unwrap_or(MODLOADER_OPTIONS[0]);
    if state
        .loader_support
        .supports_loader(selected_label, game_version)
    {
        return;
    }

    tracing::warn!(
        target: "vertexlauncher/ui/create_instance",
        selected_modloader = %selected_label,
        game_version = %game_version,
        "Selected modloader is not currently marked supported for this game version; keeping user selection."
    );
}

fn build_draft(state: &CreateInstanceState) -> Result<CreateInstanceDraft, String> {
    let name = state.name.trim();
    if name.is_empty() {
        return Err("Instance name is required.".to_owned());
    }

    let game_version = state.game_version.trim();
    if game_version.is_empty() {
        return Err("Minecraft game version is required.".to_owned());
    }

    let modloader = if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
        let custom = state.custom_modloader.trim();
        if custom.is_empty() {
            return Err("Custom modloader id is required.".to_owned());
        }
        custom.to_owned()
    } else {
        MODLOADER_OPTIONS
            .get(state.selected_modloader)
            .copied()
            .unwrap_or(MODLOADER_OPTIONS[0])
            .to_owned()
    };

    if support_catalog_ready(state)
        && state.selected_modloader != CUSTOM_MODLOADER_INDEX
        && !state
            .loader_support
            .supports_loader(modloader.as_str(), game_version)
    {
        return Err(format!(
            "{modloader} is not available for Minecraft {game_version}."
        ));
    }

    let raw_modloader_version = state.modloader_version.trim();
    let modloader_version = if matches!(
        normalized_loader_label_for_modal(modloader.as_str()),
        LoaderSelectionKind::Vanilla | LoaderSelectionKind::Custom
    ) {
        raw_modloader_version.to_owned()
    } else if raw_modloader_version.is_empty()
        || is_latest_modloader_version_alias(raw_modloader_version)
    {
        resolve_latest_modloader_version_from_state(state, modloader.as_str(), game_version)
            .ok_or_else(|| {
                format!(
                    "Could not resolve latest {modloader} version for Minecraft {game_version}. Refresh modloader versions and try again."
                )
            })?
    } else {
        raw_modloader_version.to_owned()
    };
    let thumbnail_path = {
        if state.thumbnail_path.as_os_str().is_empty() {
            None
        } else {
            Some(state.thumbnail_path.clone())
        }
    };
    let description = {
        let trimmed = state.description.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };

    Ok(CreateInstanceDraft {
        name: name.to_owned(),
        description,
        thumbnail_path,
        modloader,
        game_version: game_version.to_owned(),
        modloader_version,
    })
}

fn support_catalog_ready(state: &CreateInstanceState) -> bool {
    state.version_catalog_filter.is_some() && state.version_catalog_error.is_none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoaderSelectionKind {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
    Custom,
}

fn normalized_loader_label_for_modal(label: &str) -> LoaderSelectionKind {
    match label.trim().to_ascii_lowercase().as_str() {
        "vanilla" => LoaderSelectionKind::Vanilla,
        "fabric" => LoaderSelectionKind::Fabric,
        "forge" => LoaderSelectionKind::Forge,
        "neoforge" => LoaderSelectionKind::NeoForge,
        "quilt" => LoaderSelectionKind::Quilt,
        _ => LoaderSelectionKind::Custom,
    }
}

fn is_latest_modloader_version_alias(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "latest" | "latest available" | "use latest version" | "auto" | "default"
    )
}

fn resolve_latest_modloader_version_from_state(
    state: &CreateInstanceState,
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

#[cfg(test)]
mod tests {
    use super::file_uri_from_path;
    use std::path::Path;

    #[test]
    fn file_uri_from_path_percent_encodes_spaces() {
        assert_eq!(
            file_uri_from_path(Path::new("/tmp/vertex launcher/image preview.png")),
            "file:///tmp/vertex%20launcher/image%20preview.png"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn file_uri_from_windows_path_uses_valid_file_url() {
        assert_eq!(
            file_uri_from_path(Path::new(
                r"C:\Users\clove\AppData\Local\vertex launcher\preview.png"
            )),
            "file:///C:/Users/clove/AppData/Local/vertex%20launcher/preview.png"
        );
    }
}
