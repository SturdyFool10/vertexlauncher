#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverProviderFilter {
    #[default]
    All,
    Modrinth,
    CurseForge,
}

impl DiscoverProviderFilter {
    pub(crate) const ALL: [Self; 3] = [Self::All, Self::Modrinth, Self::CurseForge];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All Sources",
            Self::Modrinth => "Modrinth",
            Self::CurseForge => "CurseForge",
        }
    }
}
