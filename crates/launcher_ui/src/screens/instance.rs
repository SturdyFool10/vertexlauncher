use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    JavaRuntimeVersion,
};
use egui::Ui;
use installation::{
    DownloadPolicy, GameSetupResult, InstallProgress, InstallProgressCallback, InstallStage,
    LaunchRequest, LaunchResult, LoaderSupportIndex, MinecraftVersionEntry, VersionCatalog,
    ensure_game_files, ensure_openjdk_runtime, fetch_version_catalog_with_refresh,
    is_instance_running, launch_instance, running_instance_for_account, stop_running_instance,
};
use instances::{InstanceStore, set_instance_settings, set_instance_versions};
use modprovider::{UnifiedContentEntry, UnifiedSearchResult, search_minecraft_content};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};

use crate::app::tokio_runtime;
use crate::screens::LaunchAuthContext;
use crate::ui::components::settings_widgets;
use crate::{console, install_activity, notification};

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
    discover_in_flight: bool,
    discover_results_tx: Option<mpsc::Sender<(String, Result<UnifiedSearchResult, String>)>>,
    discover_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, Result<UnifiedSearchResult, String>)>>>>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    selected_game_version_index: usize,
    loader_support: LoaderSupportIndex,
    version_catalog_include_snapshots: Option<bool>,
    version_catalog_error: Option<String>,
    version_catalog_in_flight: bool,
    version_catalog_results_tx: Option<mpsc::Sender<(bool, Result<VersionCatalog, String>)>>,
    version_catalog_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(bool, Result<VersionCatalog, String>)>>>>,
    runtime_prepare_in_flight: bool,
    runtime_prepare_results_tx:
        Option<mpsc::Sender<(String, String, Result<RuntimePrepareOutcome, String>)>>,
    runtime_prepare_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<RuntimePrepareOutcome, String>)>>>>,
    runtime_progress_tx: Option<mpsc::Sender<InstallProgress>>,
    runtime_progress_rx: Option<Arc<Mutex<mpsc::Receiver<InstallProgress>>>>,
    runtime_latest_progress: Option<InstallProgress>,
    runtime_last_notification_at: Option<Instant>,
    launch_username: Option<String>,
}

