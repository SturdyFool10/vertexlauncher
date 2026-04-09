use super::*;

#[derive(Clone, Debug)]
pub(crate) struct CachedDiscoverMasonryLayout {
    pub(crate) width_bucket: u32,
    pub(crate) entries_fingerprint: u64,
    pub(crate) height_cache_revision: u64,
    pub(crate) layout: DiscoverMasonryLayout,
}
