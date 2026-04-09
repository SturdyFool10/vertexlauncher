use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MrpackManifest {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(rename = "versionId", default)]
    pub(crate) version_id: String,
    #[serde(default)]
    pub(crate) summary: Option<String>,
    pub(crate) dependencies: HashMap<String, String>,
    #[serde(default)]
    pub(crate) files: Vec<MrpackFile>,
}
