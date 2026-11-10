#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportPackageKind {
    VertexPack,
    ModrinthPack,
    CurseForgePack,
}

impl ImportPackageKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ImportPackageKind::VertexPack => vertex_i18n::src!("Vertex .vtmpack"),
            ImportPackageKind::ModrinthPack => vertex_i18n::src!("Modrinth .mrpack"),
            ImportPackageKind::CurseForgePack => vertex_i18n::src!("CurseForge modpack zip"),
        }
    }
}
