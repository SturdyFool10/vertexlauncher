use eframe::egui;
use installation::{LoaderSupportIndex, MinecraftVersionEntry, fetch_version_catalog};
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};

use crate::ui::components::settings_widgets;

const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;

#[derive(Clone, Debug)]
pub struct CreateInstanceState {
    pub name: String,
    pub thumbnail_path: String,
    pub game_version: String,
    pub modloader_version: String,
    pub selected_modloader: usize,
    pub custom_modloader: String,
    pub error: Option<String>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    selected_game_version_index: usize,
    loader_support: LoaderSupportIndex,
    version_catalog_include_snapshots: Option<bool>,
    version_catalog_error: Option<String>,
}

impl Default for CreateInstanceState {
    fn default() -> Self {
        Self {
            name: "New Instance".to_owned(),
            thumbnail_path: String::new(),
            game_version: String::new(),
            modloader_version: String::new(),
            selected_modloader: 0,
            custom_modloader: String::new(),
            error: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            version_catalog_include_snapshots: None,
            version_catalog_error: None,
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
    pub thumbnail_path: Option<String>,
    pub modloader: String,
    pub game_version: String,
    pub modloader_version: String,
}

impl CreateInstanceDraft {
    pub fn into_new_instance_spec(self) -> instances::NewInstanceSpec {
        instances::NewInstanceSpec {
            name: self.name,
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
    include_snapshots_and_betas: bool,
) -> ModalAction {
    let mut action = ModalAction::None;
    sync_version_catalog(state, include_snapshots_and_betas, false);

    egui::Window::new("")
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.window_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ctx.style().visuals.widgets.hovered.bg_stroke.color,
                ))
                .corner_radius(egui::CornerRadius::same(14))
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ctx, |ui| {
            ui.set_min_width(560.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            let text_color = ui.visuals().text_color();
            let heading = LabelOptions {
                font_size: 28.0,
                line_height: 32.0,
                weight: 700,
                color: text_color,
                wrap: false,
                ..LabelOptions::default()
            };
            let mut body = LabelOptions::default();
            body.color = ui.visuals().weak_text_color();
            body.wrap = false;

            let _ = text_ui.label(ui, "instance_create_heading", "Create Instance", &heading);
            let _ = text_ui.label(
                ui,
                "instance_create_subheading",
                "Choose name, thumbnail, modloader, and versions.",
                &body,
            );
            ui.add_space(6.0);

            let _ = settings_widgets::full_width_text_input_row(
                text_ui,
                ui,
                "instance_create_name",
                "Instance name",
                Some("Display name shown in the sidebar."),
                &mut state.name,
            );
            ui.add_space(6.0);

            let _ = settings_widgets::full_width_text_input_row(
                text_ui,
                ui,
                "instance_create_thumbnail",
                "Thumbnail path (optional)",
                Some("Path to an image file on disk."),
                &mut state.thumbnail_path,
            );
            ui.add_space(6.0);

            let refresh_style = ButtonOptions {
                min_size: egui::vec2(180.0, 30.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };
            if text_ui
                .button(
                    ui,
                    "instance_create_refresh_versions",
                    "Refresh version list",
                    &refresh_style,
                )
                .clicked()
            {
                sync_version_catalog(state, include_snapshots_and_betas, true);
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
                let response = settings_widgets::dropdown_row(
                    text_ui,
                    ui,
                    "instance_create_game_version_dropdown",
                    "Minecraft game version",
                    Some("Choose from fetched Minecraft versions."),
                    &mut selected_index,
                    &version_refs,
                );
                if response.changed() {
                    state.selected_game_version_index = selected_index;
                    if let Some(version) = state.available_game_versions.get(selected_index) {
                        state.game_version = version.id.clone();
                    }
                }
            } else {
                let _ = text_ui.label(
                    ui,
                    "instance_create_game_version_empty",
                    "No game versions available yet.",
                    &body,
                );
            }

            ui.add_space(6.0);

            let _ = text_ui.label(
                ui,
                "instance_create_modloader_label",
                "Modloader",
                &LabelOptions {
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

            let selector_style = ButtonOptions {
                min_size: egui::vec2(88.0, 30.0),
                text_color,
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
                            .unavailable_reason(option, selected_game_version.as_str())
                    };
                    let available = unavailable_reason.is_none();

                    let mut button_style = selector_style.clone();
                    if !available {
                        button_style.text_color = ui.visuals().weak_text_color();
                        button_style.fill = ui.visuals().widgets.noninteractive.bg_fill;
                        button_style.fill_hovered = ui.visuals().widgets.noninteractive.bg_fill;
                        button_style.fill_active = ui.visuals().widgets.noninteractive.bg_fill;
                        button_style.fill_selected = ui.visuals().widgets.noninteractive.bg_fill;
                    }

                    let response = text_ui.selectable_button(
                        ui,
                        ("instance_create_modloader", index),
                        option,
                        state.selected_modloader == index,
                        &button_style,
                    );

                    if let Some(reason) = unavailable_reason.as_deref() {
                        let tooltip_options = TooltipOptions::default();
                        text_ui.tooltip_for_response(
                            ui,
                            ("instance_create_modloader_unavailable_tooltip", index),
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

            if state.selected_modloader == CUSTOM_MODLOADER_INDEX {
                ui.add_space(6.0);
                let _ = settings_widgets::full_width_text_input_row(
                    text_ui,
                    ui,
                    "instance_create_custom_modloader",
                    "Custom modloader id",
                    Some("Use any custom loader name."),
                    &mut state.custom_modloader,
                );
            }

            ui.add_space(6.0);
            let _ = settings_widgets::full_width_text_input_row(
                text_ui,
                ui,
                "instance_create_modloader_version",
                "Modloader version (optional)",
                Some("Leave blank to auto/select latest when runtime installation is wired for loader versions."),
                &mut state.modloader_version,
            );

            if let Some(error) = state.error.as_deref() {
                ui.add_space(8.0);
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

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            let mut create_clicked = false;
            let mut cancel_clicked = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let create_style = ButtonOptions {
                    min_size: egui::vec2(160.0, 34.0),
                    text_color: ui.visuals().widgets.active.fg_stroke.color,
                    fill: ui.visuals().selection.bg_fill,
                    fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
                    fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().selection.stroke,
                    ..ButtonOptions::default()
                };
                let cancel_style = ButtonOptions {
                    min_size: egui::vec2(100.0, 34.0),
                    text_color,
                    fill: ui.visuals().widgets.inactive.bg_fill,
                    fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                    fill_active: ui.visuals().widgets.active.bg_fill,
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().widgets.inactive.bg_stroke,
                    ..ButtonOptions::default()
                };

                create_clicked = text_ui
                    .button(
                        ui,
                        "instance_create_confirm",
                        "Create instance",
                        &create_style,
                    )
                    .clicked();
                cancel_clicked = text_ui
                    .button(ui, "instance_create_cancel", "Cancel", &cancel_style)
                    .clicked();
            });

            if cancel_clicked {
                state.error = None;
                action = ModalAction::Cancel;
            } else if create_clicked {
                match build_draft(state) {
                    Ok(draft) => {
                        state.error = None;
                        action = ModalAction::Create(draft);
                    }
                    Err(error) => state.error = Some(error),
                }
            }
        });

    action
}

fn sync_version_catalog(
    state: &mut CreateInstanceState,
    include_snapshots_and_betas: bool,
    force_refresh: bool,
) {
    let should_refresh = force_refresh
        || state.version_catalog_include_snapshots != Some(include_snapshots_and_betas)
        || state.available_game_versions.is_empty();
    if !should_refresh {
        return;
    }

    match fetch_version_catalog(include_snapshots_and_betas) {
        Ok(catalog) => {
            state.available_game_versions = catalog.game_versions;
            state.loader_support = catalog.loader_support;
            state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);
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
        Err(err) => {
            state.version_catalog_error = Some(format!("Failed to fetch version catalog: {err}"));
            state.available_game_versions.clear();
            state.loader_support = LoaderSupportIndex::default();
            state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);
            state.selected_game_version_index = 0;
            state.game_version.clear();
        }
    }
}

fn ensure_selected_modloader_is_supported(state: &mut CreateInstanceState, game_version: &str) {
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

    state.selected_modloader = 0;
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

    if state.selected_modloader != CUSTOM_MODLOADER_INDEX
        && !state
            .loader_support
            .supports_loader(modloader.as_str(), game_version)
    {
        return Err(format!(
            "{modloader} is not available for Minecraft {game_version}."
        ));
    }

    let modloader_version = state.modloader_version.trim().to_owned();
    let thumbnail_path = {
        let trimmed = state.thumbnail_path.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };

    Ok(CreateInstanceDraft {
        name: name.to_owned(),
        thumbnail_path,
        modloader,
        game_version: game_version.to_owned(),
        modloader_version,
    })
}
