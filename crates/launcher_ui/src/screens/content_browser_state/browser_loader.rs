use vertex_i18n::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BrowserLoader {
    Any,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

impl BrowserLoader {
    pub(crate) const ALL: [BrowserLoader; 5] = [
        BrowserLoader::Any,
        BrowserLoader::Fabric,
        BrowserLoader::Forge,
        BrowserLoader::NeoForge,
        BrowserLoader::Quilt,
    ];

    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            BrowserLoader::Any => tr!("Any"),
            BrowserLoader::Fabric => tr!("Fabric"),
            BrowserLoader::Forge => tr!("Forge"),
            BrowserLoader::NeoForge => tr!("NeoForge"),
            BrowserLoader::Quilt => tr!("Quilt"),
        }
    }

    pub(crate) fn modrinth_slug(self) -> Option<&'static str> {
        match self {
            BrowserLoader::Any => None,
            BrowserLoader::Fabric => Some("fabric"),
            BrowserLoader::Forge => Some("forge"),
            BrowserLoader::NeoForge => Some("neoforge"),
            BrowserLoader::Quilt => Some("quilt"),
        }
    }

    pub(crate) fn curseforge_mod_loader_type(self) -> Option<u32> {
        match self {
            BrowserLoader::Any => None,
            BrowserLoader::Forge => Some(1),
            BrowserLoader::Fabric => Some(4),
            BrowserLoader::Quilt => Some(5),
            BrowserLoader::NeoForge => Some(6),
        }
    }
}
