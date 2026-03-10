use auth::{CachedAccount, CachedAccountRenewalEvent, CachedAccountsState};
use launcher_ui::{notification, privacy};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use super::webview_sign_in;

pub const REPAINT_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug)]
pub enum AuthUiStatus {
    Idle,
    RefreshingCachedSession,
    RefreshingActiveSession,
    Starting,
    AwaitingBrowser,
    WaitingForAuthorization,
    Error(String),
}

impl AuthUiStatus {
    fn status_message(&self) -> Option<&str> {
        match self {
            AuthUiStatus::Idle => None,
            AuthUiStatus::RefreshingCachedSession => Some("Refreshing cached account session..."),
            AuthUiStatus::RefreshingActiveSession => Some("Refreshing account token..."),
            AuthUiStatus::Starting => Some("Preparing Microsoft sign-in..."),
            AuthUiStatus::AwaitingBrowser => {
                Some("Complete sign-in in the Microsoft webview window...")
            }
            AuthUiStatus::WaitingForAuthorization => Some("Finalizing sign-in..."),
            AuthUiStatus::Error(message) => Some(message.as_str()),
        }
    }
}

enum AuthFlowEvent {
    AwaitingBrowser,
    WaitingForAuthorization,
    Completed(CachedAccount),
    Failed(String),
}

