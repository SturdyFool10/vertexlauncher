use config::Config;
use curseforge::{Client as CurseForgeClient, MINECRAFT_GAME_ID};
use egui::Ui;
use installation::{MinecraftVersionEntry, fetch_version_catalog};
use instances::{InstanceStore, instance_root_path};
use modprovider::{ContentSource, UnifiedContentEntry};
use modrinth::Client as ModrinthClient;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use textui::{LabelOptions, TextUi};

use crate::app::tokio_runtime;
use crate::assets;
use crate::notification;
use crate::ui::components::remote_tiled_image;

use super::AppScreen;

const CONTENT_SEARCH_PER_PROVIDER_LIMIT: u32 = 35;
const CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE: u32 = 3;
const DEFAULT_DISCOVERY_QUERY_MOD: &str = "mod";
const DEFAULT_DISCOVERY_QUERY_RESOURCE_PACK: &str = "resource pack";
const DEFAULT_DISCOVERY_QUERY_SHADER: &str = "shader";
const DEFAULT_DISCOVERY_QUERY_DATA_PACK: &str = "data pack";
const CONTENT_MANIFEST_FILE_NAME: &str = ".vertex-content-manifest.toml";
const DETAIL_VERSION_FETCH_PAGE_SIZE: u32 = 50;
const DETAIL_VERSION_FETCH_MAX_PAGES: u32 = 5;
const TILE_ACTION_BUTTON_WIDTH: f32 = 28.0;
const TILE_ACTION_BUTTON_HEIGHT: f32 = 28.0;
const TILE_ACTION_BUTTON_GAP_XS: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ContentBrowserPage {
    #[default]
    Browse,
    Detail,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ContentDetailTab {
    #[default]
    Overview,
    Versions,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum BrowserContentType {
    Mod,
    ResourcePack,
    Shader,
    DataPack,
}

impl BrowserContentType {
    const ORDERED: [BrowserContentType; 4] = [
        BrowserContentType::Mod,
        BrowserContentType::ResourcePack,
        BrowserContentType::Shader,
        BrowserContentType::DataPack,
    ];

    fn label(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "Mod",
            BrowserContentType::ResourcePack => "Resource Pack",
            BrowserContentType::Shader => "Shader",
            BrowserContentType::DataPack => "Data Pack",
        }
    }

    fn folder_name(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "mods",
            BrowserContentType::ResourcePack => "resourcepacks",
            BrowserContentType::Shader => "shaderpacks",
            BrowserContentType::DataPack => "datapacks",
        }
    }

    fn default_discovery_query(self) -> &'static str {
        match self {
            BrowserContentType::Mod => DEFAULT_DISCOVERY_QUERY_MOD,
            BrowserContentType::ResourcePack => DEFAULT_DISCOVERY_QUERY_RESOURCE_PACK,
            BrowserContentType::Shader => DEFAULT_DISCOVERY_QUERY_SHADER,
            BrowserContentType::DataPack => DEFAULT_DISCOVERY_QUERY_DATA_PACK,
        }
    }

    fn modrinth_project_type(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "mod",
            BrowserContentType::ResourcePack => "resourcepack",
            BrowserContentType::Shader => "shader",
            BrowserContentType::DataPack => "datapack",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentScope {
    All,
    Mods,
    ResourcePacks,
    Shaders,
    DataPacks,
}

impl ContentScope {
    const ALL: [ContentScope; 5] = [
        ContentScope::All,
        ContentScope::Mods,
        ContentScope::ResourcePacks,
        ContentScope::Shaders,
        ContentScope::DataPacks,
    ];

    fn label(self) -> &'static str {
        match self {
            ContentScope::All => "All Types",
            ContentScope::Mods => "Mods",
            ContentScope::ResourcePacks => "Resource Packs",
            ContentScope::Shaders => "Shaders",
            ContentScope::DataPacks => "Data Packs",
        }
    }

    fn includes(self, content_type: BrowserContentType) -> bool {
        match self {
            ContentScope::All => true,
            ContentScope::Mods => content_type == BrowserContentType::Mod,
            ContentScope::ResourcePacks => content_type == BrowserContentType::ResourcePack,
            ContentScope::Shaders => content_type == BrowserContentType::Shader,
            ContentScope::DataPacks => content_type == BrowserContentType::DataPack,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BrowserLoader {
    Any,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

impl BrowserLoader {
    const ALL: [BrowserLoader; 5] = [
        BrowserLoader::Any,
        BrowserLoader::Fabric,
        BrowserLoader::Forge,
        BrowserLoader::NeoForge,
        BrowserLoader::Quilt,
    ];

    fn label(self) -> &'static str {
        match self {
            BrowserLoader::Any => "Any",
            BrowserLoader::Fabric => "Fabric",
            BrowserLoader::Forge => "Forge",
            BrowserLoader::NeoForge => "NeoForge",
            BrowserLoader::Quilt => "Quilt",
        }
    }

    fn modrinth_slug(self) -> Option<&'static str> {
        match self {
            BrowserLoader::Any => None,
            BrowserLoader::Fabric => Some("fabric"),
            BrowserLoader::Forge => Some("forge"),
            BrowserLoader::NeoForge => Some("neoforge"),
            BrowserLoader::Quilt => Some("quilt"),
        }
    }

    fn curseforge_mod_loader_type(self) -> Option<u32> {
        match self {
            BrowserLoader::Any => None,
            BrowserLoader::Forge => Some(1),
            BrowserLoader::Fabric => Some(4),
            BrowserLoader::Quilt => Some(5),
            BrowserLoader::NeoForge => Some(6),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModSortMode {
    Relevance,
    LastUpdated,
    Popularity,
}

impl ModSortMode {
    const ALL: [ModSortMode; 3] = [
        ModSortMode::Popularity,
        ModSortMode::Relevance,
        ModSortMode::LastUpdated,
    ];

    fn label(self) -> &'static str {
        match self {
            ModSortMode::Relevance => "Relevance",
            ModSortMode::LastUpdated => "Last Update",
            ModSortMode::Popularity => "Popularity",
        }
    }
}

#[derive(Clone, Debug)]
struct BrowserProjectEntry {
    dedupe_key: String,
    name: String,
    summary: String,
    content_type: BrowserContentType,
    icon_url: Option<String>,
    modrinth_project_id: Option<String>,
    curseforge_project_id: Option<u64>,
    sources: Vec<ContentSource>,
    popularity_score: Option<u64>,
    updated_at: Option<String>,
    relevance_rank: u32,
}

#[derive(Clone, Debug, Default)]
struct BrowserSearchSnapshot {
    entries: Vec<BrowserProjectEntry>,
    warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct BrowserSearchResult {
    entries: Vec<BrowserProjectEntry>,
    warnings: Vec<String>,
    query: String,
}

#[derive(Clone, Debug)]
struct BrowserSearchRequest {
    query: Option<String>,
    game_version: Option<String>,
    loader: BrowserLoader,
    content_scope: ContentScope,
    mod_sort_mode: ModSortMode,
    page: u32,
}

#[derive(Clone, Debug)]
enum ContentInstallRequest {
    Latest {
        entry: BrowserProjectEntry,
        game_version: String,
        loader: BrowserLoader,
    },
    Exact {
        entry: BrowserProjectEntry,
        version: BrowserVersionEntry,
        game_version: String,
        loader: BrowserLoader,
    },
}

#[derive(Clone, Debug)]
struct QueuedContentDownload {
    request: ContentInstallRequest,
}

#[derive(Clone, Debug)]
struct ContentDownloadOutcome {
    project_name: String,
    added_files: Vec<String>,
    removed_files: Vec<String>,
}

#[derive(Clone, Debug)]
struct BrowserVersionEntry {
    source: ManagedContentSource,
    version_id: String,
    version_name: String,
    file_name: String,
    file_url: String,
    published_at: String,
    loaders: Vec<String>,
    game_versions: Vec<String>,
    dependencies: Vec<DependencyRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VersionRowAction {
    Download,
    Installed,
    Switch,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ContentInstallManifest {
    projects: HashMap<String, InstalledContentProject>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstalledContentProject {
    project_key: String,
    name: String,
    folder_name: String,
    file_path: String,
    modrinth_project_id: Option<String>,
    curseforge_project_id: Option<u64>,
    selected_source: ManagedContentSource,
    selected_version_id: String,
    selected_version_name: String,
    explicitly_installed: bool,
    direct_dependencies: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct InstalledContentIdentity {
    pub name: String,
    pub source: ContentSource,
    pub modrinth_project_id: Option<String>,
    pub curseforge_project_id: Option<u64>,
}

impl InstalledContentIdentity {
    pub(crate) fn placeholder_entry(&self, content_type: &str) -> UnifiedContentEntry {
        let id = match self.source {
            ContentSource::Modrinth => self
                .modrinth_project_id
                .as_deref()
                .map(|value| format!("modrinth:{value}"))
                .unwrap_or_default(),
            ContentSource::CurseForge => self
                .curseforge_project_id
                .map(|value| format!("curseforge:{value}"))
                .unwrap_or_default(),
        };
        UnifiedContentEntry {
            id,
            name: self.name.clone(),
            summary: String::new(),
            content_type: content_type.to_owned(),
            source: self.source,
            project_url: None,
            icon_url: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ManagedContentSource {
    Modrinth,
    CurseForge,
}

impl ManagedContentSource {
    fn label(self) -> &'static str {
        match self {
            ManagedContentSource::Modrinth => "Modrinth",
            ManagedContentSource::CurseForge => "CurseForge",
        }
    }
}

impl From<ContentSource> for ManagedContentSource {
    fn from(value: ContentSource) -> Self {
        match value {
            ContentSource::Modrinth => ManagedContentSource::Modrinth,
            ContentSource::CurseForge => ManagedContentSource::CurseForge,
        }
    }
}

impl From<ManagedContentSource> for ContentSource {
    fn from(value: ManagedContentSource) -> Self {
        match value {
            ManagedContentSource::Modrinth => ContentSource::Modrinth,
            ManagedContentSource::CurseForge => ContentSource::CurseForge,
        }
    }
}

#[derive(Clone, Debug)]
struct DetailVersionsResult {
    project_key: String,
    versions: Result<Vec<BrowserVersionEntry>, String>,
}

#[derive(Clone, Debug)]
struct ContentBrowserState {
    query_input: String,
    minecraft_version_filter: String,
    content_scope: ContentScope,
    mod_sort_mode: ModSortMode,
    loader: BrowserLoader,
    active_instance_id: Option<String>,
    auto_populated_instance_id: Option<String>,
    current_page: u32,
    current_view: ContentBrowserPage,
    detail_entry: Option<BrowserProjectEntry>,
    detail_tab: ContentDetailTab,
    detail_versions: Vec<BrowserVersionEntry>,
    detail_versions_project_key: Option<String>,
    detail_versions_error: Option<String>,
    detail_versions_in_flight: bool,
    detail_loader_filter: BrowserLoader,
    detail_minecraft_version_filter: String,
    detail_versions_tx: Option<mpsc::Sender<DetailVersionsResult>>,
    detail_versions_rx: Option<Arc<Mutex<mpsc::Receiver<DetailVersionsResult>>>>,
    available_game_versions: Vec<MinecraftVersionEntry>,
    version_catalog_error: Option<String>,
    version_catalog_in_flight: bool,
    version_catalog_tx: Option<mpsc::Sender<Result<Vec<MinecraftVersionEntry>, String>>>,
    version_catalog_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<Vec<MinecraftVersionEntry>, String>>>>>,
    results: BrowserSearchSnapshot,
    search_in_flight: bool,
    search_tx: Option<mpsc::Sender<Result<BrowserSearchResult, String>>>,
    search_rx: Option<Arc<Mutex<mpsc::Receiver<Result<BrowserSearchResult, String>>>>>,
    download_queue: VecDeque<QueuedContentDownload>,
    download_in_flight: bool,
    download_tx: Option<mpsc::Sender<Result<ContentDownloadOutcome, String>>>,
    download_rx: Option<Arc<Mutex<mpsc::Receiver<Result<ContentDownloadOutcome, String>>>>>,
    status_message: Option<String>,
    search_notification_active: bool,
    download_notification_active: bool,
}

impl Default for ContentBrowserState {
    fn default() -> Self {
        Self {
            query_input: String::new(),
            minecraft_version_filter: String::new(),
            content_scope: ContentScope::All,
            mod_sort_mode: ModSortMode::Popularity,
            loader: BrowserLoader::Any,
            active_instance_id: None,
            auto_populated_instance_id: None,
            current_page: 1,
            current_view: ContentBrowserPage::Browse,
            detail_entry: None,
            detail_tab: ContentDetailTab::Overview,
            detail_versions: Vec::new(),
            detail_versions_project_key: None,
            detail_versions_error: None,
            detail_versions_in_flight: false,
            detail_loader_filter: BrowserLoader::Any,
            detail_minecraft_version_filter: String::new(),
            detail_versions_tx: None,
            detail_versions_rx: None,
            available_game_versions: Vec::new(),
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_tx: None,
            version_catalog_rx: None,
            results: BrowserSearchSnapshot::default(),
            search_in_flight: false,
            search_tx: None,
            search_rx: None,
            download_queue: VecDeque::new(),
            download_in_flight: false,
            download_tx: None,
            download_rx: None,
            status_message: None,
            search_notification_active: false,
            download_notification_active: false,
        }
    }
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    selected_instance_id: Option<&str>,
    instances: &InstanceStore,
    config: &Config,
) -> ContentBrowserOutput {
    let mut output = ContentBrowserOutput::default();
    let state_id = ui.make_persistent_id("content_browser_state");
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<ContentBrowserState>(state_id))
        .unwrap_or_default();

    poll_search(&mut state);
    poll_detail_versions(&mut state);
    poll_downloads(&mut state);
    poll_version_catalog(&mut state);

    if state.search_in_flight
        || state.detail_versions_in_flight
        || state.download_in_flight
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
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
        return output;
    };

    let Some(instance) = instances.find(instance_id) else {
        let _ = text_ui.label(
            ui,
            "content_browser_missing_instance",
            "Selected instance no longer exists.",
            &LabelOptions {
                color: ui.visuals().error_fg_color,
                wrap: true,
                ..LabelOptions::default()
            },
        );
        ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
        return output;
    };

    if state.active_instance_id.as_deref() != Some(instance.id.as_str()) {
        state.active_instance_id = Some(instance.id.clone());
        state.loader = browser_loader_from_modloader(instance.modloader.as_str());
        state.minecraft_version_filter = instance.game_version.clone();
        state.content_scope = ContentScope::Mods;
        state.mod_sort_mode = ModSortMode::Popularity;
        state.download_queue.clear();
        state.download_in_flight = false;
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
        state.auto_populated_instance_id = None;
    }

    let installations_root = PathBuf::from(config.minecraft_installations_root());
    let instance_root = instance_root_path(&installations_root, instance);
    let game_version = instance.game_version.trim().to_owned();

    request_version_catalog(&mut state);
    apply_pending_external_detail_open(&mut state);

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
        &LabelOptions {
            color: ui.visuals().weak_text_color(),
            wrap: true,
            ..LabelOptions::default()
        },
    );
    ui.add_space(8.0);

    maybe_start_queued_download(&mut state, instance_root.as_path());

    if let Some(status) = state.status_message.as_deref() {
        let _ = text_ui.label(
            ui,
            ("content_browser_status", instance.id.as_str()),
            status,
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
    }

    for warning in &state.results.warnings {
        let _ = text_ui.label(
            ui,
            ("content_browser_warning", instance.id.as_str(), warning),
            warning,
            &LabelOptions {
                color: ui.visuals().warn_fg_color,
                wrap: true,
                ..LabelOptions::default()
            },
        );
    }

    ui.add_space(8.0);
    match state.current_view {
        ContentBrowserPage::Browse => {
            render_controls(ui, text_ui, instance.id.as_str(), &mut state);

            if state.auto_populated_instance_id.as_deref() != Some(instance.id.as_str())
                && !state.search_in_flight
                && state.query_input.trim().is_empty()
            {
                let request = BrowserSearchRequest {
                    query: None,
                    game_version: normalize_optional(state.minecraft_version_filter.as_str()),
                    loader: state.loader,
                    content_scope: ContentScope::Mods,
                    mod_sort_mode: ModSortMode::Popularity,
                    page: 1,
                };
                state.current_page = 1;
                request_search(&mut state, request);
                state.auto_populated_instance_id = Some(instance.id.clone());
            }

            let results_height = (ui.available_height() - 42.0).max(140.0);
            let render_outcome = render_results(
                ui,
                text_ui,
                instance.id.as_str(),
                &mut state,
                results_height,
            );
            if let Some(page) = render_outcome.requested_page
                && page != state.current_page
            {
                state.current_page = page;
                request_search_for_current_filters(&mut state, false);
            }
            if let Some(entry) = render_outcome.open_entry {
                open_detail_page(&mut state, &entry);
            }
        }
        ContentBrowserPage::Detail => {
            render_detail_page(
                ui,
                text_ui,
                instance.id.as_str(),
                instance_root.as_path(),
                &mut state,
            );
        }
    }

    ui.horizontal(|ui| {
        if state.current_view == ContentBrowserPage::Detail {
            if ui.button("Back to Mod Browser").clicked() {
                state.current_view = ContentBrowserPage::Browse;
            }
        }
        if ui.button("Back to Instance").clicked() {
            output.requested_screen = Some(AppScreen::Instance);
        }
    });

    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
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
        .inner_margin(egui::Margin::same(10));
    frame.show(ui, |ui| {
        let _ = text_ui.label(
            ui,
            ("content_browser_heading", instance_id),
            "Content Browser",
            &LabelOptions {
                font_size: 18.0,
                line_height: 22.0,
                weight: 700,
                color: ui.visuals().text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let edit = egui::TextEdit::singleline(&mut state.query_input)
                .hint_text("Search across Modrinth + CurseForge")
                .desired_width((ui.available_width() - 300.0).max(200.0));
            let response = ui.add(edit);
            let run_search = response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                && !state.search_in_flight;
            if run_search {
                request_search_for_current_filters(state, true);
            }

            egui::ComboBox::from_id_salt(("content_browser_loader", instance_id))
                .selected_text(format!("Loader: {}", state.loader.label()))
                .show_ui(ui, |ui| {
                    for loader in BrowserLoader::ALL {
                        ui.selectable_value(&mut state.loader, loader, loader.label());
                    }
                });

            if ui
                .add_enabled(!state.search_in_flight, egui::Button::new("Search"))
                .clicked()
            {
                request_search_for_current_filters(state, true);
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("content_browser_minecraft_version", instance_id))
                .selected_text(format!(
                    "Minecraft: {}",
                    selected_minecraft_version_label(
                        state.minecraft_version_filter.as_str(),
                        &state.available_game_versions,
                    )
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.minecraft_version_filter,
                        String::new(),
                        "Any version",
                    );
                    for version in &state.available_game_versions {
                        ui.selectable_value(
                            &mut state.minecraft_version_filter,
                            version.id.clone(),
                            version.display_label(),
                        );
                    }
                });

            egui::ComboBox::from_id_salt(("content_browser_scope", instance_id))
                .selected_text(format!("Content: {}", state.content_scope.label()))
                .show_ui(ui, |ui| {
                    for scope in ContentScope::ALL {
                        ui.selectable_value(&mut state.content_scope, scope, scope.label());
                    }
                });

            egui::ComboBox::from_id_salt(("content_browser_mod_sort", instance_id))
                .selected_text(format!("Mods sort: {}", state.mod_sort_mode.label()))
                .show_ui(ui, |ui| {
                    for mode in ModSortMode::ALL {
                        ui.selectable_value(&mut state.mod_sort_mode, mode, mode.label());
                    }
                });
        });

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
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
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

fn render_results(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    instance_id: &str,
    state: &mut ContentBrowserState,
    max_height: f32,
) -> RenderResultsOutcome {
    let mut outcome = RenderResultsOutcome::default();
    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(8));
    frame.show(ui, |ui| {
        ui.set_min_height(max_height);
        egui::ScrollArea::vertical()
            .id_salt(("content_browser_results_scroll", instance_id))
            .max_height(max_height)
            .show(ui, |ui| {
                if state.results.entries.is_empty() {
                    let _ = text_ui.label(
                        ui,
                        ("content_browser_empty", instance_id),
                        "No results yet. Search to browse installable content.",
                        &LabelOptions {
                            color: ui.visuals().weak_text_color(),
                            wrap: true,
                            ..LabelOptions::default()
                        },
                    );
                    return;
                }

                for content_type in BrowserContentType::ORDERED {
                    let mut grouped: Vec<BrowserProjectEntry> = state
                        .results
                        .entries
                        .iter()
                        .filter(|entry| entry.content_type == content_type)
                        .cloned()
                        .collect();
                    if grouped.is_empty() {
                        continue;
                    }
                    if content_type == BrowserContentType::Mod {
                        grouped.sort_by(|left, right| {
                            compare_mod_entries(left, right, state.mod_sort_mode)
                        });
                    } else {
                        grouped.sort_by(|left, right| {
                            left.name
                                .to_ascii_lowercase()
                                .cmp(&right.name.to_ascii_lowercase())
                        });
                    }

                    let _ = text_ui.label(
                        ui,
                        (
                            "content_browser_group_heading",
                            instance_id,
                            content_type.label(),
                        ),
                        &format!("{} ({})", content_type.label(), grouped.len()),
                        &LabelOptions {
                            font_size: 17.0,
                            line_height: 22.0,
                            weight: 700,
                            color: ui.visuals().text_color(),
                            wrap: false,
                            ..LabelOptions::default()
                        },
                    );
                    ui.add_space(6.0);

                    for entry in grouped {
                        let tile_outcome = render_result_tile(
                            ui,
                            text_ui,
                            (instance_id, &entry.dedupe_key),
                            &entry,
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
                }
            });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !state.search_in_flight && state.current_page > 1,
                    egui::Button::new("Previous"),
                )
                .clicked()
            {
                outcome.requested_page = Some(state.current_page.saturating_sub(1).max(1));
            }

            let _ = text_ui.label(
                ui,
                ("content_browser_page_label", instance_id),
                "Page",
                &LabelOptions {
                    color: ui.visuals().weak_text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
            );
            let mut page_value = state.current_page.max(1);
            ui.add(
                egui::DragValue::new(&mut page_value)
                    .range(1..=10_000)
                    .speed(0.1)
                    .max_decimals(0),
            );
            if ui
                .add_enabled(!state.search_in_flight, egui::Button::new("Go"))
                .clicked()
            {
                outcome.requested_page = Some(page_value.max(1));
            }

            if ui
                .add_enabled(!state.search_in_flight, egui::Button::new("Next"))
                .clicked()
            {
                outcome.requested_page = Some(state.current_page.saturating_add(1).max(1));
            }

            ui.add_space(8.0);
            let _ = text_ui.label(
                ui,
                ("content_browser_page_current", instance_id),
                &format!("Current: {}", state.current_page.max(1)),
                &LabelOptions {
                    color: ui.visuals().weak_text_color(),
                    wrap: false,
                    ..LabelOptions::default()
                },
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
) -> ResultTileOutcome {
    let frame = egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            let thumbnail_size = egui::vec2(96.0, 96.0);
            let mut open_clicked = false;
            let mut download_clicked = false;
            let action_cluster_width =
                (TILE_ACTION_BUTTON_WIDTH * 2.0) + (TILE_ACTION_BUTTON_GAP_XS * 2.0);

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
                        let row_width = ui.available_width().max(120.0);
                        let header_width = (row_width - action_cluster_width).max(80.0);

                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(header_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_max_width(header_width);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.set_max_width(header_width);
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        let _ = text_ui.label(
                                            ui,
                                            (id_source, "name"),
                                            title_text,
                                            &LabelOptions {
                                                font_size: 18.0,
                                                line_height: 22.0,
                                                weight: 700,
                                                color: ui.visuals().text_color(),
                                                wrap: false,
                                                ..LabelOptions::default()
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
                                    });
                                    if ui.min_rect().height() < TILE_ACTION_BUTTON_HEIGHT {
                                        ui.add_space(
                                            TILE_ACTION_BUTTON_HEIGHT - ui.min_rect().height(),
                                        );
                                    }
                                },
                            );
                            ui.add_space(TILE_ACTION_BUTTON_GAP_XS);
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            (id_source, "download-svg").hash(&mut hasher);
                            let download_button_id =
                                format!("content-browser-download-{}", hasher.finish());
                            if render_rounded_icon_button(
                                ui,
                                download_button_id.as_str(),
                                assets::DOWNLOAD_SVG,
                                "Quick install latest compatible version",
                                ui.visuals().selection.bg_fill,
                                TILE_ACTION_BUTTON_WIDTH,
                                TILE_ACTION_BUTTON_HEIGHT,
                                true,
                            ) {
                                download_clicked = true;
                            }
                            ui.add_space(TILE_ACTION_BUTTON_GAP_XS);
                            let mut info_hasher = std::collections::hash_map::DefaultHasher::new();
                            (id_source, "info-svg").hash(&mut info_hasher);
                            let info_button_id =
                                format!("content-browser-info-{}", info_hasher.finish());
                            if render_rounded_icon_button(
                                ui,
                                info_button_id.as_str(),
                                assets::ADJUSTMENTS_SVG,
                                "Open mod details",
                                ui.visuals().widgets.inactive.weak_bg_fill,
                                TILE_ACTION_BUTTON_WIDTH,
                                TILE_ACTION_BUTTON_HEIGHT,
                                true,
                            ) {
                                open_clicked = true;
                            }
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
                                            &LabelOptions {
                                                color: ui.visuals().text_color(),
                                                wrap: true,
                                                ..LabelOptions::default()
                                            },
                                        );
                                    });
                            });
                    },
                );
            });

            (open_clicked, download_clicked)
        });

    let response = ui.interact(
        frame.response.rect,
        ui.make_persistent_id((id_source, "open_detail")),
        egui::Sense::click(),
    );
    let (button_open_clicked, button_download_clicked) = frame.inner;
    ResultTileOutcome {
        open_clicked: button_open_clicked || (response.clicked() && !button_download_clicked),
        download_clicked: button_download_clicked,
    }
}

fn request_search_for_current_filters(state: &mut ContentBrowserState, reset_page: bool) {
    if reset_page {
        state.current_page = 1;
    }
    request_search(
        state,
        BrowserSearchRequest {
            query: normalize_optional(state.query_input.as_str()),
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
            &LabelOptions {
                color: ui.visuals().weak_text_color(),
                wrap: true,
                ..LabelOptions::default()
            },
        );
        return;
    };

    request_detail_versions(state);
    let manifest = load_content_manifest(instance_root);

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
                            font_size: 22.0,
                            line_height: 28.0,
                            weight: 700,
                            color: ui.visuals().text_color(),
                            wrap: true,
                            ..LabelOptions::default()
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
                    });
                });
            });
        });

    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for (tab, label) in [
            (ContentDetailTab::Overview, "Overview"),
            (ContentDetailTab::Versions, "Versions"),
        ] {
            let selected = state.detail_tab == tab;
            if ui.selectable_label(selected, label).clicked() {
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
                                &LabelOptions {
                                    color: ui.visuals().text_color(),
                                    wrap: true,
                                    ..LabelOptions::default()
                                },
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
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                egui::ComboBox::from_id_salt((
                    "detail_loader_filter",
                    instance_id,
                    &entry.dedupe_key,
                ))
                .selected_text(format!("Loader: {}", state.detail_loader_filter.label()))
                .show_ui(ui, |ui| {
                    for loader in BrowserLoader::ALL {
                        ui.selectable_value(
                            &mut state.detail_loader_filter,
                            loader,
                            loader.label(),
                        );
                    }
                });
                egui::ComboBox::from_id_salt((
                    "detail_minecraft_version_filter",
                    instance_id,
                    &entry.dedupe_key,
                ))
                .selected_text(format!(
                    "Minecraft: {}",
                    selected_detail_minecraft_version_label(state)
                ))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.detail_minecraft_version_filter,
                        String::new(),
                        "Any version",
                    );
                    for version in &state.available_game_versions {
                        ui.selectable_value(
                            &mut state.detail_minecraft_version_filter,
                            version.id.clone(),
                            version.display_label(),
                        );
                    }
                });
            });

            if let Some(error) = state.detail_versions_error.as_deref() {
                ui.add_space(8.0);
                let _ = text_ui.label(
                    ui,
                    ("detail_versions_error", instance_id, &entry.dedupe_key),
                    error,
                    &LabelOptions {
                        color: ui.visuals().warn_fg_color,
                        wrap: true,
                        ..LabelOptions::default()
                    },
                );
            }

            if state.detail_versions_in_flight {
                ui.add_space(8.0);
                let _ = text_ui.label(
                    ui,
                    ("detail_versions_loading", instance_id, &entry.dedupe_key),
                    "Loading versions...",
                    &LabelOptions {
                        color: ui.visuals().weak_text_color(),
                        wrap: true,
                        ..LabelOptions::default()
                    },
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
                    &LabelOptions {
                        color: ui.visuals().weak_text_color(),
                        wrap: true,
                        ..LabelOptions::default()
                    },
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
                                                    weight: 700,
                                                    color: ui.visuals().text_color(),
                                                    wrap: false,
                                                    ..LabelOptions::default()
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
                                                &LabelOptions {
                                                    color: ui.visuals().weak_text_color(),
                                                    wrap: true,
                                                    ..LabelOptions::default()
                                                },
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
                                            ) && enabled
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
) -> bool {
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

    response.on_hover_text(tooltip).clicked()
}

