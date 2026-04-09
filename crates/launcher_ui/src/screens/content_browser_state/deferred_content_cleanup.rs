use super::*;

#[derive(Debug, Default)]
pub(crate) struct DeferredContentCleanup {
    pub(crate) stale_paths: Vec<PathBuf>,
    pub(crate) staged_paths: Vec<PathBuf>,
}
