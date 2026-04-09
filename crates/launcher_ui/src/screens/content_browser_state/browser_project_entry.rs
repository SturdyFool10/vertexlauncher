use super::*;

#[derive(Clone, Debug)]
pub(crate) struct BrowserProjectEntry {
    pub(crate) dedupe_key: String,
    pub(crate) name: String,
    pub(crate) summary: String,
    pub(crate) content_type: BrowserContentType,
    pub(crate) icon_url: Option<String>,
    pub(crate) modrinth_project_id: Option<String>,
    pub(crate) curseforge_project_id: Option<u64>,
    pub(crate) sources: Vec<ContentSource>,
    pub(crate) popularity_score: Option<u64>,
    pub(crate) updated_at: Option<String>,
    pub(crate) relevance_rank: u32,
}
