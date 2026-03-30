use crate::screens::{AppScreen, InstancePresenceSection};

#[derive(Debug, Clone, Default)]
pub struct InstanceScreenOutput {
    pub instances_changed: bool,
    pub requested_screen: Option<AppScreen>,
    pub presence_section: InstancePresenceSection,
}
