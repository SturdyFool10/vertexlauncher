use vertex_i18n::tr;
use super::*;

impl AuthState {
    /// Applies the result of the background token-renewal worker, if it has finished.
    pub(super) fn poll_renewal(&mut self) {
        if let Some(renewal) = self.renewal.as_mut() {
            match renewal.try_recv() {
                Ok(RenewalResult::Bulk {
                    result,
                    failed_account_errors,
                    succeeded_profile_ids,
                }) => {
                    match result {
                        Ok(renewed) => {
                            let active_refresh =
                                matches!(self.status, AuthUiStatus::RefreshingActiveSession);
                            self.apply_accounts_state(renewed, true);
                            self.refreshed.extend(
                                self.accounts_state
                                    .accounts
                                    .iter()
                                    .map(|account| account.minecraft_profile.id.clone()),
                            );
                            for profile_id in succeeded_profile_ids {
                                self.failed_account_errors.remove(&profile_id);
                            }
                            for (profile_id, error) in failed_account_errors {
                                self.failed_account_errors.insert(profile_id, error);
                            }
                            self.retain_failed_accounts_for_known_profiles();
                            let refreshed_has_token = self
                                .accounts_state
                                .active_account()
                                .and_then(|account| account.minecraft_access_token.as_deref())
                                .map(str::trim)
                                .is_some_and(|token| !token.is_empty());
                            self.status = if active_refresh && !refreshed_has_token {
                                notification::warn!("auth", "Active account token could not be renewed. Continuing in offline mode.");
                                AuthUiStatus::Idle
                            } else {
                                AuthUiStatus::Idle
                            };
                            self.schedule_missing_avatars();
                        }
                        Err(err) => {
                            let prefix =
                                if matches!(self.status, AuthUiStatus::RefreshingActiveSession) {
                                    tr!("Failed to renew active account token").into_owned()
                                } else {
                                    tr!("Loaded cached accounts, but token renewal failed").into_owned()
                                };
                            tracing::warn!(
                                target: "vertexlauncher/auth_state",
                                active_refresh = matches!(self.status, AuthUiStatus::RefreshingActiveSession),
                                failed_accounts = failed_account_errors.len(),
                                error = %err,
                                "Account renewal worker returned an error."
                            );
                            if is_http_auth_error(err.as_str()) {
                                notification::warn!("auth", "{prefix}: {err}. Continuing with cached account in offline mode.", prefix = prefix, err = err);
                                self.status = AuthUiStatus::Idle;
                            } else {
                                notification::error!("auth", "{prefix}: {err}", prefix = prefix, err = err);
                                self.status = AuthUiStatus::Error(tr!("{prefix}: {err}", prefix = prefix, err = err).into_owned());
                            }
                        }
                    }
                    self.renewal = None;
                }
                Ok(RenewalResult::Single { profile_id, result }) => {
                    self.refreshed.insert(profile_id.clone());
                    match result {
                        Ok(renewed) => {
                            self.apply_accounts_state(renewed, true);
                            self.failed_account_errors.remove(&profile_id);
                            self.retain_failed_accounts_for_known_profiles();
                            self.status = AuthUiStatus::Idle;
                            self.schedule_missing_avatars();
                        }
                        Err(err) => {
                            self.failed_account_errors
                                .insert(profile_id.clone(), err.clone());
                            if is_http_auth_error(err.as_str()) {
                                notification::warn!("auth", "Failed to renew account token for {profile_id}: {err}. Continuing in offline mode.", profile_id = profile_id, err = err);
                                self.status = AuthUiStatus::Idle;
                            } else {
                                notification::error!("auth", "Failed to renew account token for {profile_id}: {err}", profile_id = profile_id, err = err);
                                self.status = AuthUiStatus::Error(tr!("Failed to renew account token for {profile_id}: {err}", profile_id = profile_id, err = err).into_owned());
                            }
                        }
                    }
                    self.renewal = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!(
                        target: "vertexlauncher/auth_state",
                        account_count = self.accounts_state.accounts.len(),
                        "Cached-account renewal worker disconnected unexpectedly."
                    );
                    notification::error!("auth", "Loaded cached accounts, but token renewal worker stopped unexpectedly.");
                    self.status = AuthUiStatus::Error(
                        tr!("Loaded cached accounts, but token renewal worker stopped unexpectedly.")
                            .into_owned(),
                    );
                    self.renewal = None;
                }
            }
        }
    }

