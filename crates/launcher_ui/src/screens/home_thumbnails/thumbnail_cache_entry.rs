use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ThumbnailCacheEntry {
    pub(crate) bytes: Option<Arc<[u8]>>,
    pub(crate) approx_bytes: usize,
    pub(crate) last_touched_frame: u64,
}
