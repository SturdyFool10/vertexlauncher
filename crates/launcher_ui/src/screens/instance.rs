use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    JavaRuntimeVersion,
};
use curseforge::Client as CurseForgeClient;
use egui::Ui;
use installation::{
    DownloadPolicy, GameSetupResult, InstallProgress, InstallProgressCallback, InstallStage,
    LaunchRequest, LaunchResult, LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry,
    VersionCatalog, ensure_game_files, ensure_openjdk_runtime, fetch_loader_versions_for_game,
    fetch_version_catalog_with_refresh, is_instance_running_for_account, launch_instance,
    running_instance_for_account, stop_running_instance_for_account,
};
use instances::{InstanceStore, set_instance_settings, set_instance_versions};
use modprovider::{ContentSource, UnifiedContentEntry, search_minecraft_content};
use modrinth::Client as ModrinthClient;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};

use crate::app::tokio_runtime;
use crate::screens::content_browser::InstalledContentIdentity;
use crate::screens::{AppScreen, LaunchAuthContext};
use crate::ui::{
    components::{icon_button, remote_tiled_image, settings_widgets},
    style,
};
use crate::{assets, console, install_activity, notification};

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;
const INSTALLED_CONTENT_SCROLLBAR_RESERVE: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstalledContentTab {
    Mods,
    ResourcePacks,
    ShaderPacks,
    DataPacks,
}

impl InstalledContentTab {
    const ALL: [InstalledContentTab; 4] = [
        InstalledContentTab::Mods,
        InstalledContentTab::ResourcePacks,
        InstalledContentTab::ShaderPacks,
        InstalledContentTab::DataPacks,
    ];

    fn label(self) -> &'static str {
        match self {
            InstalledContentTab::Mods => "Mods",
            InstalledContentTab::ResourcePacks => "Resource Packs",
            InstalledContentTab::ShaderPacks => "Shader Packs",
            InstalledContentTab::DataPacks => "DataPacks",
        }
    }

    fn folder_name(self) -> &'static str {
        match self {
            InstalledContentTab::Mods => "mods",
            InstalledContentTab::ResourcePacks => "resourcepacks",
            InstalledContentTab::ShaderPacks => "shaderpacks",
            InstalledContentTab::DataPacks => "datapacks",
        }
    }

    fn content_type_key(self) -> &'static str {
        match self {
            InstalledContentTab::Mods => "mod",
            InstalledContentTab::ResourcePacks => "resource pack",
            InstalledContentTab::ShaderPacks => "shader",
            InstalledContentTab::DataPacks => "data pack",
        }
    }
}

#[derive(Clone, Debug)]
struct InstalledContentFile {
    file_name: String,
    file_path: PathBuf,
    lookup_query: String,
    lookup_key: String,
    managed_identity: Option<InstalledContentIdentity>,
}

#[derive(Debug, Clone, Default)]
pub struct InstanceScreenOutput {
    pub instances_changed: bool,
    pub requested_screen: Option<AppScreen>,
}

#[derive(Clone, Debug)]
struct InstanceScreenState {
    running: bool,
    status_message: Option<String>,
    name_input: String,
    description_input: String,
    thumbnail_input: String,
    selected_modloader: usize,
    custom_modloader: String,
    game_version_input: String,
    modloader_version_input: String,
    memory_override_enabled: bool,
    memory_override_mib: u128,
    cli_args_input: String,
    java_override_enabled: bool,
    java_override_runtime_major: Option<u8>,
    selected_content_tab: InstalledContentTab,
    content_metadata_cache: HashMap<String, Option<UnifiedContentEntry>>,
    content_lookup_in_flight: HashSet<String>,
    content_lookup_results_tx: Option<mpsc::Sender<(String, Option<UnifiedContentEntry>)>>,
    content_lookup_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, Option<UnifiedContentEntry>)>>>>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    selected_game_version_index: usize,
    loader_support: LoaderSupportIndex,
    loader_versions: LoaderVersionIndex,
    modloader_versions_cache: BTreeMap<String, Vec<String>>,
    modloader_versions_in_flight: HashSet<String>,
    modloader_versions_results_tx: Option<mpsc::Sender<(String, Result<Vec<String>, String>)>>,
    modloader_versions_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, Result<Vec<String>, String>)>>>>,
    modloader_versions_status_key: Option<String>,
    modloader_versions_status: Option<String>,
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
    runtime_prepare_instance_root: Option<String>,
    runtime_prepare_user_key: Option<String>,
    show_settings_modal: bool,
    launch_username: Option<String>,
    launch_user_key: Option<String>,
}

