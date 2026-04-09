use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverProviderEntry {
    pub(crate) project_ref: DiscoverProjectRef,
    pub(crate) name: String,
    pub(crate) summary: String,
    pub(crate) author: Option<String>,
    pub(crate) icon_url: Option<String>,
    pub(crate) primary_url: Option<String>,
    pub(crate) source: DiscoverSource,
    pub(crate) popularity_score: Option<u64>,
    pub(crate) updated_at: Option<String>,
    pub(crate) relevance_rank: u32,
}
