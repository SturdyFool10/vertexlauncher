use super::*;

#[derive(Clone, Debug)]
pub(super) struct ProviderSearchEntry {
    pub(super) name: String,
    pub(super) summary: String,
    pub(super) content_type: BrowserContentType,
    pub(super) source: ContentSource,
    pub(super) modrinth_project_id: Option<String>,
    pub(super) curseforge_project_id: Option<u64>,
    pub(super) icon_url: Option<String>,
    pub(super) popularity_score: Option<u64>,
    pub(super) updated_at: Option<String>,
    pub(super) relevance_rank: u32,
}