#[derive(Clone, Debug)]
struct RuntimePrepareOutcome {
    operation: RuntimePrepareOperation,
    setup: GameSetupResult,
    configured_java: Option<(u8, String)>,
    launch: Option<LaunchResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimePrepareOperation {
    Launch,
    ReinstallProfile,
}

impl InstanceScreenState {
    fn from_instance(instance: &instances::InstanceRecord, config: &Config) -> Self {
        let (selected_modloader, custom_modloader) = split_modloader(&instance.modloader);
        Self {
            running: false,
            status_message: None,
            name_input: instance.name.clone(),
            description_input: instance.description.clone().unwrap_or_default(),
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
            java_override_enabled: instance.java_override_enabled,
            java_override_runtime_major: instance.java_override_runtime_major,
            selected_content_tab: InstalledContentTab::Mods,
            content_metadata_cache: HashMap::new(),
            content_lookup_in_flight: HashSet::new(),
            content_lookup_results_tx: None,
            content_lookup_results_rx: None,
            available_game_versions: Vec::new(),
            selected_game_version_index: 0,
            loader_support: LoaderSupportIndex::default(),
            loader_versions: LoaderVersionIndex::default(),
            modloader_versions_cache: BTreeMap::new(),
            modloader_versions_in_flight: HashSet::new(),
            modloader_versions_results_tx: None,
            modloader_versions_results_rx: None,
            modloader_versions_status_key: None,
            modloader_versions_status: None,
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
            runtime_prepare_instance_root: None,
            runtime_prepare_user_key: None,
            show_settings_modal: false,
            launch_username: None,
            launch_user_key: None,
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
    account_avatars_by_key: &HashMap<String, Vec<u8>>,
) -> InstanceScreenOutput {
    let mut output = InstanceScreenOutput::default();
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
        return output;
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
        return output;
    };

    let state_id = ui.make_persistent_id(("instance_screen_state", instance_id));
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<InstanceScreenState>(state_id))
        .unwrap_or_else(|| InstanceScreenState::from_instance(&instance_snapshot, config));

    poll_background_tasks(&mut state, config);
    sync_version_catalog(&mut state, config.include_snapshots_and_betas(), false);
    if state.version_catalog_in_flight
        || !state.modloader_versions_in_flight.is_empty()
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
        account_avatars_by_key,
    );
    render_install_feedback(
        ui,
        text_ui,
        instance_id,
        state.runtime_latest_progress.as_ref(),
        external_activity.as_ref(),
        state.runtime_prepare_in_flight,
    );
    ui.add_space(10.0);
    output.instances_changed |= render_instance_settings_modal(
        ui.ctx(),
        text_ui,
        instance_id,
        &mut state,
        instances,
        config,
    );

    render_installed_content_section(
        ui,
        text_ui,
        instance_id,
        instance_root_path.as_path(),
        &mut state,
        &mut output,
    );

    ui.ctx().data_mut(|d| d.insert_temp(state_id, state));
    output
}

fn render_installed_content_section(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    instance_root: &Path,
    state: &mut InstanceScreenState,
    output: &mut InstanceScreenOutput,
) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(10.0);

    let add_button_style = ButtonOptions {
        min_size: egui::vec2((ui.available_width() - 30.0).max(160.0), 34.0),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    };

    ui.horizontal(|ui| {
        if text_ui
            .button(
                ui,
                ("instance_add_content_label", instance_id),
                "Open Content Browser",
                &add_button_style,
            )
            .clicked()
        {
            output.requested_screen = Some(AppScreen::ContentBrowser);
        }

        let plus_button_id = format!("instance_add_content_plus_{instance_id}");
        let add_menu_button = icon_button::svg(
            ui,
            plus_button_id.as_str(),
            assets::PLUS_SVG,
            "Add content options",
            false,
            20.0,
        );

        let popup_id = ui.id().with(("instance_add_content_popup", instance_id));
        let _ = egui::Popup::menu(&add_menu_button)
            .id(popup_id)
            .width(220.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
            .show(|ui| {
                let popup_button_style = ButtonOptions {
                    min_size: egui::vec2(ui.available_width().max(120.0), style::CONTROL_HEIGHT),
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
                        ("instance_content_popup_local", instance_id),
                        "Refresh local files",
                        &popup_button_style,
                    )
                    .clicked()
                {
                    state.content_metadata_cache.clear();
                    state.status_message =
                        Some("Refreshed installed content metadata cache.".to_owned());
                }
                if text_ui
                    .button(
                        ui,
                        ("instance_content_popup_mods", instance_id),
                        "Open content browser",
                        &popup_button_style,
                    )
                    .clicked()
                {
                    output.requested_screen = Some(AppScreen::ContentBrowser);
                }
            });
    });

    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for tab in InstalledContentTab::ALL {
            let selected = state.selected_content_tab == tab;
            let tab_style = ButtonOptions {
                min_size: egui::vec2(120.0, 30.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };
            if text_ui
                .selectable_button(
                    ui,
                    ("instance_content_tab", instance_id, tab.label()),
                    tab.label(),
                    selected,
                    &tab_style,
                )
                .clicked()
            {
                state.selected_content_tab = tab;
            }
        }
    });

    ui.add_space(10.0);

    let installed_files = list_installed_content_files(instance_root, state.selected_content_tab);
    if installed_files.is_empty() {
        let _ = text_ui.label(
            ui,
            (
                "instance_content_empty",
                instance_id,
                state.selected_content_tab.label(),
            ),
            &format!(
                "No {} installed.",
                state.selected_content_tab.label().to_lowercase()
            ),
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    }

    let mut pending_delete: Option<PathBuf> = None;
    let scroll_height = ui.available_height().max(180.0);
    egui::ScrollArea::vertical()
        .id_salt((
            "instance_installed_content_scroll",
            instance_id,
            state.selected_content_tab.label(),
        ))
        .auto_shrink([false, false])
        .max_height(scroll_height)
        .show(ui, |ui| {
            let row_width = (ui.max_rect().width() - INSTALLED_CONTENT_SCROLLBAR_RESERVE).max(1.0);
            ui.set_min_width(row_width);
            ui.set_max_width(row_width);
            for (entry_index, entry) in installed_files.iter().enumerate() {
                if !state.content_metadata_cache.contains_key(&entry.lookup_key) {
                    request_content_metadata_lookup(
                        state,
                        entry.lookup_key.as_str(),
                        entry.lookup_query.as_str(),
                        entry.managed_identity.as_ref(),
                        state.selected_content_tab,
                    );
                }
                let placeholder_metadata = entry.managed_identity.as_ref().map(|identity| {
                    identity.placeholder_entry(state.selected_content_tab.content_type_key())
                });
                let metadata = state
                    .content_metadata_cache
                    .get(&entry.lookup_key)
                    .and_then(|meta| meta.as_ref())
                    .or(placeholder_metadata.as_ref());

                let rendered = ui
                    .scope_builder(
                        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(row_width, f32::INFINITY),
                        )),
                        |ui| {
                            ui.set_min_width(row_width);
                            ui.set_max_width(row_width);
                            render_installed_content_entry(
                                ui,
                                text_ui,
                                (instance_id, entry_index),
                                entry,
                                metadata,
                            )
                        },
                    )
                    .inner;

                if rendered.delete_clicked {
                    pending_delete = Some(entry.file_path.clone());
                } else if rendered.open_clicked {
                    if let Some(metadata) = metadata.cloned() {
                        super::content_browser::request_open_detail_for_content(metadata);
                        output.requested_screen = Some(AppScreen::ContentBrowser);
                    } else {
                        state.status_message = Some(
                            "Still loading content metadata. Try again in a moment.".to_owned(),
                        );
                    }
                }
                ui.add_space(8.0);
            }
        });

    if let Some(path) = pending_delete {
        let delete_result = if path.is_dir() {
            std::fs::remove_dir_all(path.as_path())
        } else {
            std::fs::remove_file(path.as_path())
        };
        match delete_result {
            Ok(()) => {
                state.status_message = Some("Removed installed content.".to_owned());
            }
            Err(err) => {
                state.status_message = Some(format!("Failed to remove content: {err}"));
            }
        }
    }
}

