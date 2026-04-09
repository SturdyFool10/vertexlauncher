use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ContentBrowserPage {
    #[default]
    Browse,
    Detail,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ContentDetailTab {
    #[default]
    Overview,
    Versions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum BrowserContentType {
    Mod,
    ResourcePack,
    Shader,
    DataPack,
}

impl BrowserContentType {
    pub(super) const ORDERED: [BrowserContentType; 4] = [
        BrowserContentType::Mod,
        BrowserContentType::ResourcePack,
        BrowserContentType::Shader,
        BrowserContentType::DataPack,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "Mod",
            BrowserContentType::ResourcePack => "Resource Pack",
            BrowserContentType::Shader => "Shader",
            BrowserContentType::DataPack => "Data Pack",
        }
    }

    pub(super) fn folder_name(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "mods",
            BrowserContentType::ResourcePack => "resourcepacks",
            BrowserContentType::Shader => "shaderpacks",
            BrowserContentType::DataPack => "datapacks",
        }
    }

    pub(super) fn default_discovery_query(self) -> &'static str {
        match self {
            BrowserContentType::Mod => DEFAULT_DISCOVERY_QUERY_MOD,
            BrowserContentType::ResourcePack => DEFAULT_DISCOVERY_QUERY_RESOURCE_PACK,
            BrowserContentType::Shader => DEFAULT_DISCOVERY_QUERY_SHADER,
            BrowserContentType::DataPack => DEFAULT_DISCOVERY_QUERY_DATA_PACK,
        }
    }

    pub(super) fn modrinth_project_type(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "mod",
            BrowserContentType::ResourcePack => "resourcepack",
            BrowserContentType::Shader => "shader",
            BrowserContentType::DataPack => "datapack",
        }
    }

    pub(super) fn index(self) -> usize {
        match self {
            BrowserContentType::Mod => 0,
            BrowserContentType::ResourcePack => 1,
            BrowserContentType::Shader => 2,
            BrowserContentType::DataPack => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ContentScope {
    All,
    Mods,
    ResourcePacks,
    Shaders,
    DataPacks,
}

impl ContentScope {
    pub(super) const ALL: [ContentScope; 5] = [
        ContentScope::All,
        ContentScope::Mods,
        ContentScope::ResourcePacks,
        ContentScope::Shaders,
        ContentScope::DataPacks,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            ContentScope::All => "All Types",
            ContentScope::Mods => "Mods",
            ContentScope::ResourcePacks => "Resource Packs",
            ContentScope::Shaders => "Shaders",
            ContentScope::DataPacks => "Data Packs",
        }
    }

    pub(super) fn includes(self, content_type: BrowserContentType) -> bool {
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
pub(super) enum BrowserLoader {
    Any,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

impl BrowserLoader {
    pub(super) const ALL: [BrowserLoader; 5] = [
        BrowserLoader::Any,
        BrowserLoader::Fabric,
        BrowserLoader::Forge,
        BrowserLoader::NeoForge,
        BrowserLoader::Quilt,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            BrowserLoader::Any => "Any",
            BrowserLoader::Fabric => "Fabric",
            BrowserLoader::Forge => "Forge",
            BrowserLoader::NeoForge => "NeoForge",
            BrowserLoader::Quilt => "Quilt",
        }
    }

    pub(super) fn modrinth_slug(self) -> Option<&'static str> {
        match self {
            BrowserLoader::Any => None,
            BrowserLoader::Fabric => Some("fabric"),
            BrowserLoader::Forge => Some("forge"),
            BrowserLoader::NeoForge => Some("neoforge"),
            BrowserLoader::Quilt => Some("quilt"),
        }
    }

    pub(super) fn curseforge_mod_loader_type(self) -> Option<u32> {
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
pub(super) enum ModSortMode {
    Relevance,
    LastUpdated,
    Popularity,
}

impl ModSortMode {
    pub(super) const ALL: [ModSortMode; 3] = [
        ModSortMode::Popularity,
        ModSortMode::Relevance,
        ModSortMode::LastUpdated,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            ModSortMode::Relevance => "Relevance",
            ModSortMode::LastUpdated => "Last Update",
            ModSortMode::Popularity => "Popularity",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct BrowserProjectEntry {
    pub(super) dedupe_key: String,
    pub(super) name: String,
    pub(super) summary: String,
    pub(super) content_type: BrowserContentType,
    pub(super) icon_url: Option<String>,
    pub(super) modrinth_project_id: Option<String>,
    pub(super) curseforge_project_id: Option<u64>,
    pub(super) sources: Vec<ContentSource>,
    pub(super) popularity_score: Option<u64>,
    pub(super) updated_at: Option<String>,
    pub(super) relevance_rank: u32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct BrowserSearchSnapshot {
    pub(super) entries: Vec<BrowserProjectEntry>,
    pub(super) warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) enum SearchUpdate {
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
pub(super) struct BrowserSearchRequest {
    pub(super) query: Option<String>,
    pub(super) tags: Vec<String>,
    pub(super) game_version: Option<String>,
    pub(super) loader: BrowserLoader,
    pub(super) content_scope: ContentScope,
    pub(super) mod_sort_mode: ModSortMode,
    pub(super) page: u32,
}

#[derive(Clone, Debug)]
pub(super) enum ContentInstallRequest {
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
pub(super) struct QueuedContentDownload {
    pub(super) request: ContentInstallRequest,
}

#[derive(Clone, Debug)]
pub(super) struct ContentDownloadOutcome {
    pub(super) project_name: String,
    pub(super) added_files: Vec<String>,
    pub(super) removed_files: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BulkContentUpdate {
    pub entry: UnifiedContentEntry,
    pub installed_file_path: PathBuf,
    pub version_id: String,
}

#[derive(Debug, Default)]
pub(super) struct DeferredContentCleanup {
    pub(super) stale_paths: Vec<PathBuf>,
    pub(super) staged_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ActiveContentDownload {
    pub(super) dedupe_key: String,
    pub(super) version_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct BrowserVersionEntry {
    pub(super) source: ManagedContentSource,
    pub(super) version_id: String,
    pub(super) version_name: String,
    pub(super) file_name: String,
    pub(super) file_url: String,
    pub(super) published_at: String,
    pub(super) loaders: Vec<String>,
    pub(super) game_versions: Vec<String>,
    pub(super) dependencies: Vec<DependencyRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VersionRowAction {
    Download,
    Installed,
    Switch,
}

#[derive(Clone, Debug)]
pub(super) struct DetailVersionsResult {
    pub(super) project_key: String,
    pub(super) versions: Result<Vec<BrowserVersionEntry>, String>,
}

#[derive(Clone, Debug)]
pub struct ContentBrowserState {
    pub(super) query_input: String,
    pub(super) search_tags: Vec<String>,
    pub(super) minecraft_version_filter: String,
    pub(super) content_scope: ContentScope,
    pub(super) mod_sort_mode: ModSortMode,
    pub(super) loader: BrowserLoader,
    pub(super) active_instance_id: Option<String>,
    pub(super) active_instance_name: Option<String>,
    pub(super) auto_populated_instance_id: Option<String>,
    pub(super) current_page: u32,
    pub(super) current_view: ContentBrowserPage,
    pub(super) detail_entry: Option<BrowserProjectEntry>,
    pub(super) detail_tab: ContentDetailTab,
    pub(super) detail_versions: Vec<BrowserVersionEntry>,
    pub(super) detail_versions_cache: HashMap<String, Result<Vec<BrowserVersionEntry>, String>>,
    pub(super) detail_versions_project_key: Option<String>,
    pub(super) detail_versions_error: Option<String>,
    pub(super) detail_versions_in_flight: bool,
    pub(super) detail_loader_filter: BrowserLoader,
    pub(super) detail_minecraft_version_filter: String,
    pub(super) detail_versions_tx: Option<mpsc::Sender<DetailVersionsResult>>,
    pub(super) detail_versions_rx: Option<Arc<Mutex<mpsc::Receiver<DetailVersionsResult>>>>,
    pub(super) available_game_versions: Vec<MinecraftVersionEntry>,
    pub(super) version_catalog_error: Option<String>,
    pub(super) version_catalog_in_flight: bool,
    pub(super) version_catalog_tx: Option<mpsc::Sender<Result<Vec<MinecraftVersionEntry>, String>>>,
    pub(super) version_catalog_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<Vec<MinecraftVersionEntry>, String>>>>>,
    pub(super) results: BrowserSearchSnapshot,
    pub(super) active_search_request: Option<BrowserSearchRequest>,
    pub(super) search_cache: HashMap<BrowserSearchRequest, BrowserSearchSnapshot>,
    pub(super) search_completed_tasks: usize,
    pub(super) search_total_tasks: usize,
    pub(super) search_in_flight: bool,
    pub(super) search_tx: Option<mpsc::Sender<SearchUpdate>>,
    pub(super) search_rx: Option<Arc<Mutex<mpsc::Receiver<SearchUpdate>>>>,
    pub(super) download_queue: VecDeque<QueuedContentDownload>,
    pub(super) download_in_flight: bool,
    pub(super) active_download: Option<ActiveContentDownload>,
    pub(super) download_tx: Option<mpsc::Sender<Result<ContentDownloadOutcome, String>>>,
    pub(super) download_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<ContentDownloadOutcome, String>>>>>,
    pub(super) identify_in_flight: bool,
    pub(super) identify_tx: Option<mpsc::Sender<(PathBuf, Result<UnifiedContentEntry, String>)>>,
    pub(super) identify_rx:
        Option<Arc<Mutex<mpsc::Receiver<(PathBuf, Result<UnifiedContentEntry, String>)>>>>,
    pub(super) status_message: Option<String>,
    pub(super) search_notification_active: bool,
    pub(super) download_notification_active: bool,
    pub(super) cached_manifest: Option<ContentInstallManifest>,
    pub(super) manifest_dirty: bool,
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

pub(super) fn trim_content_browser_search_cache(state: &mut ContentBrowserState) {
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
