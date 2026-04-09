use super::*;

#[derive(Clone, Debug)]
pub(crate) struct CurseForgeDownloadPlan {
    pub(super) requirement: CurseForgeManualDownloadRequirement,
    pub(super) download_url: String,
    pub(super) source_label: &'static str,
}