#[derive(Clone, Debug)]
struct RuntimePrepareOutcome {
    setup: GameSetupResult,
    configured_java: Option<(JavaRuntimeVersion, String)>,
    launch: LaunchResult,
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
            discover_in_flight: false,
            discover_results_tx: None,
            discover_results_rx: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            version_catalog_include_snapshots: None,
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_results_tx: None,
            version_catalog_results_rx: None,
            runtime_prepare_in_flight: false,
            runtime_prepare_results_tx: None,
            runtime_prepare_results_rx: None,
            runtime_progress_tx: None,
            runtime_progress_rx: None,
            runtime_latest_progress: None,
            runtime_last_notification_at: None,
            launch_username: None,
        }
    }
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    active_username: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    active_account_owns_minecraft: bool,
    instances: &mut InstanceStore,
    config: &mut Config,
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

    poll_background_tasks(&mut state, config);
    sync_version_catalog(&mut state, config.include_snapshots_and_betas(), false);
    if state.version_catalog_in_flight
        || state.discover_in_flight
        || state.runtime_prepare_in_flight
    {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
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
    let external_activity =
        install_activity::snapshot().filter(|activity| activity.instance_id == state.name_input);
    let external_install_active = external_activity
        .as_ref()
        .is_some_and(|activity| !matches!(activity.stage, InstallStage::Complete));
    render_runtime_row(
        ui,
        text_ui,
        &mut state,
        instance_id,
        instance_root_path.as_path(),
        selected_game_version_for_runtime.as_str(),
        config,
        external_install_active,
        active_username,
        active_launch_auth,
        active_account_owns_minecraft,
    );
    render_install_feedback(
        ui,
        text_ui,
        instance_id,
        state.runtime_latest_progress.as_ref(),
        external_activity.as_ref(),
        state.runtime_prepare_in_flight,
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
            } else if support_catalog_ready(&state)
                && !state
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

    let mut discover_clicked = false;
    ui.add_enabled_ui(!state.discover_in_flight, |ui| {
        discover_clicked = text_ui
            .button(
                ui,
                ("instance_discover_run", instance_id),
                "Search both platforms",
                &action_button_style,
            )
            .clicked();
    });
    if discover_clicked {
        let query = state.discover_query.clone();
        request_discovery_search(&mut state, query);
    }
    if state.discover_in_flight {
        ui.horizontal(|ui| {
            ui.spinner();
            let _ = text_ui.label(
                ui,
                ("instance_discover_loading", instance_id),
                "Searching Modrinth and CurseForge...",
                &muted_style,
            );
        });
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

fn render_install_feedback(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    local_progress: Option<&InstallProgress>,
    external_activity: Option<&install_activity::InstallActivitySnapshot>,
    runtime_prepare_in_flight: bool,
) {
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
        ui.add(
            egui::ProgressBar::new(fraction)
                .show_percentage()
                .text(progress_label),
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
        ui.add(
            egui::ProgressBar::new(0.0)
                .animate(true)
                .show_percentage()
                .text("Starting installation..."),
        );
        return;
    }

    if let Some(activity) = external_activity {
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
        ui.add(
            egui::ProgressBar::new(fraction)
                .show_percentage()
                .text(progress_label),
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

fn render_runtime_row(
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
    let instance_root_display = instance_root.display().to_string();
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
    let launch_access_token = active_launch_auth.map(|auth| auth.access_token.clone());
    let launch_xuid = active_launch_auth.and_then(|auth| auth.xuid.clone());
    let launch_user_type = active_launch_auth.map(|auth| auth.user_type.clone());
    let account_running_root = launch_account
        .as_deref()
        .and_then(running_instance_for_account);
    let launch_disabled_for_account = !state.running
        && account_running_root
            .as_deref()
            .is_some_and(|running_root| running_root != instance_root_display.as_str());
    let launch_disabled_for_missing_ownership = !state.running && !active_account_owns_minecraft;
    let launch_disabled = launch_disabled_for_account || launch_disabled_for_missing_ownership;

    ui.horizontal(|ui| {
        if !state.runtime_prepare_in_flight && !external_install_active {
            let action_label = if state.running { "Stop" } else { "Launch" };
            let response = ui
                .add_enabled_ui(!launch_disabled, |ui| {
                    text_ui.button(
                        ui,
                        ("instance_runtime_toggle", id),
                        action_label,
                        &button_style,
                    )
                })
                .inner;
            if response.clicked() {
                if state.running {
                    if stop_running_instance(instance_root) {
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
                    state.launch_username = launch_account.clone();
                    request_runtime_prepare(
                        state,
                        instance_root.to_path_buf(),
                        game_version.trim().to_owned(),
                        selected_modloader_value(state),
                        normalize_optional(state.modloader_version_input.as_str()),
                        recommended_java_runtime(game_version),
                        choose_java_executable(config, game_version),
                        config.download_max_concurrent(),
                        config.parsed_download_speed_limit_bps(),
                        max_memory_mib,
                        extra_jvm_args,
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
                "Runtime state: Installing"
            } else if state.running {
                "Runtime state: Running"
            } else {
                "Runtime state: Stopped"
            },
            &muted_style,
        );
        if state.runtime_prepare_in_flight || external_install_active {
            ui.add_space(8.0);
            ui.spinner();
        }
        if launch_disabled_for_account {
            let blocked_account = launch_account.as_deref().unwrap_or("this account");
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
    });
    if state.running && !is_instance_running(instance_root) {
        state.running = false;
        state.status_message = Some("Minecraft process exited.".to_owned());
    }
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

fn poll_background_tasks(state: &mut InstanceScreenState, config: &mut Config) {
    poll_version_catalog(state);
    poll_discovery_search(state);
    poll_runtime_progress(state);
    poll_runtime_prepare(state, config);
}

fn sync_version_catalog(
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

fn ensure_version_catalog_channel(state: &mut InstanceScreenState) {
    if state.version_catalog_results_tx.is_some() && state.version_catalog_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(bool, Result<VersionCatalog, String>)>();
    state.version_catalog_results_tx = Some(tx);
    state.version_catalog_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn apply_version_catalog(
    state: &mut InstanceScreenState,
    include_snapshots_and_betas: bool,
    catalog: VersionCatalog,
) {
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

fn apply_version_catalog_error(
    state: &mut InstanceScreenState,
    include_snapshots_and_betas: bool,
    error: &str,
) {
    state.version_catalog_error = Some(format!("Failed to fetch version catalog: {error}"));
    state.available_game_versions.clear();
    state.loader_support = LoaderSupportIndex::default();
    state.version_catalog_include_snapshots = Some(include_snapshots_and_betas);
    state.selected_game_version_index = 0;
    state.game_version_input.clear();
}

fn poll_version_catalog(state: &mut InstanceScreenState) {
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

fn ensure_discovery_channel(state: &mut InstanceScreenState) {
    if state.discover_results_tx.is_some() && state.discover_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Result<UnifiedSearchResult, String>)>();
    state.discover_results_tx = Some(tx);
    state.discover_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_discovery_search(state: &mut InstanceScreenState, query: String) {
    let query = query.trim().to_owned();
    if query.is_empty() {
        state.discover_status = Some("Search query cannot be empty.".to_owned());
        state.discover_results.clear();
        state.discover_types.clear();
        state.discover_warnings.clear();
        return;
    }
    if state.discover_in_flight {
        return;
    }

    ensure_discovery_channel(state);
    let Some(tx) = state.discover_results_tx.as_ref().cloned() else {
        return;
    };

    state.discover_in_flight = true;
    state.discover_status = Some(format!("Searching for \"{query}\"..."));
    let query_for_search = query.clone();
    let query_for_result = query.clone();
    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            search_minecraft_content(query_for_search.as_str(), 25).map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| format!("discover task join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send((query_for_result, result));
    });
}

fn poll_discovery_search(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.discover_results_rx.as_ref() {
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
        state.discover_results_tx = None;
        state.discover_results_rx = None;
        state.discover_in_flight = false;
    }

    for (query, result) in updates {
        state.discover_in_flight = false;
        match result {
            Ok(search) => {
                state.discover_results = search.entries;
                state.discover_types = search.discovered_types;
                state.discover_warnings = search.warnings;
                state.discover_status = Some(format!(
                    "Loaded {} entries for \"{query}\".",
                    state.discover_results.len()
                ));
            }
            Err(err) => {
                state.discover_status = Some(format!("Search failed: {err}"));
                state.discover_results.clear();
                state.discover_types.clear();
                state.discover_warnings.clear();
            }
        }
    }
}

fn ensure_runtime_prepare_channel(state: &mut InstanceScreenState) {
    if state.runtime_prepare_results_tx.is_some() && state.runtime_prepare_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, String, Result<RuntimePrepareOutcome, String>)>();
    state.runtime_prepare_results_tx = Some(tx);
    state.runtime_prepare_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn ensure_runtime_progress_channel(state: &mut InstanceScreenState) {
    if state.runtime_progress_tx.is_some() && state.runtime_progress_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<InstallProgress>();
    state.runtime_progress_tx = Some(tx);
    state.runtime_progress_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_runtime_prepare(
    state: &mut InstanceScreenState,
    instance_root: PathBuf,
    game_version: String,
    modloader: String,
    modloader_version: Option<String>,
    required_java_runtime: Option<JavaRuntimeVersion>,
    java_executable: Option<String>,
    download_max_concurrent: u32,
    download_speed_limit_bps: Option<u64>,
    max_memory_mib: u128,
    extra_jvm_args: Option<String>,
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
    state.status_message = Some(format!("Preparing Minecraft {game_version}..."));
    let instance_root_display = instance_root.display().to_string();
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
    let player_name_for_task = player_name;
    let player_uuid_for_task = player_uuid;
    let access_token_for_task = access_token;
    let xuid_for_task = xuid;
    let user_type_for_task = user_type;
    let launch_account_name_for_task = launch_account_name;
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
    } else if let Some(runtime) = required_java_runtime {
        format!("auto-provisioned OpenJDK {}", runtime.major())
    } else {
        "java from PATH".to_owned()
    };
    let username = launch_account_name_for_task
        .as_deref()
        .or(player_name_for_task.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Player");
    let tab_id = console::ensure_instance_tab(state.name_input.as_str(), username);
    console::push_line_to_tab(
        tab_id.as_str(),
        format!(
            "Launch request: root={} | Minecraft {} | {}{} | max memory={} MiB | {}",
            instance_root_display,
            game_version_for_task,
            modloader_for_task,
            modloader_version_display,
            max_memory_mib.max(512),
            java_launch_mode
        ),
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
            } else if let Some(runtime) = required_java_runtime {
                let installed = ensure_openjdk_runtime(runtime.major()).map_err(|err| {
                    format!("failed to auto-install OpenJDK {}: {err}", runtime.major())
                })?;
                let installed = installed.display().to_string();
                configured_java = Some((runtime, installed.clone()));
                installed
            } else {
                "java".to_owned()
            };
            ensure_game_files(
                instance_root.as_path(),
                game_version_for_task.as_str(),
                modloader_for_task.as_str(),
                modloader_version_for_task.as_deref(),
                Some(java_path.as_str()),
                &download_policy,
                Some(progress_callback),
            )
            .and_then(|setup| {
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
                };
                launch_instance(&launch_request).map(|launch| RuntimePrepareOutcome {
                    setup,
                    configured_java,
                    launch,
                })
            })
            .map_err(|err| err.to_string())
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

fn poll_runtime_progress(state: &mut InstanceScreenState) {
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

fn poll_runtime_prepare(state: &mut InstanceScreenState, config: &mut Config) {
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
        state.runtime_prepare_results_tx = None;
        state.runtime_prepare_results_rx = None;
        state.runtime_prepare_in_flight = false;
        state.runtime_progress_tx = None;
        state.runtime_progress_rx = None;
    }

    for (game_version, instance_root_display, result) in updates {
        state.runtime_prepare_in_flight = false;
        match result {
            Ok(outcome) => {
                if let Some((runtime, path)) = outcome.configured_java {
                    config.set_java_runtime_path(runtime, Some(path));
                }
                let setup = outcome.setup;
                let launch = outcome.launch;
                let username = state
                    .launch_username
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("Player");
                let tab_id = console::ensure_instance_tab(state.name_input.as_str(), username);
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

fn should_emit_progress_notification(
    state: &InstanceScreenState,
    _progress: &InstallProgress,
) -> bool {
    match state.runtime_last_notification_at {
        Some(last) => last.elapsed() >= Duration::from_millis(250),
        None => true,
    }
}

fn progress_fraction(progress: &InstallProgress) -> f32 {
    if progress.total_files > 0 {
        return (progress.downloaded_files as f32 / progress.total_files as f32).clamp(0.0, 1.0);
    }
    if let Some(total_bytes) = progress.total_bytes
        && total_bytes > 0
    {
        return (progress.downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
    }
    if matches!(progress.stage, InstallStage::Complete) {
        1.0
    } else {
        0.0
    }
}

fn progress_fraction_from_activity(progress: &install_activity::InstallActivitySnapshot) -> f32 {
    if progress.total_files > 0 {
        return (progress.downloaded_files as f32 / progress.total_files as f32).clamp(0.0, 1.0);
    }
    if let Some(total_bytes) = progress.total_bytes
        && total_bytes > 0
    {
        return (progress.downloaded_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0);
    }
    if matches!(progress.stage, InstallStage::Complete) {
        1.0
    } else {
        0.0
    }
}

fn stage_label(stage: InstallStage) -> &'static str {
    match stage {
        InstallStage::PreparingFolders => "Preparing folders",
        InstallStage::ResolvingMetadata => "Resolving metadata",
        InstallStage::DownloadingCore => "Downloading core files",
        InstallStage::InstallingModloader => "Installing modloader",
        InstallStage::Complete => "Complete",
    }
}

fn format_bytes(bytes: u64) -> String {
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

fn selected_game_version(state: &InstanceScreenState) -> &str {
    state
        .available_game_versions
        .get(state.selected_game_version_index)
        .map(|entry| entry.id.as_str())
        .unwrap_or_else(|| state.game_version_input.as_str())
}

fn choose_java_executable(config: &Config, game_version: &str) -> Option<String> {
    if let Some(runtime) = recommended_java_runtime(game_version)
        && let Some(path) = config.java_runtime_path(runtime)
    {
        let trimmed = path.trim();
        if !trimmed.is_empty() && Path::new(trimmed).exists() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

fn recommended_java_runtime(game_version: &str) -> Option<JavaRuntimeVersion> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        return Some(JavaRuntimeVersion::Java21);
    }
    if minor <= 16 {
        return Some(JavaRuntimeVersion::Java8);
    }
    if minor == 17 {
        return Some(JavaRuntimeVersion::Java16);
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        return Some(JavaRuntimeVersion::Java21);
    }
    Some(JavaRuntimeVersion::Java17)
}

fn ensure_selected_modloader_is_supported(state: &mut InstanceScreenState, game_version: &str) {
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

    state.selected_modloader = 0;
}

fn support_catalog_ready(state: &InstanceScreenState) -> bool {
    state.version_catalog_include_snapshots.is_some() && state.version_catalog_error.is_none()
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
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = "/proc/meminfo", context = "detect total memory");
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
