#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomePresenceSection {
    Activity,
    Screenshots,
}

impl Default for HomePresenceSection {
    fn default() -> Self {
        Self::Activity
    }
}
