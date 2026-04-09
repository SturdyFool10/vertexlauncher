use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ScreenshotScanInstance {
    pub(crate) instance_name: String,
    pub(crate) screenshots_dir: PathBuf,
}
