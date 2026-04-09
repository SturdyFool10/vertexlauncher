use serde::{Deserialize, Serialize};

use crate::types::CachedAccount;

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
