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
    pub(crate) detail_versions_tx: Option<mpsc::Sender<DetailVersionsResult>>,
    pub(crate) detail_versions_rx: Option<Arc<Mutex<mpsc::Receiver<DetailVersionsResult>>>>,
    pub(crate) available_game_versions: Vec<MinecraftVersionEntry>,
    pub(crate) version_catalog_error: Option<String>,
    pub(crate) version_catalog_in_flight: bool,
    pub(crate) version_catalog_tx: Option<mpsc::Sender<Result<Vec<MinecraftVersionEntry>, String>>>,
    pub(crate) version_catalog_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<Vec<MinecraftVersionEntry>, String>>>>>,
    pub(crate) results: BrowserSearchSnapshot,
    pub(crate) active_search_request: Option<BrowserSearchRequest>,
    pub(crate) search_cache: HashMap<BrowserSearchRequest, BrowserSearchSnapshot>,
    pub(crate) search_completed_tasks: usize,
    pub(crate) search_total_tasks: usize,
    pub(crate) search_in_flight: bool,
    pub(crate) search_tx: Option<mpsc::Sender<SearchUpdate>>,
    pub(crate) search_rx: Option<Arc<Mutex<mpsc::Receiver<SearchUpdate>>>>,
    pub(crate) download_queue: VecDeque<QueuedContentDownload>,
    pub(crate) download_in_flight: bool,
    pub(crate) active_download: Option<ActiveContentDownload>,
    pub(crate) download_tx: Option<mpsc::Sender<Result<ContentDownloadOutcome, String>>>,
    pub(crate) download_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<ContentDownloadOutcome, String>>>>>,
    pub(crate) identify_in_flight: bool,
    pub(crate) identify_tx: Option<mpsc::Sender<(PathBuf, Result<UnifiedContentEntry, String>)>>,
    pub(crate) identify_rx:
        Option<Arc<Mutex<mpsc::Receiver<(PathBuf, Result<UnifiedContentEntry, String>)>>>>,
    pub(crate) status_message: Option<String>,
    pub(crate) search_notification_active: bool,
    pub(crate) download_notification_active: bool,
    pub(crate) cached_manifest: Option<ContentInstallManifest>,
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
