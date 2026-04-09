use super::*;

#[derive(Clone, Debug)]
pub(crate) struct BrowserVersionEntry {
    pub(crate) source: ManagedContentSource,
    pub(crate) version_id: String,
    pub(crate) version_name: String,
    pub(crate) file_name: String,
    pub(crate) file_url: String,
    pub(crate) published_at: String,
    pub(crate) loaders: Vec<String>,
    pub(crate) game_versions: Vec<String>,
    pub(crate) dependencies: Vec<DependencyRef>,
}