fn themed_svg_bytes(svg_bytes: &[u8], color: egui::Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", color_hex.as_str())
        .into_bytes()
}

fn selected_detail_minecraft_version_label(state: &ContentBrowserState) -> String {
    selected_minecraft_version_label(
        state.detail_minecraft_version_filter.as_str(),
        &state.available_game_versions,
    )
}

fn selected_minecraft_version_label(
    selected_filter: &str,
    available_game_versions: &[MinecraftVersionEntry],
) -> String {
    let selected = selected_filter.trim();
    if selected.is_empty() {
        return "Any version".to_owned();
    }

    available_game_versions
        .iter()
        .find(|version| version.id == selected)
        .map(MinecraftVersionEntry::display_label)
        .unwrap_or_else(|| selected.to_owned())
}

fn version_row_action(
    manifest: &ContentInstallManifest,
    entry: &BrowserProjectEntry,
    version: &BrowserVersionEntry,
) -> VersionRowAction {
    let Some(installed) = manifest.projects.get(&entry.dedupe_key) else {
        return VersionRowAction::Download;
    };
    if installed.selected_source == version.source
        && installed.selected_version_id == version.version_id
    {
        VersionRowAction::Installed
    } else {
        VersionRowAction::Switch
    }
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
        .any(|value| normalize_type_key(value).contains(expected))
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

fn normalize_type_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_content_type(value: &str) -> Option<BrowserContentType> {
    let normalized = normalize_type_key(value);
    if normalized.contains("shader") {
        Some(BrowserContentType::Shader)
    } else if normalized.contains("resource pack") || normalized.contains("texture pack") {
        Some(BrowserContentType::ResourcePack)
    } else if normalized.contains("data pack") || normalized.contains("datapack") {
        Some(BrowserContentType::DataPack)
    } else if normalized.contains("mod") {
        Some(BrowserContentType::Mod)
    } else {
        None
    }
}

fn ensure_search_channel(state: &mut ContentBrowserState) {
    if state.search_tx.is_some() && state.search_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<BrowserSearchResult, String>>();
    state.search_tx = Some(tx);
    state.search_rx = Some(Arc::new(Mutex::new(rx)));
}

fn ensure_detail_versions_channel(state: &mut ContentBrowserState) {
    if state.detail_versions_tx.is_some() && state.detail_versions_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<DetailVersionsResult>();
    state.detail_versions_tx = Some(tx);
    state.detail_versions_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_detail_versions(state: &mut ContentBrowserState) {
    let Some(entry) = state.detail_entry.clone() else {
        return;
    };
    if state.detail_versions_in_flight {
        return;
    }
    if state.detail_versions_project_key.as_deref() == Some(entry.dedupe_key.as_str())
        && (!state.detail_versions.is_empty() || state.detail_versions_error.is_some())
    {
        return;
    }

    ensure_detail_versions_channel(state);
    let Some(tx) = state.detail_versions_tx.as_ref().cloned() else {
        return;
    };

    state.detail_versions_in_flight = true;
    state.detail_versions_error = None;
    let project_key = entry.dedupe_key.clone();
    let _ = tokio_runtime::spawn(async move {
        let versions = tokio_runtime::spawn_blocking(move || fetch_versions_for_entry(&entry))
            .await
            .map_err(|err| format!("detail versions join error: {err}"))
            .and_then(|inner| inner);
        let _ = tx.send(DetailVersionsResult {
            project_key,
            versions,
        });
    });
}

fn request_version_catalog(state: &mut ContentBrowserState) {
    if state.version_catalog_in_flight
        || !state.available_game_versions.is_empty()
        || state.version_catalog_error.is_some()
    {
        return;
    }

    ensure_version_catalog_channel(state);
    let Some(tx) = state.version_catalog_tx.as_ref().cloned() else {
        return;
    };

    state.version_catalog_in_flight = true;
    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            fetch_version_catalog(false)
                .map(|catalog| catalog.game_versions)
                .map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| format!("version catalog task join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send(result);
    });
}

fn apply_pending_external_detail_open(state: &mut ContentBrowserState) {
    let Some(store) = PENDING_EXTERNAL_DETAIL_OPEN.get() else {
        return;
    };
    let Ok(mut pending) = store.lock() else {
        return;
    };
    let Some(entry) = pending.take() else {
        return;
    };

    let Some(content_type) = parse_content_type(entry.content_type.as_str()) else {
        return;
    };

    let mut browser_entry = BrowserProjectEntry {
        dedupe_key: format!(
            "{}::{}",
            content_type.label().to_ascii_lowercase(),
            normalize_lookup_key(entry.name.as_str())
        ),
        name: entry.name,
        summary: entry.summary,
        content_type,
        icon_url: entry.icon_url,
        modrinth_project_id: None,
        curseforge_project_id: None,
        sources: vec![entry.source],
        popularity_score: None,
        updated_at: None,
        relevance_rank: 0,
    };

    match entry.source {
        ContentSource::Modrinth => {
            browser_entry.modrinth_project_id = entry
                .id
                .strip_prefix("modrinth:")
                .map(str::to_owned)
                .or_else(|| (!entry.id.trim().is_empty()).then(|| entry.id.clone()));
        }
        ContentSource::CurseForge => {
            browser_entry.curseforge_project_id = entry
                .id
                .strip_prefix("curseforge:")
                .or_else(|| (!entry.id.trim().is_empty()).then_some(entry.id.as_str()))
                .and_then(|value| value.parse::<u64>().ok());
        }
    }

    open_detail_page(state, &browser_entry);
}

fn ensure_version_catalog_channel(state: &mut ContentBrowserState) {
    if state.version_catalog_tx.is_some() && state.version_catalog_rx.is_some() {
        return;
    }

    let (tx, rx) = mpsc::channel::<Result<Vec<MinecraftVersionEntry>, String>>();
    state.version_catalog_tx = Some(tx);
    state.version_catalog_rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_version_catalog(state: &mut ContentBrowserState) {
    let mut should_reset_channel = false;
    let mut updates = Vec::new();

    if let Some(rx) = state.version_catalog_rx.as_ref() {
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
            Err(_) => {
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.version_catalog_tx = None;
        state.version_catalog_rx = None;
        state.version_catalog_in_flight = false;
    }

    for update in updates {
        state.version_catalog_in_flight = false;
        match update {
            Ok(versions) => {
                state.available_game_versions = versions;
                state.version_catalog_error = None;
            }
            Err(err) => {
                state.version_catalog_error = Some(err);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ProviderSearchEntry {
    name: String,
    summary: String,
    content_type: BrowserContentType,
    source: ContentSource,
    modrinth_project_id: Option<String>,
    curseforge_project_id: Option<u64>,
    icon_url: Option<String>,
    popularity_score: Option<u64>,
    updated_at: Option<String>,
    relevance_rank: u32,
}

fn request_search(state: &mut ContentBrowserState, request: BrowserSearchRequest) {
    if state.search_in_flight {
        return;
    }

    ensure_search_channel(state);
    let Some(tx) = state.search_tx.as_ref().cloned() else {
        return;
    };

    state.search_in_flight = true;
    state.search_notification_active = true;
    notification::progress!(
        notification::Severity::Info,
        "content-browser/search",
        0.1f32,
        "Searching content..."
    );
    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || run_search_request(request))
            .await
            .map_err(|err| format!("content browser search join error: {err}"))
            .and_then(|inner| inner);
        let _ = tx.send(result);
    });
}

fn fetch_versions_for_entry(
    entry: &BrowserProjectEntry,
) -> Result<Vec<BrowserVersionEntry>, String> {
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let mut versions = Vec::new();

    if let Some(project_id) = entry.modrinth_project_id.as_deref() {
        let project_versions = modrinth
            .list_project_versions(project_id, &[], &[])
            .map_err(|err| format!("Modrinth versions failed for {project_id}: {err}"))?;
        for version in project_versions {
            let Some(file) = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
            else {
                continue;
            };
            let mut dependencies = Vec::new();
            for dep in version.dependencies {
                if !dep.dependency_type.eq_ignore_ascii_case("required") {
                    continue;
                }
                if let Some(dep_project) = dep.project_id {
                    dependencies.push(DependencyRef::ModrinthProject(dep_project));
                } else if let Some(dep_version) = dep.version_id
                    && let Ok(version_detail) = modrinth.get_version(dep_version.as_str())
                    && !version_detail.project_id.trim().is_empty()
                {
                    dependencies.push(DependencyRef::ModrinthProject(version_detail.project_id));
                }
            }
            versions.push(BrowserVersionEntry {
                source: ManagedContentSource::Modrinth,
                version_id: version.id,
                version_name: version.version_number,
                file_name: file.filename.clone(),
                file_url: file.url.clone(),
                published_at: version.date_published,
                loaders: version.loaders,
                game_versions: version.game_versions,
                dependencies,
            });
        }
    }

    if let Some(curseforge_project_id) = entry.curseforge_project_id
        && let Some(curseforge) = curseforge.as_ref()
    {
        let files = fetch_curseforge_versions(curseforge, curseforge_project_id)?;
        for file in files {
            let Some(download_url) = file.download_url else {
                continue;
            };
            let mut dependencies = Vec::new();
            for dep in file.dependencies {
                if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
                    dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
                }
            }
            let (loaders, game_versions) = split_curseforge_game_versions(file.game_versions);
            versions.push(BrowserVersionEntry {
                source: ManagedContentSource::CurseForge,
                version_id: file.id.to_string(),
                version_name: file.display_name.clone(),
                file_name: file.file_name,
                file_url: download_url,
                published_at: file.file_date,
                loaders,
                game_versions,
                dependencies,
            });
        }
    }

    versions.sort_by(|left, right| {
        right
            .published_at
            .cmp(&left.published_at)
            .then_with(|| left.version_name.cmp(&right.version_name))
    });
    Ok(versions)
}

fn fetch_curseforge_versions(
    client: &CurseForgeClient,
    project_id: u64,
) -> Result<Vec<curseforge::File>, String> {
    let mut index = 0u32;
    let mut files = Vec::new();
    for _ in 0..DETAIL_VERSION_FETCH_MAX_PAGES {
        let batch = client
            .list_mod_files(
                project_id,
                None,
                None,
                index,
                DETAIL_VERSION_FETCH_PAGE_SIZE,
            )
            .map_err(|err| format!("CurseForge files failed for {project_id}: {err}"))?;
        let batch_len = batch.len() as u32;
        files.extend(batch);
        if batch_len < DETAIL_VERSION_FETCH_PAGE_SIZE {
            break;
        }
        index = index.saturating_add(DETAIL_VERSION_FETCH_PAGE_SIZE);
    }
    Ok(files)
}

fn split_curseforge_game_versions(values: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut loaders = Vec::new();
    let mut game_versions = Vec::new();
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "fabric" | "forge" | "neoforge" | "quilt"
        ) {
            loaders.push(value);
        } else if value.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
            game_versions.push(value);
        }
    }
    (loaders, game_versions)
}

