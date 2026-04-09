use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BrowserContentType {
    Mod,
    ResourcePack,
    Shader,
    DataPack,
}

impl BrowserContentType {
    pub(crate) const ORDERED: [BrowserContentType; 4] = [
        BrowserContentType::Mod,
        BrowserContentType::ResourcePack,
        BrowserContentType::Shader,
        BrowserContentType::DataPack,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "Mod",
            BrowserContentType::ResourcePack => "Resource Pack",
            BrowserContentType::Shader => "Shader",
            BrowserContentType::DataPack => "Data Pack",
        }
    }

    pub(crate) fn folder_name(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "mods",
            BrowserContentType::ResourcePack => "resourcepacks",
            BrowserContentType::Shader => "shaderpacks",
            BrowserContentType::DataPack => "datapacks",
        }
    }

    pub(crate) fn default_discovery_query(self) -> &'static str {
        match self {
            BrowserContentType::Mod => DEFAULT_DISCOVERY_QUERY_MOD,
            BrowserContentType::ResourcePack => DEFAULT_DISCOVERY_QUERY_RESOURCE_PACK,
            BrowserContentType::Shader => DEFAULT_DISCOVERY_QUERY_SHADER,
            BrowserContentType::DataPack => DEFAULT_DISCOVERY_QUERY_DATA_PACK,
        }
    }

    pub(crate) fn modrinth_project_type(self) -> &'static str {
        match self {
            BrowserContentType::Mod => "mod",
            BrowserContentType::ResourcePack => "resourcepack",
            BrowserContentType::Shader => "shader",
            BrowserContentType::DataPack => "datapack",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            BrowserContentType::Mod => 0,
            BrowserContentType::ResourcePack => 1,
            BrowserContentType::Shader => 2,
            BrowserContentType::DataPack => 3,
        }
    }
}
