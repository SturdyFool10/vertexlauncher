use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ServerEntry {
    pub(crate) instance_id: String,
    pub(crate) instance_name: String,
    pub(crate) server_name: String,
    pub(crate) address: String,
    pub(crate) favorite_id: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) icon_png: Option<Arc<[u8]>>,
    pub(crate) last_used_at_ms: Option<u64>,
    pub(crate) favorite: bool,
}
