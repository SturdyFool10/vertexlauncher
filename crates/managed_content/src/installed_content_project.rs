use serde::{Deserialize, Serialize};

use crate::ManagedContentSource;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledContentProject {
    #[serde(default)]
    pub project_key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub folder_name: String,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub modrinth_project_id: Option<String>,
    #[serde(default)]
    pub curseforge_project_id: Option<u64>,
    #[serde(default)]
    pub selected_source: Option<ManagedContentSource>,
    #[serde(default)]
    pub selected_version_id: Option<String>,
    #[serde(default)]
    pub selected_version_name: Option<String>,
    #[serde(default)]
    pub explicitly_installed: bool,
    #[serde(default)]
    pub direct_dependencies: Vec<String>,
}
