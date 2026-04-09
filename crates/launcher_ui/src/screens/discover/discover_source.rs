#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverSource {
    Modrinth,
    CurseForge,
}

impl DiscoverSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth",
            Self::CurseForge => "CurseForge",
        }
    }
}
