use super::*;

pub(crate) struct CurseForgeConcurrentDownloadResult {
    pub(super) staged_files: HashMap<u64, PathBuf>,
    pub(super) failed_requirements: Vec<CurseForgeManualDownloadRequirement>,
}
