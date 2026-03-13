use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::InstalledContentProject;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentInstallManifest {
    #[serde(default)]
    pub projects: BTreeMap<String, InstalledContentProject>,
}
