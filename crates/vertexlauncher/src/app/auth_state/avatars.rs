use super::*;

impl AuthState {
    pub(super) fn sync_account_avatar_cache(&mut self) {
        let known_profile_ids = self
            .accounts_state
            .accounts
            .iter()
            .map(|account| account.minecraft_profile.id.as_str())
            .collect::<HashSet<_>>();
        self.account_avatars
            .retain(|profile_id, _| known_profile_ids.contains(profile_id.as_str()));

        for account in &self.accounts_state.accounts {
            if let Some(bytes) = account.avatar_png_bytes() {
                self.account_avatars
                    .insert(account.minecraft_profile.id.to_ascii_lowercase(), bytes);
            }
        }
    }

    pub(super) fn rebuild_active_avatar_png(&mut self) {
        self.active_avatar_png =
            active_avatar_from_map(&self.accounts_state, &self.account_avatars);
    }

    pub(super) fn schedule_missing_avatars(&mut self) {
        for account in &self.accounts_state.accounts {
            let profile_id = account.minecraft_profile.id.clone();
            let profile_key = profile_id.to_ascii_lowercase();
            if profile_id.trim().is_empty()
                || self.account_avatars.contains_key(&profile_key)
                || self.avatar_loads_in_flight.contains(&profile_id)
                || account
                    .avatar_source_skin_url
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(|value| value.is_empty())
            {
                continue;
            }

            self.avatar_loads_in_flight.insert(profile_id.clone());
            let tx = self.avatar_result_tx.clone();
            let account = account.clone();
            let _ = tokio_runtime::spawn_blocking_detached(move || {
                let result = match auth::resolve_cached_account_avatar(&account) {
                    Ok(avatar_png) => AvatarLoadResult {
                        profile_id,
                        avatar_png,
                        error: None,
                    },
                    Err(err) => AvatarLoadResult {
                        profile_id,
                        avatar_png: None,
                        error: Some(err.to_string()),
                    },
                };
                if let Err(err) = tx.send(result) {
                    tracing::error!(
                        target: "vertexlauncher/auth/avatar",
                        error = %err,
                        "Failed to deliver avatar background-load result."
                    );
                }
            });
        }
    }

    pub(super) fn poll_avatar_loads(&mut self) {
        loop {
            match self.avatar_result_rx.try_recv() {
                Ok(result) => {
                    self.avatar_loads_in_flight.remove(&result.profile_id);
                    if let Some(error) = result.error {
                        tracing::warn!(
                            target: "vertexlauncher/auth/avatar",
                            profile_id = result.profile_id,
                            error,
                            "Failed to resolve account avatar in background worker."
                        );
                        notification::warn!("auth/avatar", "Failed to resolve account avatar in background worker: {error}", error = error);
                        continue;
                    }

                    if let Some(account) = self
                        .accounts_state
                        .accounts
                        .iter_mut()
                        .find(|account| account.minecraft_profile.id == result.profile_id)
                    {
                        account.set_avatar_png_bytes(result.avatar_png.as_deref());
                    }

                    if let Some(avatar_png) = result.avatar_png {
                        self.account_avatars
                            .insert(result.profile_id.to_ascii_lowercase(), avatar_png);
                    }

                    self.rebuild_active_avatar_png();
                    self.save_accounts_in_background("Failed to persist background-loaded avatar");
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/auth/avatar",
                        "Avatar background-load worker disconnected unexpectedly."
                    );
                    break;
                }
            }
        }
    }
}

pub(super) fn decoded_cached_avatars(state: &CachedAccountsState) -> HashMap<String, Vec<u8>> {
    state
        .accounts
        .iter()
        .filter_map(|account| {
            account
                .avatar_png_bytes()
                .map(|bytes| (account.minecraft_profile.id.to_ascii_lowercase(), bytes))
        })
        .collect()
}

pub(super) fn active_avatar_from_map(
    state: &CachedAccountsState,
    avatars: &HashMap<String, Vec<u8>>,
) -> Option<Vec<u8>> {
    let profile_id = state
        .active_account()?
        .minecraft_profile
        .id
        .to_ascii_lowercase();
    avatars.get(&profile_id).cloned()
}
