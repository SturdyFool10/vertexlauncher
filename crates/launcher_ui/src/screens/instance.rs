use config::{
    Config, INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, INSTANCE_DEFAULT_MAX_MEMORY_MIB_STEP,
    JavaRuntimeVersion,
};
use egui::Ui;
use installation::{
    DownloadPolicy, GameSetupResult, InstallProgress, InstallProgressCallback, InstallStage,
    LaunchRequest, LaunchResult, LoaderSupportIndex, LoaderVersionIndex, MinecraftVersionEntry,
    VersionCatalog, ensure_game_files, ensure_openjdk_runtime, fetch_loader_versions_for_game,
    fetch_version_catalog_with_refresh, is_instance_running, launch_instance,
    running_instance_for_account, stop_running_instance,
};
use instances::{InstanceStore, set_instance_settings, set_instance_versions};
use modprovider::{ContentSource, UnifiedContentEntry, search_minecraft_content};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use textui::{ButtonOptions, LabelOptions, TextUi, TooltipOptions};

use crate::app::tokio_runtime;
use crate::screens::{AppScreen, LaunchAuthContext};
use crate::ui::{
    components::{icon_button, settings_widgets},
    style,
};
use crate::{assets, console, install_activity, notification};

const RESERVED_SYSTEM_MEMORY_MIB: u128 = 4 * 1024;
const FALLBACK_TOTAL_MEMORY_MIB: u128 = 20 * 1024;
const MODLOADER_OPTIONS: [&str; 6] = ["Vanilla", "Fabric", "Forge", "NeoForge", "Quilt", "Custom"];
const CUSTOM_MODLOADER_INDEX: usize = MODLOADER_OPTIONS.len() - 1;

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
}

#[derive(Clone, Debug)]
struct InstalledContentFile {
    file_name: String,
    file_path: PathBuf,
    lookup_query: String,
    lookup_key: String,
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
    configured_java: Option<(JavaRuntimeVersion, String)>,
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
                "Add Content",
                &add_button_style,
            )
            .clicked()
        {
            output.requested_screen = Some(AppScreen::Library);
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
                        "Browse local content",
                        &popup_button_style,
                    )
                    .clicked()
                {
                    state.status_message = Some(
                        "Local content picker will be added with the content browser.".to_owned(),
                    );
                }
                if text_ui
                    .button(
                        ui,
                        ("instance_content_popup_mods", instance_id),
                        "Browse mods",
                        &popup_button_style,
                    )
                    .clicked()
                {
                    output.requested_screen = Some(AppScreen::Library);
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
        .max_height(scroll_height)
        .show(ui, |ui| {
            for (entry_index, entry) in installed_files.iter().enumerate() {
                if !state.content_metadata_cache.contains_key(&entry.lookup_key) {
                    request_content_metadata_lookup(
                        state,
                        entry.lookup_key.as_str(),
                        entry.lookup_query.as_str(),
                        state.selected_content_tab,
                    );
                }
                let metadata = state
                    .content_metadata_cache
                    .get(&entry.lookup_key)
                    .and_then(|meta| meta.as_ref());

                let rendered = render_installed_content_entry(
                    ui,
                    text_ui,
                    (instance_id, entry_index),
                    entry,
                    metadata,
                );

                if rendered.delete_clicked {
                    pending_delete = Some(entry.file_path.clone());
                } else if rendered.open_clicked {
                    output.requested_screen = Some(AppScreen::Library);
                    state.status_message = Some(format!(
                        "Content browser routing for {} will be added next.",
                        rendered.display_name
                    ));
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
    display_name: String,
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

    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            let mut delete_clicked = false;
            ui.horizontal_top(|ui| {
                let delete_response = icon_button::svg(
                    ui,
                    format!("instance_content_delete_{}", entry.lookup_key).as_str(),
                    assets::TRASH_X_SVG,
                    "Delete this content",
                    false,
                    20.0,
                );
                if delete_response.clicked() {
                    delete_clicked = true;
                }

                ui.add_space(8.0);
                render_content_thumbnail(ui, id_source, metadata);
                ui.add_space(8.0);

                ui.vertical(|ui| {
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

                    egui::Frame::new()
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
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
                                    color: ui.visuals().text_color(),
                                    wrap: false,
                                    ..LabelOptions::default()
                                },
                            );
                        });

                    ui.add_space(4.0);
                    let _ = text_ui.label(
                        ui,
                        (id_source, "description"),
                        description.as_str(),
                        &LabelOptions {
                            color: ui.visuals().weak_text_color(),
                            wrap: true,
                            ..LabelOptions::default()
                        },
                    );
                });
            });
            delete_clicked
        });

    let entry_response = ui.interact(
        frame.response.rect,
        ui.make_persistent_id((id_source, "entry_click")),
        egui::Sense::click(),
    );

    InstalledEntryRenderResult {
        display_name,
        open_clicked: entry_response.clicked(),
        delete_clicked: frame.inner,
    }
}

fn render_content_thumbnail(
    ui: &mut Ui,
    id_source: impl Hash,
    metadata: Option<&UnifiedContentEntry>,
) {
    let size = egui::vec2(48.0, 48.0);
    let image = if let Some(icon_url) = metadata.and_then(|value| value.icon_url.as_deref()) {
        egui::Image::from_uri(icon_url)
    } else {
        let mut hasher = DefaultHasher::new();
        id_source.hash(&mut hasher);
        egui::Image::from_bytes(
            format!("bytes://instance/default-content-icon/{}", hasher.finish()),
            assets::LIBRARY_SVG,
        )
    };
    ui.add(image.fit_to_exact_size(size));
}

fn list_installed_content_files(
    instance_root: &Path,
    tab: InstalledContentTab,
) -> Vec<InstalledContentFile> {
    let dir = instance_root.join(tab.folder_name());
    let mut files = Vec::new();
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

        let lookup_query = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(file_name.as_str())
            .to_owned();
        let lookup_key = format!(
            "{}::{}",
            tab.folder_name(),
            normalize_lookup_key(lookup_query.as_str())
        );
        files.push(InstalledContentFile {
            file_name,
            file_path: path,
            lookup_query,
            lookup_key,
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
                                        recommended_java_runtime(game_version.as_str()),
                                        choose_java_executable(config, game_version.as_str()),
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
                        match set_instance_settings(
                            instances,
                            instance_id,
                            memory_override,
                            cli_override,
                        ) {
                            Ok(()) => {
                                instances_changed = true;
                                state.status_message = Some("Saved instance settings.".to_owned());
                            }
                            Err(err) => state.status_message = Some(err.to_string()),
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
    let account_running_root = launch_account
        .as_deref()
        .and_then(running_instance_for_account);
    let launch_disabled_for_account = !state.running
        && account_running_root
            .as_deref()
            .is_some_and(|running_root| running_root != instance_root_key.as_str());
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

    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
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
            score += 300;
        }

        let normalized_name = normalize_lookup_key(entry.name.as_str());
        if normalized_name == target_key {
            score += 600;
        } else {
            if normalized_name.contains(target_key) || target_key.contains(normalized_name.as_str())
            {
                score += 220;
            }
            let mut overlap = 0i32;
            for token in &lookup_tokens {
                if token.len() < 2 {
                    continue;
                }
                if normalized_name
                    .split_whitespace()
                    .any(|entry_token| entry_token == *token)
                {
                    overlap += 1;
                }
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
    } else if let Some(runtime) = required_java_runtime {
        format!("auto-provisioned OpenJDK {}", runtime.major())
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
                if let Some((runtime, path)) = outcome.configured_java {
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