fn run_search_request(request: BrowserSearchRequest) -> Result<BrowserSearchResult, String> {
    let query = request
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let game_version = request
        .game_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();

    let mut warnings = Vec::new();
    let curseforge_class_ids = if let Some(client) = curseforge.as_ref() {
        resolve_curseforge_class_ids(client, &mut warnings)
    } else {
        warnings.push(
            "CurseForge API key missing (set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY). Showing Modrinth results only."
                .to_owned(),
        );
        HashMap::new()
    };

    let mut provider_entries = Vec::new();
    let page = request.page.max(1);
    let provider_offset = page
        .saturating_sub(1)
        .saturating_mul(CONTENT_SEARCH_PER_PROVIDER_LIMIT);
    for content_type in BrowserContentType::ORDERED {
        if !request.content_scope.includes(content_type) {
            continue;
        }
        let query_for_type = query
            .clone()
            .unwrap_or_else(|| content_type.default_discovery_query().to_owned());
        let mod_loader = if content_type == BrowserContentType::Mod {
            request.loader.modrinth_slug()
        } else {
            None
        };
        match modrinth.search_projects_with_filters(
            query_for_type.as_str(),
            CONTENT_SEARCH_PER_PROVIDER_LIMIT,
            provider_offset,
            Some(content_type.modrinth_project_type()),
            game_version.as_deref(),
            mod_loader,
        ) {
            Ok(entries) => {
                let mut scored_entries: Vec<ProviderSearchEntry> = entries
                    .into_iter()
                    .enumerate()
                    .map(|(idx, entry)| ProviderSearchEntry {
                        name: entry.title,
                        summary: entry.description,
                        content_type,
                        source: ContentSource::Modrinth,
                        modrinth_project_id: Some(entry.project_id),
                        curseforge_project_id: None,
                        icon_url: entry.icon_url,
                        popularity_score: Some(entry.downloads),
                        updated_at: entry.date_modified,
                        relevance_rank: idx as u32,
                    })
                    .collect();
                if content_type == BrowserContentType::Mod
                    && request.mod_sort_mode == ModSortMode::Popularity
                {
                    rescore_modrinth_entries_by_version_popularity(
                        &mut scored_entries,
                        game_version.as_deref(),
                        request.loader,
                        &modrinth,
                    );
                }
                provider_entries.extend(scored_entries);
            }
            Err(err) => warnings.push(format!(
                "Modrinth search failed for {}: {err}",
                content_type.label()
            )),
        }

        let Some(curseforge) = curseforge.as_ref() else {
            continue;
        };
        let Some(class_id) = curseforge_class_ids.get(&content_type).copied() else {
            continue;
        };
        let mod_loader_type = if content_type == BrowserContentType::Mod {
            request.loader.curseforge_mod_loader_type()
        } else {
            None
        };
        match curseforge.search_projects_with_filters(
            MINECRAFT_GAME_ID,
            query_for_type.as_str(),
            provider_offset,
            CONTENT_SEARCH_PER_PROVIDER_LIMIT,
            Some(class_id),
            game_version.as_deref(),
            mod_loader_type,
        ) {
            Ok(entries) => {
                let mut scored_entries: Vec<ProviderSearchEntry> = entries
                    .into_iter()
                    .enumerate()
                    .map(|(idx, entry)| ProviderSearchEntry {
                        name: entry.name,
                        summary: entry.summary,
                        content_type,
                        source: ContentSource::CurseForge,
                        modrinth_project_id: None,
                        curseforge_project_id: Some(entry.id),
                        icon_url: entry.icon_url,
                        popularity_score: Some(entry.download_count),
                        updated_at: entry.date_modified,
                        relevance_rank: idx as u32,
                    })
                    .collect();
                if content_type == BrowserContentType::Mod
                    && request.mod_sort_mode == ModSortMode::Popularity
                {
                    rescore_curseforge_entries_by_version_popularity(
                        &mut scored_entries,
                        game_version.as_deref(),
                        request.loader,
                        curseforge,
                    );
                }
                provider_entries.extend(scored_entries);
            }
            Err(err) => warnings.push(format!(
                "CurseForge search failed for {}: {err}",
                content_type.label()
            )),
        }
    }

    let mut deduped = dedupe_browser_entries(provider_entries);
    deduped.sort_by(|left, right| {
        left.content_type.cmp(&right.content_type).then_with(|| {
            if left.content_type == BrowserContentType::Mod {
                compare_mod_entries(left, right, request.mod_sort_mode)
            } else {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }
        })
    });
    Ok(BrowserSearchResult {
        entries: deduped,
        warnings,
        query: query.unwrap_or_default(),
    })
}

