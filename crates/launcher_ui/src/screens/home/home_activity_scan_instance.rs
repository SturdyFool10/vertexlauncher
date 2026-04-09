use super::*;

#[derive(Debug, Clone)]
pub(crate) struct HomeActivityScanInstance {
    pub(crate) instance_id: String,
    pub(crate) instance_name: String,
    pub(crate) instance_root: PathBuf,
    pub(crate) favorite_world_ids: Vec<String>,
    pub(crate) favorite_server_ids: Vec<String>,
}
