use serde::{Deserialize, Serialize};

use crate::types::{MinecraftCapeState, MinecraftSkinState};

/// Minecraft profile data persisted in account cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftProfileState {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<MinecraftSkinState>,
    #[serde(default)]
    pub capes: Vec<MinecraftCapeState>,
}
