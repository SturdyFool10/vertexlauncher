#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportPackageKind {
    VertexPack,
    ModrinthPack,
    CurseForgePack,
}

impl ImportPackageKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ImportPackageKind::VertexPack => "Vertex .vtmpack",
            ImportPackageKind::ModrinthPack => "Modrinth .mrpack",
            ImportPackageKind::CurseForgePack => "CurseForge modpack zip",
        }
    }
}
