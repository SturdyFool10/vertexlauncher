#[derive(Clone, Debug)]
pub struct CurseForgeManualDownloadRequirement {
    pub project_id: u64,
    pub file_id: u64,
    pub project_name: String,
    pub file_name: String,
    pub display_name: String,
    pub download_page_url: String,
}
