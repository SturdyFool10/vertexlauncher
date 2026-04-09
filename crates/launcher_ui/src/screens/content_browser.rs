use config::Config;
use content_resolver::detect_installed_content_kind;
use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use egui::Ui;
use installation::{
    DownloadBatchTask, DownloadPolicy, InstallProgressCallback, InstallStage,
    MinecraftVersionEntry, download_batch_with_progress, fetch_version_catalog,
};
use instances::{InstanceStore, instance_root_path};
use managed_content::{
    ContentInstallManifest, InstalledContentProject, ManagedContentSource, load_content_manifest,
    save_content_manifest,
};
use modprovider::{ContentSource, UnifiedContentEntry};
use modrinth::Client as ModrinthClient;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use textui::TextUi;
use textui_egui::prelude::*;
use ui_foundation::{UiMetrics, themed_text_input};

use crate::app::tokio_runtime;
use crate::assets;
use crate::install_activity;
use crate::notification;
use crate::ui::{
    components::{remote_tiled_image, settings_widgets},
    style,
};

use super::AppScreen;

#[path = "content_browser_install.rs"]
mod content_browser_install;
#[path = "content_browser_state.rs"]
mod content_browser_state;
#[path = "content_browser_workers.rs"]
mod content_browser_workers;
use self::content_browser_install::*;
pub(crate) use self::content_browser_state::BulkContentUpdate;
pub use self::content_browser_state::ContentBrowserState;
use self::content_browser_state::*;
use self::content_browser_workers::*;

const CONTENT_SEARCH_PER_PROVIDER_LIMIT: u32 = 35;
const CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE: u32 = 3;
const DEFAULT_DISCOVERY_QUERY_MOD: &str = "mod";
const MODRINTH_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(250);
const CURSEFORGE_DOWNLOAD_MIN_SPACING: Duration = Duration::from_millis(500);
const DEFAULT_DISCOVERY_QUERY_RESOURCE_PACK: &str = "resource pack";
const DEFAULT_DISCOVERY_QUERY_SHADER: &str = "shader";
const DEFAULT_DISCOVERY_QUERY_DATA_PACK: &str = "data pack";
const DETAIL_VERSION_FETCH_PAGE_SIZE: u32 = 50;
const DETAIL_VERSION_FETCH_MAX_PAGES: u32 = 5;
const TILE_ACTION_BUTTON_WIDTH: f32 = 28.0;
const TILE_ACTION_BUTTON_HEIGHT: f32 = 28.0;
const TILE_ACTION_BUTTON_GAP_XS: f32 = 4.0;
const TILE_DOWNLOAD_PROGRESS_WIDTH: f32 = 96.0;
const CONTENT_UPDATE_LOG_TARGET: &str = "vertexlauncher/content_update";
const VERTEX_PREFETCH_DIR_NAME: &str = "vertex_prefetch";
const VERSION_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(75);
const DETAIL_VERSIONS_FETCH_TIMEOUT: Duration = Duration::from_secs(45);
const CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES: usize = 12;
const CONTENT_BROWSER_VERSION_DROPDOWN_ID_KEY: &str = "content_browser_version_dropdown_id";
const CONTENT_BROWSER_SCOPE_DROPDOWN_ID_KEY: &str = "content_browser_scope_dropdown_id";
const CONTENT_BROWSER_SORT_DROPDOWN_ID_KEY: &str = "content_browser_sort_dropdown_id";
const CONTENT_BROWSER_LOADER_DROPDOWN_ID_KEY: &str = "content_browser_loader_dropdown_id";

#[derive(Clone, Copy, Debug)]
struct ContentBrowserUiMetrics {
    action_button_width: f32,
    action_button_height: f32,
    download_progress_width: f32,
    result_thumbnail_size: f32,
}

impl ContentBrowserUiMetrics {
    fn from_ui(ui: &Ui) -> Self {
        let metrics = UiMetrics::from_ui(ui, 860.0);
        Self {
            action_button_width: metrics.scaled_width(0.02, TILE_ACTION_BUTTON_WIDTH, 34.0),
            action_button_height: metrics.scaled_height(0.036, TILE_ACTION_BUTTON_HEIGHT, 34.0),
            download_progress_width: metrics.scaled_width(
                0.08,
                TILE_DOWNLOAD_PROGRESS_WIDTH,
                124.0,
            ),
            result_thumbnail_size: metrics.scaled_width(0.075, 84.0, 108.0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ContentBrowserOutput {
    pub requested_screen: Option<AppScreen>,
}

static PENDING_EXTERNAL_DETAIL_OPEN: OnceLock<Mutex<Option<UnifiedContentEntry>>> = OnceLock::new();

pub(crate) fn request_open_detail_for_content(entry: UnifiedContentEntry) {
    let store = PENDING_EXTERNAL_DETAIL_OPEN.get_or_init(|| Mutex::new(None));
    if let Ok(mut pending) = store.lock() {
        *pending = Some(entry);
    }
}

pub fn version_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_VERSION_DROPDOWN_ID_KEY))
    })
}

pub fn scope_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_SCOPE_DROPDOWN_ID_KEY)))
}

pub fn sort_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_SORT_DROPDOWN_ID_KEY)))
}

pub fn loader_dropdown_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new(CONTENT_BROWSER_LOADER_DROPDOWN_ID_KEY))
    })
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    instances: &InstanceStore,
    config: &Config,
    state: &mut ContentBrowserState,
    force_reset: bool,
) -> ContentBrowserOutput {
    let mut output = ContentBrowserOutput::default();
    if force_reset {
        *state = ContentBrowserState::default();
    }

    poll_search(state);
    poll_detail_versions(state);
    poll_downloads(state);
    poll_version_catalog(state);
    poll_identify_results(state);

    if state.search_in_flight
        || state.detail_versions_in_flight
        || state.download_in_flight
        || state.identify_in_flight
        || state.version_catalog_in_flight
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    let Some(instance_id) = selected_instance_id else {
        let _ = text_ui.label(
            ui,
            "content_browser_no_instance",
            "Select an instance first. Content Browser installs into the selected instance.",
            &style::muted(ui),
        );
        return output;
    };

    let Some(instance) = instances.find(instance_id) else {
        let _ = text_ui.label(
            ui,
            "content_browser_missing_instance",
            "Selected instance no longer exists.",
            &style::error_text(ui),
        );
        return output;
    };

    if state.active_instance_id.as_deref() != Some(instance.id.as_str()) {
        state.active_instance_id = Some(instance.id.clone());
        state.active_instance_name = Some(instance.name.clone());
        state.loader = browser_loader_from_modloader(instance.modloader.as_str());
        state.minecraft_version_filter = instance.game_version.clone();
        state.content_scope = ContentScope::Mods;
        state.mod_sort_mode = ModSortMode::Popularity;
        state.download_queue.clear();
        state.download_in_flight = false;
        state.active_download = None;
        state.cached_manifest = None;
        state.manifest_dirty = true;
        state.current_page = 1;
        state.current_view = ContentBrowserPage::Browse;
        state.detail_entry = None;
        state.detail_tab = ContentDetailTab::Overview;
        state.detail_versions.clear();
        state.detail_versions_project_key = None;
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
        state.detail_loader_filter = state.loader;
        state.detail_minecraft_version_filter = instance.game_version.clone();
        state.results = BrowserSearchSnapshot::default();
        state.active_search_request = None;
        state.search_completed_tasks = 0;
        state.search_total_tasks = 0;
        state.search_in_flight = false;
        state.search_notification_active = false;
        state.auto_populated_instance_id = None;
    }

    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let instance_root = instance_root_path(&installations_root, instance);
    let game_version = instance.game_version.trim().to_owned();

    request_version_catalog(state);
    apply_pending_external_detail_open(state);

    let _ = text_ui.label(
        ui,
        ("content_browser_context", instance.id.as_str()),
        &format!(
            "Instance: {} | Minecraft {} | Loader {}",
            instance.name,
            if game_version.is_empty() {
                "n/a"
            } else {
                game_version.as_str()
            },
            instance.modloader.trim()
        ),
        &style::caption(ui),
    );
    ui.add_space(style::SPACE_MD);

    maybe_start_queued_download(state, instance.name.as_str(), instance_root.as_path());

    if let Some(status) = state.status_message.as_deref() {
        let _ = text_ui.label(
            ui,
            ("content_browser_status", instance.id.as_str()),
            status,
            &style::muted(ui),
        );
    }

    for warning in &state.results.warnings {
        let _ = text_ui.label(
            ui,
            ("content_browser_warning", instance.id.as_str(), warning),
            warning,
            &style::warning_text(ui),
        );
    }

    ui.add_space(style::SPACE_MD);
    match state.current_view {
        ContentBrowserPage::Browse => {
            if state.manifest_dirty || state.cached_manifest.is_none() {
                state.cached_manifest = Some(load_content_manifest(instance_root.as_path()));
                state.manifest_dirty = false;
            }
            let manifest = state.cached_manifest.clone().expect("just populated");
            render_controls(ui, text_ui, instance.id.as_str(), state);

            if state.auto_populated_instance_id.as_deref() != Some(instance.id.as_str())
                && !state.search_in_flight
                && state.query_input.trim().is_empty()
                && state.search_tags.is_empty()
            {
                let request = BrowserSearchRequest {
                    query: None,
                    tags: Vec::new(),
                    game_version: normalize_optional(state.minecraft_version_filter.as_str()),
                    loader: state.loader,
                    content_scope: ContentScope::Mods,
                    mod_sort_mode: ModSortMode::Popularity,
                    page: 1,
                };
                state.current_page = 1;
                request_search(state, request);
                state.auto_populated_instance_id = Some(instance.id.clone());
            }

            let results_height = (ui.available_height() - 42.0).max(140.0);
            let render_outcome = render_results(
                ui,
                text_ui,
                instance.id.as_str(),
                state,
                &manifest,
                results_height,
            );
            if let Some(page) = render_outcome.requested_page
                && page != state.current_page
            {
                state.current_page = page;
                request_search_for_current_filters(state, false);
            }
            if let Some(entry) = render_outcome.open_entry {
                open_detail_page(state, &entry);
            }
        }
        ContentBrowserPage::Detail => {
            render_detail_page(
                ui,
                text_ui,
                instance.id.as_str(),
                instance_root.as_path(),
                state,
            );
        }
    }

    ui.add_space(style::SPACE_LG);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = style::SPACE_SM;
        let button_style = style::neutral_button(ui);
        if state.current_view == ContentBrowserPage::Detail {
            if text_ui
                .button(
                    ui,
                    ("content_browser_back_to_browser", instance.id.as_str()),
                    "Back to Mod Browser",
                    &button_style,
                )
                .clicked()
            {
                state.current_view = ContentBrowserPage::Browse;
            }
        }
        if text_ui
            .button(
                ui,
                ("content_browser_back_to_instance", instance.id.as_str()),
                "Back to Instance",
                &button_style,
            )
            .clicked()
        {
            output.requested_screen = Some(AppScreen::Instance);
        }
    });
    output
}

