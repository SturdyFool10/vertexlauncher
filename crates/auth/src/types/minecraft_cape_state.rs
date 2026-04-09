use serde::{Deserialize, Serialize};

use crate::util::decode_base64;

/// One Minecraft cape entry from profile API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftCapeState {
    pub id: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub alias: Option<String>,
    /// Base64-encoded PNG bytes for this cape texture.
    #[serde(default)]
    pub texture_png_base64: Option<String>,
}

impl MinecraftCapeState {
    /// Decodes cape PNG bytes from base64, if present and valid.
    pub fn texture_png_bytes(&self) -> Option<Vec<u8>> {
        self.texture_png_base64
            .as_deref()
            .and_then(|raw| decode_base64(raw).ok())
    }
}