fn resolve_curseforge_class_ids(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> HashMap<BrowserContentType, u32> {
    let mut by_type = HashMap::new();
    match client.list_content_classes(MINECRAFT_GAME_ID) {
        Ok(classes) => {
            for class_entry in classes {
                let normalized = normalize_type_key(class_entry.name.as_str());
                if normalized.contains("shader") {
                    by_type.insert(BrowserContentType::Shader, class_entry.id);
                } else if normalized.contains("resource")
                    || normalized.contains("texture pack")
                    || normalized.contains("texture")
                {
                    by_type.insert(BrowserContentType::ResourcePack, class_entry.id);
                } else if normalized.contains("data pack") || normalized.contains("datapack") {
                    by_type.insert(BrowserContentType::DataPack, class_entry.id);
                } else if normalized.contains("mod") {
                    by_type.insert(BrowserContentType::Mod, class_entry.id);
                }
            }
        }
        Err(err) => warnings.push(format!("CurseForge class discovery failed: {err}")),
    }
    by_type.entry(BrowserContentType::Mod).or_insert(6);
    by_type
}

fn rescore_modrinth_entries_by_version_popularity(
    entries: &mut [ProviderSearchEntry],
    game_version: Option<&str>,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
) {
    let mut loaders = Vec::new();
    if let Some(loader_slug) = loader.modrinth_slug() {
        loaders.push(loader_slug.to_owned());
    }
    let game_versions = game_version
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| vec![value.to_owned()])
        .unwrap_or_default();

    for entry in entries {
        let Some(project_id) = entry.modrinth_project_id.as_deref() else {
            continue;
        };
        let Ok(versions) = modrinth.list_project_versions(project_id, &loaders, &game_versions)
        else {
            continue;
        };
        let latest_version_downloads = versions
            .into_iter()
            .filter(|version| !version.files.is_empty())
            .max_by(|left, right| left.date_published.cmp(&right.date_published))
            .map(|version| version.downloads);
        if let Some(downloads) = latest_version_downloads {
            entry.popularity_score = Some(downloads);
        }
    }
}