#[derive(Debug, Clone)]
struct InstalledEntryRenderResult {
    open_clicked: bool,
    delete_clicked: bool,
}

fn render_installed_content_entry(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    entry: &InstalledContentFile,
    metadata: Option<&UnifiedContentEntry>,
) -> InstalledEntryRenderResult {
    const INSTALLED_TILE_GAP: f32 = 8.0;
    const INSTALLED_TILE_THUMBNAIL_FRAME_PADDING: f32 = 8.0;
    const INSTALLED_DESCRIPTION_LINE_HEIGHT: f32 = 20.0;
    const INSTALLED_DESCRIPTION_FRAME_Y_PADDING: i8 = 3;

    let display_name = metadata
        .map(|value| value.name.clone())
        .unwrap_or_else(|| entry.file_name.clone());
    let description = metadata
        .map(|value| {
            if value.summary.trim().is_empty() {
                entry.file_name.clone()
            } else {
                value.summary.clone()
            }
        })
        .unwrap_or_else(|| entry.file_name.clone());
    let platform_label = metadata
        .map(|value| value.source.label())
        .unwrap_or("Unknown");
    let available_width = ui.available_width().max(1.0);
    let tile_width = (available_width - (style::SPACE_XS * 2.0)).max(1.0);
    let side_padding = ((available_width - tile_width) * 0.5).max(0.0);

    let (delete_clicked, open_clicked) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if side_padding > 0.0 {
                ui.add_space(side_padding);
            }

            let frame_response = egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .corner_radius(egui::CornerRadius::same(style::CORNER_RADIUS_MD))
                .inner_margin(egui::Margin::same(style::SPACE_MD as i8))
                .show(ui, |ui| {
                    ui.set_min_width(tile_width);
                    ui.set_max_width(tile_width);

                    let mut delete_clicked = false;
                    let action_button_width = 28.0;
                    let content_width = ui.available_width().max(1.0);
                    let thumbnail_size = ((content_width - 52.0) * 0.14).clamp(32.0, 48.0);
                    let thumbnail_frame_size =
                        thumbnail_size + INSTALLED_TILE_THUMBNAIL_FRAME_PADDING;
                    let thumbnail_lane_height = 92.0_f32.max(thumbnail_frame_size);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        let delete_button_id =
                            format!("instance-content-delete-{}", entry.lookup_key);
                        if render_installed_content_action_button(
                            ui,
                            delete_button_id.as_str(),
                            assets::TRASH_X_SVG,
                            "Delete this content",
                            action_button_width,
                            action_button_width,
                        ) {
                            delete_clicked = true;
                        }

                        ui.add_space(INSTALLED_TILE_GAP);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width().max(1.0), 0.0),
                            egui::Layout::left_to_right(egui::Align::TOP),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(thumbnail_frame_size, thumbnail_lane_height),
                                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                                    |ui| {
                                        egui::Frame::new()
                                            .fill(ui.visuals().extreme_bg_color)
                                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                            .corner_radius(egui::CornerRadius::same(
                                                style::CORNER_RADIUS_SM,
                                            ))
                                            .inner_margin(egui::Margin::same(4))
                                            .show(ui, |ui| {
                                                render_content_thumbnail(
                                                    ui,
                                                    id_source,
                                                    metadata,
                                                    thumbnail_size,
                                                );
                                            });
                                    },
                                );

                                ui.add_space(INSTALLED_TILE_GAP);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width().max(1.0), 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(ui.available_width().max(1.0));
                                        let _ = text_ui.label(
                                            ui,
                                            (id_source, "name"),
                                            display_name.as_str(),
                                            &LabelOptions {
                                                font_size: 19.0,
                                                line_height: 24.0,
                                                weight: 700,
                                                color: ui.visuals().text_color(),
                                                wrap: true,
                                                ..LabelOptions::default()
                                            },
                                        );

                                        ui.add_space(4.0);
                                        egui::Frame::new()
                                            .fill(ui.visuals().selection.bg_fill)
                                            .stroke(egui::Stroke::NONE)
                                            .corner_radius(egui::CornerRadius::same(6))
                                            .inner_margin(egui::Margin::symmetric(6, 3))
                                            .show(ui, |ui| {
                                                let _ = text_ui.label(
                                                    ui,
                                                    (id_source, "platform_badge"),
                                                    platform_label,
                                                    &LabelOptions {
                                                        font_size: 13.0,
                                                        line_height: 16.0,
                                                        color: ui.visuals().selection.stroke.color,
                                                        wrap: false,
                                                        ..LabelOptions::default()
                                                    },
                                                );
                                            });

                                        ui.add_space(4.0);
                                        egui::Frame::new()
                                            .fill(ui.visuals().extreme_bg_color)
                                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                            .corner_radius(egui::CornerRadius::same(
                                                style::CORNER_RADIUS_SM,
                                            ))
                                            .inner_margin(egui::Margin::symmetric(
                                                6,
                                                INSTALLED_DESCRIPTION_FRAME_Y_PADDING,
                                            ))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width().max(1.0));
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(
                                                        ui.available_width().max(1.0),
                                                        INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                    ),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_min_height(
                                                            INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                        );
                                                        ui.set_max_height(
                                                            INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                        );
                                                        egui::ScrollArea::vertical()
                                                            .id_salt((
                                                                id_source,
                                                                "installed_description_scroll",
                                                            ))
                                                            .max_height(
                                                                INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                            )
                                                            .auto_shrink([false, false])
                                                            .show(ui, |ui| {
                                                                ui.set_width(
                                                                    ui.available_width().max(1.0),
                                                                );
                                                                let _ = text_ui.label(
                                                                    ui,
                                                                    (id_source, "description"),
                                                                    description.as_str(),
                                                                    &LabelOptions {
                                                                        line_height:
                                                                            INSTALLED_DESCRIPTION_LINE_HEIGHT,
                                                                        color: ui.visuals()
                                                                            .text_color(),
                                                                        wrap: true,
                                                                        ..LabelOptions::default()
                                                                    },
                                                                );
                                                            });
                                                    },
                                                );
                                            });
                                    },
                                );
                            },
                        );
                    });

                    delete_clicked
                });

            if side_padding > 0.0 {
                ui.add_space(side_padding);
            }

            (frame_response.inner, frame_response.response.clicked())
        })
        .inner;

    InstalledEntryRenderResult {
        open_clicked: open_clicked && !delete_clicked,
        delete_clicked,
    }
}

