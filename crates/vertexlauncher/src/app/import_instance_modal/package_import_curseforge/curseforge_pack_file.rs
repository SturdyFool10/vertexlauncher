use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CurseForgePackFile {
    #[serde(rename = "projectID")]
    pub(crate) project_id: u64,
    #[serde(rename = "fileID")]
    pub(crate) file_id: u64,
    #[serde(default)]
    pub(crate) required: bool,
}
