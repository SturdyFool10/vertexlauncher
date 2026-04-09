use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackModLoader {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) primary: bool,
}