fn render_controls(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut ContentBrowserState,
) {
    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(style::SPACE_XL as i8));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_SM, style::SPACE_MD);
        let response = themed_text_input(
            text_ui,
            ui,
            ("content_browser_query", instance_id),
            &mut state.query_input,
            InputOptions {
                desired_width: Some(ui.available_width().max(160.0)),
                placeholder_text: Some(
                    "Search project names, summaries, and tags. Press Enter to search".to_owned(),
                ),
                ..InputOptions::default()
            },
        );
        let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
        let submit_pressed = enter_pressed && (response.has_focus() || response.lost_focus());
        if submit_pressed {
            if ui.input(|input| input.modifiers.shift) {
                if add_search_tag(&mut state.search_tags, state.query_input.as_str()) {
                    state.query_input.clear();
                    if !state.search_in_flight {
                        request_search_for_current_filters(state, true);
                    }
                }
            } else if !state.search_in_flight {
                request_search_for_current_filters(state, true);
            }
        }

        if !state.search_tags.is_empty() {
            ui.add_space(style::SPACE_SM);
            if render_search_tag_chips(ui, &mut state.search_tags) {
                request_search_for_current_filters(state, true);
            }
        }
        ui.add_space(style::SPACE_MD);
        let gap = style::SPACE_MD;
        let column_width = ((ui.available_width() - gap * 3.0) / 4.0).max(1.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;

            ui.allocate_ui_with_layout(
                egui::vec2(column_width, style::CONTROL_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let mut version_options =
                        Vec::<String>::with_capacity(state.available_game_versions.len() + 1);
                    version_options.push("Any version".to_owned());
                    for version in &state.available_game_versions {
                        version_options.push(version.display_label());
                    }
                    let version_option_refs = version_options
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>();
                    let mut selected_version_index = 0_usize;
                    if !state.minecraft_version_filter.trim().is_empty()
                        && let Some(found_index) = state
                            .available_game_versions
                            .iter()
                            .position(|version| version.id == state.minecraft_version_filter)
                    {
                        selected_version_index = found_index + 1;
                    }
                    let response = settings_widgets::dropdown_picker(
                        text_ui,
                        ui,
                        ("content_browser_minecraft_version", instance_id),
                        &mut selected_version_index,
                        &version_option_refs,
                        Some(column_width),
                    );
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new(CONTENT_BROWSER_VERSION_DROPDOWN_ID_KEY),
                            response.id,
                        )
                    });
                    if response.changed() {
                        state.minecraft_version_filter = if selected_version_index == 0 {
                            String::new()
                        } else {
                            state.available_game_versions[selected_version_index - 1]
                                .id
                                .clone()
                        };
                        request_search_for_current_filters(state, true);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(column_width, style::CONTROL_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let scope_options = ContentScope::ALL.map(ContentScope::label);
                    let mut scope_index = ContentScope::ALL
                        .iter()
                        .position(|scope| *scope == state.content_scope)
                        .unwrap_or(0);
                    let response = settings_widgets::dropdown_picker(
                        text_ui,
                        ui,
                        ("content_browser_scope", instance_id),
                        &mut scope_index,
                        &scope_options,
                        Some(column_width),
                    );
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new(CONTENT_BROWSER_SCOPE_DROPDOWN_ID_KEY),
                            response.id,
                        )
                    });
                    if response.changed() {
                        state.content_scope = ContentScope::ALL[scope_index];
                        request_search_for_current_filters(state, true);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(column_width, style::CONTROL_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let sort_options = ModSortMode::ALL.map(ModSortMode::label);
                    let mut sort_index = ModSortMode::ALL
                        .iter()
                        .position(|mode| *mode == state.mod_sort_mode)
                        .unwrap_or(0);
                    let response = settings_widgets::dropdown_picker(
                        text_ui,
                        ui,
                        ("content_browser_mod_sort", instance_id),
                        &mut sort_index,
                        &sort_options,
                        Some(column_width),
                    );
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new(CONTENT_BROWSER_SORT_DROPDOWN_ID_KEY),
                            response.id,
                        )
                    });
                    if response.changed() {
                        state.mod_sort_mode = ModSortMode::ALL[sort_index];
                        request_search_for_current_filters(state, true);
                    }
                },
            );

            ui.allocate_ui_with_layout(
                egui::vec2(column_width, style::CONTROL_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let loader_options = BrowserLoader::ALL.map(BrowserLoader::label);
                    let mut loader_index = BrowserLoader::ALL
                        .iter()
                        .position(|loader| *loader == state.loader)
                        .unwrap_or(0);
                    let response = settings_widgets::dropdown_picker(
                        text_ui,
                        ui,
                        ("content_browser_loader", instance_id),
                        &mut loader_index,
                        &loader_options,
                        Some(column_width),
                    );
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(
                            egui::Id::new(CONTENT_BROWSER_LOADER_DROPDOWN_ID_KEY),
                            response.id,
                        )
                    });
                    if response.changed() {
                        state.loader = BrowserLoader::ALL[loader_index];
                        request_search_for_current_filters(state, true);
                    }
                },
            );
        });

        ui.add_space(style::SPACE_MD);
        let identify_response = ui.add_enabled_ui(!state.identify_in_flight, |ui| {
            settings_widgets::full_width_button(
                text_ui,
                ui,
                ("content_browser_identify_file_button", instance_id),
                "Identify Content File",
                ui.available_width(),
                false,
            )
        });
        if identify_response.inner.clicked()
            && let Some(selected_path) = rfd::FileDialog::new()
                .set_title("Identify Content File")
                .add_filter("Minecraft Content", &["jar", "zip"])
                .add_filter("Mods", &["jar"])
                .add_filter("Packs", &["zip"])
                .pick_file()
        {
            request_identify_file(state, selected_path);
        }

        let queue_status = if state.download_in_flight {
            format!("Downloads: active, {} queued", state.download_queue.len())
        } else if state.download_queue.is_empty() {
            "Downloads: idle".to_owned()
        } else {
            format!("Downloads: idle, {} queued", state.download_queue.len())
        };
        let _ = text_ui.label(
            ui,
            ("content_browser_queue", instance_id),
            queue_status.as_str(),
            &style::muted(ui),
        );
    });
}

