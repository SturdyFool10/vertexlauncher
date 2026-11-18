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
    pub(crate) screenshot_candidates: Vec<ScreenshotCandidate>,
    pub(crate) screenshot_layout_revision: u64,
    /// File listings from the worker: (serial, instance count, forced, candidates).
    pub(crate) candidate_scans: launcher_runtime::WorkerChannel<(u64, usize, bool, Vec<ScreenshotCandidate>)>,
    pub(crate) candidate_scan_serial: u64,
    pub(crate) candidate_scan_in_flight: bool,
    pub(crate) thumbnails: HomeThumbnailState,
    pub(crate) screenshot_viewer: Option<ScreenshotViewerState>,
    pub(crate) pending_delete_screenshot_key: Option<String>,
    pub(crate) delete_screenshot_in_flight: bool,
    pub(crate) delete_screenshot_results:
        launcher_runtime::WorkerChannel<(String, String, Result<(), String>)>,
}

impl HomeState {
    pub(crate) fn mark_screenshot_layout_dirty(&mut self) {
        self.screenshot_layout_revision = self.screenshot_layout_revision.saturating_add(1);
    }




}
