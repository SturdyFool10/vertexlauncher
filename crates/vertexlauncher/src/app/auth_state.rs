use auth::{
    CachedAccount, CachedAccountRenewalEvent, CachedAccountsState, DeviceCodeLoginFlow,
    DeviceCodePrompt, LoginEvent,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::{notification, privacy};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use super::{system_browser_sign_in, webview_sign_in};

#[path = "auth_state/account_ui_entry.rs"]
mod account_ui_entry;
#[path = "auth_state/auth_flow_event.rs"]
mod auth_flow_event;
#[path = "auth_state/auth_ui_status.rs"]
mod auth_ui_status;
#[path = "auth_state/avatar_load_result.rs"]
mod avatar_load_result;
#[path = "auth_state/launch_auth_context.rs"]
mod launch_auth_context;
#[path = "auth_state/renewal_result.rs"]
mod renewal_result;

pub use self::account_ui_entry::AccountUiEntry;
use self::auth_flow_event::AuthFlowEvent;
use self::auth_ui_status::AuthUiStatus;
use self::avatar_load_result::AvatarLoadResult;
pub use self::launch_auth_context::LaunchAuthContext;
use self::renewal_result::RenewalResult;

pub const REPAINT_INTERVAL: Duration = Duration::from_millis(200);

pub struct AuthState {
    accounts_state: CachedAccountsState,
    account_avatars: HashMap<String, Vec<u8>>,
    failed_account_errors: HashMap<String, String>,
    avatar_result_tx: Sender<AvatarLoadResult>,
    avatar_result_rx: Receiver<AvatarLoadResult>,
    avatar_loads_in_flight: HashSet<String>,
    active_avatar_png: Option<Vec<u8>>,
    flow: Option<Receiver<AuthFlowEvent>>,
    device_code_flow: Option<DeviceCodeLoginFlow>,
    device_code_prompt: Option<DeviceCodePrompt>,
    device_code_expiry: Option<Instant>,
    renewal: Option<Receiver<RenewalResult>>,
    streamer_mode: bool,
    status: AuthUiStatus,
}

impl AuthState {
    pub fn load(streamer_mode: bool) -> Self {
        let (avatar_result_tx, avatar_result_rx) = mpsc::channel();
        let (accounts_state, status) = match auth::load_cached_accounts() {
            Ok(state) => (state, AuthUiStatus::Idle),
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/auth_state",
                    error = %err,
                    "Failed to load cached accounts during auth-state startup."
                );
                (
                    CachedAccountsState::default(),
                    AuthUiStatus::Error(format!("Failed to load cached account state: {err}")),
                )
            }
        };
        let account_avatars = decoded_cached_avatars(&accounts_state);
        let active_avatar_png = active_avatar_from_map(&accounts_state, &account_avatars);

        let mut auth_state = Self {
            accounts_state,
            account_avatars,
            failed_account_errors: HashMap::new(),
            avatar_result_tx,
            avatar_result_rx,
            avatar_loads_in_flight: HashSet::new(),
            active_avatar_png,
            flow: None,
            device_code_flow: None,
            device_code_prompt: None,
            device_code_expiry: None,
            renewal: None,
            streamer_mode,
            status,
        };

        if !auth_state.accounts_state.accounts.is_empty() {
            match microsoft_client_id() {
                Ok(client_id) => {
                    auth_state
                        .spawn_renewal_worker(client_id, AuthUiStatus::RefreshingCachedSession);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "vertexlauncher/auth_state",
                        error = %err,
                        account_count = auth_state.accounts_state.accounts.len(),
                        "Skipping cached-account renewal during startup."
                    );
                    auth_state.status = AuthUiStatus::Error(format!(
                        "Loaded cached accounts, but token renewal was skipped: {err}"
                    ));
                }
            }
        }

        auth_state.schedule_missing_avatars();
        auth_state
    }

    pub fn poll(&mut self) {
        self.poll_avatar_loads();

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
                                notification::warn!(
                                    "auth",
                                    "Active account token could not be renewed. Continuing in offline mode."
                                );
                                AuthUiStatus::Idle
                            } else {
                                AuthUiStatus::Idle
                            };
                            self.schedule_missing_avatars();
                        }
                        Err(err) => {
                            let prefix =
                                if matches!(self.status, AuthUiStatus::RefreshingActiveSession) {
                                    "Failed to renew active account token"
                                } else {
                                    "Loaded cached accounts, but token renewal failed"
                                };
                            tracing::warn!(
                                target: "vertexlauncher/auth_state",
                                active_refresh = matches!(self.status, AuthUiStatus::RefreshingActiveSession),
                                failed_accounts = failed_account_errors.len(),
                                error = %err,
                                "Account renewal worker returned an error."
                            );
                            if is_http_auth_error(err.as_str()) {
                                notification::warn!(
                                    "auth",
                                    "{prefix}: {err}. Continuing with cached account in offline mode."
                                );
                                self.status = AuthUiStatus::Idle;
                            } else {
                                notification::error!("auth", "{prefix}: {err}");
                                self.status = AuthUiStatus::Error(format!("{prefix}: {err}"));
                            }
                        }
                    }
                    self.renewal = None;
                }
                Ok(RenewalResult::Single { profile_id, result }) => {
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
                                notification::warn!(
                                    "auth",
                                    "Failed to renew account token for {profile_id}: {err}. Continuing in offline mode."
                                );
                                self.status = AuthUiStatus::Idle;
                            } else {
                                notification::error!(
                                    "auth",
                                    "Failed to renew account token for {profile_id}: {err}"
                                );
                                self.status = AuthUiStatus::Error(format!(
                                    "Failed to renew account token for {profile_id}: {err}"
                                ));
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
                    notification::error!(
                        "auth",
                        "Loaded cached accounts, but token renewal worker stopped unexpectedly."
                    );
                    self.status = AuthUiStatus::Error(
                        "Loaded cached accounts, but token renewal worker stopped unexpectedly."
                            .to_owned(),
                    );
                    self.renewal = None;
                }
            }
        }

        let mut flow_finished = false;

        while self.flow.is_some() {
            let next_event = {
                let Some(flow) = self.flow.as_mut() else {
                    break;
                };
                flow.try_recv()
            };
            match next_event {
                Ok(event) => match event {
                    AuthFlowEvent::AwaitingBrowser => {
                        self.status = AuthUiStatus::AwaitingBrowser;
                    }
                    AuthFlowEvent::AwaitingExternalBrowser => {
                        self.status = AuthUiStatus::AwaitingExternalBrowser;
                    }
                    AuthFlowEvent::WaitingForAuthorization => {
                        self.status = AuthUiStatus::WaitingForAuthorization;
                    }
                    AuthFlowEvent::Completed(account) => {
                        self.failed_account_errors
                            .remove(&account.minecraft_profile.id);
                        self.accounts_state.upsert_and_activate(account);
                        self.sync_account_avatar_cache();
                        self.rebuild_active_avatar_png();
                        self.status = AuthUiStatus::Idle;

                        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
                            tracing::warn!(
                                target: "vertexlauncher/auth_state",
                                error = %err,
                                active_profile = self.accounts_state.active_profile_id.as_deref().unwrap_or_default(),
                                "Sign-in succeeded but persisting cached account state failed."
                            );
                            let message = format!(
                                "Sign-in succeeded, but failed to cache account state: {err}"
                            );
                            if is_nonfatal_account_cache_error(message.as_str()) {
                                notification::warn!("auth", "{message}");
                            } else {
                                notification::error!("auth", "{message}");
                                self.status = AuthUiStatus::Error(message);
                            }
                        }

                        flow_finished = true;
                        self.schedule_missing_avatars();
                    }
                    AuthFlowEvent::Failed(err) => {
                        tracing::error!(
                            target: "vertexlauncher/auth_state",
                            error = %err,
                            "Interactive sign-in flow failed."
                        );
                        notification::error!("auth", "{err}");
                        self.status = AuthUiStatus::Error(err);
                        flow_finished = true;
                    }
                },
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !flow_finished {
                        tracing::error!(
                            target: "vertexlauncher/auth_state",
                            "Interactive sign-in worker disconnected before completion."
                        );
                        notification::error!(
                            "auth",
                            "Sign-in stopped unexpectedly before completion"
                        );
                        self.status = AuthUiStatus::Error(
                            "Sign-in stopped unexpectedly before completion".to_owned(),
                        );
                    }
                    flow_finished = true;
                    break;
                }
            }
        }

        if flow_finished {
            self.flow = None;
        }

        // Proactively restart device code flow when the code expires
        if self.device_code_flow.is_some()
            && self
                .device_code_expiry
                .is_some_and(|exp| exp <= Instant::now())
        {
            self.device_code_flow = None;
            self.device_code_prompt = None;
            self.device_code_expiry = None;
            self.restart_device_code_sign_in();
        }

        if let Some(device_flow) = self.device_code_flow.as_mut() {
            let mut device_flow_finished = false;
            for event in device_flow.poll_events() {
                match event {
                    LoginEvent::DeviceCode(prompt) => {
                        self.status = AuthUiStatus::AwaitingDeviceCode(format!(
                            "Go to {} and enter code: {}",
                            prompt.verification_uri, prompt.user_code
                        ));
                        self.device_code_expiry =
                            Some(Instant::now() + Duration::from_secs(prompt.expires_in_secs));
                        self.device_code_prompt = Some(prompt);
                    }
                    LoginEvent::WaitingForAuthorization => {
                        self.status = AuthUiStatus::WaitingForAuthorization;
                    }
                    LoginEvent::Completed(account) => {
                        self.failed_account_errors
                            .remove(&account.minecraft_profile.id);
                        self.accounts_state.upsert_and_activate(account);
                        self.sync_account_avatar_cache();
                        self.rebuild_active_avatar_png();
                        self.status = AuthUiStatus::Idle;

                        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
                            let message = format!(
                                "Sign-in succeeded, but failed to cache account state: {err}"
                            );
                            if is_nonfatal_account_cache_error(message.as_str()) {
                                notification::warn!("auth", "{message}");
                            } else {
                                notification::error!("auth", "{message}");
                                self.status = AuthUiStatus::Error(message);
                            }
                        }

                        device_flow_finished = true;
                        self.schedule_missing_avatars();
                    }
                    LoginEvent::Failed(err) => {
                        let expired = self
                            .device_code_expiry
                            .is_some_and(|exp| exp <= Instant::now());
                        if expired {
                            // Code timed out — restart silently with a fresh code
                            device_flow_finished = true;
                        } else {
                            notification::error!("auth", "{err}");
                            self.status = AuthUiStatus::Error(err);
                            device_flow_finished = true;
                        }
                    }
                }
            }
            if device_flow_finished {
                let should_restart = self
                    .device_code_expiry
                    .is_some_and(|exp| exp <= Instant::now());
                self.device_code_flow = None;
                self.device_code_prompt = None;
                self.device_code_expiry = None;
                if should_restart {
                    self.restart_device_code_sign_in();
                }
            }
        }
    }

    pub fn start_webview_sign_in(&mut self) {
        if self.flow.is_some() || self.device_code_flow.is_some() || self.renewal.is_some() {
            return;
        }

        let client_id = match microsoft_client_id() {
            Ok(client_id) => client_id,
            Err(err) => {
                self.status = AuthUiStatus::Error(err);
                return;
            }
        };

        self.status = AuthUiStatus::Starting;

        let (sender, receiver) = mpsc::channel();
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            run_sign_in_flow(client_id, sender);
        });

        self.flow = Some(receiver);
    }

    pub fn start_device_code_sign_in(&mut self) {
        if self.flow.is_some() || self.device_code_flow.is_some() || self.renewal.is_some() {
            return;
        }

        let client_id = match microsoft_client_id() {
            Ok(client_id) => client_id,
            Err(err) => {
                self.status = AuthUiStatus::Error(err);
                return;
            }
        };

        self.status = AuthUiStatus::Starting;
        self.device_code_flow = Some(auth::start_device_code_login(client_id));
    }

    pub fn start_system_browser_sign_in(&mut self, theme: &launcher_ui::ui::theme::Theme) {
        if self.flow.is_some() || self.renewal.is_some() {
            return;
        }

        self.device_code_flow = None;
        self.device_code_prompt = None;
        self.device_code_expiry = None;
        self.status = AuthUiStatus::Starting;

        let colors = system_browser_sign_in::CallbackPageColors::from_theme(theme);
        let (sender, receiver) = mpsc::channel();
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            run_system_browser_sign_in_flow(sender, colors);
        });

        self.flow = Some(receiver);
    }

    pub fn select_account(&mut self, profile_id: &str) {
        if !self.accounts_state.set_active_profile_id(profile_id) {
            return;
        }

        self.ensure_active_account_token_ready();
        self.rebuild_active_avatar_png();
        if !self.auth_busy() && !self.failed_account_errors.contains_key(profile_id) {
            self.status = AuthUiStatus::Idle;
        }

        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
            let message =
                format!("Switched account in memory, but failed to cache account state: {err}");
            if is_nonfatal_account_cache_error(message.as_str()) {
                notification::warn!("auth", "{message}");
            } else {
                notification::error!("auth", "{message}");
                self.status = AuthUiStatus::Error(message);
            }
        }
    }

    pub fn remove_account(&mut self, profile_id: &str) {
        if !self.accounts_state.remove_by_profile_id(profile_id) {
            return;
        }

        self.failed_account_errors.remove(profile_id);
        self.sync_account_avatar_cache();
        self.rebuild_active_avatar_png();
        self.status = AuthUiStatus::Idle;

        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
            let message =
                format!("Removed account in memory, but failed to cache account state: {err}");
            if is_nonfatal_account_cache_error(message.as_str()) {
                notification::warn!("auth", "{message}");
            } else {
                notification::error!("auth", "{message}");
                self.status = AuthUiStatus::Error(message);
            }
        }
    }

    pub fn refresh_account_token(&mut self, profile_id: &str) {
        if self.auth_busy() {
            return;
        }

        let Some(account) = self
            .accounts_state
            .accounts
            .iter()
            .find(|account| account.minecraft_profile.id == profile_id)
        else {
            return;
        };

        let has_refresh_token = account
            .microsoft_refresh_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        if !has_refresh_token {
            let message = format!(
                "Account '{}' has no renewable Microsoft refresh token.",
                account.minecraft_profile.name
            );
            self.failed_account_errors
                .insert(profile_id.to_owned(), message.clone());
            self.status = AuthUiStatus::Error(message);
            return;
        }

        let client_id = match microsoft_client_id() {
            Ok(client_id) => client_id,
            Err(err) => {
                self.status = AuthUiStatus::Error(err);
                return;
            }
        };

        self.spawn_single_account_renewal_worker(client_id, profile_id.to_owned());
    }

    fn ensure_active_account_token_ready(&mut self) {
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
            notification::warn!(
                "auth",
                "Active account has no renewable token; launching in offline mode."
            );
            self.status = AuthUiStatus::Idle;
            return;
        }

        let client_id = match microsoft_client_id() {
            Ok(client_id) => client_id,
            Err(err) => {
                notification::warn!(
                    "auth",
                    "Active account token is missing and renewal is unavailable: {err}. Launching in offline mode."
                );
                self.status = AuthUiStatus::Idle;
                return;
            }
        };

        self.spawn_renewal_worker(client_id, AuthUiStatus::RefreshingActiveSession);
    }

    pub fn should_request_repaint(&self) -> bool {
        self.flow.is_some() || self.renewal.is_some() || !self.avatar_loads_in_flight.is_empty()
    }

    pub fn sign_in_in_progress(&self) -> bool {
        self.flow.is_some() || self.device_code_flow.is_some()
    }

    pub fn set_streamer_mode(&mut self, enabled: bool) {
        self.streamer_mode = enabled;
    }

    pub fn auth_busy(&self) -> bool {
        self.flow.is_some() || self.device_code_flow.is_some() || self.renewal.is_some()
    }

    pub fn token_refresh_in_progress(&self) -> bool {
        self.renewal.is_some()
    }

    pub fn display_name(&self) -> Option<&str> {
        self.accounts_state
            .active_account()
            .map(|account| account.minecraft_profile.name.as_str())
    }

    pub fn active_account_owns_minecraft(&self) -> bool {
        self.accounts_state.active_account().is_some_and(|account| {
            !account.minecraft_profile.id.trim().is_empty()
                && !account.minecraft_profile.name.trim().is_empty()
        })
    }

    pub fn active_launch_context(&self) -> Option<LaunchAuthContext> {
        let account = self.accounts_state.active_account()?;
        let player_name = account.minecraft_profile.name.trim();
        let player_uuid = account.minecraft_profile.id.trim();
        if player_name.is_empty() || player_uuid.is_empty() {
            return None;
        }
        let access_token = account
            .minecraft_access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let user_type = account
            .user_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("msa")
            .to_owned();
        let xuid = account
            .xuid
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        Some(LaunchAuthContext {
            account_key: player_uuid.to_ascii_lowercase(),
            player_name: player_name.to_owned(),
            player_uuid: player_uuid.to_owned(),
            access_token,
            xuid,
            user_type,
        })
    }

    pub fn avatar_png(&self) -> Option<&[u8]> {
        self.active_avatar_png.as_deref()
    }

    pub fn account_avatars_by_key(&self) -> &HashMap<String, Vec<u8>> {
        &self.account_avatars
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status.status_message()
    }

    pub fn device_code_prompt(&self) -> Option<&DeviceCodePrompt> {
        self.device_code_prompt.as_ref()
    }

    pub fn cancel_device_code_sign_in(&mut self) {
        self.device_code_flow = None;
        self.device_code_prompt = None;
        self.device_code_expiry = None;
        self.status = AuthUiStatus::Idle;
    }

    fn restart_device_code_sign_in(&mut self) {
        self.status = AuthUiStatus::Starting;
        // client_id is ignored by the device-code flow worker; it uses its own credentials
        self.device_code_flow = Some(auth::start_device_code_login(""));
    }

    pub fn account_entries(&self) -> Vec<AccountUiEntry> {
        let active_id = self.accounts_state.active_profile_id.as_deref();

        let mut entries = self
            .accounts_state
            .accounts
            .iter()
            .map(|account| AccountUiEntry {
                profile_id: account.minecraft_profile.id.clone(),
                display_name: account.minecraft_profile.name.clone(),
                is_active: active_id
                    .map(|id| id == account.minecraft_profile.id)
                    .unwrap_or(false),
                is_failed: self
                    .failed_account_errors
                    .contains_key(&account.minecraft_profile.id),
            })
            .collect::<Vec<_>>();

        if let Some(active_pos) = entries.iter().position(|entry| entry.is_active) {
            if active_pos != 0 {
                let active = entries.remove(active_pos);
                entries.insert(0, active);
            }
        }

        entries
    }

    fn spawn_renewal_worker(&mut self, client_id: String, status: AuthUiStatus) {
        if self.renewal.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel::<RenewalResult>();
        let streamer_mode = self.streamer_mode;
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
                emit_cached_account_renewal_notification(event, streamer_mode);
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

    fn spawn_single_account_renewal_worker(&mut self, client_id: String, profile_id: String) {
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

    fn apply_accounts_state(&mut self, state: CachedAccountsState, preserve_active: bool) {
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

    fn retain_failed_accounts_for_known_profiles(&mut self) {
        let known_profile_ids = self
            .accounts_state
            .accounts
            .iter()
            .map(|account| account.minecraft_profile.id.as_str())
            .collect::<HashSet<_>>();
        self.failed_account_errors
            .retain(|profile_id, _| known_profile_ids.contains(profile_id.as_str()));
    }

    fn sync_account_avatar_cache(&mut self) {
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

    fn rebuild_active_avatar_png(&mut self) {
        self.active_avatar_png =
            active_avatar_from_map(&self.accounts_state, &self.account_avatars);
    }

    fn schedule_missing_avatars(&mut self) {
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

    fn poll_avatar_loads(&mut self) {
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
                        notification::warn!(
                            "auth/avatar",
                            "Failed to resolve account avatar in background worker: {error}"
                        );
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
                    if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
                        tracing::warn!(
                            target: "vertexlauncher/auth/avatar",
                            error = %err,
                            "Failed to persist background-loaded avatar."
                        );
                        notification::warn!(
                            "auth/avatar",
                            "Failed to persist background-loaded avatar: {err}"
                        );
                    }
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

fn decoded_cached_avatars(state: &CachedAccountsState) -> HashMap<String, Vec<u8>> {
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

fn active_avatar_from_map(
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

fn emit_cached_account_renewal_notification(event: CachedAccountRenewalEvent, streamer_mode: bool) {
    match event {
        CachedAccountRenewalEvent::Started {
            profile_id,
            display_name,
        } => {
            let display_name = privacy::redact_account_label(streamer_mode, &display_name);
            notification::emit_spinner(
                notification::Severity::Info,
                "Login Renewal",
                format!("Renewing Login Token for {display_name}"),
                renewal_replace_key(&profile_id),
            );
        }
        CachedAccountRenewalEvent::Succeeded {
            profile_id,
            display_name,
        } => {
            let display_name = privacy::redact_account_label(streamer_mode, &display_name);
            notification::emit_replace(
                notification::Severity::Info,
                "Login Renewal",
                format!("login token for {display_name} successful, you are ready to play!"),
                renewal_replace_key(&profile_id),
            );
        }
        CachedAccountRenewalEvent::Failed {
            profile_id,
            display_name,
            error,
        } => {
            let display_name_for_notification =
                privacy::redact_account_label(streamer_mode, &display_name);
            tracing::warn!(
                target: "vertexlauncher/auth/renew",
                profile_fingerprint = %webview_sign_in::fingerprint_for_log(&profile_id),
                error = %webview_sign_in::sanitize_message_for_log(&error),
                "Auto-renew login notification reported an error."
            );
            notification::emit_replace(
                notification::Severity::Error,
                "Login Renewal",
                format!(
                    "Error in attepting to renew login for {display_name_for_notification}: {error}"
                ),
                renewal_replace_key(&profile_id),
            );
        }
    }
}

fn renewal_replace_key(profile_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    profile_id.hash(&mut hasher);
    format!("login-renewal:{:016x}", hasher.finish())
}

fn emit_auth_flow_event(
    sender: &mpsc::Sender<AuthFlowEvent>,
    target: &'static str,
    event_name: &'static str,
    started_at: std::time::Instant,
    event: AuthFlowEvent,
) -> bool {
    if let Err(err) = sender.send(event) {
        match target {
            "vertexlauncher/auth/signin" => tracing::error!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                event = event_name,
                error = %err,
                "Failed to deliver auth flow event to UI."
            ),
            "vertexlauncher/auth/signin/browser" => tracing::error!(
                target: "vertexlauncher/auth/signin/browser",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                event = event_name,
                error = %err,
                "Failed to deliver auth flow event to UI."
            ),
            _ => tracing::error!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                event = event_name,
                error = %err,
                requested_target = target,
                "Failed to deliver auth flow event to UI."
            ),
        };
        return false;
    }
    true
}

fn run_sign_in_flow(client_id: String, sender: mpsc::Sender<AuthFlowEvent>) {
    let started_at = std::time::Instant::now();
    tracing::info!(
        target: "vertexlauncher/auth/signin",
        "Starting Microsoft sign-in flow."
    );

    let flow = match auth::login_begin(client_id) {
        Ok(flow) => flow,
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin",
                error = %webview_sign_in::sanitize_message_for_log(&err.to_string()),
                "Failed to initialize Microsoft sign-in flow."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin",
                "failed:init",
                started_at,
                AuthFlowEvent::Failed(err.to_string()),
            );
            return;
        }
    };

    tracing::info!(
        target: "vertexlauncher/auth/signin",
        auth_url = %webview_sign_in::sanitize_url_for_log(&flow.auth_request_uri),
        redirect_url = %webview_sign_in::sanitize_url_for_log(&flow.redirect_uri),
        expected_state = %webview_sign_in::fingerprint_for_log(flow.expected_state()),
        "Microsoft sign-in flow initialized; opening embedded webview."
    );

    if !emit_auth_flow_event(
        &sender,
        "vertexlauncher/auth/signin",
        "awaiting_browser",
        started_at,
        AuthFlowEvent::AwaitingBrowser,
    ) {
        return;
    }

    let auth_code = match webview_sign_in::open_microsoft_sign_in(
        &flow.auth_request_uri,
        &flow.redirect_uri,
        flow.expected_state(),
    ) {
        Ok(auth_code) => auth_code,
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                error = %webview_sign_in::sanitize_message_for_log(&err),
                "Microsoft sign-in webview failed before returning a callback URL."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin",
                "failed:webview",
                started_at,
                AuthFlowEvent::Failed(err),
            );
            return;
        }
    };

    tracing::info!(
        target: "vertexlauncher/auth/signin",
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "Microsoft sign-in webview returned an auth code; exchanging tokens."
    );

    if !emit_auth_flow_event(
        &sender,
        "vertexlauncher/auth/signin",
        "waiting_for_authorization",
        started_at,
        AuthFlowEvent::WaitingForAuthorization,
    ) {
        return;
    }

    match auth::login_finish(&auth_code, flow) {
        Ok(account) => {
            tracing::info!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "Microsoft sign-in flow completed successfully."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin",
                "completed",
                started_at,
                AuthFlowEvent::Completed(account),
            );
        }
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                error = %webview_sign_in::sanitize_message_for_log(&err.to_string()),
                "Microsoft sign-in callback exchange failed."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin",
                "failed:exchange",
                started_at,
                AuthFlowEvent::Failed(err.to_string()),
            );
        }
    }
}