fn rescore_curseforge_entries_by_version_popularity(
    entries: &mut [ProviderSearchEntry],
    game_version: Option<&str>,
    loader: BrowserLoader,
    curseforge: &CurseForgeClient,
) {
    let game_version = game_version
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mod_loader_type = loader.curseforge_mod_loader_type();

    for entry in entries {
        let Some(project_id) = entry.curseforge_project_id else {
            continue;
        };
        let Ok(files) = curseforge.list_mod_files(project_id, game_version, mod_loader_type, 0, 50)
        else {
            continue;
        };
        let latest_file_downloads = files
            .into_iter()
            .filter(|file| file.download_url.is_some())
            .max_by(|left, right| left.file_date.cmp(&right.file_date))
            .map(|file| file.download_count);
        if let Some(downloads) = latest_file_downloads {
            entry.popularity_score = Some(downloads);
        }
    }
}

fn poll_search(state: &mut ContentBrowserState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.search_rx.as_ref() {
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
        state.search_tx = None;
        state.search_rx = None;
        state.search_in_flight = false;
    }

    for update in updates {
        state.search_in_flight = false;
        if state.search_notification_active {
            state.search_notification_active = false;
        }
        match update {
            Ok(search) => {
                state.query_input = search.query;
                state.results.entries = search.entries;
                state.results.warnings = search.warnings;
                notification::progress!(
                    notification::Severity::Info,
                    "content-browser/search",
                    1.0f32,
                    "Content search complete."
                );
            }
            Err(err) => {
                state.results.entries.clear();
                state.results.warnings = vec![err];
                notification::warn!("content-browser/search", "Content search failed.");
            }
        }
    }
}