#[derive(Default)]
struct RenderResultsOutcome {
    requested_page: Option<u32>,
    open_entry: Option<BrowserProjectEntry>,
}

#[derive(Default)]
struct ResultTileOutcome {
    open_clicked: bool,
    download_clicked: bool,
}

struct IconButtonOutcome {
    clicked: bool,
    rect: egui::Rect,
}

struct ResultTileInnerOutcome {
    open_clicked: bool,
    download_clicked: bool,
    download_button_rect: egui::Rect,
    info_button_rect: egui::Rect,
}

fn render_results(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut ContentBrowserState,
    manifest: &ContentInstallManifest,
    max_height: f32,
) -> RenderResultsOutcome {
    let mut outcome = RenderResultsOutcome::default();
    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(style::SPACE_XL as i8));
    frame.show(ui, |ui| {
        ui.set_min_height(max_height);
        if state.search_in_flight {
            let progress_label = if state.search_total_tasks > 0 {
                format!(
                    "Searching content... {}/{} result groups loaded.",
                    state.search_completed_tasks, state.search_total_tasks
                )
            } else {
                "Searching content...".to_owned()
            };
            let _ = text_ui.label(
                ui,
                ("content_browser_search_progress", instance_id),
                progress_label.as_str(),
                &style::muted(ui),
            );
            ui.add_space(style::SPACE_MD);
        }
        egui::ScrollArea::vertical()
            .id_salt(("content_browser_results_scroll", instance_id))
            .max_height(max_height)
            .show(ui, |ui| {
                ui.add_space(style::SPACE_XS);
                if state.results.entries.is_empty() {
                    let empty_message = if state.search_in_flight {
                        "Searching content..."
                    } else {
                        "No results yet. Search to browse installable content."
                    };
                    let _ = text_ui.label(
                        ui,
                        ("content_browser_empty", instance_id),
                        empty_message,
                        &style::muted(ui),
                    );
                    return;
                }

                let grouped_counts =
                    count_entries_by_content_type(state.results.entries.as_slice());
                let mut current_group = None;
                for entry in &state.results.entries {
                    if current_group != Some(entry.content_type) {
                        current_group = Some(entry.content_type);
                        let group_count = grouped_counts[entry.content_type.index()];
                        let _ = text_ui.label(
                            ui,
                            (
                                "content_browser_group_heading",
                                instance_id,
                                entry.content_type.label(),
                            ),
                            &format!("{} ({group_count})", entry.content_type.label()),
                            &style::stat_label(ui),
                        );
                        ui.add_space(6.0);
                    }

                    let installed_project =
                        installed_project_for_entry(manifest, entry).map(|(_, project)| project);
                    let download_enabled = installed_project.is_none();
                    let download_in_flight = state
                        .active_download
                        .as_ref()
                        .is_some_and(|active| active.dedupe_key == entry.dedupe_key);
                    let tile_outcome = render_result_tile(
                        ui,
                        text_ui,
                        (instance_id, &entry.dedupe_key),
                        entry,
                        installed_project,
                        download_enabled,
                        download_in_flight,
                    );
                    if tile_outcome.download_clicked {
                        state.download_queue.push_back(QueuedContentDownload {
                            request: ContentInstallRequest::Latest {
                                entry: entry.clone(),
                                game_version: state.minecraft_version_filter.clone(),
                                loader: state.loader,
                            },
                        });
                        state.status_message = Some(format!(
                            "Queued {} for download ({} in queue).",
                            entry.name,
                            state.download_queue.len()
                        ));
                    }
                    if tile_outcome.open_clicked && outcome.open_entry.is_none() {
                        outcome.open_entry = Some(entry.clone());
                    }
                    ui.add_space(8.0);
                }
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let pagination_button_style =
                style::neutral_button_with_min_size(ui, egui::vec2(72.0, 30.0));
            if ui
                .add_enabled_ui(!state.search_in_flight && state.current_page > 1, |ui| {
                    text_ui.button(
                        ui,
                        ("content_browser_previous_page", instance_id),
                        "Previous",
                        &pagination_button_style,
                    )
                })
                .inner
                .clicked()
            {
                outcome.requested_page = Some(state.current_page.saturating_sub(1).max(1));
            }

            let _ = text_ui.label(
                ui,
                ("content_browser_page_label", instance_id),
                "Page",
                &style::caption(ui),
            );
            let mut page_value = state.current_page.max(1);
            ui.add(
                egui::DragValue::new(&mut page_value)
                    .range(1..=10_000)
                    .speed(0.1)
                    .max_decimals(0),
            );
            if ui
                .add_enabled_ui(!state.search_in_flight, |ui| {
                    text_ui.button(
                        ui,
                        ("content_browser_go_page", instance_id),
                        "Go",
                        &pagination_button_style,
                    )
                })
                .inner
                .clicked()
            {
                outcome.requested_page = Some(page_value.max(1));
            }

            if ui
                .add_enabled_ui(!state.search_in_flight, |ui| {
                    text_ui.button(
                        ui,
                        ("content_browser_next_page", instance_id),
                        "Next",
                        &pagination_button_style,
                    )
                })
                .inner
                .clicked()
            {
                outcome.requested_page = Some(state.current_page.saturating_add(1).max(1));
            }

            ui.add_space(8.0);
            let _ = text_ui.label(
                ui,
                ("content_browser_page_current", instance_id),
                &format!("Current: {}", state.current_page.max(1)),
                &style::muted_single_line(ui),
            );
        });
    });
    outcome
}

