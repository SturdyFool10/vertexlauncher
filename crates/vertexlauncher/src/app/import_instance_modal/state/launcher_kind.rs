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
            Self::Modrinth => "Modrinth launcher instance",
            Self::CurseForge => "CurseForge instance",
            Self::Prism => "Prism / MultiMC / PolyMC instance",
            Self::ATLauncher => "ATLauncher instance",
            Self::Unknown => "Generic launcher instance",
        }
    }
}
