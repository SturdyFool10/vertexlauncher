use super::*;

#[derive(Debug, Clone)]
pub(crate) struct HomeActivityScanRequest {
    pub(crate) scanned_instance_count: usize,
    pub(crate) instances: Vec<HomeActivityScanInstance>,
}
