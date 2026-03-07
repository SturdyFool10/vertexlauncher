use std::sync::mpsc::{self, Receiver};

use serde::{Deserialize, Serialize};

use crate::util::decode_base64;

#[derive(Debug, Clone)]
pub struct MinecraftLoginFlow {
    pub verifier: String,
    pub challenge: String,
    pub session_id: String,
    pub auth_request_uri: String,
    pub(crate) state: String,
    pub(crate) client_id: String,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftProfileState {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<MinecraftSkinState>,
    #[serde(default)]
    pub capes: Vec<MinecraftCapeState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAccount {
    pub minecraft_profile: MinecraftProfileState,
    #[serde(default)]
    pub minecraft_access_token: Option<String>,
    #[serde(default)]
    pub xuid: Option<String>,
    #[serde(default)]
    pub user_type: Option<String>,
    /// Base64-encoded PNG bytes for the generated profile avatar.
    #[serde(default)]
    pub avatar_png_base64: Option<String>,
    pub cached_at_unix_secs: u64,
}

impl CachedAccount {
    pub fn avatar_png_bytes(&self) -> Option<Vec<u8>> {
        self.avatar_png_base64
            .as_deref()
            .and_then(|raw| decode_base64(raw).ok())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedAccountsState {
    #[serde(default)]
    pub active_profile_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<CachedAccount>,
}

impl CachedAccountsState {
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

    pub fn active_account(&self) -> Option<&CachedAccount> {
        let active_id = self.active_profile_id.as_deref()?;
        self.accounts
            .iter()
            .find(|account| account.minecraft_profile.id == active_id)
            .or_else(|| self.accounts.first())
    }

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

#[derive(Debug, Clone)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in_secs: u64,
    pub poll_interval_secs: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum LoginEvent {
    DeviceCode(DeviceCodePrompt),
    WaitingForAuthorization,
    Completed(CachedAccount),
    Failed(String),
}

#[derive(Debug)]
pub struct DeviceCodeLoginFlow {
    pub(crate) receiver: Receiver<LoginEvent>,
    pub(crate) finished: bool,
}

impl DeviceCodeLoginFlow {
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

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}
