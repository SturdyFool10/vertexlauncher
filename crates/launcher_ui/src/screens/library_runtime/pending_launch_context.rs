#[derive(Debug, Clone)]
pub(super) struct PendingLaunchContext {
    pub(super) instance_name: String,
    pub(super) instance_root_display: String,
    pub(super) tab_user_key: Option<String>,
    pub(super) tab_username: String,
}