fn run_system_browser_sign_in_flow(
    sender: mpsc::Sender<AuthFlowEvent>,
    colors: system_browser_sign_in::CallbackPageColors,
) {
    let started_at = std::time::Instant::now();
    tracing::info!(
        target: "vertexlauncher/auth/signin/browser",
        "Starting Microsoft system-browser sign-in flow."
    );

    let callback_listener = match system_browser_sign_in::prepare_loopback_callback_listener() {
        Ok(callback_listener) => callback_listener,
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin/browser",
                error = %webview_sign_in::sanitize_message_for_log(&err),
                "Failed to allocate localhost loopback redirect URI."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin/browser",
                "failed:prepare_listener",
                started_at,
                AuthFlowEvent::Failed(err),
            );
            return;
        }
    };
    let flow = match auth::login_begin_with_device_code_client_redirect_uri(
        callback_listener.redirect_uri().to_owned(),
    ) {
        Ok(flow) => flow,
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin/browser",
                error = %webview_sign_in::sanitize_message_for_log(&err.to_string()),
                "Failed to initialize Microsoft system-browser sign-in flow."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin/browser",
                "failed:init",
                started_at,
                AuthFlowEvent::Failed(err.to_string()),
            );
            return;
        }
    };

    tracing::info!(
        target: "vertexlauncher/auth/signin/browser",
        auth_url = %webview_sign_in::sanitize_url_for_log(&flow.auth_request_uri),
        redirect_url = %webview_sign_in::sanitize_url_for_log(&flow.redirect_uri),
        expected_state = %webview_sign_in::fingerprint_for_log(flow.expected_state()),
        "Microsoft system-browser sign-in flow initialized; opening default browser."
    );

    if !emit_auth_flow_event(
        &sender,
        "vertexlauncher/auth/signin/browser",
        "awaiting_external_browser",
        started_at,
        AuthFlowEvent::AwaitingExternalBrowser,
    ) {
        return;
    }

    let callback_url = match system_browser_sign_in::open_microsoft_sign_in(
        &flow.auth_request_uri,
        callback_listener,
        &colors,
    ) {
        Ok(callback_url) => callback_url,
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin/browser",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                error = %webview_sign_in::sanitize_message_for_log(&err),
                "Microsoft system-browser sign-in failed before returning a callback URL."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin/browser",
                "failed:browser",
                started_at,
                AuthFlowEvent::Failed(err),
            );
            return;
        }
    };

    if !emit_auth_flow_event(
        &sender,
        "vertexlauncher/auth/signin/browser",
        "waiting_for_authorization",
        started_at,
        AuthFlowEvent::WaitingForAuthorization,
    ) {
        return;
    }

    finish_browser_sign_in_flow(
        flow,
        callback_url,
        sender,
        started_at,
        "Microsoft system-browser callback received; exchanging tokens.",
    );
}