struct AvatarLoadResult {
    profile_id: String,
    avatar_png: Option<Vec<u8>>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AccountUiEntry {
    pub profile_id: String,
    pub display_name: String,
    pub is_active: bool,
    pub avatar_png: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct LaunchAuthContext {
    pub account_key: String,
    pub player_name: String,
    pub player_uuid: String,
    pub access_token: String,
    pub xuid: Option<String>,
    pub user_type: String,
}

pub struct AuthState {
    accounts_state: CachedAccountsState,
    account_avatars: HashMap<String, Vec<u8>>,
    avatar_result_tx: Sender<AvatarLoadResult>,
    avatar_result_rx: Receiver<AvatarLoadResult>,
    avatar_loads_in_flight: HashSet<String>,
    active_avatar_png: Option<Vec<u8>>,
    flow: Option<Receiver<AuthFlowEvent>>,
    renewal: Option<Receiver<Result<CachedAccountsState, String>>>,
    streamer_mode: bool,
    status: AuthUiStatus,
}

impl AuthState {
    pub fn load() -> Self {
        let (avatar_result_tx, avatar_result_rx) = mpsc::channel();
        let (accounts_state, status) = match auth::load_cached_accounts() {
            Ok(state) => (state, AuthUiStatus::Idle),
            Err(err) => (
                CachedAccountsState::default(),
                AuthUiStatus::Error(format!("Failed to load cached account state: {err}")),
            ),
        };
        let account_avatars = decoded_cached_avatars(&accounts_state);
        let active_avatar_png = active_avatar_from_map(&accounts_state, &account_avatars);

        let mut auth_state = Self {
            accounts_state,
            account_avatars,
            avatar_result_tx,
            avatar_result_rx,
            avatar_loads_in_flight: HashSet::new(),
            active_avatar_png,
            flow: None,
            renewal: None,
            streamer_mode: false,
            status,
        };

        if !auth_state.accounts_state.accounts.is_empty() {
            match microsoft_client_id() {
                Ok(client_id) => {
                    auth_state
                        .spawn_renewal_worker(client_id, AuthUiStatus::RefreshingCachedSession);
                }
                Err(err) => {
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
                Ok(Ok(renewed)) => {
                    let active_refresh =
                        matches!(self.status, AuthUiStatus::RefreshingActiveSession);
                    self.apply_accounts_state(renewed, true);
                    let refreshed_has_token = self
                        .accounts_state
                        .active_account()
                        .and_then(|account| account.minecraft_access_token.as_deref())
                        .map(str::trim)
                        .is_some_and(|token| !token.is_empty());
                    self.status = if active_refresh && !refreshed_has_token {
                        AuthUiStatus::Error(
                            "Active account does not have a renewable session. Sign in again."
                                .to_owned(),
                        )
                    } else {
                        AuthUiStatus::Idle
                    };
                    self.renewal = None;
                    self.schedule_missing_avatars();
                }
                Ok(Err(err)) => {
                    let prefix = if matches!(self.status, AuthUiStatus::RefreshingActiveSession) {
                        "Failed to renew active account token"
                    } else {
                        "Loaded cached accounts, but token renewal failed"
                    };
                    self.status = AuthUiStatus::Error(format!("{prefix}: {err}"));
                    self.renewal = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
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
                let flow = self.flow.as_mut().expect("flow existence already checked");
                flow.try_recv()
            };
            match next_event {
                Ok(event) => match event {
                    AuthFlowEvent::AwaitingBrowser => {
                        self.status = AuthUiStatus::AwaitingBrowser;
                    }
                    AuthFlowEvent::WaitingForAuthorization => {
                        self.status = AuthUiStatus::WaitingForAuthorization;
                    }
                    AuthFlowEvent::Completed(account) => {
                        self.accounts_state.upsert_and_activate(account);
                        self.sync_account_avatar_cache();
                        self.rebuild_active_avatar_png();
                        self.status = AuthUiStatus::Idle;

                        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
                            self.status = AuthUiStatus::Error(format!(
                                "Sign-in succeeded, but failed to cache account state: {err}",
                            ));
                        }

                        flow_finished = true;
                        self.schedule_missing_avatars();
                    }
                    AuthFlowEvent::Failed(err) => {
                        self.status = AuthUiStatus::Error(err);
                        flow_finished = true;
                    }
                },
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !flow_finished {
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
    }

    pub fn start_sign_in(&mut self) {
        if self.flow.is_some() || self.renewal.is_some() {
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
        std::thread::spawn(move || {
            run_sign_in_flow(client_id, sender);
        });

        self.flow = Some(receiver);
    }

    pub fn select_account(&mut self, profile_id: &str) {
        if !self.accounts_state.set_active_profile_id(profile_id) {
            return;
        }

        self.ensure_active_account_token_ready();
        self.rebuild_active_avatar_png();
        if !self.auth_busy() && !matches!(self.status, AuthUiStatus::Error(_)) {
            self.status = AuthUiStatus::Idle;
        }

        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
            self.status = AuthUiStatus::Error(format!(
                "Switched account in memory, but failed to cache account state: {err}",
            ));
        }
    }

    pub fn remove_account(&mut self, profile_id: &str) {
        if !self.accounts_state.remove_by_profile_id(profile_id) {
            return;
        }

        self.sync_account_avatar_cache();
        self.rebuild_active_avatar_png();
        self.status = AuthUiStatus::Idle;

        if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
            self.status = AuthUiStatus::Error(format!(
                "Removed account in memory, but failed to cache account state: {err}",
            ));
        }
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

        let client_id = match microsoft_client_id() {
            Ok(client_id) => client_id,
            Err(err) => {
                self.status = AuthUiStatus::Error(format!(
                    "Active account token is missing and renewal is unavailable: {err}",
                ));
                return;
            }
        };

        self.spawn_renewal_worker(client_id, AuthUiStatus::RefreshingActiveSession);
    }

    pub fn should_request_repaint(&self) -> bool {
        self.flow.is_some() || self.renewal.is_some() || !self.avatar_loads_in_flight.is_empty()
    }

    pub fn sign_in_in_progress(&self) -> bool {
        self.flow.is_some()
    }

    pub fn set_streamer_mode(&mut self, enabled: bool) {
        self.streamer_mode = enabled;
    }

    pub fn auth_busy(&self) -> bool {
        self.flow.is_some() || self.renewal.is_some()
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
        self.active_launch_context().is_some()
    }

    pub fn active_launch_context(&self) -> Option<LaunchAuthContext> {
        let account = self.accounts_state.active_account()?;
        let access_token = account
            .minecraft_access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned();
        let player_name = account.minecraft_profile.name.trim();
        let player_uuid = account.minecraft_profile.id.trim();
        if player_name.is_empty() || player_uuid.is_empty() {
            return None;
        }
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

    pub fn status_message(&self) -> Option<&str> {
        self.status.status_message()
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
                avatar_png: self
                    .account_avatars
                    .get(&account.minecraft_profile.id)
                    .cloned(),
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

        let (tx, rx) = mpsc::channel::<Result<CachedAccountsState, String>>();
        let streamer_mode = self.streamer_mode;
        std::thread::spawn(move || {
            let result = auth::renew_cached_accounts_tokens_with_callback(&client_id, |event| {
                emit_cached_account_renewal_notification(event, streamer_mode);
            })
            .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        self.renewal = Some(rx);
        self.status = status;
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
        self.rebuild_active_avatar_png();
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
                    .insert(account.minecraft_profile.id.clone(), bytes);
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
            if profile_id.trim().is_empty()
                || self.account_avatars.contains_key(&profile_id)
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
            std::thread::spawn(move || {
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
                let _ = tx.send(result);
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
                            .insert(result.profile_id.clone(), avatar_png);
                    }

                    self.rebuild_active_avatar_png();
                    if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
                        tracing::warn!(
                            target: "vertexlauncher/auth/avatar",
                            error = %err,
                            "Failed to persist background-loaded avatar."
                        );
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
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
                .map(|bytes| (account.minecraft_profile.id.clone(), bytes))
        })
        .collect()
}

fn active_avatar_from_map(
    state: &CachedAccountsState,
    avatars: &HashMap<String, Vec<u8>>,
) -> Option<Vec<u8>> {
    let profile_id = state.active_account()?.minecraft_profile.id.as_str();
    avatars.get(profile_id).cloned()
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
                format!("login-renewal:{profile_id}"),
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
                format!("login-renewal:{profile_id}"),
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
                profile_id,
                display_name,
                error,
                "Auto-renew login notification reported an error."
            );
            notification::emit_replace(
                notification::Severity::Error,
                "Login Renewal",
                format!("Error in attepting to renew login for {display_name_for_notification}"),
                format!("login-renewal:{profile_id}"),
            );
        }
    }
}

fn run_sign_in_flow(client_id: String, sender: mpsc::Sender<AuthFlowEvent>) {
    let flow = match auth::login_begin(client_id) {
        Ok(flow) => flow,
        Err(err) => {
            let _ = sender.send(AuthFlowEvent::Failed(err.to_string()));
            return;
        }
    };

    let _ = sender.send(AuthFlowEvent::AwaitingBrowser);

    let callback_url = match webview_sign_in::open_microsoft_sign_in(
        &flow.auth_request_uri,
        auth::oauth_redirect_uri(),
        flow.expected_state(),
    ) {
        Ok(auth_code) => auth_code,
        Err(err) => {
            let _ = sender.send(AuthFlowEvent::Failed(err));
            return;
        }
    };

    let _ = sender.send(AuthFlowEvent::WaitingForAuthorization);

    match auth::login_finish(&callback_url, flow) {
        Ok(account) => {
            let _ = sender.send(AuthFlowEvent::Completed(account));
        }
        Err(err) => {
            let _ = sender.send(AuthFlowEvent::Failed(err.to_string()));
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