fn render_result_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    entry: &BrowserProjectEntry,
    installed_project: Option<&InstalledContentProject>,
    download_enabled: bool,
    download_in_flight: bool,
) -> ResultTileOutcome {
    let metrics = ContentBrowserUiMetrics::from_ui(ui);
    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            let thumbnail_size =
                egui::vec2(metrics.result_thumbnail_size, metrics.result_thumbnail_size);
            let mut open_clicked = false;
            let mut download_clicked = false;
            let mut download_button_rect = egui::Rect::NOTHING;
            let mut info_button_rect = egui::Rect::NOTHING;
            let installed_label = installed_project
                .and_then(|project| project.selected_version_name.as_deref())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| "Installed".to_owned());

            let render_thumbnail = |ui: &mut Ui| {
                let thumb_frame = egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::same(0));
                thumb_frame.show(ui, |ui| {
                    if let Some(icon_url) = entry.icon_url.as_deref() {
                        remote_tiled_image::show(
                            ui,
                            icon_url,
                            thumbnail_size,
                            (id_source, "remote-icon"),
                            assets::LIBRARY_SVG,
                        );
                    } else {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        id_source.hash(&mut hasher);
                        ui.add(
                            egui::Image::from_bytes(
                                format!("bytes://content-browser/default/{}", hasher.finish()),
                                assets::LIBRARY_SVG,
                            )
                            .fit_to_exact_size(thumbnail_size),
                        );
                    }
                });
            };

            ui.horizontal_top(|ui| {
                render_thumbnail(ui);
                ui.add_space(10.0);
                let text_column_width = ui.available_width().max(140.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(text_column_width, thumbnail_size.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let title_text = if entry.name.trim().is_empty() {
                            "Unnamed"
                        } else {
                            entry.name.trim()
                        };
                        let summary = if entry.summary.trim().is_empty() {
                            "No description provided."
                        } else {
                            entry.summary.trim()
                        };
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            let mut info_hasher = std::collections::hash_map::DefaultHasher::new();
                            (id_source, "info-svg").hash(&mut info_hasher);
                            let info_button_id =
                                format!("content-browser-info-{}", info_hasher.finish());
                            let info_button = render_rounded_icon_button(
                                ui,
                                info_button_id.as_str(),
                                assets::ADJUSTMENTS_SVG,
                                "Open mod details",
                                ui.visuals().widgets.inactive.weak_bg_fill,
                                metrics.action_button_width,
                                metrics.action_button_height,
                                true,
                            );
                            info_button_rect = info_button.rect;
                            if info_button.clicked {
                                open_clicked = true;
                            }
                            ui.add_space(TILE_ACTION_BUTTON_GAP_XS);

                            if download_in_flight {
                                let progress_width = metrics
                                    .download_progress_width
                                    .min(ui.available_width().max(64.0))
                                    .max(metrics.action_button_width * 2.5);
                                let progress = ui.add_sized(
                                    egui::vec2(progress_width, metrics.action_button_height),
                                    egui::ProgressBar::new(0.0)
                                        .animate(true)
                                        .text("Downloading"),
                                );
                                download_button_rect = progress.rect;
                            } else {
                                let mut download_hasher =
                                    std::collections::hash_map::DefaultHasher::new();
                                (id_source, "download-svg").hash(&mut download_hasher);
                                let download_button_id = format!(
                                    "content-browser-download-{}",
                                    download_hasher.finish()
                                );
                                let download_button = render_rounded_icon_button(
                                    ui,
                                    download_button_id.as_str(),
                                    if download_enabled {
                                        assets::DOWNLOAD_SVG
                                    } else {
                                        assets::CHECK_SVG
                                    },
                                    if download_enabled {
                                        "Quick install latest compatible version"
                                    } else {
                                        "Already installed in this instance"
                                    },
                                    ui.visuals().selection.bg_fill,
                                    metrics.action_button_width,
                                    metrics.action_button_height,
                                    download_enabled,
                                );
                                download_button_rect = download_button.rect;
                                if download_button.clicked {
                                    download_clicked = true;
                                }
                            }
                            ui.add_space(TILE_ACTION_BUTTON_GAP_XS);

                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width().max(80.0), 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.set_max_width(ui.available_width().max(80.0));
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        let _ = text_ui.label(
                                            ui,
                                            (id_source, "name"),
                                            title_text,
                                            &LabelOptions {
                                                font_size: 18.0,
                                                line_height: 22.0,
                                                ..style::stat_label(ui)
                                            },
                                        );
                                        render_chip(
                                            ui,
                                            text_ui,
                                            (id_source, "type"),
                                            entry.content_type.label(),
                                        );
                                        for source in &entry.sources {
                                            render_chip(
                                                ui,
                                                text_ui,
                                                (id_source, "source", source.label()),
                                                source.label(),
                                            );
                                        }
                                        if installed_project.is_some() {
                                            render_chip(
                                                ui,
                                                text_ui,
                                                (id_source, "installed"),
                                                installed_label.as_str(),
                                            );
                                        }
                                    });
                                    if ui.min_rect().height() < TILE_ACTION_BUTTON_HEIGHT {
                                        ui.add_space(
                                            TILE_ACTION_BUTTON_HEIGHT - ui.min_rect().height(),
                                        );
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        egui::Frame::new()
                            .fill(ui.visuals().selection.bg_fill.gamma_multiply(0.25))
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::same(6))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt((id_source, "tile_summary_scroll"))
                                    .max_height(52.0)
                                    .show(ui, |ui| {
                                        let _ = text_ui.label(
                                            ui,
                                            (id_source, "summary"),
                                            summary,
                                            &style::body(ui),
                                        );
                                    });
                            });
                    },
                );
            });

            ResultTileInnerOutcome {
                open_clicked,
                download_clicked,
                download_button_rect,
                info_button_rect,
            }
        });

    let response = ui.interact(
        frame.response.rect,
        ui.make_persistent_id((id_source, "open_detail")),
        egui::Sense::click(),
    );
    let ResultTileInnerOutcome {
        open_clicked: button_open_clicked,
        download_clicked: button_download_clicked,
        download_button_rect,
        info_button_rect,
    } = frame.inner;
    let pointer_pos = response.interact_pointer_pos();
    let pointer_over_download =
        response.clicked() && pointer_pos.is_some_and(|pos| download_button_rect.contains(pos));
    let pointer_over_info =
        response.clicked() && pointer_pos.is_some_and(|pos| info_button_rect.contains(pos));
    let overlay_clicked_download = pointer_over_download && download_enabled;
    let overlay_clicked_info = pointer_over_info;
    let pointer_over_action_button = pointer_over_download || pointer_over_info;
    ResultTileOutcome {
        open_clicked: button_open_clicked
            || overlay_clicked_info
            || (response.clicked() && !button_download_clicked && !pointer_over_action_button),
        download_clicked: button_download_clicked || overlay_clicked_download,
    }
}

fn request_search_for_current_filters(state: &mut ContentBrowserState, reset_page: bool) {
    if reset_page {
        state.current_page = 1;
    }
    request_search(
        state,
        BrowserSearchRequest {
            query: normalize_optional(compose_search_query(
                state.query_input.as_str(),
                state.search_tags.as_slice(),
            )),
            tags: state.search_tags.clone(),
            game_version: normalize_optional(state.minecraft_version_filter.as_str()),
            loader: state.loader,
            content_scope: state.content_scope,
            mod_sort_mode: state.mod_sort_mode,
            page: state.current_page.max(1),
        },
    );
}

fn render_chip(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    id_source: impl std::hash::Hash + Copy,
    label: &str,
) {
    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.weak_bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            let _ = text_ui.label(
                ui,
                (id_source, "chip", label),
                label,
                &LabelOptions {
                    font_size: 12.0,
                    line_height: 16.0,
                    color: ui.visuals().text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
            );
        });
}

fn render_search_tag_chips(ui: &mut Ui, search_tags: &mut Vec<String>) -> bool {
    let mut removed_index: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_SM, style::SPACE_SM);
        for (index, tag) in search_tags.iter().enumerate() {
            let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.28);
            let stroke = egui::Stroke::new(1.0, ui.visuals().selection.bg_fill.gamma_multiply(0.7));
            let text_color = ui.visuals().text_color();
            let themed_svg = themed_svg_bytes(assets::X_SVG, text_color);
            let uri = format!(
                "bytes://content-browser/tag-remove/{index}-{:02x}{:02x}{:02x}.svg",
                text_color.r(),
                text_color.g(),
                text_color.b()
            );
            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_height(24.0);
                        let icon_button = egui::Button::image(
                            egui::Image::from_bytes(uri, themed_svg)
                                .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                        )
                        .frame(false)
                        .min_size(egui::vec2(22.0, 22.0));
                        if ui
                            .add(icon_button)
                            .on_hover_text(format!("Remove tag: {tag}"))
                            .clicked()
                        {
                            removed_index = Some(index);
                        }
                        ui.label(tag.as_str());
                    });
                });
        }
    });
    if let Some(index) = removed_index {
        search_tags.remove(index);
        true
    } else {
        false
    }
}

fn add_search_tag(search_tags: &mut Vec<String>, candidate: &str) -> bool {
    let Some(normalized) = normalize_search_tag(candidate) else {
        return false;
    };
    if search_tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(&normalized))
    {
        return false;
    }
    search_tags.push(normalized);
    true
}

fn normalize_search_tag(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn compose_search_query(input: &str, tags: &[String]) -> String {
    let mut parts = Vec::with_capacity(tags.len() + 1);
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_owned());
        }
    }
    let trimmed_input = input.trim();
    if !trimmed_input.is_empty() {
        parts.push(trimmed_input.to_owned());
    }
    parts.join(" ")
}

fn open_detail_page(state: &mut ContentBrowserState, entry: &BrowserProjectEntry) {
    let same_entry = state
        .detail_entry
        .as_ref()
        .is_some_and(|current| current.dedupe_key == entry.dedupe_key);
    state.current_view = ContentBrowserPage::Detail;
    if !same_entry {
        state.detail_entry = Some(entry.clone());
        state.detail_tab = ContentDetailTab::Overview;
        state.detail_versions.clear();
        state.detail_versions_project_key = None;
        state.detail_versions_error = None;
        state.detail_versions_in_flight = false;
        state.detail_loader_filter = state.loader;
        state.detail_minecraft_version_filter = state.minecraft_version_filter.clone();
    }
    request_detail_versions(state);
}

