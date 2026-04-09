use super::*;

#[derive(Debug, Clone)]
pub(crate) struct HomeActivityScanResult {
    pub(crate) request_id: u64,
    pub(crate) scanned_instance_count: usize,
    pub(crate) worlds: Vec<WorldEntry>,
    pub(crate) servers: Vec<ServerEntry>,
}
