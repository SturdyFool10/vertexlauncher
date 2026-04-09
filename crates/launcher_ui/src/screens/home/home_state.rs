use super::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct HomeState {
    pub(crate) active_tab: HomeTab,
    pub(crate) worlds: Vec<WorldEntry>,
    pub(crate) servers: Vec<ServerEntry>,
    pub(crate) server_pings: HashMap<String, ServerPingSnapshot>,
    pub(crate) last_scan_at: Option<Instant>,
    pub(crate) scanned_instance_count: usize,
    pub(crate) activity_scan_pending: bool,
    pub(crate) latest_requested_activity_scan_id: u64,
    pub(crate) server_ping_in_flight: HashSet<String>,
    pub(crate) screenshots: Vec<ScreenshotEntry>,
    pub(crate) last_screenshot_scan_at: Option<Instant>,
    pub(crate) scanned_screenshot_instance_count: usize,
    pub(crate) screenshot_scan_pending: bool,
    pub(crate) screenshot_scan_ready: bool,
    pub(crate) screenshot_tasks_total: usize,
    pub(crate) screenshot_tasks_done: usize,
    pub(crate) screenshot_candidates: Vec<ScreenshotCandidate>,
    pub(crate) screenshot_loaded_count: usize,
    pub(crate) latest_requested_screenshot_scan_id: u64,
    pub(crate) screenshot_images: LazyImageBytes,
    pub(crate) screenshot_layout_revision: u64,
    pub(crate) screenshot_masonry_layout_cache: Option<CachedVirtualMasonryLayout>,
    pub(crate) thumbnails: HomeThumbnailState,
    pub(crate) screenshot_viewer: Option<ScreenshotViewerState>,
    pub(crate) pending_delete_screenshot_key: Option<String>,
    pub(crate) delete_screenshot_in_flight: bool,
    pub(crate) delete_screenshot_results_tx:
        Option<mpsc::Sender<(String, String, Result<(), String>)>>,
    pub(crate) delete_screenshot_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, String, Result<(), String>)>>>>,
}

impl HomeState {
    pub(crate) fn mark_screenshot_layout_dirty(&mut self) {
        self.screenshot_layout_revision = self.screenshot_layout_revision.saturating_add(1);
        self.screenshot_masonry_layout_cache = None;
    }

    pub(crate) fn purge_screenshot_state(&mut self, ctx: &egui::Context) {
        self.latest_requested_screenshot_scan_id =
            self.latest_requested_screenshot_scan_id.saturating_add(1);
        self.screenshots.clear();
        self.last_screenshot_scan_at = None;
        self.scanned_screenshot_instance_count = 0;
        self.screenshot_scan_pending = false;
        self.screenshot_scan_ready = false;
        self.screenshot_tasks_total = 0;
        self.screenshot_tasks_done = 0;
        self.screenshot_candidates.clear();
        self.screenshot_loaded_count = 0;
        self.screenshot_images.clear(ctx);
        self.screenshot_viewer = None;
        self.pending_delete_screenshot_key = None;
        self.delete_screenshot_in_flight = false;
        self.delete_screenshot_results_tx = None;
        self.delete_screenshot_results_rx = None;
        self.mark_screenshot_layout_dirty();
    }

    pub(crate) fn purge_activity_image_state(&mut self, ctx: &egui::Context) {
        for world in &mut self.worlds {
            if world.thumbnail_png.take().is_some() {
                image_textures::evict_source_key(&home_world_thumbnail_uri(
                    world.instance_id.as_str(),
                    world.world_id.as_str(),
                ));
            }
        }
        for server in &mut self.servers {
            if server.icon_png.take().is_some() {
                image_textures::evict_source_key(&home_server_icon_uri(
                    server.instance_id.as_str(),
                    server.favorite_id.as_str(),
                ));
            }
        }
        purge_home_activity_thumbnail_state(ctx, &mut self.thumbnails);
        self.last_scan_at = None;
    }
}
