use config::{Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP};
use egui::Ui;
use installation::{
    LoaderSupportIndex, MinecraftVersionEntry, ensure_game_files, fetch_version_catalog,
};
use instances::{InstanceStore, set_instance_settings, set_instance_versions};
use modprovider::{UnifiedContentEntry, search_minecraft_content};
use std::path::Path;
use std::sync::OnceLock;
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};

use crate::ui::components::settings_widgets;

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;

#[derive(Clone, Debug)]
struct InstanceScreenState {
    running: bool,
    mod_file_path: String,
    status_message: Option<String>,
    name_input: String,
    thumbnail_input: String,
    selected_modloader: usize,
    custom_modloader: String,
    game_version_input: String,
    modloader_version_input: String,
    memory_override_enabled: bool,
    memory_override_mib: u128,
    cli_args_input: String,
    discover_query: String,
    discover_results: Vec<UnifiedContentEntry>,
    discover_types: Vec<String>,
    discover_warnings: Vec<String>,
    discover_status: Option<String>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    selected_game_version_index: usize,
    loader_support: LoaderSupportIndex,
    version_catalog_include_snapshots: Option<bool>,
    version_catalog_error: Option<String>,
}

impl InstanceScreenState {
    fn from_instance(instance: &instances::InstanceRecord, config: &Config) -> Self {
        let (selected_modloader, custom_modloader) = split_modloader(&instance.modloader);
        Self {
            running: false,
            mod_file_path: String::new(),
            status_message: None,
            name_input: instance.name.clone(),
            thumbnail_input: instance.thumbnail_path.clone().unwrap_or_default(),
            selected_modloader,
            custom_modloader,
            game_version_input: instance.game_version.clone(),
            modloader_version_input: instance.modloader_version.clone(),
            memory_override_enabled: instance.max_memory_mib.is_some(),
            memory_override_mib: instance
                .max_memory_mib
                .unwrap_or(config.default_instance_max_memory_mib()),
            cli_args_input: instance
                .cli_args
                .clone()
                .unwrap_or_else(|| config.default_instance_cli_args().to_owned()),
            discover_query: String::new(),
            discover_results: Vec::new(),
            discover_types: Vec::new(),
            discover_warnings: Vec::new(),
            discover_status: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            version_catalog_include_snapshots: None,
            version_catalog_error: None,
        }
    }
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    instances: &mut InstanceStore,
    config: &Config,
) -> bool {
    let mut instances_changed = false;
    let text_color = ui.visuals().text_color();
    let heading_style = LabelOptions {
        font_size: 30.0,
        line_height: 34.0,
        weight: 700,
        color: text_color,
        wrap: false,
        ..LabelOptions::default()
    };
    let body_style = LabelOptions {
        color: text_color,
        wrap: true,
        ..LabelOptions::default()
    };
    let mut muted_style = body_style.clone();
    muted_style.color = ui.visuals().weak_text_color();

    let Some(instance_id) = selected_instance_id else {
        let _ = text_ui.label(
            ui,
            "instance_screen_empty_heading",
            "Instance",
            &heading_style,
        );
        ui.add_space(8.0);
        let _ = text_ui.label(
            ui,
            "instance_screen_empty_body",
            "Select an instance from the left sidebar or click + to create one.",
            &body_style,
        );
        return false;
    };

    let Some(instance_snapshot) = instances.find(instance_id).cloned() else {
        let _ = text_ui.label(
            ui,
            "instance_screen_missing_heading",
            "Instance",
            &heading_style,
        );
        ui.add_space(8.0);
        let _ = text_ui.label(
            ui,
            "instance_screen_missing_body",
            "Selected instance no longer exists.",
            &body_style,
        );
        return false;
    };

    let state_id = ui.make_persistent_id(("instance_screen_state", instance_id));
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<InstanceScreenState>(state_id))
        .unwrap_or_else(|| InstanceScreenState::from_instance(&instance_snapshot, config));

    sync_version_catalog(&mut state, config.include_snapshots_and_betas(), false);
    let selected_game_version_for_loader = selected_game_version(&state).to_owned();
    ensure_selected_modloader_is_supported(&mut state, selected_game_version_for_loader.as_str());

    let installations_root = std::path::PathBuf::from(config.minecraft_installations_root());
    let instance_root_path = instances::instance_root_path(&installations_root, &instance_snapshot);

    let _ = text_ui.label(
        ui,
        ("instance_screen_heading", instance_id),
        &format!("Instance: {}", instance_snapshot.name),
        &heading_style,
    );
    ui.add_space(6.0);
    let _ = text_ui.label(
        ui,
        ("instance_screen_root", instance_id),
        &format!("Root: {}", instance_root_path.display()),
        &muted_style,
    );
    ui.add_space(12.0);

    let selected_game_version_for_runtime = selected_game_version(&state).to_owned();
    render_runtime_row(
        ui,
        text_ui,
        &mut state,
        instance_id,
        instance_root_path.as_path(),
        selected_game_version_for_runtime.as_str(),
    );
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let section_style = LabelOptions {
        font_size: 22.0,
        line_height: 26.0,
        weight: 700,
        color: text_color,
        wrap: false,
        ..LabelOptions::default()
    };
    let _ = text_ui.label(
        ui,
        ("instance_versions_heading", instance_id),
        "Instance Metadata & Versions",
        &section_style,
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
        ("instance_thumbnail_input", instance_id),
        "Thumbnail path (optional)",
        Some("Local image path for this instance."),
        &mut state.thumbnail_input,
    );
    ui.add_space(6.0);

    let refresh_style = ButtonOptions {
        min_size: egui::vec2(190.0, 30.0),
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
            ("instance_refresh_versions", instance_id),
            "Refresh version list",
            &refresh_style,
        )
        .clicked()
    {
        sync_version_catalog(&mut state, config.include_snapshots_and_betas(), true);
    }

    if let Some(catalog_error) = state.version_catalog_error.as_deref() {
        let _ = text_ui.label(
            ui,
            ("instance_version_catalog_error", instance_id),
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
            ("instance_game_version_dropdown", instance_id),
            "Minecraft game version",
            Some("Pick from available Minecraft versions."),
            &mut selected_index,
            &version_refs,
        );
        if response.changed() {
            state.selected_game_version_index = selected_index;
            if let Some(version) = state.available_game_versions.get(selected_index) {
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

    let _ = text_ui.label(
        ui,
        ("instance_modloader_label", instance_id),
        "Modloader",
        &body_style,
    );
    ui.add_space(4.0);
    let selected_game_version_for_loader = selected_game_version(&state).to_owned();
    render_modloader_selector(
        ui,
        text_ui,
        &mut state,
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

    let _ = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        ("instance_modloader_version_input", instance_id),
        "Modloader version",
        Some("Version for the selected modloader. Leave blank for latest/default."),
        &mut state.modloader_version_input,
    );
    ui.add_space(8.0);

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

    if text_ui
        .button(
            ui,
            ("instance_save_versions", instance_id),
            "Save metadata & versions",
            &action_button_style,
        )
        .clicked()
    {
        let trimmed_name = state.name_input.trim();
        if trimmed_name.is_empty() {
            state.status_message = Some("Name cannot be empty.".to_owned());
        } else {
            let modloader = selected_modloader_value(&state);
            let game_version = state.game_version_input.trim().to_owned();
            if game_version.is_empty() {
                state.status_message = Some("Minecraft game version cannot be empty.".to_owned());
            } else if modloader.trim().is_empty() {
                state.status_message = Some("Modloader cannot be empty.".to_owned());
            } else if !state
                .loader_support
                .supports_loader(modloader.as_str(), game_version.as_str())
                && state.selected_modloader != CUSTOM_MODLOADER_INDEX
            {
                state.status_message = Some(format!(
                    "{modloader} is not available for Minecraft {game_version}.",
                ));
            } else {
                let mut update_failed = None;

                if let Some(instance) = instances.find_mut(instance_id) {
                    instance.name = trimmed_name.to_owned();
                    instance.thumbnail_path = normalize_optional(state.thumbnail_input.as_str());
                } else {
                    update_failed = Some("Instance was removed before save.".to_owned());
                }

                if update_failed.is_none()
                    && let Err(err) = set_instance_versions(
                        instances,
                        instance_id,
                        modloader,
                        game_version,
                        state.modloader_version_input.trim().to_owned(),
                    )
                {
                    update_failed = Some(err.to_string());
                }

                if let Some(err) = update_failed {
                    state.status_message = Some(err);
                } else {
                    instances_changed = true;
                    state.status_message = Some("Saved metadata and version settings.".to_owned());
                }
            }
        }
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let _ = text_ui.label(
        ui,
        ("instance_settings_heading", instance_id),
        "Instance Settings",
        &section_style,
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

    let memory_slider_max = memory_slider_max_mib();
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
        let memory_override = if state.memory_override_enabled {
            Some(
                state
                    .memory_override_mib
                    .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max),
            )
        } else {
            None
        };
        let cli_override = normalize_optional(state.cli_args_input.as_str());
        match set_instance_settings(instances, instance_id, memory_override, cli_override) {
            Ok(()) => {
                instances_changed = true;
                state.status_message = Some("Saved instance settings.".to_owned());
            }
            Err(err) => state.status_message = Some(err.to_string()),
        }
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let _ = text_ui.label(
        ui,
        ("instance_mods_heading", instance_id),
        "Mods",
        &section_style,
    );
    ui.add_space(8.0);
    let _ = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        ("instance_mod_file_path", instance_id),
        "Mod file path (.jar)",
        Some("Copies the selected file into this instance's mods folder."),
        &mut state.mod_file_path,
    );
    ui.add_space(8.0);

    if text_ui
        .button(
            ui,
            ("instance_add_mod", instance_id),
            "Add mod file",
            &action_button_style,
        )
        .clicked()
    {
        match instances::add_mod_file_to_instance(
            instances,
            instance_id,
            &installations_root,
            &state.mod_file_path,
        ) {
            Ok(path) => {
                state.status_message = Some(format!("Added mod: {}", path.display()));
                state.mod_file_path.clear();
            }
            Err(err) => state.status_message = Some(err.to_string()),
        }
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let _ = text_ui.label(
        ui,
        ("instance_discover_heading", instance_id),
        "Discover Content (Modrinth + CurseForge)",
        &section_style,
    );
    ui.add_space(8.0);

    let _ = settings_widgets::full_width_text_input_row(
        text_ui,
        ui,
        ("instance_discover_query", instance_id),
        "Search mods, resource packs, shaders, datapacks, and more",
        Some("Searches both platforms and merges into one list."),
        &mut state.discover_query,
    );
    ui.add_space(8.0);

    if text_ui
        .button(
            ui,
            ("instance_discover_run", instance_id),
            "Search both platforms",
            &action_button_style,
        )
        .clicked()
    {
        match search_minecraft_content(&state.discover_query, 25) {
            Ok(search) => {
                state.discover_results = search.entries;
                state.discover_types = search.discovered_types;
                state.discover_warnings = search.warnings;
                state.discover_status =
                    Some(format!("Loaded {} entries.", state.discover_results.len()));
            }
            Err(err) => {
                state.discover_status = Some(err.to_string());
                state.discover_results.clear();
                state.discover_types.clear();
                state.discover_warnings.clear();
            }
        }
    }

    if let Some(status) = state.discover_status.as_deref() {
        let _ = text_ui.label(
            ui,
            ("instance_discover_status", instance_id),
            status,
            &muted_style,
        );
    }
    for (warning_index, warning) in state.discover_warnings.iter().enumerate() {
        let _ = text_ui.label(
            ui,
            ("instance_discover_warning", instance_id, warning_index),
            warning,
            &LabelOptions {
                color: ui.visuals().warn_fg_color,
                wrap: true,
                ..LabelOptions::default()
            },
        );
    }

    if !state.discover_types.is_empty() {
        let joined = state.discover_types.join(", ");
        let _ = text_ui.label(
            ui,
            ("instance_discover_types", instance_id),
            &format!("Discovered content types: {joined}"),
            &muted_style,
        );
    }

    ui.add_space(8.0);
    egui::ScrollArea::vertical()
        .id_salt(("instance_discover_results_scroll", instance_id))
        .max_height(320.0)
        .show(ui, |ui| {
            for (entry_index, entry) in state.discover_results.iter().enumerate() {
                render_discovery_entry(
                    ui,
                    text_ui,
                    ("instance_discover_entry", instance_id, entry_index),
                    entry,
                );
                ui.add_space(6.0);
            }
        });

    if let Some(status_message) = state.status_message.as_deref() {
        ui.add_space(10.0);
        let status_color = if status_message.starts_with("Added mod:")
            || status_message.starts_with("Saved")
            || status_message.starts_with("Prepared")
            || status_message.starts_with("Launch")
            || status_message.starts_with("Stop")
        {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().error_fg_color
        };
        let _ = text_ui.label(
            ui,
            ("instance_status_message", instance_id),
            status_message,
            &LabelOptions {
                color: status_color,
                wrap: true,
                ..LabelOptions::default()
            },
        );
    }

    ui.ctx().data_mut(|d| d.insert_temp(state_id, state));
    instances_changed
}

fn render_runtime_row(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut InstanceScreenState,
    id: &str,
    instance_root: &Path,
    game_version: &str,
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

    ui.horizontal(|ui| {
        let action_label = if state.running { "Stop" } else { "Launch" };
        if text_ui
            .button(
                ui,
                ("instance_runtime_toggle", id),
                action_label,
                &button_style,
            )
            .clicked()
        {
            if state.running {
                state.running = false;
                state.status_message = Some("Stop requested for this instance.".to_owned());
            } else if game_version.trim().is_empty() {
                state.status_message =
                    Some("Cannot launch: choose a Minecraft game version first.".to_owned());
            } else {
                match ensure_game_files(instance_root, game_version) {
                    Ok(setup) => {
                        state.running = true;
                        state.status_message = Some(format!(
                            "Prepared Minecraft {} in {} ({} file(s) downloaded).",
                            game_version,
                            instance_root.display(),
                            setup.downloaded_files
                        ));
                    }
                    Err(err) => {
                        state.running = false;
                        state.status_message = Some(format!("Failed to prepare game files: {err}"));
                    }
                }
            }
        }
        ui.add_space(10.0);
        let _ = text_ui.label(
            ui,
            ("instance_runtime_state", id),
            if state.running {
                "Runtime state: Running"
            } else {
                "Runtime state: Stopped"
            },
            &muted_style,
        );
    });
}

fn render_modloader_selector(
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

fn split_modloader(modloader: &str) -> (usize, String) {
    for (index, option) in MODLOADER_OPTIONS.iter().enumerate() {
        if option.eq_ignore_ascii_case(modloader.trim()) {
            return (index, String::new());
        }
    }

    (CUSTOM_MODLOADER_INDEX, modloader.trim().to_owned())
}

fn selected_modloader_value(state: &InstanceScreenState) -> String {
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

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn sync_version_catalog(
    state: &mut InstanceScreenState,
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
        Err(err) => {
            state.version_catalog_error = Some(format!("Failed to fetch version catalog: {err}"));
            state.available_game_versions.clear();
            state.loader_support = LoaderSupportIndex::default();
            state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);
            state.selected_game_version_index = 0;
            state.game_version_input.clear();
        }
    }
}

fn selected_game_version(state: &InstanceScreenState) -> &str {
    state
        .available_game_versions
        .get(state.selected_game_version_index)
        .map(|entry| entry.id.as_str())
        .unwrap_or_else(|| state.game_version_input.as_str())
}

fn ensure_selected_modloader_is_supported(state: &mut InstanceScreenState, game_version: &str) {
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

fn render_discovery_entry(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    entry: &UnifiedContentEntry,
) {
    egui::Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                render_badge(
                    ui,
                    text_ui,
                    (id_source, "source_badge"),
                    entry.source.label(),
                );
                ui.add_space(4.0);
                render_badge(
                    ui,
                    text_ui,
                    (id_source, "type_badge"),
                    entry.content_type.as_str(),
                );
                ui.add_space(8.0);
                let _ = text_ui.label(
                    ui,
                    (id_source, "name"),
                    entry.name.as_str(),
                    &LabelOptions {
                        font_size: 19.0,
                        line_height: 24.0,
                        weight: 700,
                        color: ui.visuals().text_color(),
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
            });

            if !entry.summary.trim().is_empty() {
                ui.add_space(4.0);
                let _ = text_ui.label(
                    ui,
                    (id_source, "summary"),
                    entry.summary.as_str(),
                    &LabelOptions {
                        color: ui.visuals().weak_text_color(),
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
            }

            if let Some(project_url) = entry.project_url.as_deref() {
                ui.add_space(6.0);
                let open_style = ButtonOptions {
                    min_size: egui::vec2(140.0, 30.0),
                    text_color: ui.visuals().widgets.active.fg_stroke.color,
                    fill: ui.visuals().selection.bg_fill,
                    fill_hovered: ui.visuals().selection.bg_fill.gamma_multiply(1.1),
                    fill_active: ui.visuals().selection.bg_fill.gamma_multiply(0.9),
                    fill_selected: ui.visuals().selection.bg_fill,
                    stroke: ui.visuals().selection.stroke,
                    ..ButtonOptions::default()
                };
                if text_ui
                    .button(ui, (id_source, "open"), "Open project page", &open_style)
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::same_tab(project_url));
                }
            }
        });
}

fn render_badge(ui: &mut Ui, text_ui: &mut TextUi, id_source: impl std::hash::Hash, text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            let _ = text_ui.label(
                ui,
                id_source,
                text,
                &LabelOptions {
                    font_size: 14.0,
                    line_height: 18.0,
                    color: ui.visuals().text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
            );
        });
}

fn memory_slider_max_mib() -> u128 {
    static CACHED: OnceLock<u128> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let total_mib = detect_total_memory_mib().unwrap_or(FALLBACK_TOTAL_MEMORY_MIB);
        total_mib
            .saturating_sub(RESERVED_SYSTEM_MEMORY_MIB)
            .max(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN)
    })
}

#[cfg(target_os = "linux")]
fn detect_total_memory_mib() -> Option<u128> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u128>().ok()?;
    Some(kib / 1024)
}

#[cfg(target_os = "windows")]
fn detect_total_memory_mib() -> Option<u128> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };

    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }

    Some((status.ullTotalPhys as u128) / (1024 * 1024))
}

#[cfg(target_os = "macos")]
fn detect_total_memory_mib() -> Option<u128> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let bytes = String::from_utf8(output.stdout).ok()?;
    let bytes = bytes.trim().parse::<u128>().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn detect_total_memory_mib() -> Option<u128> {
    None
}