fn finish_browser_sign_in_flow(
    flow: auth::MinecraftLoginFlow,
    callback_url: String,
    sender: mpsc::Sender<AuthFlowEvent>,
    started_at: std::time::Instant,
    callback_message: &str,
) {
    tracing::info!(
        target: "vertexlauncher/auth/signin",
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        callback_url = %webview_sign_in::sanitize_url_for_log(&callback_url),
        "{}",
        callback_message
    );

    match auth::login_finish_from_redirect(&callback_url, flow) {
        Ok(account) => {
            tracing::info!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "Microsoft sign-in flow completed successfully."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin",
                "completed",
                started_at,
                AuthFlowEvent::Completed(account),
            );
        }
        Err(err) => {
            tracing::error!(
                target: "vertexlauncher/auth/signin",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                error = %webview_sign_in::sanitize_message_for_log(&err.to_string()),
                "Microsoft sign-in callback exchange failed."
            );
            let _ = emit_auth_flow_event(
                &sender,
                "vertexlauncher/auth/signin",
                "failed:exchange",
                started_at,
                AuthFlowEvent::Failed(err.to_string()),
            );
        }
    }
}

fn microsoft_client_id() -> Result<String, String> {
    let client_id = std::env::var("VERTEX_MSA_CLIENT_ID")
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
        .or_else(|| auth::builtin_client_id().map(str::to_owned))
        .ok_or_else(|| {
            "Microsoft OAuth client ID is not configured. Set VERTEX_MSA_CLIENT_ID or set \
auth::BUILTIN_MICROSOFT_CLIENT_ID in crates/auth/src/lib.rs."
                .to_owned()
        })?;

    if is_valid_microsoft_client_id(&client_id) {
        Ok(client_id)
    } else {
        Err(format!(
            "Invalid Microsoft client id '{client_id}'. Set VERTEX_MSA_CLIENT_ID to a valid \
16-character hex id or GUID application id.",
        ))
    }
}