fn render_content_thumbnail(
    ui: &mut Ui,
    id_source: impl Hash,
    metadata: Option<&UnifiedContentEntry>,
    size: f32,
) {
    let size = egui::vec2(size, size);
    if let Some(icon_url) = metadata.and_then(|value| value.icon_url.as_deref()) {
        remote_tiled_image::show(
            ui,
            icon_url,
            size,
            (id_source, "remote-icon"),
            assets::LIBRARY_SVG,
        );
    } else {
        let mut hasher = DefaultHasher::new();
        id_source.hash(&mut hasher);
        ui.add(
            egui::Image::from_bytes(
                format!(
                    "bytes://instance/default-content-icon/{}.svg",
                    hasher.finish()
                ),
                assets::LIBRARY_SVG,
            )
            .fit_to_exact_size(size),
        );
    }
}

fn render_installed_content_action_button(
    ui: &mut Ui,
    icon_id: &str,
    svg_bytes: &'static [u8],
    tooltip: &str,
    width: f32,
    height: f32,
) -> bool {
    let icon_color = ui.visuals().error_fg_color;
    let themed_svg = apply_color_to_svg(svg_bytes, icon_color);
    let uri = format!(
        "bytes://instance-installed-content-action/{icon_id}-{:02x}{:02x}{:02x}.svg",
        icon_color.r(),
        icon_color.g(),
        icon_color.b()
    );
    let button_size = egui::vec2(width, height);
    let icon_size = (height - 10.0).max(12.0);
    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());
    let visuals = ui.visuals();
    let button_fill = if response.is_pointer_button_down_on() {
        visuals.widgets.active.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.extreme_bg_color
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), button_fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let image = egui::Image::from_bytes(uri, themed_svg)
        .fit_to_exact_size(egui::vec2(icon_size, icon_size));
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(icon_size, icon_size));
    let _ = ui.put(icon_rect, image);

    response.on_hover_text(tooltip).clicked()
}

