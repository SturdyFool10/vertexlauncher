use super::*;

pub(crate) struct PreparedTextCacheEntry {
    pub(crate) fingerprint: u64,
    pub(crate) layout: Arc<PreparedTextLayout>,
    pub(crate) last_used_frame: u64,
}
