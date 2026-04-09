use super::*;

#[derive(Debug, Clone)]
pub(super) struct ServerPingResult {
    pub(super) address: String,
    pub(super) snapshot: ServerPingSnapshot,
}
