use content_resolver::{InstalledContentHashCacheUpdate, ResolvedInstalledContent};

#[derive(Clone, Debug)]
pub(super) struct ContentLookupResultEntry {
    pub(super) request_serial: u64,
    pub(super) lookup_key: String,
    pub(super) resolution: Option<ResolvedInstalledContent>,
}

#[derive(Clone, Debug)]
pub(super) struct ContentLookupResult {
    pub(super) results: Vec<ContentLookupResultEntry>,
    pub(super) hash_cache_updates: Vec<InstalledContentHashCacheUpdate>,
}
