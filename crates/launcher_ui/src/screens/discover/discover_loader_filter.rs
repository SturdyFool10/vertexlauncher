use vertex_i18n::tr;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverLoaderFilter {
    #[default]
    Any,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

impl DiscoverLoaderFilter {
    pub(crate) const ALL: [Self; 5] = [
        Self::Any,
        Self::Fabric,
        Self::Forge,
        Self::NeoForge,
        Self::Quilt,
    ];

    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Any => tr!("Any Loader"),
            Self::Fabric => tr!("Fabric"),
            Self::Forge => tr!("Forge"),
            Self::NeoForge => tr!("NeoForge"),
            Self::Quilt => tr!("Quilt"),
        }
    }

    pub(crate) fn modrinth_slug(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Fabric => Some("fabric"),
            Self::Forge => Some("forge"),
            Self::NeoForge => Some("neoforge"),
            Self::Quilt => Some("quilt"),
        }
    }

    pub(crate) fn curseforge_mod_loader_type(self) -> Option<u32> {
        match self {
            Self::Any => None,
            Self::Forge => Some(1),
            Self::Fabric => Some(4),
            Self::Quilt => Some(5),
            Self::NeoForge => Some(6),
        }
    }
}
