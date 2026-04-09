use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HomeTab {
    #[default]
    InstancesAndWorlds,
    Screenshots,
}

impl HomeTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::InstancesAndWorlds => "Instances & Worlds",
            Self::Screenshots => "Screenshots",
        }
    }
}

impl HomeTab {
    pub(crate) fn presence_section(self) -> HomePresenceSection {
        match self {
            Self::InstancesAndWorlds => HomePresenceSection::Activity,
            Self::Screenshots => HomePresenceSection::Screenshots,
        }
    }
}