fn render_detail_page(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    instance_root: &Path,
    state: &mut ContentBrowserState,
) {
    let Some(entry) = state.detail_entry.clone() else {
        let _ = text_ui.label(
            ui,
            ("content_browser_detail_missing", instance_id),
            "No content item selected.",
            &style::muted(ui),
        );
        return;
    };

    request_detail_versions(state);
    if state.manifest_dirty || state.cached_manifest.is_none() {
        state.cached_manifest = Some(load_content_manifest(instance_root));
        state.manifest_dirty = false;
    }
    let manifest = state.cached_manifest.clone().expect("just populated");
    let installed_project =
        installed_project_for_entry(&manifest, &entry).map(|(_, project)| project);

    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                render_browser_thumbnail(
                    ui,
                    ("detail", instance_id, &entry.dedupe_key),
                    &entry,
                    96.0,
                );
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    let _ = text_ui.label(
                        ui,
                        (
                            "content_browser_detail_title",
                            instance_id,
                            &entry.dedupe_key,
                        ),
                        entry.name.as_str(),
                        &LabelOptions {
                            wrap: true,
                            ..style::subtitle(ui)
                        },
                    );
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        render_chip(
                            ui,
                            text_ui,
                            ("detail-type", instance_id, &entry.dedupe_key),
                            entry.content_type.label(),
                        );
                        for source in &entry.sources {
                            render_chip(
                                ui,
                                text_ui,
                                (
                                    "detail-source",
                                    instance_id,
                                    &entry.dedupe_key,
                                    source.label(),
                                ),
                                source.label(),
                            );
                        }
                        if let Some(installed) = installed_project {
                            let installed_label = installed
                                .selected_version_name
                                .as_deref()
                                .filter(|value| !value.trim().is_empty())
                                .unwrap_or("Installed");
                            render_chip(
                                ui,
                                text_ui,
                                ("detail-installed", instance_id, &entry.dedupe_key),
                                installed_label,
                            );
                        }
                        ui.add_space(style::SPACE_XS);
                    });
                });
            });
        });

    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        let tab_style = style::neutral_button_with_min_size(ui, egui::vec2(96.0, 30.0));
        for (tab, label) in [
            (ContentDetailTab::Overview, "Overview"),
            (ContentDetailTab::Versions, "Versions"),
        ] {
            let selected = state.detail_tab == tab;
            if text_ui
                .selectable_button(
                    ui,
                    (
                        "content_browser_detail_tab",
                        instance_id,
                        &entry.dedupe_key,
                        label,
                    ),
                    label,
                    selected,
                    &tab_style,
                )
                .clicked()
            {
                state.detail_tab = tab;
            }
        }
    });
    ui.add_space(10.0);

    match state.detail_tab {
        ContentDetailTab::Overview => {
            egui::Frame::new()
                .fill(ui.visuals().widgets.noninteractive.bg_fill)
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt((
                            "content_browser_detail_overview",
                            instance_id,
                            &entry.dedupe_key,
                        ))
                        .max_height(ui.available_height().max(180.0))
                        .show(ui, |ui| {
                            let body = if entry.summary.trim().is_empty() {
                                "No description provided."
                            } else {
                                entry.summary.trim()
                            };
                            let _ = text_ui.label(
                                ui,
                                (
                                    "content_browser_detail_body",
                                    instance_id,
                                    &entry.dedupe_key,
                                ),
                                body,
                                &style::body(ui),
                            );
                        });
                });
        }
        ContentDetailTab::Versions => {
            render_detail_versions_tab(ui, text_ui, instance_id, state, &entry, &manifest);
        }
    }
}

fn render_detail_versions_tab(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut ContentBrowserState,
    entry: &BrowserProjectEntry,
    manifest: &ContentInstallManifest,
) {
    egui::Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            let loader_options = BrowserLoader::ALL.map(BrowserLoader::label);
            let mut detail_loader_index = BrowserLoader::ALL
                .iter()
                .position(|loader| *loader == state.detail_loader_filter)
                .unwrap_or(0);
            let detail_loader_response = settings_widgets::full_width_dropdown_row(
                text_ui,
                ui,
                ("detail_loader_filter", instance_id, &entry.dedupe_key),
                "Loader",
                None,
                &mut detail_loader_index,
                &loader_options,
            );
            if detail_loader_response.changed() {
                state.detail_loader_filter = BrowserLoader::ALL[detail_loader_index];
            }

            ui.add_space(style::SPACE_SM);
            let mut version_options =
                Vec::<String>::with_capacity(state.available_game_versions.len() + 1);
            version_options.push("Any version".to_owned());
            for version in &state.available_game_versions {
                version_options.push(version.display_label());
            }
            let version_option_refs = version_options
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            let mut selected_version_index = 0_usize;
            if !state.detail_minecraft_version_filter.trim().is_empty()
                && let Some(found_index) = state
                    .available_game_versions
                    .iter()
                    .position(|version| version.id == state.detail_minecraft_version_filter)
            {
                selected_version_index = found_index + 1;
            }
            let detail_version_response = settings_widgets::full_width_dropdown_row(
                text_ui,
                ui,
                (
                    "detail_minecraft_version_filter",
                    instance_id,
                    &entry.dedupe_key,
                ),
                "Minecraft Version",
                None,
                &mut selected_version_index,
                &version_option_refs,
            );
            if detail_version_response.changed() {
                state.detail_minecraft_version_filter = if selected_version_index == 0 {
                    String::new()
                } else {
                    state.available_game_versions[selected_version_index - 1]
                        .id
                        .clone()
                };
            }

            if let Some(error) = state.detail_versions_error.as_deref() {
                ui.add_space(8.0);
                let _ = text_ui.label(
                    ui,
                    ("detail_versions_error", instance_id, &entry.dedupe_key),
                    error,
                    &style::warning_text(ui),
                );
            }

            if state.detail_versions_in_flight {
                ui.add_space(8.0);
                let _ = text_ui.label(
                    ui,
                    ("detail_versions_loading", instance_id, &entry.dedupe_key),
                    "Loading versions...",
                    &style::muted(ui),
                );
            }

            ui.add_space(8.0);
            let filtered_versions: Vec<&BrowserVersionEntry> = state
                .detail_versions
                .iter()
                .filter(|version| version_matches_loader(version, state.detail_loader_filter))
                .filter(|version| {
                    version_matches_game_version(
                        version,
                        state.detail_minecraft_version_filter.as_str(),
                    )
                })
                .collect();

            if filtered_versions.is_empty() && !state.detail_versions_in_flight {
                let _ = text_ui.label(
                    ui,
                    ("detail_versions_empty", instance_id, &entry.dedupe_key),
                    "No versions match the current filters.",
                    &style::muted(ui),
                );
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt(("detail_versions_scroll", instance_id, &entry.dedupe_key))
                .max_height(ui.available_height().max(180.0))
                .show(ui, |ui| {
                    for version in filtered_versions {
                        let action = version_row_action(manifest, entry, version);
                        egui::Frame::new()
                            .fill(ui.visuals().widgets.inactive.bg_fill)
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(
                                            (ui.available_width() - TILE_ACTION_BUTTON_WIDTH - 8.0)
                                                .max(160.0),
                                            ui.available_height(),
                                        ),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            let _ = text_ui.label(
                                                ui,
                                                (
                                                    "detail_version_name",
                                                    instance_id,
                                                    &entry.dedupe_key,
                                                    &version.version_id,
                                                ),
                                                version.version_name.as_str(),
                                                &LabelOptions {
                                                    font_size: 17.0,
                                                    line_height: 22.0,
                                                    ..style::stat_label(ui)
                                                },
                                            );
                                            ui.add_space(2.0);
                                            ui.horizontal_wrapped(|ui| {
                                                ui.spacing_mut().item_spacing.x = 4.0;
                                                render_chip(
                                                    ui,
                                                    text_ui,
                                                    (
                                                        "detail_version_source",
                                                        instance_id,
                                                        &entry.dedupe_key,
                                                        &version.version_id,
                                                    ),
                                                    version.source.label(),
                                                );
                                                for loader in &version.loaders {
                                                    render_chip(
                                                        ui,
                                                        text_ui,
                                                        (
                                                            "detail_version_loader",
                                                            instance_id,
                                                            &entry.dedupe_key,
                                                            &version.version_id,
                                                            loader,
                                                        ),
                                                        loader.as_str(),
                                                    );
                                                }
                                                for game_version in
                                                    version.game_versions.iter().take(3)
                                                {
                                                    render_chip(
                                                        ui,
                                                        text_ui,
                                                        (
                                                            "detail_version_mc",
                                                            instance_id,
                                                            &entry.dedupe_key,
                                                            &version.version_id,
                                                            game_version,
                                                        ),
                                                        game_version.as_str(),
                                                    );
                                                }
                                            });
                                            ui.add_space(4.0);
                                            let _ = text_ui.label(
                                                ui,
                                                (
                                                    "detail_version_file",
                                                    instance_id,
                                                    &entry.dedupe_key,
                                                    &version.version_id,
                                                ),
                                                &format!(
                                                    "{} | {}",
                                                    version.file_name, version.published_at
                                                ),
                                                &style::muted(ui),
                                            );
                                        },
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Min),
                                        |ui| {
                                            let (icon, tooltip, fill, enabled) = match action {
                                                VersionRowAction::Installed => (
                                                    assets::CHECK_SVG,
                                                    "Installed version",
                                                    ui.visuals().selection.bg_fill,
                                                    false,
                                                ),
                                                VersionRowAction::Switch => (
                                                    assets::REFRESH_SVG,
                                                    "Switch to this version",
                                                    ui.visuals().warn_fg_color,
                                                    true,
                                                ),
                                                VersionRowAction::Download => (
                                                    assets::DOWNLOAD_SVG,
                                                    "Install this version",
                                                    ui.visuals().selection.bg_fill,
                                                    true,
                                                ),
                                            };
                                            let mut hasher =
                                                std::collections::hash_map::DefaultHasher::new();
                                            (
                                                &entry.dedupe_key,
                                                &version.version_id,
                                                "detail_version_action",
                                            )
                                                .hash(&mut hasher);
                                            let action_id = format!(
                                                "detail-version-action-{}",
                                                hasher.finish()
                                            );
                                            if render_rounded_icon_button(
                                                ui,
                                                action_id.as_str(),
                                                icon,
                                                tooltip,
                                                fill,
                                                TILE_ACTION_BUTTON_WIDTH,
                                                TILE_ACTION_BUTTON_HEIGHT,
                                                enabled,
                                            )
                                            .clicked
                                                && enabled
                                            {
                                                let requested_game_version = if state
                                                    .detail_minecraft_version_filter
                                                    .trim()
                                                    .is_empty()
                                                {
                                                    state.minecraft_version_filter.clone()
                                                } else {
                                                    state.detail_minecraft_version_filter.clone()
                                                };
                                                state.download_queue.push_back(
                                                    QueuedContentDownload {
                                                        request: ContentInstallRequest::Exact {
                                                            entry: entry.clone(),
                                                            version: version.clone(),
                                                            game_version: requested_game_version,
                                                            loader: state.detail_loader_filter,
                                                        },
                                                    },
                                                );
                                                state.status_message = Some(format!(
                                                    "Queued {} {}.",
                                                    match action {
                                                        VersionRowAction::Switch => "switch for",
                                                        VersionRowAction::Installed => "installed",
                                                        VersionRowAction::Download => "install for",
                                                    },
                                                    entry.name
                                                ));
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(8.0);
                    }
                });
        });
}

