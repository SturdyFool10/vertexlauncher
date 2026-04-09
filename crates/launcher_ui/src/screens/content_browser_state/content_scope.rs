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

    pub(crate) fn label(self) -> &'static str {
        match self {
            ContentScope::All => "All Types",
            ContentScope::Mods => "Mods",
            ContentScope::ResourcePacks => "Resource Packs",
            ContentScope::Shaders => "Shaders",
            ContentScope::DataPacks => "Data Packs",
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
