use super::*;

#[derive(Debug, Clone)]
pub struct DiscoverState {
    pub(crate) query_input: String,
    pub(crate) search_tags: Vec<String>,
    pub(crate) game_version_filter: String,
    pub(crate) provider_filter: DiscoverProviderFilter,
    pub(crate) loader_filter: DiscoverLoaderFilter,
    pub(crate) sort_mode: DiscoverSortMode,
    pub(crate) page: u32,
    pub(crate) search_in_flight: bool,
    pub(crate) search_request_serial: u64,
    pub(crate) initial_search_requested: bool,
    pub(crate) status_message: Option<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) entries: Vec<DiscoverEntry>,
    pub(crate) tile_height_cache: HashMap<(String, u32), f32>,
    pub(crate) tile_height_cache_revision: u64,
    pub(crate) has_more_results: bool,
    pub(crate) masonry_layout_cache: Option<CachedDiscoverMasonryLayout>,
    pub(crate) cached_snapshots: HashMap<DiscoverSearchRequest, DiscoverSearchSnapshot>,
    pub(crate) available_game_versions: Vec<MinecraftVersionEntry>,
    pub(crate) version_catalog_error: Option<String>,
    pub(crate) version_catalog_in_flight: bool,
    pub(crate) version_catalog_tx: Option<mpsc::Sender<Result<Vec<MinecraftVersionEntry>, String>>>,
    pub(crate) version_catalog_rx:
        Option<Arc<Mutex<mpsc::Receiver<Result<Vec<MinecraftVersionEntry>, String>>>>>,
    pub(crate) search_results_tx: Option<mpsc::Sender<DiscoverSearchResult>>,
    pub(crate) search_results_rx: Option<Arc<Mutex<mpsc::Receiver<DiscoverSearchResult>>>>,
    pub(crate) detail_entry: Option<DiscoverEntry>,
    pub(crate) detail_selected_source: Option<DiscoverSource>,
    pub(crate) detail_versions: Vec<DiscoverVersionEntry>,
    pub(crate) detail_versions_error: Option<String>,
    pub(crate) detail_versions_in_flight: bool,
    pub(crate) detail_version_request_serial: u64,
    pub(crate) detail_version_results_tx: Option<mpsc::Sender<DiscoverVersionsResult>>,
    pub(crate) detail_version_results_rx: Option<Arc<Mutex<mpsc::Receiver<DiscoverVersionsResult>>>>,
    pub(crate) install_in_flight: bool,
    pub(crate) install_message: Option<String>,
    pub(crate) install_completed_steps: usize,
    pub(crate) install_total_steps: usize,
    pub(crate) install_error: Option<String>,
}

impl Default for DiscoverState {
    fn default() -> Self {
        Self {
            query_input: String::new(),
            search_tags: Vec::new(),
            game_version_filter: String::new(),
            provider_filter: DiscoverProviderFilter::default(),
            loader_filter: DiscoverLoaderFilter::default(),
            sort_mode: DiscoverSortMode::default(),
            page: 1,
            search_in_flight: false,
            search_request_serial: 0,
            initial_search_requested: false,
            status_message: None,
            warnings: Vec::new(),
            entries: Vec::new(),
            tile_height_cache: HashMap::new(),
            tile_height_cache_revision: 0,
            has_more_results: true,
            masonry_layout_cache: None,
            cached_snapshots: HashMap::new(),
            available_game_versions: Vec::new(),
            version_catalog_error: None,
            version_catalog_in_flight: false,
            version_catalog_tx: None,
            version_catalog_rx: None,
            search_results_tx: None,
            search_results_rx: None,
            detail_entry: None,
            detail_selected_source: None,
            detail_versions: Vec::new(),
            detail_versions_error: None,
            detail_versions_in_flight: false,
            detail_version_request_serial: 0,
            detail_version_results_tx: None,
            detail_version_results_rx: None,
            install_in_flight: false,
            install_message: None,
            install_completed_steps: 0,
            install_total_steps: 0,
            install_error: None,
        }
    }
}

impl DiscoverState {
    pub fn begin_install(&mut self, message: impl Into<String>) {
        self.install_in_flight = true;
        self.install_error = None;
        self.install_message = Some(message.into());
        self.install_completed_steps = 0;
        self.install_total_steps = 0;
    }

    pub fn apply_install_progress(
        &mut self,
        message: impl Into<String>,
        completed_steps: usize,
        total_steps: usize,
    ) {
        self.install_in_flight = true;
        self.install_error = None;
        self.install_message = Some(message.into());
        self.install_completed_steps = completed_steps;
        self.install_total_steps = total_steps;
    }

    pub fn finish_install(&mut self, result: Result<String, String>) {
        self.install_in_flight = false;
        match result {
            Ok(message) => {
                self.install_error = None;
                self.install_message = Some(message);
            }
            Err(error) => {
                self.install_error = Some(error);
            }
        }
    }

    pub fn purge_inactive_state(&mut self) {
        self.search_in_flight = false;
        self.initial_search_requested = false;
        self.status_message = None;
        self.warnings.clear();
        self.entries.clear();
        self.tile_height_cache.clear();
        self.tile_height_cache_revision = self.tile_height_cache_revision.saturating_add(1);
        self.masonry_layout_cache = None;
        self.cached_snapshots.clear();
        self.has_more_results = true;
        self.search_results_tx = None;
        self.search_results_rx = None;
        self.detail_entry = None;
        self.detail_selected_source = None;
        self.detail_versions.clear();
        self.detail_versions_error = None;
        self.detail_versions_in_flight = false;
        self.detail_version_request_serial = 0;
        self.detail_version_results_tx = None;
        self.detail_version_results_rx = None;
        self.install_in_flight = false;
        self.install_message = None;
        self.install_completed_steps = 0;
        self.install_total_steps = 0;
        self.install_error = None;
    }
}
