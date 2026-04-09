use super::*;

#[derive(Debug, Clone)]
pub(super) struct ThumbnailCacheEntry {
    pub(super) bytes: Option<Arc<[u8]>>,
    pub(super) approx_bytes: usize,
    pub(super) last_touched_frame: u64,
}
