use serde::{Deserialize, Serialize};

use crate::{ContentInstallManifest, ManagedContentSource};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModpackInstallState {
    pub format: String,
    pub pack_name: String,
    pub version_id: String,
    pub version_name: String,
    pub modrinth_project_id: Option<String>,
    pub curseforge_project_id: Option<u64>,
    pub source: Option<ManagedContentSource>,
    pub base_manifest: ContentInstallManifest,
}
