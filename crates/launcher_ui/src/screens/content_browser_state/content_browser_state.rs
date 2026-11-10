use super::*;

#[derive(Clone, Debug)]
pub struct ContentBrowserState {
    pub(crate) query_input: String,
    pub(crate) search_tags: Vec<String>,
    pub(crate) minecraft_version_filter: String,
    pub(crate) content_scope: ContentScope,
    pub(crate) mod_sort_mode: ModSortMode,
    pub(crate) loader: BrowserLoader,
    pub(crate) active_instance_id: Option<String>,
    pub(crate) active_instance_name: Option<String>,
    /// Where the queued installs go, remembered so the queue keeps draining while the screen is not shown.
    pub(crate) active_instance_root: Option<std::path::PathBuf>,
    pub(crate) auto_populated_instance_id: Option<String>,
    pub(crate) current_page: u32,
    pub(crate) current_view: ContentBrowserPage,
    pub(crate) detail_entry: Option<BrowserProjectEntry>,
    pub(crate) detail_tab: ContentDetailTab,
    pub(crate) detail_versions: Vec<BrowserVersionEntry>,
    pub(crate) detail_versions_cache: HashMap<String, Result<Vec<BrowserVersionEntry>, String>>,
    pub(crate) detail_versions_project_key: Option<String>,
    pub(crate) detail_versions_error: Option<String>,
    pub(crate) detail_versions_in_flight: bool,
    pub(crate) detail_loader_filter: BrowserLoader,
    pub(crate) detail_minecraft_version_filter: String,
    pub(crate) detail_versions_results: launcher_runtime::WorkerChannel<DetailVersionsResult>,
    pub(crate) available_game_versions: Vec<MinecraftVersionEntry>,
    /// Dropdown labels for `available_game_versions`; refreshed whenever the catalog changes.
    pub(crate) game_version_options: Vec<String>,
    pub(crate) version_catalog_error: Option<String>,
    pub(crate) version_catalog_in_flight: bool,
    pub(crate) version_catalog:
        launcher_runtime::WorkerChannel<Result<Vec<MinecraftVersionEntry>, String>>,
    pub(crate) results: BrowserSearchSnapshot,
    pub(crate) active_search_request: Option<BrowserSearchRequest>,
    pub(crate) search_cache: HashMap<BrowserSearchRequest, BrowserSearchSnapshot>,
    pub(crate) search_completed_tasks: usize,
    pub(crate) search_total_tasks: usize,
    pub(crate) search_in_flight: bool,
    pub(crate) search: launcher_runtime::WorkerChannel<SearchUpdate>,
    pub(crate) download_queue: VecDeque<QueuedContentDownload>,
    pub(crate) download_in_flight: bool,
    /// Everything in the batch being downloaded and installed right now.
    pub(crate) active_downloads: Vec<ActiveContentDownload>,
    /// Concurrent transfers and speed limit the install queue downloads under.
    pub(crate) download_policy: DownloadPolicy,
    /// Each finished request's dedupe key with how it went.
    pub(crate) download: launcher_runtime::WorkerChannel<(String, Result<ContentDownloadOutcome, String>)>,
    pub(crate) identify_in_flight: bool,
    pub(crate) identify:
        launcher_runtime::WorkerChannel<(PathBuf, Result<UnifiedContentEntry, String>)>,
    pub(crate) status_message: Option<String>,
    pub(crate) search_notification_active: bool,
    pub(crate) download_notification_active: bool,
    pub(crate) cached_manifest: Option<Arc<ContentInstallManifest>>,
    pub(crate) manifest_load_in_flight: bool,
    pub(crate) manifest_load: launcher_runtime::WorkerChannel<Arc<ContentInstallManifest>>,
    pub(crate) manifest_dirty: bool,
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
            active_instance_root: None,
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
            detail_versions_results: Default::default(),
            available_game_versions: Vec::new(),
            game_version_options: game_version_option_labels(&[]),
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog: Default::default(),
            results: BrowserSearchSnapshot::default(),
            active_search_request: None,
            search_cache: HashMap::new(),
            search_completed_tasks: 0,
            search_total_tasks: 0,
            search_in_flight: false,
            search: Default::default(),
            download_queue: VecDeque::new(),
            download_in_flight: false,
            active_downloads: Vec::new(),
            download_policy: DownloadPolicy::default(),
            download: Default::default(),
            identify_in_flight: false,
            identify: Default::default(),
            status_message: None,
            search_notification_active: false,
            download_notification_active: false,
            cached_manifest: None,
            manifest_load_in_flight: false,
            manifest_load: Default::default(),
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
        self.detail_versions_results.reset();
        self.results = BrowserSearchSnapshot::default();
        self.active_search_request = None;
        self.search_cache.clear();
        self.search_completed_tasks = 0;
        self.search_total_tasks = 0;
        self.search_in_flight = false;
        self.search.reset();
        self.download_queue.clear();
        self.download_in_flight = false;
        self.active_downloads.clear();
        self.download.reset();
        self.identify_in_flight = false;
        self.identify.reset();
        self.status_message = None;
        self.search_notification_active = false;
        self.download_notification_active = false;
    }
}
