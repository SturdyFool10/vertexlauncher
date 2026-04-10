use super::mrpack_file_env::MrpackFileEnv;
use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MrpackFile {
    pub(crate) path: PathBuf,
    #[serde(default)]
    pub(crate) downloads: Vec<String>,
    #[serde(default)]
    pub(crate) env: Option<MrpackFileEnv>,
}
