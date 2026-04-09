use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};

use super::MinecraftProfileState;
use crate::util::decode_base64;

/// Cached account record used by launcher UI/auth state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAccount {
    pub minecraft_profile: MinecraftProfileState,
    #[serde(default)]
    pub minecraft_access_token: Option<String>,
    #[serde(default)]
    pub microsoft_refresh_token: Option<String>,
    #[serde(default)]
    pub microsoft_client_id: Option<String>,
    #[serde(default)]
    pub microsoft_token_uri: Option<String>,
    #[serde(default)]
    pub microsoft_scope: Option<String>,
    #[serde(skip, default)]
    pub(crate) refresh_token_state: super::RefreshTokenState,
    #[serde(default)]
    pub xuid: Option<String>,
    #[serde(default)]
    pub user_type: Option<String>,
    /// Base64-encoded PNG bytes for the generated profile avatar.
    #[serde(default)]
    pub avatar_png_base64: Option<String>,
    #[serde(default)]
    pub avatar_source_skin_url: Option<String>,
    pub cached_at_unix_secs: u64,
}

impl CachedAccount {
    /// Decodes avatar PNG bytes from base64, if present and valid.
    pub fn avatar_png_bytes(&self) -> Option<Vec<u8>> {
        self.avatar_png_base64
            .as_deref()
            .and_then(|raw| decode_base64(raw).ok())
    }

    pub fn set_avatar_png_bytes(&mut self, bytes: Option<&[u8]>) {
        self.avatar_png_base64 = bytes.map(|raw| BASE64_STANDARD.encode(raw));
    }
}
