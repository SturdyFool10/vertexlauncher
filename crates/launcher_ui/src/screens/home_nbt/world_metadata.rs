#[derive(Debug, Clone, Default)]
pub(crate) struct WorldMetadata {
    pub(crate) level_name: Option<String>,
    pub(crate) game_mode: Option<String>,
    pub(crate) hardcore: Option<bool>,
    pub(crate) cheats_enabled: Option<bool>,
    pub(crate) difficulty: Option<String>,
    pub(crate) version_name: Option<String>,
    pub(crate) last_played_ms: Option<u64>,
}
