use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BrowserSearchRequest {
    pub(crate) query: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) game_version: Option<String>,
    pub(crate) loader: BrowserLoader,
    pub(crate) content_scope: ContentScope,
    pub(crate) mod_sort_mode: ModSortMode,
    pub(crate) page: u32,
}