fn render_browser_thumbnail(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + Copy,
    entry: &BrowserProjectEntry,
    size: f32,
) {
    let thumbnail_size = egui::vec2(size, size);
    if let Some(icon_url) = entry.icon_url.as_deref() {
        remote_tiled_image::show(
            ui,
            icon_url,
            thumbnail_size,
            (id_source, "remote-icon"),
            assets::LIBRARY_SVG,
        );
    } else {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id_source.hash(&mut hasher);
        ui.add(
            egui::Image::from_bytes(
                format!("bytes://content-browser/default/{}", hasher.finish()),
                assets::LIBRARY_SVG,
            )
            .fit_to_exact_size(thumbnail_size),
        );
    }
}

fn render_rounded_icon_button(
    ui: &mut Ui,
    icon_id: &str,
    svg_bytes: &'static [u8],
    tooltip: &str,
    fill: egui::Color32,
    width: f32,
    height: f32,
    enabled: bool,
) -> IconButtonOutcome {
    let text_color = ui.visuals().text_color();
    let themed_svg = themed_svg_bytes(svg_bytes, text_color);
    let uri = format!(
        "bytes://content-browser-rounded/{icon_id}-{:02x}{:02x}{:02x}.svg",
        text_color.r(),
        text_color.g(),
        text_color.b()
    );
    let button_size = egui::vec2(width, height);
    let icon_size = (height - 10.0).max(12.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(button_size, sense);
    let button_fill = if enabled {
        if response.is_pointer_button_down_on() {
            fill.gamma_multiply(0.9)
        } else if response.hovered() {
            fill.gamma_multiply(1.08)
        } else {
            fill
        }
    } else {
        ui.visuals().widgets.inactive.weak_bg_fill
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(8), button_fill);
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let image = egui::Image::from_bytes(uri, themed_svg)
        .fit_to_exact_size(egui::vec2(icon_size, icon_size));
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(icon_size, icon_size));
    let _ = ui.put(icon_rect, image);

    IconButtonOutcome {
        clicked: response.on_hover_text(tooltip).clicked(),
        rect,
    }
}

fn themed_svg_bytes(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", color_hex.as_str())
        .into_bytes()
}

fn version_row_action(
    manifest: &ContentInstallManifest,
    entry: &BrowserProjectEntry,
    version: &BrowserVersionEntry,
) -> VersionRowAction {
    let Some((_, installed)) = installed_project_for_entry(manifest, entry) else {
        return VersionRowAction::Download;
    };
    if installed.selected_source == Some(version.source.into())
        && installed.selected_version_id.as_deref() == Some(version.version_id.as_str())
    {
        VersionRowAction::Installed
    } else {
        VersionRowAction::Switch
    }
}

fn installed_project_for_entry<'a>(
    manifest: &'a ContentInstallManifest,
    entry: &BrowserProjectEntry,
) -> Option<(&'a str, &'a InstalledContentProject)> {
    manifest
        .projects
        .get_key_value(entry.dedupe_key.as_str())
        .map(|(key, project)| (key.as_str(), project))
        .or_else(|| {
            manifest
                .projects
                .iter()
                .find(|(_, project)| installed_project_matches_entry(project, entry))
                .map(|(key, project)| (key.as_str(), project))
        })
}

fn installed_project_matches_entry(
    project: &InstalledContentProject,
    entry: &BrowserProjectEntry,
) -> bool {
    if let (Some(project_id), Some(entry_project_id)) = (
        project.modrinth_project_id.as_deref(),
        entry.modrinth_project_id.as_deref(),
    ) && project_id == entry_project_id
    {
        return true;
    }

    if let (Some(project_id), Some(entry_project_id)) =
        (project.curseforge_project_id, entry.curseforge_project_id)
        && project_id == entry_project_id
    {
        return true;
    }

    false
}

fn version_matches_loader(version: &BrowserVersionEntry, loader: BrowserLoader) -> bool {
    if loader == BrowserLoader::Any || version.loaders.is_empty() {
        return true;
    }
    let Some(expected) = loader.modrinth_slug() else {
        return true;
    };
    version
        .loaders
        .iter()
        .any(|value| normalize_search_key(value).contains(expected))
}

fn version_matches_game_version(version: &BrowserVersionEntry, game_version_filter: &str) -> bool {
    let filter = game_version_filter.trim();
    if filter.is_empty() || version.game_versions.is_empty() {
        return true;
    }
    version
        .game_versions
        .iter()
        .any(|value| value.trim() == filter)
}

fn browser_loader_from_modloader(modloader: &str) -> BrowserLoader {
    match modloader.trim().to_ascii_lowercase().as_str() {
        "fabric" => BrowserLoader::Fabric,
        "forge" => BrowserLoader::Forge,
        "neoforge" => BrowserLoader::NeoForge,
        "quilt" => BrowserLoader::Quilt,
        _ => BrowserLoader::Any,
    }
}

fn normalize_search_key(value: &str) -> String {
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
    normalize_inline_whitespace(normalized.as_str())
}

