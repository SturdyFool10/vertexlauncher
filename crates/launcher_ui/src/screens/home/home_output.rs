use super::*;

#[derive(Debug, Clone, Default)]
pub struct HomeOutput {
    pub requested_screen: Option<AppScreen>,
    pub selected_instance_id: Option<String>,
    pub delete_requested_instance_id: Option<String>,
    pub presence_section: HomePresenceSection,
}
