use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ServerPingSnapshot {
    pub(crate) status: ServerPingStatus,
    /// The MOTD as the game would draw it: one entry per line, each a list of styled runs. Empty when the server sent none.
    pub(crate) motd: Vec<Vec<crate::mc_text::Span>>,
    pub(crate) players_online: Option<u32>,
    pub(crate) players_max: Option<u32>,
    /// Names from the status response's player sample; servers may cap, hide, or fake this list.
    pub(crate) players_sample: Vec<String>,
    pub(crate) checked_at: Instant,
}
