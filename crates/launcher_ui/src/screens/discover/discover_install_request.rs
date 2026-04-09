use super::*;

#[derive(Debug, Clone)]
pub struct DiscoverInstallRequest {
    pub instance_name: String,
    pub project_summary: Option<String>,
    pub icon_url: Option<String>,
    pub version_name: String,
    pub source: DiscoverInstallSource,
}
