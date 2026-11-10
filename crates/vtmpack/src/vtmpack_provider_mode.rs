#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VtmpackProviderMode {
    IncludeCurseForge,
    ExcludeCurseForge,
}

impl VtmpackProviderMode {
    pub fn label(self) -> &'static str {
        match self {
            VtmpackProviderMode::IncludeCurseForge => vertex_i18n::src!("Allow CurseForge project IDs"),
            VtmpackProviderMode::ExcludeCurseForge => vertex_i18n::src!("Exclude CurseForge as a provider"),
        }
    }
}