    pub(super) fn ensure_active_account_token_ready(&mut self) {
        let has_active_minecraft_token = self
            .accounts_state
            .active_account()
            .and_then(|account| account.minecraft_access_token.as_deref())
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        if has_active_minecraft_token {
            return;
        }

        let has_refresh_token = self
            .accounts_state
            .active_account()
            .and_then(|account| account.microsoft_refresh_token.as_deref())
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        if !has_refresh_token {
            notification::warn!("auth", "Active account has no renewable token; launching in offline mode.");
            self.status = AuthUiStatus::Idle;
            return;
        }

        let client_id = match microsoft_client_id() {
            Ok(client_id) => client_id,
            Err(err) => {
                notification::warn!("auth", "Active account token is missing and renewal is unavailable: {err}. Launching in offline mode.", err = err);
                self.status = AuthUiStatus::Idle;
                return;
            }
        };

        self.spawn_renewal_worker(client_id, AuthUiStatus::RefreshingActiveSession);
    }

    pub(super) fn spawn_renewal_worker(&mut self, client_id: String, status: AuthUiStatus) {
        if self.renewal.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel::<RenewalResult>();
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            let mut failed_account_errors = HashMap::new();
            let mut succeeded_profile_ids = HashSet::new();
            let result = auth::renew_cached_accounts_tokens_with_callback(&client_id, |event| {
                match &event {
                    CachedAccountRenewalEvent::Succeeded { profile_id, .. } => {
                        succeeded_profile_ids.insert(profile_id.clone());
                        failed_account_errors.remove(profile_id);
                    }
                    CachedAccountRenewalEvent::Failed {
                        profile_id, error, ..
                    } => {
                        failed_account_errors.insert(profile_id.clone(), error.clone());
                    }
                    CachedAccountRenewalEvent::Started { .. } => {}
                }
                emit_cached_account_renewal_notification(event);
            })
            .map_err(|err| err.to_string());
            if let Err(err) = tx.send(RenewalResult::Bulk {
                result,
                failed_account_errors,
                succeeded_profile_ids,
            }) {
                tracing::error!(
                    target: "vertexlauncher/auth/renew",
                    error = %err,
                    "Failed to deliver bulk renewal result."
                );
            }
        });
        self.renewal = Some(rx);
        self.status = status;
    }

    pub(super) fn spawn_single_account_renewal_worker(
        &mut self,
        client_id: String,
        profile_id: String,
    ) {
        if self.renewal.is_some() {
            return;
        }

        let is_active_target =
            self.accounts_state.active_profile_id.as_deref() == Some(profile_id.as_str());
        let (tx, rx) = mpsc::channel::<RenewalResult>();
        self.failed_account_errors.remove(&profile_id);
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            let result = auth::renew_cached_account_token(&client_id, profile_id.as_str())
                .map_err(|err| err.to_string());
            if let Err(err) = tx.send(RenewalResult::Single {
                profile_id: profile_id.clone(),
                result,
            }) {
                tracing::error!(
                    target: "vertexlauncher/auth/renew",
                    profile_fingerprint = %webview_sign_in::fingerprint_for_log(&profile_id),
                    error = %err,
                    "Failed to deliver single-account renewal result."
                );
            }
        });
        self.renewal = Some(rx);
        self.status = if is_active_target {
            AuthUiStatus::RefreshingActiveSession
        } else {
            AuthUiStatus::RefreshingCachedSession
        };
    }

    pub(super) fn apply_accounts_state(
        &mut self,
        state: CachedAccountsState,
        preserve_active: bool,
    ) {
        let previous_active = if preserve_active {
            self.accounts_state.active_profile_id.clone()
        } else {
            None
        };
        self.accounts_state = state;
        if let Some(active_id) = previous_active.as_deref() {
            let _ = self.accounts_state.set_active_profile_id(active_id);
        }
        self.sync_account_avatar_cache();
        self.retain_failed_accounts_for_known_profiles();
        self.rebuild_active_avatar_png();
    }

    pub(super) fn retain_failed_accounts_for_known_profiles(&mut self) {
        let known_profile_ids = self
            .accounts_state
            .accounts
            .iter()
            .map(|account| account.minecraft_profile.id.as_str())
            .collect::<HashSet<_>>();
        self.failed_account_errors
            .retain(|profile_id, _| known_profile_ids.contains(profile_id.as_str()));
    }
}
