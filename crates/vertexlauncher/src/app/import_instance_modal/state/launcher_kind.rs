#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LauncherKind {
    Modrinth,
    CurseForge,
    Prism,
    ATLauncher,
    Unknown,
}

impl LauncherKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Modrinth => vertex_i18n::src!("Modrinth launcher instance"),
            Self::CurseForge => vertex_i18n::src!("CurseForge instance"),
            Self::Prism => vertex_i18n::src!("Prism / MultiMC / PolyMC instance"),
            Self::ATLauncher => vertex_i18n::src!("ATLauncher instance"),
            Self::Unknown => vertex_i18n::src!("Generic launcher instance"),
        }
    }
}
