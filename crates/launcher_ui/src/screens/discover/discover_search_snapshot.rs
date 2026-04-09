use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct DiscoverSearchSnapshot {
    pub(crate) entries: Vec<DiscoverEntry>,
    pub(crate) warnings: Vec<String>,
    pub(crate) has_more: bool,
}
