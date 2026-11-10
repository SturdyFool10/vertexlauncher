use vertex_i18n::tr;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ContentScope {
    All,
    Mods,
    ResourcePacks,
    Shaders,
    DataPacks,
}

impl ContentScope {
    pub(crate) const ALL: [ContentScope; 5] = [
        ContentScope::All,
        ContentScope::Mods,
        ContentScope::ResourcePacks,
        ContentScope::Shaders,
        ContentScope::DataPacks,
    ];

    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            ContentScope::All => tr!("All Types"),
            ContentScope::Mods => tr!("Mods"),
            ContentScope::ResourcePacks => tr!("Resource Packs"),
            ContentScope::Shaders => tr!("Shaders"),
            ContentScope::DataPacks => tr!("Data Packs"),
        }
    }

    pub(crate) fn includes(self, content_type: BrowserContentType) -> bool {
        match self {
            ContentScope::All => true,
            ContentScope::Mods => content_type == BrowserContentType::Mod,
            ContentScope::ResourcePacks => content_type == BrowserContentType::ResourcePack,
            ContentScope::Shaders => content_type == BrowserContentType::Shader,
            ContentScope::DataPacks => content_type == BrowserContentType::DataPack,
        }
    }
}
