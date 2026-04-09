use super::*;

#[derive(Clone, Debug)]
pub(crate) struct AsyncRasterCacheEntry {
    pub(crate) layout: Arc<PreparedTextLayout>,
    pub(crate) last_used_frame: u64,
}