fn is_valid_microsoft_client_id(value: &str) -> bool {
    is_hex_client_id(value) || is_guid_client_id(value)
}

fn is_hex_client_id(value: &str) -> bool {
    value.len() == 16 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_guid_client_id(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }

    for (index, ch) in value.chars().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if ch != '-' {
                return false;
            }
            continue;
        }

        if !ch.is_ascii_hexdigit() {
            return false;
        }
    }

    true
}

fn is_nonfatal_account_cache_error(error_text: &str) -> bool {
    let lowered = error_text.to_ascii_lowercase();
    let mentions_secure_storage = lowered.contains("secure storage")
        || lowered.contains("platform secure storage")
        || lowered.contains("couldn't access platform secure storage")
        || lowered.contains("secret service")
        || lowered.contains("org/freedesktop/secrets");
    let likely_session_bus_issue = lowered.contains("can't find session")
        || lowered.contains("dbus error")
        || lowered.contains("no such object path")
        || lowered.contains("no such interface")
        || lowered.contains("windows error_no_such_logon_session");
    mentions_secure_storage && likely_session_bus_issue
}

fn is_http_auth_error(error_text: &str) -> bool {
    let lowered = error_text.to_ascii_lowercase();
    lowered.contains("http status")
        || lowered.contains("http request failed")
        || lowered.contains("transport")
        || lowered.contains("connection")
}

