use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverEntry {
    pub(crate) dedupe_key: String,
    pub(crate) name: String,
    pub(crate) summary: String,
    pub(crate) author: Option<String>,
    pub(crate) icon_url: Option<String>,
    pub(crate) primary_url: Option<String>,
    pub(crate) sources: Vec<DiscoverSource>,
    pub(crate) provider_refs: Vec<DiscoverProviderRef>,
    pub(crate) popularity_score: Option<u64>,
    pub(crate) updated_at: Option<String>,
    pub(crate) relevance_rank: u32,
}
