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

    fn index(self) -> usize {
        match self {
            BrowserContentType::Mod => 0,
            BrowserContentType::ResourcePack => 1,
            BrowserContentType::Shader => 2,
            BrowserContentType::DataPack => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
enum SearchUpdate {
    Snapshot {
        request: BrowserSearchRequest,
        snapshot: BrowserSearchSnapshot,
        completed_tasks: usize,
        total_tasks: usize,
        finished: bool,
    },
    Failed {
        request: BrowserSearchRequest,
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BrowserSearchRequest {
    query: Option<String>,
    tags: Vec<String>,
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
pub(crate) struct BulkContentUpdate {
    pub entry: UnifiedContentEntry,
    pub installed_file_path: PathBuf,
    pub version_id: String,
}

#[derive(Debug, Default)]
struct DeferredContentCleanup {
    stale_paths: Vec<PathBuf>,
    staged_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveContentDownload {
    dedupe_key: String,
    version_id: Option<String>,
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

#[derive(Clone, Debug)]
struct DetailVersionsResult {
    project_key: String,
    versions: Result<Vec<BrowserVersionEntry>, String>,
}

#[derive(Clone, Debug)]
pub struct ContentBrowserState {
    query_input: String,
    search_tags: Vec<String>,
    minecraft_version_filter: String,
    content_scope: ContentScope,
    mod_sort_mode: ModSortMode,
    loader: BrowserLoader,
    active_instance_id: Option<String>,
    active_instance_name: Option<String>,
    auto_populated_instance_id: Option<String>,
    current_page: u32,
    current_view: ContentBrowserPage,
    detail_entry: Option<BrowserProjectEntry>,
    detail_tab: ContentDetailTab,
    detail_versions: Vec<BrowserVersionEntry>,
    detail_versions_cache: HashMap<String, Result<Vec<BrowserVersionEntry>, String>>,
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
    active_search_request: Option<BrowserSearchRequest>,
    search_cache: HashMap<BrowserSearchRequest, BrowserSearchSnapshot>,
    search_completed_tasks: usize,
    search_total_tasks: usize,
    search_in_flight: bool,
    search_tx: Option<mpsc::Sender<SearchUpdate>>,
    search_rx: Option<Arc<Mutex<mpsc::Receiver<SearchUpdate>>>>,
    download_queue: VecDeque<QueuedContentDownload>,
    download_in_flight: bool,
    active_download: Option<ActiveContentDownload>,
    download_tx: Option<mpsc::Sender<Result<ContentDownloadOutcome, String>>>,
    download_rx: Option<Arc<Mutex<mpsc::Receiver<Result<ContentDownloadOutcome, String>>>>>,
    identify_in_flight: bool,
    identify_tx: Option<mpsc::Sender<(PathBuf, Result<UnifiedContentEntry, String>)>>,
    identify_rx: Option<Arc<Mutex<mpsc::Receiver<(PathBuf, Result<UnifiedContentEntry, String>)>>>>,
    status_message: Option<String>,
    search_notification_active: bool,
    download_notification_active: bool,
    cached_manifest: Option<ContentInstallManifest>,
    manifest_dirty: bool,
}

impl Default for ContentBrowserState {
    fn default() -> Self {
        Self {
            query_input: String::new(),
            search_tags: Vec::new(),
            minecraft_version_filter: String::new(),
            content_scope: ContentScope::All,
            mod_sort_mode: ModSortMode::Popularity,
            loader: BrowserLoader::Any,
            active_instance_id: None,
            active_instance_name: None,
            auto_populated_instance_id: None,
            current_page: 1,
            current_view: ContentBrowserPage::Browse,
            detail_entry: None,
            detail_tab: ContentDetailTab::Overview,
            detail_versions: Vec::new(),
            detail_versions_cache: HashMap::new(),
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
            active_search_request: None,
            search_cache: HashMap::new(),
            search_completed_tasks: 0,
            search_total_tasks: 0,
            search_in_flight: false,
            search_tx: None,
            search_rx: None,
            download_queue: VecDeque::new(),
            download_in_flight: false,
            active_download: None,
            download_tx: None,
            download_rx: None,
            identify_in_flight: false,
            identify_tx: None,
            identify_rx: None,
            status_message: None,
            search_notification_active: false,
            download_notification_active: false,
            cached_manifest: None,
            manifest_dirty: true,
        }
    }
}

impl ContentBrowserState {
    pub fn purge_inactive_state(&mut self) {
        self.current_view = ContentBrowserPage::Browse;
        self.detail_entry = None;
        self.detail_tab = ContentDetailTab::Overview;
        self.detail_versions.clear();
        self.detail_versions_cache.clear();
        self.detail_versions_project_key = None;
        self.detail_versions_error = None;
        self.detail_versions_in_flight = false;
        self.detail_versions_tx = None;
        self.detail_versions_rx = None;
        self.results = BrowserSearchSnapshot::default();
        self.active_search_request = None;
        self.search_cache.clear();
        self.search_completed_tasks = 0;
        self.search_total_tasks = 0;
        self.search_in_flight = false;
        self.search_tx = None;
        self.search_rx = None;
        self.download_queue.clear();
        self.download_in_flight = false;
        self.active_download = None;
        self.download_tx = None;
        self.download_rx = None;
        self.identify_in_flight = false;
        self.identify_tx = None;
        self.identify_rx = None;
        self.status_message = None;
        self.search_notification_active = false;
        self.download_notification_active = false;
    }
}

fn trim_content_browser_search_cache(state: &mut ContentBrowserState) {
    if state.search_cache.len() <= CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let active_request = state.active_search_request.clone();
    state.search_cache.retain(|request, _| {
        active_request
            .as_ref()
            .is_some_and(|active| active == request)
            || request.page >= state.current_page.saturating_sub(2)
    });
    if state.search_cache.len() <= CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES {
        return;
    }
    let mut requests = state.search_cache.keys().cloned().collect::<Vec<_>>();
    requests.sort_by_key(|request| request.page);
    for request in requests {
        if state.search_cache.len() <= CONTENT_BROWSER_SEARCH_CACHE_MAX_ENTRIES {
            break;
        }
        if active_request.as_ref() != Some(&request) {
            state.search_cache.remove(&request);
        }
    }
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

fn ensure_search_channel(state: &mut ContentBrowserState) {
    if state.search_tx.is_some() && state.search_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<SearchUpdate>();
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
    if let Some(cached) = state
        .detail_versions_cache
        .get(entry.dedupe_key.as_str())
        .cloned()
    {
        state.detail_versions_in_flight = false;
        state.detail_versions_project_key = Some(entry.dedupe_key.clone());
        match cached {
            Ok(versions) => {
                state.detail_versions = versions;
                state.detail_versions_error = None;
            }
            Err(error) => {
                state.detail_versions.clear();
                state.detail_versions_error = Some(error);
            }
        }
        return;
    }
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
    let _ = tokio_runtime::spawn_detached(async move {
        let versions: Result<Vec<BrowserVersionEntry>, String> = match tokio::time::timeout(
            DETAIL_VERSIONS_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || fetch_versions_for_entry(&entry)),
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
        if let Err(err) = tx.send(DetailVersionsResult {
            project_key,
            versions,
        }) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver content detail-version result."
            );
        }
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
    let _ = tokio_runtime::spawn_detached(async move {
        let result: Result<Vec<MinecraftVersionEntry>, String> = match tokio::time::timeout(
            VERSION_CATALOG_FETCH_TIMEOUT,
            tokio_runtime::spawn_blocking(move || {
                fetch_version_catalog(false)
                    .map(|catalog| catalog.game_versions)
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
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver content version catalog result."
            );
        }
    });
}

fn apply_pending_external_detail_open(state: &mut ContentBrowserState) {
    let Some(store) = PENDING_EXTERNAL_DETAIL_OPEN.get() else {
        return;
    };
    let Ok(mut pending) = store.lock() else {
        tracing::error!(
            target: "vertexlauncher/content_browser",
            "Content browser pending external detail-open store mutex was poisoned."
        );
        return;
    };
    let Some(entry) = pending.take() else {
        return;
    };

    let Ok(browser_entry) = browser_entry_from_unified_content(&entry) else {
        return;
    };

    open_detail_page(state, &browser_entry);
}

fn browser_entry_from_unified_content(
    entry: &UnifiedContentEntry,
) -> Result<BrowserProjectEntry, String> {
    let Some(content_type) = parse_content_type(entry.content_type.as_str()) else {
        return Err(format!("Unsupported content type for {}.", entry.name));
    };
    let name_key = normalize_search_key(entry.name.as_str());
    if name_key.is_empty() {
        return Err("Content entry name cannot be empty.".to_owned());
    }

    let mut browser_entry = BrowserProjectEntry {
        dedupe_key: format!("{}::{name_key}", content_type.label().to_ascii_lowercase()),
        name: entry.name.clone(),
        summary: entry.summary.clone(),
        content_type,
        icon_url: entry.icon_url.clone(),
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

    Ok(browser_entry)
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
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            "Content-browser version catalog worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    "Content-browser version catalog receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.version_catalog_tx = None;
        state.version_catalog_rx = None;
        state.version_catalog_in_flight = false;
        state.version_catalog_error =
            Some("Version catalog worker stopped unexpectedly.".to_owned());
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

fn ensure_identify_channel(state: &mut ContentBrowserState) {
    if state.identify_tx.is_some() && state.identify_rx.is_some() {
        return;
    }

    let (tx, rx) = mpsc::channel::<(PathBuf, Result<UnifiedContentEntry, String>)>();
    state.identify_tx = Some(tx);
    state.identify_rx = Some(Arc::new(Mutex::new(rx)));
}

fn request_identify_file(state: &mut ContentBrowserState, selected_path: PathBuf) {
    if state.identify_in_flight {
        return;
    }
    if detect_installed_content_kind(selected_path.as_path()).is_none() {
        state.status_message = Some(format!(
            "Unsupported content file: {}. Expected a mod .jar or supported pack .zip.",
            selected_path.display()
        ));
        return;
    }

    ensure_identify_channel(state);
    let Some(tx) = state.identify_tx.as_ref().cloned() else {
        return;
    };

    state.identify_in_flight = true;
    state.status_message = Some(format!(
        "Identifying {} in the background...",
        selected_path.display()
    ));
    let _ = tokio_runtime::spawn_detached(async move {
        let path_for_result = selected_path.clone();
        let join = tokio_runtime::spawn_blocking(move || {
            identify_mod_file_by_hash(selected_path.as_path())
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(format!("content identification worker panicked: {err}")),
        };
        if let Err(err) = tx.send((path_for_result.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                path = %path_for_result.display(),
                error = %err,
                "Failed to deliver content identification result."
            );
        }
    });
}

fn poll_identify_results(state: &mut ContentBrowserState) {
    let Some(rx) = state.identify_rx.as_ref() else {
        return;
    };

    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => loop {
            match receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/content_browser",
                        "Content identification worker disconnected unexpectedly."
                    );
                    should_reset_channel = true;
                    break;
                }
            }
        },
        Err(_) => {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                "Content identification receiver mutex was poisoned."
            );
            should_reset_channel = true;
        }
    }

    if should_reset_channel {
        state.identify_tx = None;
        state.identify_rx = None;
        state.identify_in_flight = false;
        state.status_message =
            Some("Content identification worker stopped unexpectedly.".to_owned());
    }

    for (path, result) in updates {
        state.identify_in_flight = false;
        match result {
            Ok(entry) => {
                let project_name = entry.name.clone();
                request_open_detail_for_content(entry);
                apply_pending_external_detail_open(state);
                state.status_message = Some(format!(
                    "Identified {} from {}.",
                    project_name,
                    path.display()
                ));
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/content_browser",
                    path = %path.display(),
                    error = %err,
                    "Content identification failed."
                );
                state.status_message =
                    Some(format!("Could not identify {}: {err}", path.display()));
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

#[derive(Default)]
struct SearchTaskOutcome {
    entries: Vec<ProviderSearchEntry>,
    warnings: Vec<String>,
}

fn request_search(state: &mut ContentBrowserState, request: BrowserSearchRequest) {
    if state.search_in_flight {
        return;
    }
    let total_tasks = content_scope_task_count(request.content_scope);
    if let Some(cached) = state.search_cache.get(&request).cloned() {
        state.query_input = request.query.clone().unwrap_or_default();
        state.active_search_request = Some(request);
        state.search_completed_tasks = total_tasks;
        state.search_total_tasks = total_tasks;
        state.results = cached;
        trim_content_browser_search_cache(state);
        return;
    }

    ensure_search_channel(state);
    let Some(tx) = state.search_tx.as_ref().cloned() else {
        return;
    };

    state.active_search_request = Some(request.clone());
    state.search_completed_tasks = 0;
    state.search_total_tasks = total_tasks;
    state.results = BrowserSearchSnapshot::default();
    state.search_in_flight = true;
    state.search_notification_active = true;
    notification::progress!(
        notification::Severity::Info,
        "content-browser/search",
        0.1f32,
        "Searching content..."
    );
    let request_for_failure = request.clone();
    let _ = tokio_runtime::spawn_detached(async move {
        let worker_tx = tx.clone();
        let join = tokio_runtime::spawn_blocking(move || run_search_request(request, worker_tx));
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(format!("content search worker panicked: {err}")),
        };
        match result {
            Ok(()) => {}
            Err(err) => {
                if let Err(err_send) = tx.send(SearchUpdate::Failed {
                    request: request_for_failure,
                    error: err,
                }) {
                    tracing::error!(
                        target: "vertexlauncher/content_browser",
                        error = %err_send,
                        "Failed to deliver content search failure update."
                    );
                }
            }
        }
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
        let dependency_version_projects = modrinth_dependency_project_ids(
            &modrinth,
            project_versions
                .iter()
                .flat_map(|version| version.dependencies.iter().cloned())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        for version in project_versions {
            let Some(file) = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
            else {
                continue;
            };
            let dependencies = modrinth_dependency_refs(
                version.dependencies.as_slice(),
                &dependency_version_projects,
            );
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

fn fetch_exact_version_for_entry(
    entry: &BrowserProjectEntry,
    source: ManagedContentSource,
    version_id: &str,
) -> Result<BrowserVersionEntry, String> {
    match source {
        ManagedContentSource::Modrinth => {
            let modrinth = ModrinthClient::default();
            let version = modrinth
                .get_version(version_id)
                .map_err(|err| format!("Modrinth version lookup failed for {version_id}: {err}"))?;
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())
                .ok_or_else(|| {
                    format!("No downloadable file found for Modrinth version {version_id}.")
                })?;
            let dependency_version_projects =
                modrinth_dependency_project_ids(&modrinth, version.dependencies.as_slice());
            let dependencies = modrinth_dependency_refs(
                version.dependencies.as_slice(),
                &dependency_version_projects,
            );
            Ok(BrowserVersionEntry {
                source,
                version_id: version.id,
                version_name: version.version_number,
                file_name: file.filename.clone(),
                file_url: file.url.clone(),
                published_at: version.date_published,
                loaders: version.loaders,
                game_versions: version.game_versions,
                dependencies,
            })
        }
        ManagedContentSource::CurseForge => {
            let curseforge = CurseForgeClient::from_env()
                .ok_or_else(|| "CurseForge API key missing.".to_owned())?;
            let version_id_u64 = version_id
                .trim()
                .parse::<u64>()
                .map_err(|err| format!("Invalid CurseForge version id {version_id}: {err}"))?;
            let file = curseforge
                .get_files(&[version_id_u64])
                .map_err(|err| {
                    format!("CurseForge version lookup failed for {version_id_u64}: {err}")
                })?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    format!(
                        "Could not find CurseForge version {} for {}.",
                        version_id, entry.name
                    )
                })?;
            let download_url = file
                .download_url
                .clone()
                .ok_or_else(|| format!("CurseForge version {} has no download URL.", version_id))?;
            let mut dependencies = Vec::new();
            for dep in file.dependencies {
                if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
                    dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
                }
            }
            let (loaders, game_versions) = split_curseforge_game_versions(file.game_versions);
            Ok(BrowserVersionEntry {
                source,
                version_id: file.id.to_string(),
                version_name: file.display_name.clone(),
                file_name: file.file_name,
                file_url: download_url,
                published_at: file.file_date,
                loaders,
                game_versions,
                dependencies,
            })
        }
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

fn run_search_request(
    request: BrowserSearchRequest,
    tx: mpsc::Sender<SearchUpdate>,
) -> Result<(), String> {
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
        resolve_curseforge_class_ids_cached(client, &mut warnings)
    } else {
        warnings.push(
            "CurseForge API key missing (set VERTEX_CURSEFORGE_API_KEY or CURSEFORGE_API_KEY). Showing Modrinth results only."
                .to_owned(),
        );
        HashMap::new()
    };

    let page = request.page.max(1);
    let provider_offset = page
        .saturating_sub(1)
        .saturating_mul(CONTENT_SEARCH_PER_PROVIDER_LIMIT);
    let total_tasks = content_scope_task_count(request.content_scope);
    if total_tasks == 0 {
        if let Err(err) = tx.send(SearchUpdate::Snapshot {
            request,
            snapshot: BrowserSearchSnapshot {
                entries: Vec::new(),
                warnings,
            },
            completed_tasks: 0,
            total_tasks: 0,
            finished: true,
        }) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver empty content search snapshot."
            );
        }
        return Ok(());
    }

    let mut provider_entries = Vec::new();
    let outcomes = thread::scope(|scope| -> Result<Vec<SearchTaskOutcome>, String> {
        let mut tasks = Vec::new();
        for content_type in BrowserContentType::ORDERED {
            if !request.content_scope.includes(content_type) {
                continue;
            }
            let query_for_type = query
                .clone()
                .unwrap_or_else(|| content_type.default_discovery_query().to_owned());
            let game_version = game_version.clone();
            let modrinth = modrinth.clone();
            let curseforge = curseforge.clone();
            let curseforge_class_id = curseforge_class_ids.get(&content_type).copied();
            let loader = request.loader;
            tasks.push((
                content_type,
                scope.spawn(move || {
                    search_content_type_providers(
                        content_type,
                        query_for_type,
                        game_version,
                        provider_offset,
                        loader,
                        modrinth,
                        curseforge,
                        curseforge_class_id,
                    )
                }),
            ));
        }

        let mut outcomes = Vec::with_capacity(tasks.len());
        for (content_type, task) in tasks {
            outcomes.push(task.join().map_err(|_| {
                format!(
                    "{} search worker panicked unexpectedly.",
                    content_type.label()
                )
            })?);
        }
        Ok(outcomes)
    })?;

    let mut completed_tasks = 0usize;
    for outcome in outcomes {
        completed_tasks = completed_tasks.saturating_add(1);
        provider_entries.extend(outcome.entries);
        warnings.extend(outcome.warnings);
        if let Err(err) = tx.send(SearchUpdate::Snapshot {
            request: request.clone(),
            snapshot: build_search_snapshot(
                provider_entries.as_slice(),
                warnings.as_slice(),
                request.mod_sort_mode,
            ),
            completed_tasks,
            total_tasks,
            finished: completed_tasks >= total_tasks,
        }) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                completed_tasks,
                total_tasks,
                error = %err,
                "Failed to deliver incremental content search snapshot."
            );
        }
    }

    Ok(())
}

fn search_content_type_providers(
    content_type: BrowserContentType,
    query_for_type: String,
    game_version: Option<String>,
    provider_offset: u32,
    loader: BrowserLoader,
    modrinth: ModrinthClient,
    curseforge: Option<CurseForgeClient>,
    curseforge_class_id: Option<u32>,
) -> SearchTaskOutcome {
    let mut outcome = SearchTaskOutcome::default();
    let mod_loader = if content_type == BrowserContentType::Mod {
        loader.modrinth_slug()
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
        None,
    ) {
        Ok(entries) => {
            outcome
                .entries
                .extend(
                    entries
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
                        }),
                );
        }
        Err(err) => outcome.warnings.push(format!(
            "Modrinth search failed for {}: {err}",
            content_type.label()
        )),
    }

    let Some(curseforge) = curseforge.as_ref() else {
        return outcome;
    };
    let Some(class_id) = curseforge_class_id else {
        return outcome;
    };
    let mod_loader_type = if content_type == BrowserContentType::Mod {
        loader.curseforge_mod_loader_type()
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
        None,
    ) {
        Ok(entries) => {
            outcome
                .entries
                .extend(
                    entries
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
                        }),
                );
        }
        Err(err) => outcome.warnings.push(format!(
            "CurseForge search failed for {}: {err}",
            content_type.label()
        )),
    }

    outcome
}

fn resolve_curseforge_class_ids_cached(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> HashMap<BrowserContentType, u32> {
    static CACHE: OnceLock<Mutex<Option<HashMap<BrowserContentType, u32>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock()
        && let Some(class_ids) = cache.as_ref()
    {
        return class_ids.clone();
    }

    let class_ids = resolve_curseforge_class_ids(client, warnings);
    if let Ok(mut cache) = cache.lock() {
        *cache = Some(class_ids.clone());
    }
    class_ids
}

fn resolve_curseforge_class_ids(
    client: &CurseForgeClient,
    warnings: &mut Vec<String>,
) -> HashMap<BrowserContentType, u32> {
    let mut by_type = HashMap::new();
    match client.list_content_classes(MINECRAFT_GAME_ID) {
        Ok(classes) => {
            for class_entry in classes {
                let normalized = normalize_search_key(class_entry.name.as_str());
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
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            request = ?state.active_search_request,
                            "Content search worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    request = ?state.active_search_request,
                    "Content search receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.search_tx = None;
        state.search_rx = None;
        state.search_in_flight = false;
        state.search_completed_tasks = 0;
        state.search_total_tasks = 0;
        state
            .results
            .warnings
            .push("Content search worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        match update {
            SearchUpdate::Snapshot {
                request,
                snapshot,
                completed_tasks,
                total_tasks,
                finished,
            } => {
                if state.active_search_request.as_ref() != Some(&request) {
                    continue;
                }
                state.results = snapshot.clone();
                state.search_completed_tasks = completed_tasks;
                state.search_total_tasks = total_tasks;
                if finished {
                    state.search_in_flight = false;
                    if state.search_notification_active {
                        state.search_notification_active = false;
                    }
                    state.search_cache.insert(request, snapshot);
                    trim_content_browser_search_cache(state);
                    notification::progress!(
                        notification::Severity::Info,
                        "content-browser/search",
                        1.0f32,
                        "Content search complete."
                    );
                } else {
                    let progress = if total_tasks == 0 {
                        0.5
                    } else {
                        0.1f32 + (0.8f32 * (completed_tasks as f32 / total_tasks as f32))
                    };
                    notification::progress!(
                        notification::Severity::Info,
                        "content-browser/search",
                        progress.min(0.95),
                        "Searching content... ({}/{})",
                        completed_tasks,
                        total_tasks
                    );
                }
            }
            SearchUpdate::Failed { request, error } => {
                if state.active_search_request.as_ref() != Some(&request) {
                    continue;
                }
                state.search_in_flight = false;
                state.search_completed_tasks = 0;
                state.search_total_tasks = 0;
                if state.search_notification_active {
                    state.search_notification_active = false;
                }
                tracing::warn!(
                    target: "vertexlauncher/content_browser",
                    request = ?request,
                    error = %error,
                    "Content search failed."
                );
                state.results.warnings.push(error.clone());
                notification::warn!("content-browser/search", "Content search failed: {}", error);
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
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            project = ?state.detail_versions_project_key,
                            "Detail-versions worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    project = ?state.detail_versions_project_key,
                    "Detail-versions receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.detail_versions_tx = None;
        state.detail_versions_rx = None;
        state.detail_versions_in_flight = false;
        state.detail_versions_error =
            Some("Version details worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.detail_versions_in_flight = false;
        state
            .detail_versions_cache
            .insert(update.project_key.clone(), update.versions.clone());
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
                    tracing::warn!(
                        target: "vertexlauncher/content_browser",
                        project = %update.project_key,
                        error = %err,
                        "Detail version lookup failed."
                    );
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
        let name_key = normalize_search_key(name.as_str());
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

fn build_search_snapshot(
    provider_entries: &[ProviderSearchEntry],
    warnings: &[String],
    mod_sort_mode: ModSortMode,
) -> BrowserSearchSnapshot {
    let mut entries = dedupe_browser_entries(provider_entries.to_vec());
    entries.sort_by(|left, right| {
        left.content_type.cmp(&right.content_type).then_with(|| {
            if left.content_type == BrowserContentType::Mod {
                compare_mod_entries(left, right, mod_sort_mode)
            } else {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            }
        })
    });
    BrowserSearchSnapshot {
        entries,
        warnings: warnings.to_vec(),
    }
}

fn count_entries_by_content_type(entries: &[BrowserProjectEntry]) -> [usize; 4] {
    let mut counts = [0usize; 4];
    for entry in entries {
        counts[entry.content_type.index()] = counts[entry.content_type.index()].saturating_add(1);
    }
    counts
}

fn content_scope_task_count(scope: ContentScope) -> usize {
    BrowserContentType::ORDERED
        .iter()
        .filter(|content_type| scope.includes(**content_type))
        .count()
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

fn maybe_start_queued_download(
    state: &mut ContentBrowserState,
    instance_name: &str,
    instance_root: &Path,
) {
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
    state.active_download = Some(active_download_from_request(&next.request));
    state.download_notification_active = true;
    install_activity::set_status(
        instance_name,
        installation::InstallStage::DownloadingCore,
        "Applying content changes...",
    );
    notification::progress!(
        notification::Severity::Info,
        "content-browser/download",
        0.1f32,
        "Applying queued content operation..."
    );
    let root = instance_root.to_path_buf();
    let request = next.request.clone();

    let _ = tokio_runtime::spawn_detached(async move {
        let join = tokio_runtime::spawn_blocking(move || {
            apply_content_install_request(root.as_path(), request)
        });
        let result = match join.await {
            Ok(r) => r,
            Err(err) => Err(format!("content install worker panicked: {err}")),
        };
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/content_browser",
                error = %err,
                "Failed to deliver queued content operation result."
            );
        }
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
                        tracing::error!(
                            target: "vertexlauncher/content_browser",
                            active_download = ?state.active_download,
                            "Content download worker disconnected unexpectedly."
                        );
                        should_reset_channel = true;
                        break;
                    }
                }
            },
            Err(_) => {
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    active_download = ?state.active_download,
                    "Content download receiver mutex was poisoned."
                );
                should_reset_channel = true;
            }
        }
    }

    if should_reset_channel {
        state.download_tx = None;
        state.download_rx = None;
        state.download_in_flight = false;
        state.active_download = None;
        if let Some(instance_name) = state.active_instance_name.as_deref() {
            install_activity::clear_instance(instance_name);
        }
        state.status_message = Some("Content download worker stopped unexpectedly.".to_owned());
    }

    for update in updates {
        state.download_in_flight = false;
        state.active_download = None;
        if state.download_notification_active {
            state.download_notification_active = false;
        }
        if let Some(instance_name) = state.active_instance_name.as_deref() {
            install_activity::clear_instance(instance_name);
        }
        match update {
            Ok(result) => {
                state.manifest_dirty = true;
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
                tracing::error!(
                    target: "vertexlauncher/content_browser",
                    error = %err,
                    "Queued content operation failed."
                );
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

fn active_download_from_request(request: &ContentInstallRequest) -> ActiveContentDownload {
    match request {
        ContentInstallRequest::Latest { entry, .. } => ActiveContentDownload {
            dedupe_key: entry.dedupe_key.clone(),
            version_id: None,
        },
        ContentInstallRequest::Exact { entry, version, .. } => ActiveContentDownload {
            dedupe_key: entry.dedupe_key.clone(),
            version_id: Some(version.version_id.clone()),
        },
    }
}

fn apply_content_install_request(
    instance_root: &Path,
    request: ContentInstallRequest,
) -> Result<ContentDownloadOutcome, String> {
    apply_content_install_request_with_prefetched_downloads(
        instance_root,
        request,
        &HashMap::new(),
        &[],
    )
}

fn apply_content_install_request_with_prefetched_downloads(
    instance_root: &Path,
    request: ContentInstallRequest,
    prefetched_paths: &HashMap<PathBuf, PathBuf>,
    additional_cleanup_paths: &[PathBuf],
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
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %root_entry.name,
        version_id = %root_download.version_id,
        prefetched = !prefetched_paths.is_empty(),
        "applying content install request"
    );

    let mut manifest = load_content_manifest(instance_root);
    let mut deferred_cleanup = (!prefetched_paths.is_empty()).then(DeferredContentCleanup::default);
    let existing_project = installed_project_for_entry(&manifest, &root_entry)
        .map(|(key, project)| (key.to_owned(), project.clone()));
    let root_project_key = existing_project
        .as_ref()
        .map(|(key, _)| key.clone())
        .unwrap_or_else(|| root_entry.dedupe_key.clone());

    if let Some((existing_project_key, existing)) = existing_project {
        if existing.selected_source == Some(root_download.source)
            && existing.selected_version_id.as_deref() == Some(root_download.version_id.as_str())
        {
            if let Some(record) = manifest.projects.get_mut(existing_project_key.as_str()) {
                record.pack_managed = false;
                record.explicitly_installed = true;
            }
            for path in additional_cleanup_paths {
                if !path.exists() {
                    continue;
                }
                remove_content_path(path.as_path())?;
                removed_files.push(path.display().to_string());
            }
            save_content_manifest(instance_root, &manifest)?;
            return Ok(ContentDownloadOutcome {
                project_name: root_entry.name,
                added_files,
                removed_files,
            });
        }
        let dependents = manifest_dependents(&manifest, existing_project_key.as_str());
        if !dependents.is_empty() {
            tracing::warn!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project = %root_entry.name,
                existing_project_key = %existing_project_key,
                dependents = %dependents.join(", "),
                "rejecting content switch because dependents are still installed"
            );
            return Err(format!(
                "Cannot switch {} while it is required by {}.",
                root_entry.name,
                dependents.join(", ")
            ));
        }
        remove_installed_project(
            instance_root,
            &mut manifest,
            existing_project_key.as_str(),
            true,
            &mut removed_files,
            deferred_cleanup.as_mut(),
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
        Some(root_project_key.as_str()),
        &modrinth,
        curseforge.as_ref(),
        None,
        true,
        prefetched_paths,
        &mut visited,
        &mut added_files,
        &mut removed_files,
        deferred_cleanup.as_mut(),
    )?;
    if let Some(cleanup) = deferred_cleanup.as_mut() {
        cleanup.stale_paths.extend(
            additional_cleanup_paths
                .iter()
                .filter(|path| path.exists())
                .cloned(),
        );
    } else {
        for path in additional_cleanup_paths {
            if !path.exists() {
                continue;
            }
            remove_content_path(path.as_path())?;
            removed_files.push(path.display().to_string());
        }
    }
    if let Some(cleanup) = deferred_cleanup.as_ref() {
        apply_deferred_content_cleanup(instance_root, &manifest, cleanup, &mut removed_files)?;
    }
    save_content_manifest(instance_root, &manifest)?;
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project = %root_entry.name,
        added_files = added_files.len(),
        removed_files = removed_files.len(),
        "finished content install request"
    );

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
    project_key_override: Option<&str>,
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    parent_key: Option<&str>,
    explicit: bool,
    prefetched_paths: &HashMap<PathBuf, PathBuf>,
    visited: &mut HashSet<String>,
    added_files: &mut Vec<String>,
    removed_files: &mut Vec<String>,
    mut deferred_cleanup: Option<&mut DeferredContentCleanup>,
) -> Result<(), String> {
    let project_key = project_key_override
        .map(str::to_owned)
        .unwrap_or_else(|| entry.dedupe_key.clone());
    if let Some(parent_key) = parent_key {
        append_project_dependency(manifest, parent_key, project_key.as_str());
    }

    if !visited.insert(project_key.clone()) {
        if explicit && let Some(existing) = manifest.projects.get_mut(&project_key) {
            existing.explicitly_installed = true;
        }
        return Ok(());
    }

    let existing = manifest.projects.get(&project_key).cloned();
    let target_dir = instance_root.join(entry.content_type.folder_name());
    std::fs::create_dir_all(target_dir.as_path())
        .map_err(|err| format!("failed to create content folder {:?}: {err}", target_dir))?;
    let target_name = normalized_filename(resolved.file_name.as_str(), resolved.file_url.as_str());
    let target_path = target_dir.join(target_name.as_str());
    if let Some(existing) = existing.as_ref()
        && existing.selected_source == Some(resolved.source)
        && existing.selected_version_id.as_deref() == Some(resolved.version_id.as_str())
    {
        if explicit && let Some(record) = manifest.projects.get_mut(&project_key) {
            record.explicitly_installed = true;
        }
        return Ok(());
    }

    let previous_file_path = existing
        .as_ref()
        .map(|project| instance_root.join(project.file_path.as_path()));
    let staged_previous_path = match previous_file_path.as_ref() {
        Some(previous_file_path) => {
            stage_existing_file_for_update(previous_file_path.as_path(), target_path.as_path())?
        }
        None => None,
    };
    let previous_dependency_keys = existing
        .as_ref()
        .map(|project| project.direct_dependencies.clone())
        .unwrap_or_default();
    let explicitly_installed = explicit
        || existing
            .as_ref()
            .is_some_and(|project| project.explicitly_installed);
    let prefetched_path = prefetched_paths.get(&target_path).cloned();
    tracing::debug!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project_key = %project_key,
        project = %entry.name,
        target_path = %target_path.display(),
        has_existing = existing.is_some(),
        prefetched = prefetched_path.is_some(),
        explicit,
        "installing content project node"
    );

    let install_result = (|| -> Result<(), String> {
        if existing.is_some() || !target_path.exists() || prefetched_path.is_some() {
            if let Some(prefetched_path) = prefetched_path.as_ref() {
                if !prefetched_path.exists() {
                    return Err(format!(
                        "prefetched content file missing at {}",
                        prefetched_path.display()
                    ));
                }
                if target_path.exists() {
                    tracing::debug!(
                        target: CONTENT_UPDATE_LOG_TARGET,
                        target_path = %target_path.display(),
                        "removing existing target before placing prefetched content"
                    );
                    remove_content_path(target_path.as_path())?;
                }
                tracing::debug!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    source_path = %prefetched_path.display(),
                    target_path = %target_path.display(),
                    "placing prefetched content file"
                );
                std::fs::rename(prefetched_path, target_path.as_path()).map_err(|err| {
                    format!(
                        "failed to place prefetched content {} at {}: {err}",
                        prefetched_path.display(),
                        target_path.display()
                    )
                })?;
            } else {
                tracing::debug!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    target_path = %target_path.display(),
                    url = %resolved.file_url,
                    "downloading content file directly"
                );
                download_file(resolved.file_url.as_str(), target_path.as_path())?;
            }
            if !added_files
                .iter()
                .any(|path| path == &target_path.display().to_string())
            {
                added_files.push(target_path.display().to_string());
            }
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
                file_path: PathBuf::from(file_path),
                modrinth_project_id: entry.modrinth_project_id.clone(),
                curseforge_project_id: entry.curseforge_project_id,
                selected_source: Some(resolved.source),
                selected_version_id: Some(resolved.version_id.clone()),
                selected_version_name: Some(resolved.version_name.clone()),
                pack_managed: false,
                explicitly_installed,
                direct_dependencies: Vec::new(),
            },
        );

        let mut dependency_keys = Vec::new();
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
            dependency_keys.push(dep_entry.dedupe_key.clone());
            install_project_recursive(
                instance_root,
                manifest,
                &dep_entry,
                dep_resolved,
                game_version,
                loader,
                None,
                modrinth,
                curseforge,
                Some(project_key.as_str()),
                false,
                prefetched_paths,
                visited,
                added_files,
                removed_files,
                deferred_cleanup.as_deref_mut(),
            )?;
        }

        if let Some(record) = manifest.projects.get_mut(&project_key) {
            record.direct_dependencies = dependency_keys.clone();
            if explicit {
                record.explicitly_installed = true;
            }
        }

        if let Some(previous_file_path) = previous_file_path.as_ref() {
            finalize_updated_file_replacement(
                previous_file_path.as_path(),
                target_path.as_path(),
                staged_previous_path.as_deref(),
                removed_files,
                deferred_cleanup.as_deref_mut(),
            )?;
        }

        for dependency_key in previous_dependency_keys {
            if dependency_keys
                .iter()
                .any(|current| current == &dependency_key)
            {
                continue;
            }
            remove_installed_project(
                instance_root,
                manifest,
                dependency_key.as_str(),
                false,
                removed_files,
                deferred_cleanup.as_deref_mut(),
            )?;
        }

        Ok(())
    })();

    match install_result {
        Ok(()) => {
            tracing::debug!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project_key = %project_key,
                project = %entry.name,
                "installed content project node"
            );
            Ok(())
        }
        Err(err) => {
            tracing::error!(
                target: CONTENT_UPDATE_LOG_TARGET,
                instance_root = %instance_root.display(),
                project_key = %project_key,
                project = %entry.name,
                "content project install failed: {err}"
            );
            if let (Some(staged_previous_path), Some(previous_file_path)) =
                (staged_previous_path.as_ref(), previous_file_path.as_ref())
            {
                restore_staged_update_file(
                    staged_previous_path.as_path(),
                    previous_file_path.as_path(),
                )
                .map_err(|restore_err| {
                    format!("{err} (also failed to restore original file: {restore_err})")
                })?;
            }
            Err(err)
        }
    }
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
    mut deferred_cleanup: Option<&mut DeferredContentCleanup>,
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

    let file_path = instance_root.join(existing.file_path.as_path());
    tracing::debug!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        project_key = %project_key,
        file_path = %file_path.display(),
        deferred = deferred_cleanup.is_some(),
        force,
        "removing installed project entry"
    );
    if file_path.exists() {
        if let Some(cleanup) = deferred_cleanup.as_deref_mut() {
            cleanup.stale_paths.push(file_path.clone());
        } else {
            std::fs::remove_file(file_path.as_path())
                .map_err(|err| format!("failed to remove {}: {err}", file_path.display()))?;
            removed_files.push(file_path.display().to_string());
        }
    }

    for dependency_key in existing.direct_dependencies {
        remove_installed_project(
            instance_root,
            manifest,
            dependency_key.as_str(),
            false,
            removed_files,
            deferred_cleanup.as_deref_mut(),
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

fn content_target_path(
    instance_root: &Path,
    entry: &BrowserProjectEntry,
    version: &BrowserVersionEntry,
) -> PathBuf {
    let target_dir = instance_root.join(entry.content_type.folder_name());
    let target_name = normalized_filename(version.file_name.as_str(), version.file_url.as_str());
    target_dir.join(target_name)
}

fn content_target_path_for_resolved_download(
    instance_root: &Path,
    entry: &BrowserProjectEntry,
    resolved: &ResolvedDownload,
) -> PathBuf {
    let target_dir = instance_root.join(entry.content_type.folder_name());
    let target_name = normalized_filename(resolved.file_name.as_str(), resolved.file_url.as_str());
    target_dir.join(target_name)
}

fn stage_existing_file_for_update(
    existing_file_path: &Path,
    target_path: &Path,
) -> Result<Option<PathBuf>, String> {
    if !paths_match_for_update(existing_file_path, target_path) || !existing_file_path.exists() {
        return Ok(None);
    }

    let staged_path = staged_update_backup_path(existing_file_path);
    tracing::debug!(
        target: CONTENT_UPDATE_LOG_TARGET,
        existing_path = %existing_file_path.display(),
        staged_path = %staged_path.display(),
        "staging existing content file for replacement"
    );
    std::fs::rename(existing_file_path, staged_path.as_path()).map_err(|err| {
        format!(
            "failed to stage existing content {} for replacement: {err}",
            existing_file_path.display()
        )
    })?;
    Ok(Some(staged_path))
}

fn finalize_updated_file_replacement(
    previous_file_path: &Path,
    target_path: &Path,
    staged_previous_path: Option<&Path>,
    removed_files: &mut Vec<String>,
    deferred_cleanup: Option<&mut DeferredContentCleanup>,
) -> Result<(), String> {
    if let Some(staged_previous_path) = staged_previous_path {
        if let Some(cleanup) = deferred_cleanup {
            cleanup
                .staged_paths
                .push(staged_previous_path.to_path_buf());
        } else {
            remove_content_path(staged_previous_path)?;
            removed_files.push(staged_previous_path.display().to_string());
        }
        return Ok(());
    }

    if paths_match_for_update(previous_file_path, target_path) || !previous_file_path.exists() {
        return Ok(());
    }

    if let Some(cleanup) = deferred_cleanup {
        cleanup.stale_paths.push(previous_file_path.to_path_buf());
    } else {
        remove_content_path(previous_file_path)?;
        removed_files.push(previous_file_path.display().to_string());
    }
    Ok(())
}

fn apply_deferred_content_cleanup(
    instance_root: &Path,
    manifest: &ContentInstallManifest,
    cleanup: &DeferredContentCleanup,
    removed_files: &mut Vec<String>,
) -> Result<(), String> {
    let active_paths = manifest
        .projects
        .values()
        .map(|project| instance_root.join(project.file_path.as_path()))
        .collect::<HashSet<_>>();
    tracing::info!(
        target: CONTENT_UPDATE_LOG_TARGET,
        instance_root = %instance_root.display(),
        stale_paths = cleanup.stale_paths.len(),
        staged_paths = cleanup.staged_paths.len(),
        active_paths = active_paths.len(),
        "applying deferred content cleanup"
    );

    for staged_path in &cleanup.staged_paths {
        if !staged_path.exists() {
            continue;
        }
        tracing::debug!(
            target: CONTENT_UPDATE_LOG_TARGET,
            staged_path = %staged_path.display(),
            "removing staged content backup"
        );
        remove_content_path(staged_path.as_path())?;
        removed_files.push(staged_path.display().to_string());
    }

    for stale_path in &cleanup.stale_paths {
        if active_paths.contains(stale_path) || !stale_path.exists() {
            if active_paths.contains(stale_path) {
                tracing::debug!(
                    target: CONTENT_UPDATE_LOG_TARGET,
                    stale_path = %stale_path.display(),
                    "skipping stale-path removal because it is now active"
                );
            }
            continue;
        }
        tracing::debug!(
            target: CONTENT_UPDATE_LOG_TARGET,
            stale_path = %stale_path.display(),
            "removing deferred stale content path"
        );
        remove_content_path(stale_path.as_path())?;
        removed_files.push(stale_path.display().to_string());
    }

    Ok(())
}

fn restore_staged_update_file(
    staged_previous_path: &Path,
    previous_file_path: &Path,
) -> Result<(), String> {
    if !staged_previous_path.exists() {
        return Ok(());
    }

    if previous_file_path.exists() {
        remove_content_path(previous_file_path)?;
    }

    std::fs::rename(staged_previous_path, previous_file_path).map_err(|err| {
        format!(
            "failed to restore {} from {}: {err}",
            previous_file_path.display(),
            staged_previous_path.display()
        )
    })
}

fn staged_update_backup_path(existing_file_path: &Path) -> PathBuf {
    let parent = existing_file_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = existing_file_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("content.bin");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    let mut attempt = 0u32;
    loop {
        let candidate = parent.join(format!(
            ".vertex-update-backup-{file_name}-{timestamp}-{attempt}"
        ));
        if !candidate.exists() {
            return candidate;
        }
        attempt += 1;
    }
}

fn remove_content_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("failed to remove {}: {err}", path.display()))
    } else {
        std::fs::remove_file(path)
            .map_err(|err| format!("failed to remove {}: {err}", path.display()))
    }
}

fn paths_match_for_update(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }

    #[cfg(not(target_os = "windows"))]
    {
        left == right
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
    let dependency_version_projects = modrinth_dependency_project_ids(
        modrinth,
        versions
            .iter()
            .flat_map(|version| version.dependencies.iter().cloned())
            .collect::<Vec<_>>()
            .as_slice(),
    );

    Ok(versions
        .into_iter()
        .filter_map(|version| {
            let file = version
                .files
                .iter()
                .find(|file| file.primary)
                .or_else(|| version.files.first())?;
            let dependencies = modrinth_dependency_refs(
                version.dependencies.as_slice(),
                &dependency_version_projects,
            );
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
    let project = curseforge
        .get_mod(project_id)
        .map_err(|err| format!("CurseForge project lookup failed for {project_id}: {err}"))?;
    let Some(file_id) = project
        .latest_files_indexes
        .iter()
        .filter(|index| {
            normalize_optional(game_version)
                .as_deref()
                .is_none_or(|value| index.game_version.trim() == value)
        })
        .filter(|index| mod_loader_type.is_none_or(|value| index.mod_loader == Some(value)))
        .map(|index| index.file_id)
        .max()
    else {
        return Ok(None);
    };
    let Some(file) = curseforge
        .get_files(&[file_id])
        .map_err(|err| format!("CurseForge file lookup failed for {file_id}: {err}"))?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let Some(url) = file.download_url.clone() else {
        return Ok(None);
    };
    let mut dependencies = Vec::new();
    for dep in file.dependencies {
        if dep.relation_type == CONTENT_DOWNLOAD_REQUIRED_DEPENDENCY_RELATION_TYPE {
            dependencies.push(DependencyRef::CurseForgeProject(dep.mod_id));
        }
    }
    Ok(Some(ResolvedDownload {
        source: ManagedContentSource::CurseForge,
        version_id: file.id.to_string(),
        version_name: file.display_name.clone(),
        file_url: url,
        file_name: file.file_name,
        published_at: file.file_date,
        dependencies,
    }))
}

fn dependency_to_browser_entries(
    dependencies: &[DependencyRef],
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
) -> Result<Vec<BrowserProjectEntry>, String> {
    let modrinth_ids = dependencies
        .iter()
        .filter_map(|dependency| match dependency {
            DependencyRef::ModrinthProject(project_id) => Some(project_id.clone()),
            DependencyRef::CurseForgeProject(_) => None,
        })
        .collect::<Vec<_>>();
    let curseforge_ids = dependencies
        .iter()
        .filter_map(|dependency| match dependency {
            DependencyRef::CurseForgeProject(project_id) => Some(*project_id),
            DependencyRef::ModrinthProject(_) => None,
        })
        .collect::<Vec<_>>();

    let modrinth_projects = modrinth
        .get_projects(modrinth_ids.as_slice())
        .unwrap_or_default()
        .into_iter()
        .map(|project| (project.project_id.clone(), project))
        .collect::<HashMap<_, _>>();
    let curseforge_projects = if let Some(curseforge) = curseforge {
        curseforge
            .get_mods(curseforge_ids.as_slice())
            .unwrap_or_default()
            .into_iter()
            .map(|project| (project.id, project))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };

    let mut entries = Vec::new();
    for dependency in dependencies {
        match dependency {
            DependencyRef::ModrinthProject(project_id) => {
                let Some(project) = modrinth_projects.get(project_id.as_str()) else {
                    continue;
                };
                if let Some(entry) = browser_entry_from_modrinth_dependency_project(project) {
                    entries.push(entry);
                }
            }
            DependencyRef::CurseForgeProject(project_id) => {
                let Some(project) = curseforge_projects.get(project_id) else {
                    continue;
                };
                if let Some(entry) = browser_entry_from_curseforge_dependency_project(project) {
                    entries.push(entry);
                }
            }
        }
    }
    Ok(entries)
}

fn browser_entry_from_modrinth_dependency_project(
    project: &modrinth::Project,
) -> Option<BrowserProjectEntry> {
    let content_type = parse_content_type(project.project_type.as_str())?;
    let name_key = normalize_search_key(project.title.as_str());
    if name_key.is_empty() {
        return None;
    }
    Some(BrowserProjectEntry {
        dedupe_key: format!("{}::{name_key}", content_type.label().to_ascii_lowercase()),
        name: project.title.clone(),
        summary: project.description.clone(),
        content_type,
        icon_url: project.icon_url.clone(),
        modrinth_project_id: Some(project.project_id.clone()),
        curseforge_project_id: None,
        sources: vec![ContentSource::Modrinth],
        popularity_score: None,
        updated_at: None,
        relevance_rank: u32::MAX,
    })
}

fn browser_entry_from_curseforge_dependency_project(
    project: &curseforge::Project,
) -> Option<BrowserProjectEntry> {
    let name_key = normalize_search_key(project.name.as_str());
    if name_key.is_empty() {
        return None;
    }
    Some(BrowserProjectEntry {
        dedupe_key: format!("mod::{name_key}"),
        name: project.name.clone(),
        summary: project.summary.clone(),
        content_type: BrowserContentType::Mod,
        icon_url: project.icon_url.clone(),
        modrinth_project_id: None,
        curseforge_project_id: Some(project.id),
        sources: vec![ContentSource::CurseForge],
        popularity_score: None,
        updated_at: None,
        relevance_rank: u32::MAX,
    })
}

fn normalize_optional(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn vertex_prefetch_root(instance_root: &Path) -> PathBuf {
    instance_root.join(VERTEX_PREFETCH_DIR_NAME)
}

fn prefetched_target_path(
    instance_root: &Path,
    content_type: BrowserContentType,
    target_path: &Path,
) -> PathBuf {
    let file_name = target_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("content.bin");
    vertex_prefetch_root(instance_root)
        .join(content_type.folder_name())
        .join(file_name)
}

fn cleanup_prefetched_downloads(instance_root: &Path) -> Result<(), String> {
    let prefetch_root = vertex_prefetch_root(instance_root);
    if prefetch_root.exists() {
        tracing::debug!(
            target: CONTENT_UPDATE_LOG_TARGET,
            prefetch_root = %prefetch_root.display(),
            "removing vertex prefetch directory"
        );
        remove_content_path(prefetch_root.as_path())?;
    }
    Ok(())
}

fn modrinth_dependency_project_ids(
    modrinth: &ModrinthClient,
    dependencies: &[modrinth::ProjectDependency],
) -> HashMap<String, String> {
    let version_ids = dependencies
        .iter()
        .filter(|dependency| dependency.project_id.is_none())
        .filter_map(|dependency| dependency.version_id.as_ref())
        .cloned()
        .collect::<Vec<_>>();
    modrinth
        .get_versions(version_ids.as_slice())
        .unwrap_or_default()
        .into_iter()
        .filter(|version| !version.project_id.trim().is_empty())
        .map(|version| (version.id, version.project_id))
        .collect()
}

fn modrinth_dependency_refs(
    dependencies: &[modrinth::ProjectDependency],
    version_projects: &HashMap<String, String>,
) -> Vec<DependencyRef> {
    let mut resolved = Vec::new();
    for dependency in dependencies {
        if !dependency.dependency_type.eq_ignore_ascii_case("required") {
            continue;
        }
        if let Some(project_id) = dependency.project_id.as_ref() {
            resolved.push(DependencyRef::ModrinthProject(project_id.clone()));
            continue;
        }
        if let Some(version_id) = dependency.version_id.as_ref()
            && let Some(project_id) = version_projects.get(version_id.as_str())
        {
            resolved.push(DependencyRef::ModrinthProject(project_id.clone()));
        }
    }
    resolved
}

fn identify_mod_file_by_hash(path: &Path) -> Result<UnifiedContentEntry, String> {
    let (sha1, sha512) = modrinth::hash_file_sha1_and_sha512_hex(path)
        .map_err(|err| format!("failed to hash file: {err}"))?;
    let modrinth = ModrinthClient::default();

    for (algorithm, hash) in [("sha512", sha512.as_str()), ("sha1", sha1.as_str())] {
        let Some(version) = modrinth
            .get_version_from_hash(hash, algorithm)
            .map_err(|err| format!("Modrinth hash lookup failed: {err}"))?
        else {
            continue;
        };
        let project = modrinth
            .get_project(version.project_id.as_str())
            .map_err(|err| format!("Modrinth project lookup failed: {err}"))?;
        return Ok(UnifiedContentEntry {
            id: format!("modrinth:{}", project.project_id),
            name: project.title,
            summary: project.description.trim().to_owned(),
            content_type: project.project_type,
            source: ContentSource::Modrinth,
            project_url: Some(project.project_url),
            icon_url: project.icon_url,
        });
    }

    Err("no Modrinth project matched this file hash".to_owned())
}

fn normalized_filename(name: &str, url: &str) -> String {
    let trimmed = name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    url.rsplit('/').next().unwrap_or("download.bin").to_owned()
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    throttle_download_url(url);
    let response = ureq::get(url)
        .call()
        .map_err(|err| format!("download request failed for {url}: {err}"))?;
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
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

fn throttle_download_url(url: &str) {
    let Some(spacing) = download_spacing_for_url(url) else {
        return;
    };
    let lock = download_throttle_store(url);
    let Ok(mut next_allowed) = lock.lock() else {
        tracing::error!(
            target: "vertexlauncher/content_browser",
            url,
            throttle_spacing_ms = spacing.as_millis() as u64,
            "Content browser download throttle mutex was poisoned."
        );
        return;
    };
    let now = Instant::now();
    if *next_allowed > now {
        thread::sleep(next_allowed.saturating_duration_since(now));
    }
    *next_allowed = Instant::now() + spacing;
}

fn download_spacing_for_url(url: &str) -> Option<Duration> {
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        Some(MODRINTH_DOWNLOAD_MIN_SPACING)
    } else if host.contains("curseforge.com") || host.contains("forgecdn.net") {
        Some(CURSEFORGE_DOWNLOAD_MIN_SPACING)
    } else {
        None
    }
}

fn download_throttle_store(url: &str) -> &'static Mutex<Instant> {
    static MODRINTH: OnceLock<Mutex<Instant>> = OnceLock::new();
    static CURSEFORGE: OnceLock<Mutex<Instant>> = OnceLock::new();
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        MODRINTH.get_or_init(|| Mutex::new(Instant::now()))
    } else {
        CURSEFORGE.get_or_init(|| Mutex::new(Instant::now()))
    }
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