#[cfg(test)]
mod tests {
    use super::is_nonfatal_account_cache_error;

    #[test]
    fn marks_secure_storage_dbus_session_failures_as_nonfatal() {
        let message = "Sign-in succeeded, but failed to cache account state: Secure storage error: \
Failed to store cached accounts state in secure storage: Platform secure storage failure: DBus \
error: Can't find session /org/freedesktop/secrets/session/654";
        assert!(is_nonfatal_account_cache_error(message));
    }

    #[test]
    fn marks_secret_service_object_path_failures_as_nonfatal() {
        let message = "Failed to cache account state: Platform secure storage failure: Secret \
Service response error: No such object path '/org/freedesktop/secrets/collection/login'";
        assert!(is_nonfatal_account_cache_error(message));
    }

    #[test]
    fn marks_windows_secure_storage_session_failures_as_nonfatal() {
        let message = "Sign-in succeeded, but failed to cache account state: Secure storage error: \
Failed to store cached accounts state in secure storage: Couldn't access platform secure storage: \
Windows ERROR_NO_SUCH_LOGON_SESSION";
        assert!(is_nonfatal_account_cache_error(message));
    }

    #[test]
    fn keeps_non_secure_storage_cache_errors_fatal() {
        let message = "Sign-in succeeded, but failed to cache account state: Permission denied";
        assert!(!is_nonfatal_account_cache_error(message));
    }
}
