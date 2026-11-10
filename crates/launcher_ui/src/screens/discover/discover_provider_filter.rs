use vertex_i18n::tr;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverProviderFilter {
    #[default]
    All,
    Modrinth,
    CurseForge,
}

impl DiscoverProviderFilter {
    pub(crate) const ALL: [Self; 3] = [Self::All, Self::Modrinth, Self::CurseForge];

    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::All => tr!("All Sources"),
            Self::Modrinth => tr!("Modrinth"),
            Self::CurseForge => tr!("CurseForge"),
        }
    }
}
