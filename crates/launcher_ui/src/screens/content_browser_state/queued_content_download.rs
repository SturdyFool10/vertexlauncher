use super::*;

#[derive(Clone, Debug)]
pub(crate) struct QueuedContentDownload {
    pub(crate) request: ContentInstallRequest,
}
