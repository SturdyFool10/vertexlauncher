use super::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct HomeThumbnailState {
    pub(crate) cache_frame_index: u64,
    pub(crate) results_tx: Option<mpsc::Sender<(String, Option<Arc<[u8]>>)>>,
    pub(crate) results_rx: Option<Arc<Mutex<mpsc::Receiver<(String, Option<Arc<[u8]>>)>>>>,
    pub(crate) cache: HashMap<String, ThumbnailCacheEntry>,
    pub(crate) in_flight: HashSet<String>,
}
