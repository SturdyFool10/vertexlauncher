#[derive(Debug, Clone)]
pub enum DiscoverInstallSource {
    Modrinth {
        project_id: String,
        version_id: String,
        file_url: String,
        file_name: String,
    },
    CurseForge {
        project_id: u64,
        file_id: u64,
        file_name: String,
        download_url: Option<String>,
        manual_download_path: Option<std::path::PathBuf>,
    },
}
