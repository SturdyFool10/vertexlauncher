use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MrpackFileEnv {
    #[serde(default)]
    pub(crate) client: Option<String>,
}