fn parse_content_type(value: &str) -> Option<BrowserContentType> {
    let normalized = normalize_search_key(value);
    if normalized.contains("shader") {
        Some(BrowserContentType::Shader)
    } else if normalized.contains("resource pack")
        || normalized.contains("resourcepack")
        || normalized.contains("texture pack")
        || normalized.contains("texturepack")
    {
        Some(BrowserContentType::ResourcePack)
    } else if normalized.contains("data pack") || normalized.contains("datapack") {
        Some(BrowserContentType::DataPack)
    } else if normalized.contains("mod") {
        Some(BrowserContentType::Mod)
    } else {
        None
    }
}

pub(crate) fn update_installed_content_to_version(
    instance_root: &Path,
    entry: &UnifiedContentEntry,
    installed_file_path: &Path,
    version_id: &str,
    game_version: &str,
    loader_label: &str,
) -> Result<String, String> {
    update_installed_content_to_version_with_prefetched_downloads(
        instance_root,
        entry,
        installed_file_path,
        version_id,
        game_version,
        loader_label,
        &HashMap::new(),
    )
}

pub(crate) fn bulk_update_installed_content(
    instance_root: &Path,
    updates: &[BulkContentUpdate],
    game_version: &str,
    loader_label: &str,
    download_policy: &DownloadPolicy,
    progress: Option<&InstallProgressCallback>,
) -> Result<usize, String> {
    if updates.is_empty() {
        return Ok(0);
    }
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        root_updates = updates.len(),
        game_version = %game_version,
        loader = %loader_label,
        "planning bulk content update"
    );

    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let loader = browser_loader_from_modloader(loader_label);
    let manifest = load_content_manifest(instance_root);
    let mut planned_versions = HashMap::new();
    let mut queued_paths = HashSet::new();
    let mut download_tasks = Vec::new();
    let mut prefetched_paths = HashMap::new();
    let mut root_updates = Vec::new();

    for update in updates {
        let browser_entry = browser_entry_from_unified_content(&update.entry)?;
        let source = ManagedContentSource::from(update.entry.source);
        let version =
            fetch_exact_version_for_entry(&browser_entry, source, update.version_id.as_str())?;
        let resolved = resolved_download_from_version(version.clone());
        let should_apply = collect_content_download_tasks_for_request(
            instance_root,
            &manifest,
            &browser_entry,
            &resolved,
            game_version,
            loader,
            &modrinth,
            curseforge.as_ref(),
            &mut planned_versions,
            &mut queued_paths,
            &mut download_tasks,
            &mut prefetched_paths,
        )?;
        if should_apply {
            tracing::debug!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %update.entry.name,
                version_id = %version.version_id,
                installed_path = %update.installed_file_path.display(),
                "queued root content update"
            );
            root_updates.push((
                update.entry.clone(),
                update.installed_file_path.clone(),
                version.version_id.clone(),
            ));
        }
    }

    if !download_tasks.is_empty() {
        tracing::info!(
            target: CONTENT_UPDATE_LOG_TARGET,
            instance_root = %instance_root.display(),
            download_tasks = download_tasks.len(),
            root_updates = root_updates.len(),
            "downloading prefetched files for bulk content update"
        );
        download_batch_with_progress(
            download_tasks,
            download_policy,
            InstallStage::DownloadingCore,
            progress,
        )
        .map_err(|err| format!("failed to download queued content updates: {err}"))?;
    }

    let apply_result = (|| -> Result<usize, String> {
        let mut applied = 0usize;
        for (entry, installed_file_path, version_id) in root_updates {
            tracing::info!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %entry.name,
                version_id = %version_id,
                installed_path = %installed_file_path.display(),
                "applying prefetched content update"
            );
            update_installed_content_to_version_with_prefetched_downloads(
                instance_root,
                &entry,
                installed_file_path.as_path(),
                version_id.as_str(),
                game_version,
                loader_label,
                &prefetched_paths,
            )?;
            applied += 1;
        }
        Ok(applied)
    })();
    cleanup_prefetched_downloads(instance_root)?;
    let applied = apply_result?;
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        applied,
        "finished bulk content update apply pass"
    );
    Ok(applied)
}

fn update_installed_content_to_version_with_prefetched_downloads(
    instance_root: &Path,
    entry: &UnifiedContentEntry,
    installed_file_path: &Path,
    version_id: &str,
    game_version: &str,
    loader_label: &str,
    prefetched_paths: &HashMap<PathBuf, PathBuf>,
) -> Result<String, String> {
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %entry.name,
        version_id = %version_id,
        installed_path = %installed_file_path.display(),
        prefetched = !prefetched_paths.is_empty(),
        "starting content version update"
    );
    let browser_entry = browser_entry_from_unified_content(entry)?;
    let source = ManagedContentSource::from(entry.source);
    let version = fetch_exact_version_for_entry(&browser_entry, source, version_id)?;
    let target_path = content_target_path(instance_root, &browser_entry, &version);
    let manifest = load_content_manifest(instance_root);
    let (effective_installed_file_path, stale_requested_path) =
        resolve_installed_file_paths_for_update(
            instance_root,
            &manifest,
            &browser_entry,
            installed_file_path,
        );
    let additional_cleanup_paths = stale_requested_path
        .as_ref()
        .filter(|path| !paths_match_for_update(path.as_path(), target_path.as_path()))
        .cloned()
        .into_iter()
        .collect::<Vec<_>>();
    let staged_existing_path = stage_existing_file_for_update(
        effective_installed_file_path.as_path(),
        target_path.as_path(),
    )?;
    let mut outcome = match apply_content_install_request_with_prefetched_downloads(
        instance_root,
        ContentInstallRequest::Exact {
            entry: browser_entry,
            version,
            game_version: game_version.trim().to_owned(),
            loader: browser_loader_from_modloader(loader_label),
        },
        prefetched_paths,
        additional_cleanup_paths.as_slice(),
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %entry.name,
                version_id = %version_id,
                installed_path = %effective_installed_file_path.display(),
                target_path = %target_path.display(),
                "content version update failed before finalize: {err}"
            );
            if let Some(staged_path) = staged_existing_path {
                restore_staged_update_file(
                    staged_path.as_path(),
                    effective_installed_file_path.as_path(),
                )
                .map_err(|restore_err| {
                    format!("{err} (also failed to restore original file: {restore_err})")
                })?;
            }
            return Err(err);
        }
    };
    if prefetched_paths.is_empty() {
        finalize_updated_file_replacement(
            effective_installed_file_path.as_path(),
            target_path.as_path(),
            staged_existing_path.as_deref(),
            &mut outcome.removed_files,
            None,
        )?;
    } else if let Some(staged_path) = staged_existing_path.as_ref()
        && staged_path.exists()
    {
        remove_content_path(staged_path.as_path())?;
        outcome
            .removed_files
            .push(staged_path.display().to_string());
    }
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %entry.name,
        version_id = %version_id,
        target_path = %target_path.display(),
        added_files = outcome.added_files.len(),
        removed_files = outcome.removed_files.len(),
        "completed content version update"
    );
    Ok(format!(
        "Updated {}: {} added, {} removed.",
        outcome.project_name,
        outcome.added_files.len(),
        outcome.removed_files.len()
    ))
}

fn resolve_installed_file_paths_for_update(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    entry: &BrowserProjectEntry,
    requested_installed_file_path: &Path,
) -> (PathBuf, Option<PathBuf>) {
    let managed_installed_file_path = installed_project_for_entry(manifest, entry)
        .map(|(_, project)| instance_root.join(project.file_path.as_path()))
        .filter(|path| path.exists());
    let effective_installed_file_path = managed_installed_file_path
        .clone()
        .unwrap_or_else(|| requested_installed_file_path.to_path_buf());
    let stale_requested_path = if managed_installed_file_path.is_some()
        && requested_installed_file_path.exists()
        && !paths_match_for_update(
            effective_installed_file_path.as_path(),
            requested_installed_file_path,
        ) {
        Some(requested_installed_file_path.to_path_buf())
    } else {
        None
    };

    (effective_installed_file_path, stale_requested_path)
}

