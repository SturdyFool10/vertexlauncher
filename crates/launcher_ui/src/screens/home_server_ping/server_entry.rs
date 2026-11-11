use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ServerEntry {
    pub(crate) instance_id: String,
    pub(crate) server_name: String,
    pub(crate) address: String,
    pub(crate) favorite_id: String,
    pub(crate) icon_png: Option<Arc<[u8]>>,
    pub(crate) last_used_at_ms: Option<u64>,
    pub(crate) favorite: bool,
    /// The protocol version of the game this server came from, which the server list ping sends in its handshake.
    pub(crate) protocol_version: Option<i32>,
    /// Every instance holding this server; more than one collapses into a single entry.
    pub(crate) instances: Vec<EntryInstance>,
}
