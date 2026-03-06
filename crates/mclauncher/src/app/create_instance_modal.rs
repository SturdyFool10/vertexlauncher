use std::path::Path;

use eframe::egui;
use installation::{
    LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry, fetch_version_catalog,
};
use textui::{LabelOptions, TextUi};

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
    loader_versions: LoaderVersionIndex,
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
            loader_versions: LoaderVersionIndex::default(),
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
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_max_width = (viewport_rect.width() * 0.90).max(1.0);
    let modal_max_height = (viewport_rect.height() * 0.90).max(1.0);
    let modal_pos_x = (viewport_rect.center().x - modal_max_width * 0.5).clamp(
        viewport_rect.left(),
        viewport_rect.right() - modal_max_width,
    );
    let modal_pos_y = (viewport_rect.center().y - modal_max_height * 0.5).clamp(
        viewport_rect.top(),
        viewport_rect.bottom() - modal_max_height,
    );
    let modal_pos = egui::pos2(modal_pos_x, modal_pos_y);
    let modal_size = egui::vec2(modal_max_width, modal_max_height);
    let window_fill = {
        let base = ctx.style().visuals.window_fill;
        egui::Color32::from_rgba_premultiplied(base.r(), base.g(), base.b(), 255)
    };

    egui::Window::new("Create Instance")
        .id(egui::Id::new("create_instance_modal_window"))
        .fixed_pos(modal_pos)
        .fixed_size(modal_size)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .title_bar(false)
        .hscroll(false)
        .vscroll(true)
        .constrain(true)
        .constrain_to(viewport_rect)
        .frame(
            egui::Frame::new()
                .fill(window_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ctx.style().visuals.widgets.hovered.bg_stroke.color,
                ))
                .corner_radius(egui::CornerRadius::same(14))
                .inner_margin(egui::Margin::same(14)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
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

            let _ = text_ui.label(ui, "instance_create_heading", "Create Instance", &heading_style);
            let _ = text_ui.label(
                ui,
                "instance_create_subheading",
                "Choose name, thumbnail, modloader, and versions.",
                &body_style,
            );
            ui.add_space(6.0);
            render_thumbnail_picker(ui, text_ui, state);
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

            if settings_widgets::full_width_button(
                text_ui,
                ui,
                "instance_create_refresh_versions",
                "Refresh version list",
                ui.available_width().clamp(1.0, 200.0),
                false,
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

            ui.add_space(6.0);

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
            let available_modloader_versions = selected_modloader_versions(
                state,
                selected_game_version.as_str(),
            );
            if !available_modloader_versions.is_empty()
                && state.selected_modloader != CUSTOM_MODLOADER_INDEX
                && state.selected_modloader != 0
            {
                let mut modloader_version_options: Vec<String> =
                    Vec::with_capacity(available_modloader_versions.len() + 1);
                modloader_version_options.push("Latest available".to_owned());
                modloader_version_options.extend_from_slice(available_modloader_versions);
                let option_refs: Vec<&str> = modloader_version_options
                    .iter()
                    .map(String::as_str)
                    .collect();
                let mut selected_index = if state.modloader_version.trim().is_empty() {
                    0
                } else {
                    modloader_version_options
                        .iter()
                        .position(|entry| entry == state.modloader_version.trim())
                        .unwrap_or(0)
                };
                let changed = settings_widgets::full_width_dropdown_row(
                    text_ui,
                    ui,
                    "instance_create_modloader_version_dropdown",
                    "Modloader version",
                    Some("Fetched and cached once per day. Pick Latest available for automatic selection."),
                    &mut selected_index,
                    &option_refs,
                )
                .changed();
                if changed {
                    if selected_index == 0 {
                        state.modloader_version.clear();
                    } else if let Some(selected) = modloader_version_options.get(selected_index) {
                        state.modloader_version = selected.clone();
                    }
                }
            } else {
                let _ = settings_widgets::full_width_text_input_row(
                    text_ui,
                    ui,
                    "instance_create_modloader_version",
                    "Modloader version (optional)",
                    Some("Leave blank to auto/select latest when runtime installation is wired for loader versions."),
                    &mut state.modloader_version,
                );
            }

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
            let action_width = ui.available_width();
            let compact_actions = action_width < 320.0;

            if compact_actions {
                create_clicked = settings_widgets::full_width_button(
                    text_ui,
                    ui,
                    "instance_create_confirm",
                    "Create instance",
                    action_width,
                    true,
                )
                    .clicked();
                ui.add_space(6.0);
                cancel_clicked = settings_widgets::full_width_button(
                    text_ui,
                    ui,
                    "instance_create_cancel",
                    "Cancel",
                    action_width,
                    false,
                )
                    .clicked();
            } else {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    create_clicked = settings_widgets::full_width_button(
                        text_ui,
                        ui,
                        "instance_create_confirm",
                        "Create instance",
                        160.0,
                        true,
                    )
                        .clicked();
                    cancel_clicked = settings_widgets::full_width_button(
                        text_ui,
                        ui,
                        "instance_create_cancel",
                        "Cancel",
                        100.0,
                        false,
                    )
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
            state.loader_versions = catalog.loader_versions;
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
            state.loader_versions = LoaderVersionIndex::default();
            state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);
            state.selected_game_version_index = 0;
            state.game_version.clear();
        }
    }
}

