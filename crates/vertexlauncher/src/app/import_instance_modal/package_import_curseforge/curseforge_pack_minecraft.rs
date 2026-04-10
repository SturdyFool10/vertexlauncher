use super::curseforge_pack_mod_loader::CurseForgePackModLoader;
use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackMinecraft {
    pub(crate) version: String,
    #[serde(rename = "modLoaders", default)]
    pub(crate) mod_loaders: Vec<CurseForgePackModLoader>,
}