fn poll_detail_versions(state: &mut ContentBrowserState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.detail_versions_rx.as_ref() {
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
        state.detail_versions_tx = None;
        state.detail_versions_rx = None;
        state.detail_versions_in_flight = false;
    }

    for update in updates {
        state.detail_versions_in_flight = false;
        if state
            .detail_entry
            .as_ref()
            .is_some_and(|entry| entry.dedupe_key == update.project_key)
        {
            match update.versions {
                Ok(versions) => {
                    state.detail_versions_project_key = Some(update.project_key);
                    state.detail_versions = versions;
                    state.detail_versions_error = None;
                }
                Err(err) => {
                    state.detail_versions_project_key = Some(update.project_key);
                    state.detail_versions.clear();
                    state.detail_versions_error = Some(err);
                }
            }
        }
    }
}

fn dedupe_browser_entries(entries: Vec<ProviderSearchEntry>) -> Vec<BrowserProjectEntry> {
    let mut by_key = HashMap::<String, BrowserProjectEntry>::new();
    for entry in entries {
        let ProviderSearchEntry {
            name,
            summary,
            content_type,
            source,
            modrinth_project_id,
            curseforge_project_id,
            icon_url,
            popularity_score,
            updated_at,
            relevance_rank,
        } = entry;
        let name_key = normalize_lookup_key(name.as_str());
        if name_key.is_empty() {
            continue;
        }
        let dedupe_key = format!("{}::{name_key}", content_type.label().to_ascii_lowercase());

        let merged = by_key
            .entry(dedupe_key.clone())
            .or_insert_with(|| BrowserProjectEntry {
                dedupe_key: dedupe_key.clone(),
                name: name.clone(),
                summary: summary.clone(),
                content_type,
                icon_url: icon_url.clone(),
                modrinth_project_id: modrinth_project_id.clone(),
                curseforge_project_id,
                sources: Vec::new(),
                popularity_score,
                updated_at: updated_at.clone(),
                relevance_rank,
            });
        if merged.summary.trim().len() < summary.trim().len() {
            merged.summary = summary;
        }
        if merged.icon_url.is_none() {
            merged.icon_url = icon_url;
        }
        if merged.modrinth_project_id.is_none() {
            merged.modrinth_project_id = modrinth_project_id;
        }
        if merged.curseforge_project_id.is_none() {
            merged.curseforge_project_id = curseforge_project_id;
        }
        if let Some(popularity) = popularity_score
            && merged.popularity_score.unwrap_or(0) < popularity
        {
            merged.popularity_score = Some(popularity);
        }
        if let Some(updated_at) = updated_at
            && merged
                .updated_at
                .as_deref()
                .is_none_or(|current| current < updated_at.as_str())
        {
            merged.updated_at = Some(updated_at);
        }
        if relevance_rank < merged.relevance_rank {
            merged.relevance_rank = relevance_rank;
        }
        if !merged.sources.contains(&source) {
            merged.sources.push(source);
            merged.sources.sort_by_key(|source| source.label());
        }
    }
    by_key.into_values().collect()
}

fn compare_mod_entries(
    left: &BrowserProjectEntry,
    right: &BrowserProjectEntry,
    mode: ModSortMode,
) -> std::cmp::Ordering {
    match mode {
        ModSortMode::Relevance => left
            .relevance_rank
            .cmp(&right.relevance_rank)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
        ModSortMode::LastUpdated => right
            .updated_at
            .as_deref()
            .unwrap_or("")
            .cmp(left.updated_at.as_deref().unwrap_or(""))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
        ModSortMode::Popularity => right
            .popularity_score
            .unwrap_or(0)
            .cmp(&left.popularity_score.unwrap_or(0))
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }),
    }
}

fn ensure_download_channel(state: &mut ContentBrowserState) {
    if state.download_tx.is_some() && state.download_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<ContentDownloadOutcome, String>>();
    state.download_tx = Some(tx);
    state.download_rx = Some(Arc::new(Mutex::new(rx)));
}

fn maybe_start_queued_download(state: &mut ContentBrowserState, instance_root: &Path) {
    if state.download_in_flight {
        return;
    }
    let Some(next) = state.download_queue.pop_front() else {
        return;
    };

    ensure_download_channel(state);
    let Some(tx) = state.download_tx.as_ref().cloned() else {
        return;
    };

    state.download_in_flight = true;
    state.download_notification_active = true;
    notification::progress!(
        notification::Severity::Info,
        "content-browser/download",
        0.1f32,
        "Applying queued content operation..."
    );
    let root = instance_root.to_path_buf();
    let request = next.request.clone();

    let _ = tokio_runtime::spawn(async move {
        let result = tokio_runtime::spawn_blocking(move || {
            apply_content_install_request(root.as_path(), request)
        })
        .await
        .map_err(|err| format!("content operation join error: {err}"))
        .and_then(|inner| inner);
        let _ = tx.send(result);
    });
}

