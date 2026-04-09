use super::*;

#[derive(Debug, Clone, Default)]
pub(super) struct LibraryState {
    pub(super) runtime: RuntimeWorkflowState,
    pub(super) thumbnail_cache_frame_index: u64,
    pub(super) thumbnail_results_tx: Option<mpsc::Sender<(String, Option<Arc<[u8]>>)>>,
    pub(super) thumbnail_results_rx:
        Option<Arc<Mutex<mpsc::Receiver<(String, Option<Arc<[u8]>>)>>>>,
    pub(super) thumbnail_cache: HashMap<String, ThumbnailCacheEntry>,
    pub(super) thumbnail_in_flight: HashSet<String>,
}
