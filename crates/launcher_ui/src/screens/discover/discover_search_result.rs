use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverSearchResult {
    pub(crate) request_serial: u64,
    pub(crate) request: DiscoverSearchRequest,
    pub(crate) outcome: Result<DiscoverSearchSnapshot, String>,
}
