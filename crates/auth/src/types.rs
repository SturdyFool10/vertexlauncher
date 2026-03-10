use std::sync::mpsc::{self, Receiver};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};

use crate::util::decode_base64;

/// Browser/device-code OAuth session values required to complete login.
#[derive(Debug, Clone)]
pub struct MinecraftLoginFlow {
    pub verifier: String,
    pub auth_request_uri: String,
    pub(crate) state: String,
    pub(crate) client_id: String,
}

impl MinecraftLoginFlow {
    pub fn expected_state(&self) -> &str {
        &self.state
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MinecraftSkinVariant {
    Classic,
    Slim,
}

impl MinecraftSkinVariant {
    pub fn as_api_str(self) -> &'static str {
        match self {
            MinecraftSkinVariant::Classic => "classic",
            MinecraftSkinVariant::Slim => "slim",
        }
    }
}

/// One Minecraft skin entry from profile API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftSkinState {
    pub id: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    /// Base64-encoded PNG bytes for this skin texture.
    #[serde(default)]
    pub texture_png_base64: Option<String>,
}

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

/// Cached account record used by launcher UI/auth state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAccount {
    pub minecraft_profile: MinecraftProfileState,
    #[serde(default)]
    pub minecraft_access_token: Option<String>,
    #[serde(default)]
    pub microsoft_refresh_token: Option<String>,
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

impl MinecraftSkinState {
    /// Decodes skin PNG bytes from base64, if present and valid.
    pub fn texture_png_bytes(&self) -> Option<Vec<u8>> {
        self.texture_png_base64
            .as_deref()
            .and_then(|raw| decode_base64(raw).ok())
    }
}

impl MinecraftCapeState {
    /// Decodes cape PNG bytes from base64, if present and valid.
    pub fn texture_png_bytes(&self) -> Option<Vec<u8>> {
        self.texture_png_base64
            .as_deref()
            .and_then(|raw| decode_base64(raw).ok())
    }
}

/// Multi-account cache model with one active profile selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedAccountsState {
    #[serde(default)]
    pub active_profile_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<CachedAccount>,
}

impl CachedAccountsState {
    /// Normalizes account set by deduplicating profile ids and fixing active id.
    pub fn normalize(mut self) -> Self {
        let mut unique_accounts = Vec::with_capacity(self.accounts.len());

        for account in self.accounts.drain(..) {
            if let Some(existing_index) =
                unique_accounts.iter().position(|existing: &CachedAccount| {
                    existing.minecraft_profile.id == account.minecraft_profile.id
                })
            {
                unique_accounts[existing_index] = account;
            } else {
                unique_accounts.push(account);
            }
        }

        self.accounts = unique_accounts;

        if let Some(active_id) = self.active_profile_id.as_deref() {
            if !self
                .accounts
                .iter()
                .any(|account| account.minecraft_profile.id == active_id)
            {
                self.active_profile_id = self
                    .accounts
                    .first()
                    .map(|account| account.minecraft_profile.id.clone());
            }
        } else {
            self.active_profile_id = self
                .accounts
                .first()
                .map(|account| account.minecraft_profile.id.clone());
        }

        self
    }

    /// Returns the currently active account, falling back to first account.
    pub fn active_account(&self) -> Option<&CachedAccount> {
        let active_id = self.active_profile_id.as_deref()?;
        self.accounts
            .iter()
            .find(|account| account.minecraft_profile.id == active_id)
            .or_else(|| self.accounts.first())
    }

    /// Inserts/replaces an account by profile id and marks it active.
    pub fn upsert_and_activate(&mut self, account: CachedAccount) {
        if let Some(existing_index) = self
            .accounts
            .iter()
            .position(|existing| existing.minecraft_profile.id == account.minecraft_profile.id)
        {
            self.accounts[existing_index] = account.clone();
        } else {
            self.accounts.push(account.clone());
        }

        self.active_profile_id = Some(account.minecraft_profile.id);
        *self = std::mem::take(self).normalize();
    }

    /// Sets active profile id if it exists in `accounts`.
    ///
    /// Returns `false` when `profile_id` is unknown.
    pub fn set_active_profile_id(&mut self, profile_id: &str) -> bool {
        if !self
            .accounts
            .iter()
            .any(|account| account.minecraft_profile.id == profile_id)
        {
            return false;
        }

        self.active_profile_id = Some(profile_id.to_owned());
        true
    }

    /// Removes an account by profile id and updates active selection.
    ///
    /// Returns `false` when no account matched.
    pub fn remove_by_profile_id(&mut self, profile_id: &str) -> bool {
        let before_len = self.accounts.len();
        self.accounts
            .retain(|account| account.minecraft_profile.id != profile_id);
        if self.accounts.len() == before_len {
            return false;
        }

        if self.active_profile_id.as_deref() == Some(profile_id) {
            self.active_profile_id = self
                .accounts
                .first()
                .map(|account| account.minecraft_profile.id.clone());
        }

        true
    }
}

/// Instructions shown to user during device-code sign-in.
#[derive(Debug, Clone)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in_secs: u64,
    pub poll_interval_secs: u64,
    pub message: String,
}

/// Events produced by async device-code login polling.
#[derive(Debug, Clone)]
pub enum LoginEvent {
    DeviceCode(DeviceCodePrompt),
    WaitingForAuthorization,
    Completed(CachedAccount),
    Failed(String),
}

/// Handle for polling device-code login events from background runtime tasks.
#[derive(Debug)]
pub struct DeviceCodeLoginFlow {
    pub(crate) receiver: Receiver<LoginEvent>,
    pub(crate) finished: bool,
}

impl DeviceCodeLoginFlow {
    /// Drains currently available login events without blocking.
    pub fn poll_events(&mut self) -> Vec<LoginEvent> {
        let mut out = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    if matches!(event, LoginEvent::Completed(_) | LoginEvent::Failed(_)) {
                        self.finished = true;
                    }
                    out.push(event);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        out
    }

    /// Returns `true` once the flow has completed or failed.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}
