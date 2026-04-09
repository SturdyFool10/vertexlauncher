#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstancePresenceSection {
    Content,
    Screenshots,
    Logs,
}

impl Default for InstancePresenceSection {
    fn default() -> Self {
        Self::Content
    }
}
