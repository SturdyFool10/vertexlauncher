use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackManifest {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) author: String,
    pub(crate) minecraft: CurseForgePackMinecraft,
    #[serde(default)]
    pub(crate) files: Vec<CurseForgePackFile>,
    #[serde(default)]
    pub(crate) overrides: Option<PathBuf>,
}
