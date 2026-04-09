use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ServerPingSnapshot {
    pub(crate) status: ServerPingStatus,
    pub(crate) motd: Option<String>,
    pub(crate) players_online: Option<u32>,
    pub(crate) players_max: Option<u32>,
    pub(crate) checked_at: Instant,
}