fn render_thumbnail_picker(
    ui: &mut egui::Ui,
    text_ui: &mut TextUi,
    state: &mut CreateInstanceState,
) {
    const THUMBNAIL_PREVIEW_SIZE: f32 = 150.0;
    const PREVIEW_FRAME_PADDING: f32 = 8.0;
    let preview_inner_width = ui.available_width().clamp(64.0, THUMBNAIL_PREVIEW_SIZE);
    let preview_height = preview_inner_width;
    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(PREVIEW_FRAME_PADDING.round() as i8))
        .show(ui, |ui| {
            ui.set_width(preview_inner_width);
            ui.set_min_width(preview_inner_width);
            ui.set_max_width(preview_inner_width);
            ui.set_height(preview_height);
            let trimmed = state.thumbnail_path.trim();
            if trimmed.is_empty() {
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(
                            egui::RichText::new("No thumbnail selected")
                                .color(ui.visuals().weak_text_color()),
                        );
                    },
                );
                return;
            }

            let path = Path::new(trimmed);
            if !path.is_file() {
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(
                            egui::RichText::new("Thumbnail file was not found")
                                .color(ui.visuals().weak_text_color()),
                        );
                    },
                );
                return;
            }

            let image_uri = file_uri_from_path(path);
            let image = egui::Image::from_uri(image_uri)
                .maintain_aspect_ratio(true)
                .max_size(egui::vec2(preview_inner_width, preview_height));
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    let _ = ui.add(image);
                },
            );
        });

    ui.add_space(6.0);
    let button_count = if state.thumbnail_path.trim().is_empty() {
        1.0
    } else {
        2.0
    };
    let available = ui.available_width().max(1.0);
    let button_spacing = 6.0 * (button_count - 1.0);
    let button_width = ((available - button_spacing) / button_count).clamp(1.0, 180.0);

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        if settings_widgets::full_width_button(
            text_ui,
            ui,
            "instance_create_thumbnail_browse",
            "Choose thumbnail...",
            button_width,
            false,
        )
        .clicked()
            && let Ok(picked) =
                std::panic::catch_unwind(|| pick_thumbnail_path(state.thumbnail_path.trim()))
            && let Some(path) = picked
        {
            state.thumbnail_path = path;
        }

        if !state.thumbnail_path.trim().is_empty()
            && settings_widgets::full_width_button(
                text_ui,
                ui,
                "instance_create_thumbnail_clear",
                "Clear thumbnail",
                button_width,
                false,
            )
            .clicked()
        {
            state.thumbnail_path.clear();
        }
    });
}

fn file_uri_from_path(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy())
}

fn pick_thumbnail_path(current_path: &str) -> Option<String> {
    let mut dialog =
        rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "webp", "gif", "bmp"]);
    let current = Path::new(current_path);
    if current.is_file() {
        if let Some(parent) = current.parent() {
            dialog = dialog.set_directory(parent);
        }
    } else if current.is_dir() {
        dialog = dialog.set_directory(current);
    }
    dialog
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
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