fn list_installed_content_files(
    instance_root: &Path,
    tab: InstalledContentTab,
) -> Vec<InstalledContentFile> {
    let dir = instance_root.join(tab.folder_name());
    let mut files = Vec::new();
    let managed_identities = super::content_browser::load_managed_content_identities(instance_root);
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return files;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let allowed = match tab {
            InstalledContentTab::Mods => file_type.is_file() && extension == "jar",
            InstalledContentTab::ResourcePacks
            | InstalledContentTab::ShaderPacks
            | InstalledContentTab::DataPacks => file_type.is_dir() || extension == "zip",
        };
        if !allowed {
            continue;
        }

        let relative_path_key = normalize_installed_content_path_key(
            path.strip_prefix(instance_root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .as_ref(),
        );
        let managed_identity = managed_identities.get(relative_path_key.as_str()).cloned();
        let lookup_query = managed_identity
            .as_ref()
            .map(|identity| identity.name.clone())
            .unwrap_or_else(|| derive_installed_lookup_query(path.as_path(), file_name.as_str()));
        let lookup_key = format!(
            "{}::{}",
            tab.folder_name(),
            managed_lookup_key_suffix(managed_identity.as_ref(), lookup_query.as_str())
        );
        files.push(InstalledContentFile {
            file_name,
            file_path: path,
            lookup_query,
            lookup_key,
            managed_identity,
        });
    }

    files.sort_by(|left, right| {
        left.file_name
            .to_ascii_lowercase()
            .cmp(&right.file_name.to_ascii_lowercase())
    });
    files
}

fn normalize_lookup_key(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_installed_content_path_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn derive_installed_lookup_query(path: &Path, fallback_file_name: &str) -> String {
    let raw = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(fallback_file_name)
        .trim();
    if raw.is_empty() {
        return fallback_file_name.to_owned();
    }

    let pieces: Vec<&str> = raw
        .split(['-', '_'])
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .collect();
    if pieces.is_empty() {
        return raw.to_owned();
    }

    let mut kept = Vec::new();
    for piece in pieces {
        if looks_like_version_segment(piece) {
            break;
        }
        kept.push(piece);
    }

    if kept.is_empty() {
        raw.to_owned()
    } else {
        kept.join(" ")
    }
}

fn looks_like_version_segment(value: &str) -> bool {
    let normalized = value
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '+')
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }
    if normalized.starts_with('v')
        && normalized
            .chars()
            .skip(1)
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '+')
    {
        return true;
    }
    if normalized.starts_with("mc")
        && normalized
            .chars()
            .skip(2)
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == '+')
    {
        return true;
    }
    if normalized.len() >= 8 && normalized.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return true;
    }
    normalized.chars().any(|ch| ch.is_ascii_digit())
        && normalized.chars().any(|ch| ch == '.' || ch == '+')
}

fn managed_lookup_key_suffix(
    managed_identity: Option<&InstalledContentIdentity>,
    lookup_query: &str,
) -> String {
    if let Some(identity) = managed_identity {
        if let Some(project_id) = identity.modrinth_project_id.as_deref() {
            return format!("modrinth:{project_id}");
        }
        if let Some(project_id) = identity.curseforge_project_id {
            return format!("curseforge:{project_id}");
        }
    }
    normalize_lookup_key(lookup_query)
}

