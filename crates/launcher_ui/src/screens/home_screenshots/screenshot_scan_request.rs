use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ScreenshotScanRequest {
    pub(crate) scanned_instance_count: usize,
    pub(crate) instances: Vec<ScreenshotScanInstance>,
}
