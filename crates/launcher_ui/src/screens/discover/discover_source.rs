use vertex_i18n::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverSource {
    Modrinth,
    CurseForge,
}

impl DiscoverSource {
    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Modrinth => tr!("Modrinth"),
            Self::CurseForge => tr!("CurseForge"),
        }
    }
}