fn poll_downloads(state: &mut ContentBrowserState) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    if let Some(rx) = state.download_rx.as_ref() {
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
        state.download_tx = None;
        state.download_rx = None;
        state.download_in_flight = false;
    }

    for update in updates {
        state.download_in_flight = false;
        if state.download_notification_active {
            state.download_notification_active = false;
        }
        match update {
            Ok(result) => {
                state.status_message = Some(format!(
                    "Applied {}: {} added, {} removed.",
                    result.project_name,
                    result.added_files.len(),
                    result.removed_files.len()
                ));
                notification::progress!(
                    notification::Severity::Info,
                    "content-browser/download",
                    1.0f32,
                    "Content operation complete."
                );
            }
            Err(err) => {
                state.status_message = Some(format!("Content download failed: {err}"));
                notification::error!(
                    "content-browser/download",
                    "Content download failed: {}",
                    err
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
enum DependencyRef {
    ModrinthProject(String),
    CurseForgeProject(u64),
}

#[derive(Clone, Debug)]
struct ResolvedDownload {
    source: ManagedContentSource,
    version_id: String,
    version_name: String,
    file_url: String,
    file_name: String,
    published_at: String,
    dependencies: Vec<DependencyRef>,
}

fn apply_content_install_request(
    instance_root: &Path,
    request: ContentInstallRequest,
) -> Result<ContentDownloadOutcome, String> {
    let modrinth = ModrinthClient::default();
    let curseforge = CurseForgeClient::from_env();
    let mut added_files = Vec::new();
    let mut removed_files = Vec::new();

    let (root_entry, game_version, loader, root_download) = match request {
        ContentInstallRequest::Latest {
            entry,
            game_version,
            loader,
        } => {
            let resolved = resolve_best_download(
                &entry,
                game_version.as_str(),
                loader,
                &modrinth,
                curseforge.as_ref(),
            )?
            .ok_or_else(|| format!("No compatible downloadable file found for {}.", entry.name))?;
            (entry, game_version, loader, resolved)
        }
        ContentInstallRequest::Exact {
            entry,
            version,
            game_version,
            loader,
        } => (
            entry,
            game_version,
            loader,
            resolved_download_from_version(version),
        ),
    };

    let mut manifest = load_content_manifest(instance_root);
    if let Some(existing) = manifest.projects.get(&root_entry.dedupe_key).cloned() {
        if existing.selected_source == root_download.source
            && existing.selected_version_id == root_download.version_id
        {
            if let Some(record) = manifest.projects.get_mut(&root_entry.dedupe_key) {
                record.explicitly_installed = true;
            }
            save_content_manifest(instance_root, &manifest)?;
            return Ok(ContentDownloadOutcome {
                project_name: root_entry.name,
                added_files,
                removed_files,
            });
        }
        let dependents = manifest_dependents(&manifest, root_entry.dedupe_key.as_str());
        if !dependents.is_empty() {
            return Err(format!(
                "Cannot switch {} while it is required by {}.",
                root_entry.name,
                dependents.join(", ")
            ));
        }
        remove_installed_project(
            instance_root,
            &mut manifest,
            root_entry.dedupe_key.as_str(),
            true,
            &mut removed_files,
        )?;
    }

    let mut visited = HashSet::new();
    install_project_recursive(
        instance_root,
        &mut manifest,
        &root_entry,
        root_download,
        game_version.as_str(),
        loader,
        &modrinth,
        curseforge.as_ref(),
        None,
        true,
        &mut visited,
        &mut added_files,
    )?;
    save_content_manifest(instance_root, &manifest)?;

    Ok(ContentDownloadOutcome {
        project_name: root_entry.name,
        added_files,
        removed_files,
    })
}

#[allow(clippy::too_many_arguments)]
fn install_project_recursive(
    instance_root: &Path,
    manifest: &mut ContentInstallManifest,
    entry: &BrowserProjectEntry,
    resolved: ResolvedDownload,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    parent_key: Option<&str>,
    explicit: bool,
    visited: &mut HashSet<String>,
    added_files: &mut Vec<String>,
) -> Result<(), String> {
    let project_key = entry.dedupe_key.clone();
    if let Some(parent_key) = parent_key {
        append_project_dependency(manifest, parent_key, project_key.as_str());
    }

    if !visited.insert(project_key.clone()) {
        if explicit && let Some(existing) = manifest.projects.get_mut(&project_key) {
            existing.explicitly_installed = true;
        }
        return Ok(());
    }

    if let Some(existing) = manifest.projects.get_mut(&project_key) {
        if explicit {
            existing.explicitly_installed = true;
        }
        return Ok(());
    }

    let target_dir = instance_root.join(entry.content_type.folder_name());
    std::fs::create_dir_all(target_dir.as_path())
        .map_err(|err| format!("failed to create content folder {:?}: {err}", target_dir))?;
    let target_name = normalized_filename(resolved.file_name.as_str(), resolved.file_url.as_str());
    let target_path = target_dir.join(target_name.as_str());
    if !target_path.exists() {
        download_file(resolved.file_url.as_str(), target_path.as_path())?;
        added_files.push(target_path.display().to_string());
    }

    let file_path = target_path
        .strip_prefix(instance_root)
        .unwrap_or(target_path.as_path())
        .display()
        .to_string();
    manifest.projects.insert(
        project_key.clone(),
        InstalledContentProject {
            project_key: project_key.clone(),
            name: entry.name.clone(),
            folder_name: entry.content_type.folder_name().to_owned(),
            file_path,
            modrinth_project_id: entry.modrinth_project_id.clone(),
            curseforge_project_id: entry.curseforge_project_id,
            selected_source: resolved.source,
            selected_version_id: resolved.version_id.clone(),
            selected_version_name: resolved.version_name.clone(),
            explicitly_installed: explicit,
            direct_dependencies: Vec::new(),
        },
    );

    let mut dependency_keys = Vec::new();
    for dependency in resolved.dependencies {
        let Some(dep_entry) = dependency_to_browser_entry(&dependency, modrinth, curseforge)?
        else {
            continue;
        };
        let dep_resolved =
            resolve_best_download(&dep_entry, game_version, loader, modrinth, curseforge)?
                .ok_or_else(|| {
                    format!(
                        "No compatible downloadable file found for dependency {}.",
                        dep_entry.name
                    )
                })?;
        dependency_keys.push(dep_entry.dedupe_key.clone());
        install_project_recursive(
            instance_root,
            manifest,
            &dep_entry,
            dep_resolved,
            game_version,
            loader,
            modrinth,
            curseforge,
            Some(project_key.as_str()),
            false,
            visited,
            added_files,
        )?;
    }

    if let Some(record) = manifest.projects.get_mut(&project_key) {
        record.direct_dependencies = dependency_keys;
    }
    Ok(())
}

fn append_project_dependency(
    manifest: &mut ContentInstallManifest,
    parent_key: &str,
    dependency_key: &str,
) {
    if let Some(parent) = manifest.projects.get_mut(parent_key)
        && !parent
            .direct_dependencies
            .iter()
            .any(|existing| existing == dependency_key)
    {
        parent.direct_dependencies.push(dependency_key.to_owned());
    }
}

fn remove_installed_project(
    instance_root: &Path,
    manifest: &mut ContentInstallManifest,
    project_key: &str,
    force: bool,
    removed_files: &mut Vec<String>,
) -> Result<(), String> {
    let Some(existing) = manifest.projects.get(project_key).cloned() else {
        return Ok(());
    };
    if !force {
        if existing.explicitly_installed {
            return Ok(());
        }
        if !manifest_dependents(manifest, project_key).is_empty() {
            return Ok(());
        }
    }

    manifest.projects.remove(project_key);
    for project in manifest.projects.values_mut() {
        project
            .direct_dependencies
            .retain(|dependency| dependency != project_key);
    }

    let file_path = instance_root.join(existing.file_path.as_str());
    if file_path.exists() {
        std::fs::remove_file(file_path.as_path())
            .map_err(|err| format!("failed to remove {}: {err}", file_path.display()))?;
        removed_files.push(file_path.display().to_string());
    }

    for dependency_key in existing.direct_dependencies {
        remove_installed_project(
            instance_root,
            manifest,
            dependency_key.as_str(),
            false,
            removed_files,
        )?;
    }

    Ok(())
}

fn resolved_download_from_version(version: BrowserVersionEntry) -> ResolvedDownload {
    ResolvedDownload {
        source: version.source,
        version_id: version.version_id,
        version_name: version.version_name,
        file_url: version.file_url,
        file_name: version.file_name,
        published_at: version.published_at,
        dependencies: version.dependencies,
    }
}

fn manifest_dependents(manifest: &ContentInstallManifest, project_key: &str) -> Vec<String> {
    manifest
        .projects
        .iter()
        .filter(|(key, project)| {
            key.as_str() != project_key
                && project
                    .direct_dependencies
                    .iter()
                    .any(|dependency| dependency == project_key)
        })
        .map(|(_, project)| project.name.clone())
        .collect()
}

fn resolve_best_download(
    entry: &BrowserProjectEntry,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Option<ResolvedDownload>, String> {
    let modrinth_candidate = resolve_modrinth_download(entry, game_version, loader, modrinth)?;
    let curseforge_candidate =
        resolve_curseforge_download(entry, game_version, loader, curseforge)?;
    Ok(match (modrinth_candidate, curseforge_candidate) {
        (Some(left), Some(right)) => {
            if left.published_at >= right.published_at {
                Some(left)
            } else {
                Some(right)
            }
        }
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    })
}

fn resolve_modrinth_download(
    entry: &BrowserProjectEntry,
    game_version: &str,
    loader: BrowserLoader,
    modrinth: &ModrinthClient,
) -> Result<Option<ResolvedDownload>, String> {
    let Some(project_id) = entry.modrinth_project_id.as_deref() else {
        return Ok(None);
    };

    let mut loaders = Vec::new();
    if matches!(entry.content_type, BrowserContentType::Mod)
        && let Some(loader_slug) = loader.modrinth_slug()
    {
        loaders.push(loader_slug.to_owned());
    }
    let game_versions = if game_version.trim().is_empty() {
        Vec::new()
    } else {
        vec![game_version.trim().to_owned()]
    };

    let versions = modrinth
        .list_project_versions(project_id, &loaders, &game_versions)
        .map_err(|err| format!("Modrinth versions failed for {project_id}: {err}"))?;

    Ok(versions
        .into_iter()
        .filter_map(|version| {
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())?;
            let mut dependencies = Vec::new();
            for dep in version.dependencies {
                if !dep.dependency_type.eq_ignore_ascii_case("required") {
                    continue;
                }
                if let Some(dep_project) = dep.project_id {
                    dependencies.push(DependencyRef::ModrinthProject(dep_project));
                } else if let Some(dep_version) = dep.version_id
                    && let Ok(version) = modrinth.get_version(dep_version.as_str())
                    && !version.project_id.trim().is_empty()
                {
                    dependencies.push(DependencyRef::ModrinthProject(version.project_id));
                }
            }
            Some(ResolvedDownload {
                source: ManagedContentSource::Modrinth,
                version_id: version.id.clone(),
                version_name: version.version_number.clone(),
                file_url: file.url.clone(),
                file_name: file.filename.clone(),
                published_at: version.date_published,
                dependencies,
            })
        })
        .max_by(|left, right| left.published_at.cmp(&right.published_at)))
}

fn resolve_curseforge_download(
    entry: &BrowserProjectEntry,
    game_version: &str,
    loader: BrowserLoader,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Option<ResolvedDownload>, String> {
    let Some(curseforge) = curseforge else {
        return Ok(None);
    };
    let Some(project_id) = entry.curseforge_project_id else {
        return Ok(None);
    };

    let mod_loader_type = if matches!(entry.content_type, BrowserContentType::Mod) {
        loader.curseforge_mod_loader_type()
    } else {
        None
    };

    let files = curseforge
        .list_mod_files(
            project_id,
            normalize_optional(game_version).as_deref(),
            mod_loader_type,
            0,
            50,
        )
        .map_err(|err| format!("CurseForge files failed for {project_id}: {err}"))?;

    Ok(files
        .into_iter()
        .filter_map(|file| {
            let url = file.download_url?;
            let mut dependencies = Vec::new();
            for dep in file.dependencies {
                if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
                    dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
                }
            }
            Some(ResolvedDownload {
                source: ManagedContentSource::CurseForge,
                version_id: file.id.to_string(),
                version_name: file.display_name.clone(),
                file_url: url,
                file_name: file.file_name,
                published_at: file.file_date,
                dependencies,
            })
        })
        .max_by(|left, right| left.published_at.cmp(&right.published_at)))
}

fn dependency_to_browser_entry(
    dependency: &DependencyRef,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Option<BrowserProjectEntry>, String> {
    match dependency {
        DependencyRef::ModrinthProject(project_id) => {
            let project = modrinth.get_project(project_id.as_str()).map_err(|err| {
                format!("Modrinth dependency lookup failed for {project_id}: {err}")
            })?;
            let Some(content_type) = parse_content_type(project.project_type.as_str()) else {
                return Ok(None);
            };
            let name_key = normalize_lookup_key(project.title.as_str());
            if name_key.is_empty() {
                return Ok(None);
            }
            Ok(Some(BrowserProjectEntry {
                dedupe_key: format!("{}::{name_key}", content_type.label().to_ascii_lowercase()),
                name: project.title,
                summary: project.description,
                content_type,
                icon_url: project.icon_url,
                modrinth_project_id: Some(project.project_id),
                curseforge_project_id: None,
                sources: vec![ContentSource::Modrinth],
                popularity_score: None,
                updated_at: None,
                relevance_rank: u32::MAX,
            }))
        }
        DependencyRef::CurseForgeProject(project_id) => {
            let Some(curseforge) = curseforge else {
                return Ok(None);
            };
            let project = curseforge.get_mod(*project_id).map_err(|err| {
                format!("CurseForge dependency lookup failed for {project_id}: {err}")
            })?;
            let name_key = normalize_lookup_key(project.name.as_str());
            if name_key.is_empty() {
                return Ok(None);
            }
            Ok(Some(BrowserProjectEntry {
                dedupe_key: format!("mod::{name_key}"),
                name: project.name,
                summary: project.summary,
                content_type: BrowserContentType::Mod,
                icon_url: project.icon_url,
                modrinth_project_id: None,
                curseforge_project_id: Some(project.id),
                sources: vec![ContentSource::CurseForge],
                popularity_score: None,
                updated_at: None,
                relevance_rank: u32::MAX,
            }))
        }
    }
}

fn content_manifest_path(instance_root: &Path) -> PathBuf {
    instance_root.join(CONTENT_MANIFEST_FILE_NAME)
}

fn load_content_manifest(instance_root: &Path) -> ContentInstallManifest {
    let path = content_manifest_path(instance_root);
    let mut manifest = std::fs::read_to_string(path.as_path())
        .ok()
        .and_then(|raw| toml::from_str::<ContentInstallManifest>(&raw).ok())
        .unwrap_or_default();
    normalize_content_manifest(instance_root, &mut manifest);
    manifest
}

pub(crate) fn load_managed_content_identities(
    instance_root: &Path,
) -> HashMap<String, InstalledContentIdentity> {
    let manifest = load_content_manifest(instance_root);
    manifest
        .projects
        .into_values()
        .map(|project| {
            (
                normalize_content_path_key(project.file_path.as_str()),
                InstalledContentIdentity {
                    name: project.name,
                    source: project.selected_source.into(),
                    modrinth_project_id: project.modrinth_project_id,
                    curseforge_project_id: project.curseforge_project_id,
                },
            )
        })
        .collect()
}

fn save_content_manifest(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
) -> Result<(), String> {
    let mut normalized = manifest.clone();
    normalize_content_manifest(instance_root, &mut normalized);
    let path = content_manifest_path(instance_root);
    if normalized.projects.is_empty() {
        if path.exists() {
            let _ = std::fs::remove_file(path.as_path());
        }
        return Ok(());
    }
    let raw = toml::to_string_pretty(&normalized)
        .map_err(|err| format!("failed to serialize content manifest: {err}"))?;
    std::fs::write(path.as_path(), raw)
        .map_err(|err| format!("failed to write content manifest {}: {err}", path.display()))
}

fn normalize_content_manifest(instance_root: &Path, manifest: &mut ContentInstallManifest) {
    let missing_keys: Vec<String> = manifest
        .projects
        .iter()
        .filter_map(|(key, value)| {
            let file_path = instance_root.join(value.file_path.as_str());
            if file_path.exists() {
                None
            } else {
                Some(key.clone())
            }
        })
        .collect();
    for key in missing_keys {
        manifest.projects.remove(key.as_str());
    }

    let project_keys: HashSet<String> = manifest.projects.keys().cloned().collect();
    for (key, value) in &mut manifest.projects {
        value.project_key = key.clone();
        value
            .direct_dependencies
            .retain(|dependency| dependency != key && project_keys.contains(dependency));
        value.direct_dependencies.sort();
        value.direct_dependencies.dedup();
    }
}

fn normalize_content_path_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("./")
        .trim_start_matches(".\\")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn normalized_filename(name: &str, url: &str) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    url.rsplit('/').next().unwrap_or("download.bin").to_owned()
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .call()
        .map_err(|err| format!("download request failed for {url}: {err}"))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read download body from {url}: {err}"))?;
    let mut file = std::fs::File::create(destination)
        .map_err(|err| format!("failed to create {:?}: {err}", destination))?;
    file.write_all(&bytes)
        .map_err(|err| format!("failed to write {:?}: {err}", destination))?;
    Ok(())
}
