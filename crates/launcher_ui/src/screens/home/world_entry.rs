use super::*;

#[derive(Debug, Clone)]
pub(crate) struct WorldEntry {
    pub(crate) instance_id: String,
    pub(crate) instance_name: String,
    pub(crate) world_id: String,
    pub(crate) world_name: String,
    pub(crate) game_mode: Option<String>,
    pub(crate) hardcore: Option<bool>,
    pub(crate) cheats_enabled: Option<bool>,
    pub(crate) difficulty: Option<String>,
    pub(crate) version_name: Option<String>,
    pub(crate) thumbnail_png: Option<Arc<[u8]>>,
    pub(crate) last_used_at_ms: Option<u64>,
    pub(crate) favorite: bool,
}
