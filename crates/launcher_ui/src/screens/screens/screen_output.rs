use super::*;

/// Actions emitted by the active screen for the application shell to handle.
#[derive(Debug, Clone, Default)]
pub struct ScreenOutput {
    pub instances_changed: bool,
    pub requested_screen: Option<AppScreen>,
    pub selected_instance_id: Option<String>,
    pub delete_requested_instance_id: Option<String>,
    pub discover_install_requested: Option<DiscoverInstallRequest>,
    pub menu_presence_context: Option<MenuPresenceContext>,
}