fn render_instance_settings_modal(
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
    let mut open = state.show_settings_modal;
    let viewport_rect = ctx.input(|i| i.content_rect());
    let modal_width = (viewport_rect.width() * 0.92).max(1.0);
    let modal_height = (viewport_rect.height() * 0.92).max(1.0);
    let modal_pos_x = (viewport_rect.center().x - modal_width * 0.5)
        .clamp(viewport_rect.left(), viewport_rect.right() - modal_width);
    let modal_pos_y = (viewport_rect.center().y - modal_height * 0.5)
        .clamp(viewport_rect.top(), viewport_rect.bottom() - modal_height);
    let modal_pos = egui::pos2(modal_pos_x, modal_pos_y);
    let modal_size = egui::vec2(modal_width, modal_height);
    let window_fill = {
        let base = ctx.style().visuals.window_fill;
        egui::Color32::from_rgba_premultiplied(base.r(), base.g(), base.b(), 255)
    };
    let mut close_requested = false;

    egui::Window::new("Instance Settings")
        .id(egui::Id::new(("instance_settings_modal", instance_id)))
        .open(&mut open)
        .fixed_pos(modal_pos)
        .fixed_size(modal_size)
        .collapsible(false)
        .title_bar(false)
        .resizable(false)
        .movable(false)
        .hscroll(false)
        .vscroll(false)
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
            let text_color = ui.visuals().text_color();
            let mut muted_style = LabelOptions::default();
            muted_style.color = ui.visuals().weak_text_color();
            muted_style.wrap = true;
            let section_style = LabelOptions {
                font_size: 22.0,
                line_height: 26.0,
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
            let reinstall_button_style = ButtonOptions {
                min_size: egui::vec2(220.0, 34.0),
                text_color: ui.visuals().text_color(),
                fill: ui.visuals().widgets.inactive.bg_fill,
                fill_hovered: ui.visuals().widgets.hovered.bg_fill,
                fill_active: ui.visuals().widgets.active.bg_fill,
                fill_selected: ui.visuals().selection.bg_fill,
                stroke: ui.visuals().widgets.inactive.bg_stroke,
                ..ButtonOptions::default()
            };

            egui::ScrollArea::vertical()
                .id_salt(("instance_settings_modal_scroll", instance_id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let _ = text_ui.label(
                        ui,
                        ("instance_settings_modal_heading", instance_id),
                        "Instance Settings",
                        &section_style,
                    );
                    ui.add_space(8.0);

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
                        ("instance_description_input", instance_id),
                        "Description (optional)",
                        Some("Optional note shown in library instance tiles."),
                        &mut state.description_input,
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

                        let mut modloader_version_options: Vec<String> =
                            Vec::with_capacity(resolved_modloader_versions.len() + 1);
                        modloader_version_options.push("Latest available".to_owned());
                        modloader_version_options.extend(resolved_modloader_versions.iter().cloned());
                        let option_refs: Vec<&str> = modloader_version_options
                            .iter()
                            .map(String::as_str)
                            .collect();
                        let current_modloader_version = state.modloader_version_input.trim().to_owned();
                        let mut selected_index = if current_modloader_version.is_empty() {
                            0
                        } else {
                            modloader_version_options
                                .iter()
                                .position(|entry| entry == &current_modloader_version)
                                .unwrap_or(0)
                        };
                        if !current_modloader_version.is_empty() && selected_index == 0 {
                            state.modloader_version_input.clear();
                        }
                        if settings_widgets::full_width_dropdown_row(
                            text_ui,
                            ui,
                            ("instance_modloader_version_dropdown", instance_id),
                            "Modloader version",
                            Some("Cataloged by loader+Minecraft compatibility and cached once per day. Pick Latest available for automatic selection."),
                            &mut selected_index,
                            &option_refs,
                        )
                        .changed()
                        {
                            if selected_index == 0 {
                                state.modloader_version_input.clear();
                            } else if let Some(selected) = modloader_version_options.get(selected_index) {
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
                        && !state
                            .loader_support
                            .supports_loader(
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
                            &LabelOptions {
                                color: ui.visuals().error_fg_color,
                                wrap: true,
                                ..LabelOptions::default()
                            },
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
                                    let modloader_version = normalize_optional(
                                        saved_instance.modloader_version.as_str(),
                                    );
                                    let installations_root =
                                        PathBuf::from(config.minecraft_installations_root());
                                    let instance_root = instances::instance_root_path(
                                        &installations_root,
                                        &saved_instance,
                                    );
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
                                        config.default_instance_max_memory_mib(),
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
                        "Instance Settings",
                        &section_style,
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
                                PathBuf::from(config.minecraft_installations_root());
                            let instance_root =
                                instances::instance_root_path(&installations_root, instance);
                            match open_instance_folder(instance_root.as_path()) {
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
                                &LabelOptions {
                                    color: ui.visuals().error_fg_color,
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
                            );
                        } else {
                            if state
                                .java_override_runtime_major
                                .is_none_or(|major| !java_options.iter().any(|(m, _)| *m == major))
                            {
                                state.java_override_runtime_major = java_options.first().map(|(major, _)| *major);
                            }
                            let option_labels: Vec<&str> =
                                java_options.iter().map(|(_, label)| label.as_str()).collect();
                            let mut selected_index = java_options
                                .iter()
                                .position(|(major, _)| Some(*major) == state.java_override_runtime_major)
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
                                    java_options
                                        .iter()
                                        .find_map(|(candidate, _)| (*candidate == major).then_some(major))
                                });
                                selected.or_else(|| java_options.first().map(|(major, _)| *major))
                            }
                        } else {
                            None
                        };
                        if !state.java_override_enabled || java_override_runtime_major.is_some() {
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
                            match set_instance_settings(
                                instances,
                                instance_id,
                                memory_override,
                                cli_override,
                                state.java_override_enabled,
                                java_override_runtime_major,
                            ) {
                                Ok(()) => {
                                    instances_changed = true;
                                    state.status_message = Some("Saved instance settings.".to_owned());
                                }
                                Err(err) => state.status_message = Some(err.to_string()),
                            }
                        }
                    }

                    ui.add_space(12.0);
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
        });

    if close_requested {
        open = false;
    }
    state.show_settings_modal = open;
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
    let instance_root_key = std::fs::canonicalize(instance_root)
        .unwrap_or_else(|_| instance_root.to_path_buf())
        .display()
        .to_string();
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
                        .clone()
                        .or_else(|| launch_account.clone());
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
        let blocked_account = launch_display_name.as_deref().unwrap_or("this account");
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

fn render_stop_runtime_button(
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

fn render_runtime_avatar(
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

fn apply_color_to_svg(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg = String::from_utf8_lossy(svg_bytes).replace("currentColor", &color_hex);
    svg.into_bytes()
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

fn save_instance_metadata_and_versions(
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

fn poll_background_tasks(state: &mut InstanceScreenState, config: &mut Config) {
    poll_version_catalog(state);
    poll_modloader_versions(state);
    poll_content_lookup_results(state);
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

fn apply_version_catalog_error(
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

fn selected_modloader_versions<'a>(
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

fn modloader_versions_cache_key(loader_label: &str, game_version: &str) -> String {
    format!(
        "{}|{}",
        loader_label.trim().to_ascii_lowercase(),
        game_version.trim()
    )
}

fn ensure_modloader_versions_channel(state: &mut InstanceScreenState) {
    if state.modloader_versions_results_tx.is_some()
        && state.modloader_versions_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Result<Vec<String>, String>)>();
    state.modloader_versions_results_tx = Some(tx);
    state.modloader_versions_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_modloader_versions(
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

fn poll_modloader_versions(state: &mut InstanceScreenState) {
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

fn is_latest_modloader_version_alias(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "latest" | "latest available" | "use latest version" | "auto" | "default"
    )
}

fn resolve_latest_modloader_version_from_state(
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

fn resolve_modloader_version_for_settings(
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

fn ensure_content_lookup_channel(state: &mut InstanceScreenState) {
    if state.content_lookup_results_tx.is_some() && state.content_lookup_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Option<UnifiedContentEntry>)>();
    state.content_lookup_results_tx = Some(tx);
    state.content_lookup_results_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_content_metadata_lookup(
    state: &mut InstanceScreenState,
    lookup_key: &str,
    lookup_query: &str,
    managed_identity: Option<&InstalledContentIdentity>,
    tab: InstalledContentTab,
) {
    let normalized_key = lookup_key.trim();
    let query = lookup_query.trim();
    if normalized_key.is_empty() || query.is_empty() {
        return;
    }
    if state.content_metadata_cache.contains_key(normalized_key)
        || state.content_lookup_in_flight.contains(normalized_key)
    {
        return;
    }

    ensure_content_lookup_channel(state);
    let Some(tx) = state.content_lookup_results_tx.as_ref().cloned() else {
        return;
    };

    let key_for_state = normalized_key.to_owned();
    state.content_lookup_in_flight.insert(key_for_state.clone());
    let key_for_result = key_for_state.clone();
    let query_for_search = query.to_owned();
    let managed_identity = managed_identity.cloned();

    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            if let Some(identity) = managed_identity.as_ref()
                && let Some(entry) = fetch_exact_managed_content_metadata(identity, tab)
            {
                return Some(entry);
            }
            search_minecraft_content(query_for_search.as_str(), 25)
                .ok()
                .and_then(|search| {
                    choose_preferred_content_entry(search.entries, key_for_result.as_str(), tab)
                })
        })
        .await
        .ok()
        .flatten();
        let _ = tx.send((key_for_state, result));
    });
}

fn poll_content_lookup_results(state: &mut InstanceScreenState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.content_lookup_results_rx.as_ref() {
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
        state.content_lookup_results_tx = None;
        state.content_lookup_results_rx = None;
        state.content_lookup_in_flight.clear();
    }

    for (lookup_key, metadata) in updates {
        state.content_lookup_in_flight.remove(lookup_key.as_str());
        state.content_metadata_cache.insert(lookup_key, metadata);
    }
}

fn fetch_exact_managed_content_metadata(
    identity: &InstalledContentIdentity,
    tab: InstalledContentTab,
) -> Option<UnifiedContentEntry> {
    match identity.source {
        ContentSource::Modrinth => {
            let project_id = identity.modrinth_project_id.as_deref()?;
            let project = ModrinthClient::default().get_project(project_id).ok()?;
            Some(UnifiedContentEntry {
                id: format!("modrinth:{}", project.project_id),
                name: project.title,
                summary: project.description.trim().to_owned(),
                content_type: project.project_type,
                source: ContentSource::Modrinth,
                project_url: Some(project.project_url),
                icon_url: project.icon_url,
            })
        }
        ContentSource::CurseForge => {
            let project_id = identity.curseforge_project_id?;
            let curseforge = CurseForgeClient::from_env()?;
            let project = curseforge.get_mod(project_id).ok()?;
            Some(UnifiedContentEntry {
                id: format!("curseforge:{}", project.id),
                name: project.name,
                summary: project.summary.trim().to_owned(),
                content_type: tab.content_type_key().to_owned(),
                source: ContentSource::CurseForge,
                project_url: project.website_url,
                icon_url: project.icon_url,
            })
        }
    }
}

fn choose_preferred_content_entry(
    entries: Vec<UnifiedContentEntry>,
    lookup_key: &str,
    tab: InstalledContentTab,
) -> Option<UnifiedContentEntry> {
    let target_key = lookup_key
        .split_once("::")
        .map(|(_, value)| value)
        .unwrap_or(lookup_key);
    if target_key.trim().is_empty() {
        return None;
    }

    let lookup_tokens: Vec<&str> = target_key.split_whitespace().collect();
    let mut best: Option<(i32, UnifiedContentEntry)> = None;

    for entry in entries {
        let mut score = 0i32;
        if tab_accepts_content_type(tab, entry.content_type.as_str()) {
            score += 80;
        } else {
            continue;
        }

        let normalized_name = normalize_lookup_key(entry.name.as_str());
        let entry_tokens: Vec<&str> = normalized_name.split_whitespace().collect();
        let mut overlap = 0i32;
        for token in &lookup_tokens {
            if token.len() < 2 {
                continue;
            }
            if entry_tokens.iter().any(|entry_token| entry_token == token) {
                overlap += 1;
            }
        }
        let strong_name_match = normalized_name == target_key
            || normalized_name.contains(target_key)
            || target_key.contains(normalized_name.as_str())
            || overlap >= lookup_tokens.len().min(2) as i32;
        if !strong_name_match {
            continue;
        }

        if normalized_name == target_key {
            score += 600;
        } else {
            if normalized_name.contains(target_key) || target_key.contains(normalized_name.as_str())
            {
                score += 220;
            }
            score += overlap * 60;
        }

        if !entry.summary.trim().is_empty() {
            score += 8;
        }
        if entry.icon_url.is_some() {
            score += 10;
        }
        score += match entry.source {
            ContentSource::Modrinth => 20,
            ContentSource::CurseForge => 10,
        };

        let should_replace = best.as_ref().is_none_or(|(best_score, best_entry)| {
            score > *best_score
                || (score == *best_score
                    && content_source_priority(entry.source)
                        > content_source_priority(best_entry.source))
        });
        if should_replace {
            best = Some((score, entry));
        }
    }

    best.map(|(_, entry)| entry)
}

fn content_source_priority(source: ContentSource) -> i32 {
    match source {
        ContentSource::Modrinth => 2,
        ContentSource::CurseForge => 1,
    }
}

fn tab_accepts_content_type(tab: InstalledContentTab, content_type: &str) -> bool {
    let normalized_type = normalize_lookup_key(content_type);
    match tab {
        InstalledContentTab::Mods => normalized_type.contains("mod"),
        InstalledContentTab::ResourcePacks => {
            normalized_type.contains("resource pack") || normalized_type.contains("texture pack")
        }
        InstalledContentTab::ShaderPacks => normalized_type.contains("shader"),
        InstalledContentTab::DataPacks => {
            normalized_type.contains("data pack") || normalized_type.contains("datapack")
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

fn reinstall_instance_profile_files(instance_root: &Path) -> Result<(), std::io::Error> {
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

fn request_runtime_prepare(
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
    let instance_root_display = instance_root.display().to_string();
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
    let username = player_name_for_task
        .as_deref()
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
                let installed = installed.display().to_string();
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

fn choose_java_executable(
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

fn required_java_major(game_version: &str) -> Option<u8> {
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

fn effective_required_java_major(config: &Config, game_version: &str) -> Option<u8> {
    let required = required_java_major(game_version)?;
    if config.force_java_21_minimum() && required < 21 {
        Some(21)
    } else {
        Some(required)
    }
}

fn java_runtime_from_major(major: u8) -> Option<JavaRuntimeVersion> {
    match major {
        8 => Some(JavaRuntimeVersion::Java8),
        16 => Some(JavaRuntimeVersion::Java16),
        17 => Some(JavaRuntimeVersion::Java17),
        21 => Some(JavaRuntimeVersion::Java21),
        _ => None,
    }
}

fn configured_java_path_options(config: &Config) -> Vec<(u8, String)> {
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
    let entered_modloader_version = state.modloader_version_input.trim();
    if entered_modloader_version.is_empty()
        || is_latest_modloader_version_alias(entered_modloader_version)
    {
        return;
    }
    let Some(known_versions) = state
        .loader_versions
        .versions_for_loader(selected_label, game_version)
    else {
        return;
    };
    if known_versions
        .iter()
        .any(|version| version.eq_ignore_ascii_case(entered_modloader_version))
    {
        return;
    }

    tracing::warn!(
        target: "vertexlauncher/ui/instance",
        selected_modloader = %selected_label,
        game_version = %game_version,
        selected_modloader_version = %entered_modloader_version,
        "Selected modloader version is not currently marked compatible for this game version; keeping user selection."
    );
}

fn support_catalog_ready(state: &InstanceScreenState) -> bool {
    state.version_catalog_include_snapshots.is_some() && state.version_catalog_error.is_none()
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

fn open_instance_folder(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("folder does not exist: {}", path.display()));
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        command.arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };

    command.spawn().map(|_| ()).map_err(|err| err.to_string())
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