#[allow(clippy::too_many_arguments)]
fn collect_content_download_tasks_for_request(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    entry: &BrowserProjectEntry,
    resolved: &ResolvedDownload,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    planned_versions: &mut HashMap<String, String>,
    queued_paths: &mut HashSet<PathBuf>,
    download_tasks: &mut Vec<DownloadBatchTask>,
    prefetched_paths: &mut HashMap<PathBuf, PathBuf>,
) -> Result<bool, String> {
    let project_key = installed_project_for_entry(manifest, entry)
        .map(|(key, _)| key.to_owned())
        .unwrap_or_else(|| entry.dedupe_key.clone());
    let target_path = content_target_path_for_resolved_download(instance_root, entry, resolved);
    let existing = installed_project_for_entry(manifest, entry).map(|(_, project)| project);

    if let Some(planned_version_id) = planned_versions.get(project_key.as_str()) {
        if planned_version_id == &resolved.version_id {
            return Ok(false);
        }
        return Err(format!(
            "Bulk update requires conflicting versions of {}.",
            entry.name
        ));
    }

    if existing.is_some_and(|project| {
        project.selected_source == Some(resolved.source)
            && project.selected_version_id.as_deref() == Some(resolved.version_id.as_str())
            && target_path.exists()
    }) {
        return Ok(false);
    }

    planned_versions.insert(project_key, resolved.version_id.clone());
    if (existing.is_some() || !target_path.exists()) && queued_paths.insert(target_path.clone()) {
        let prefetched_target =
            prefetched_target_path(instance_root, entry.content_type, target_path.as_path());
        prefetched_paths.insert(target_path.clone(), prefetched_target.clone());
        download_tasks.push(DownloadBatchTask {
            url: resolved.file_url.clone(),
            destination: prefetched_target,
            expected_size: None,
        });
    }

    for dep_entry in
        dependency_to_browser_entries(resolved.dependencies.as_slice(), modrinth, curseforge)?
    {
        let dep_resolved =
            resolve_best_download(&dep_entry, game_version, loader, modrinth, curseforge)?
                .ok_or_else(|| {
                    format!(
                        "No compatible downloadable file found for dependency {}.",
                        dep_entry.name
                    )
                })?;
        let _ = collect_content_download_tasks_for_request(
            instance_root,
            manifest,
            &dep_entry,
            &dep_resolved,
            game_version,
            loader,
            modrinth,
            curseforge,
            planned_versions,
            queued_paths,
            download_tasks,
            prefetched_paths,
        )?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_root(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "vertexlauncher-content-browser-{test_name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn same_path_update_stages_and_removes_previous_file() {
        let root = temp_test_root("same-path");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let mod_path = mods_dir.join("example.jar");
        std::fs::write(mod_path.as_path(), b"old").expect("write old mod");

        let staged_path = stage_existing_file_for_update(mod_path.as_path(), mod_path.as_path())
            .expect("stage old file")
            .expect("expected staged path");
        assert!(
            !mod_path.exists(),
            "original path should be free for replacement"
        );
        assert!(
            staged_path.exists(),
            "backup path should exist after staging"
        );

        std::fs::write(mod_path.as_path(), b"new").expect("write replacement mod");
        let mut removed_files = Vec::new();
        finalize_updated_file_replacement(
            mod_path.as_path(),
            mod_path.as_path(),
            Some(staged_path.as_path()),
            &mut removed_files,
            None,
        )
        .expect("finalize replacement");

        assert!(mod_path.exists(), "new mod should remain in place");
        assert!(
            !staged_path.exists(),
            "staged backup should be removed after successful replacement"
        );
        assert_eq!(removed_files, vec![staged_path.display().to_string()]);

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn different_path_update_removes_superseded_previous_file() {
        let root = temp_test_root("different-path");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let old_path = mods_dir.join("example-1.0.jar");
        let new_path = mods_dir.join("example-2.0.jar");
        std::fs::write(old_path.as_path(), b"old").expect("write old mod");
        std::fs::write(new_path.as_path(), b"new").expect("write new mod");

        let mut removed_files = Vec::new();
        finalize_updated_file_replacement(
            old_path.as_path(),
            new_path.as_path(),
            None,
            &mut removed_files,
            None,
        )
        .expect("finalize replacement");

        assert!(!old_path.exists(), "old mod should be removed after update");
        assert!(new_path.exists(), "new mod should remain after update");
        assert_eq!(removed_files, vec![old_path.display().to_string()]);

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn deferred_cleanup_skips_active_target_paths() {
        let root = temp_test_root("deferred-cleanup");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let active_path = mods_dir.join("example.jar");
        let stale_path = mods_dir.join("old-example.jar");
        let staged_path = mods_dir.join(".vertex-update-backup-example.jar");
        std::fs::write(active_path.as_path(), b"new").expect("write active mod");
        std::fs::write(stale_path.as_path(), b"old").expect("write stale mod");
        std::fs::write(staged_path.as_path(), b"backup").expect("write staged backup");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::example".to_owned(),
            InstalledContentProject {
                project_key: "mod::example".to_owned(),
                name: "Example".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: PathBuf::from("mods/example.jar"),
                modrinth_project_id: None,
                curseforge_project_id: None,
                selected_source: None,
                selected_version_id: None,
                selected_version_name: None,
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );

        let cleanup = DeferredContentCleanup {
            stale_paths: vec![active_path.clone(), stale_path.clone()],
            staged_paths: vec![staged_path.clone()],
        };
        let mut removed_files = Vec::new();
        apply_deferred_content_cleanup(root.as_path(), &manifest, &cleanup, &mut removed_files)
            .expect("apply deferred cleanup");

        assert!(active_path.exists(), "active file should not be deleted");
        assert!(!stale_path.exists(), "stale file should be deleted");
        assert!(!staged_path.exists(), "staged backup should be deleted");
        assert_eq!(
            removed_files,
            vec![
                staged_path.display().to_string(),
                stale_path.display().to_string()
            ]
        );

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn parse_content_type_accepts_modrinth_resourcepack_slug() {
        assert_eq!(
            parse_content_type("resourcepack"),
            Some(BrowserContentType::ResourcePack)
        );
        assert_eq!(
            parse_content_type("texturepack"),
            Some(BrowserContentType::ResourcePack)
        );
    }

    #[test]
    fn resolve_installed_file_paths_prefers_manifest_managed_path() {
        let root = temp_test_root("managed-update-path");
        let mods_dir = root.join("mods");
        std::fs::create_dir_all(mods_dir.as_path()).expect("create mods dir");
        let managed_path = mods_dir.join("example-2.0.jar");
        let stale_path = mods_dir.join("example-1.0.jar");
        std::fs::write(managed_path.as_path(), b"managed").expect("write managed mod");
        std::fs::write(stale_path.as_path(), b"stale").expect("write stale mod");

        let mut manifest = ContentInstallManifest::default();
        manifest.projects.insert(
            "mod::example".to_owned(),
            InstalledContentProject {
                project_key: "mod::example".to_owned(),
                name: "Example".to_owned(),
                folder_name: "mods".to_owned(),
                file_path: PathBuf::from("mods/example-2.0.jar"),
                modrinth_project_id: Some("example-project".to_owned()),
                curseforge_project_id: None,
                selected_source: Some(ManagedContentSource::Modrinth),
                selected_version_id: Some("version-2".to_owned()),
                selected_version_name: Some("2.0".to_owned()),
                pack_managed: false,
                explicitly_installed: true,
                direct_dependencies: Vec::new(),
            },
        );
        let entry = BrowserProjectEntry {
            dedupe_key: "mod::example".to_owned(),
            name: "Example".to_owned(),
            summary: String::new(),
            content_type: BrowserContentType::Mod,
            icon_url: None,
            modrinth_project_id: Some("example-project".to_owned()),
            curseforge_project_id: None,
            sources: vec![ContentSource::Modrinth],
            popularity_score: None,
            updated_at: None,
            relevance_rank: 0,
        };

        let (effective_path, stale_path_to_remove) = resolve_installed_file_paths_for_update(
            root.as_path(),
            &manifest,
            &entry,
            stale_path.as_path(),
        );

        assert_eq!(effective_path, managed_path);
        assert_eq!(stale_path_to_remove, Some(stale_path.clone()));

        let _ = std::fs::remove_dir_all(root.as_path());
    }

    #[test]
    fn prefetched_target_uses_vertex_prefetch_tree() {
        let instance_root = PathBuf::from("instance-root");
        let target = PathBuf::from("resourcepacks/example-pack.zip");
        let prefetched = prefetched_target_path(
            instance_root.as_path(),
            BrowserContentType::ResourcePack,
            target.as_path(),
        );

        assert_eq!(
            prefetched,
            instance_root.join("vertex_prefetch/resourcepacks/example-pack.zip")
        );
    }
}
