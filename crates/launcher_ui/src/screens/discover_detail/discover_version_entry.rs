use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DiscoverVersionEntry {
    pub(crate) source: DiscoverSource,
    pub(crate) version_id: String,
    pub(crate) version_name: String,
    pub(crate) published_at: Option<String>,
    pub(crate) file_name: String,
    pub(crate) file_url: Option<String>,
    pub(crate) game_versions: Vec<String>,
    pub(crate) loaders: Vec<String>,
    pub(crate) download_count: Option<u64>,
}
