use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ScreenshotCandidate {
    pub(crate) instance_name: String,
    pub(crate) path: PathBuf,
    pub(crate) file_name: String,
    pub(crate) modified_at_ms: Option<u64>,
}
