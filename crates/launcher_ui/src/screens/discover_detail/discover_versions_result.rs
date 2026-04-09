use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverVersionsResult {
    pub(crate) request_serial: u64,
    pub(crate) versions: Result<Vec<DiscoverVersionEntry>, String>,
}
