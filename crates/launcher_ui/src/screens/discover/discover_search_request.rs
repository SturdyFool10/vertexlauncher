use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DiscoverSearchRequest {
    pub(crate) query: String,
    pub(crate) tags: Vec<String>,
    pub(crate) game_version: Option<String>,
    pub(crate) provider_filter: DiscoverProviderFilter,
    pub(crate) loader_filter: DiscoverLoaderFilter,
    pub(crate) sort_mode: DiscoverSortMode,
    pub(crate) page: u32,
}
