use auth::{CachedAccount, CachedAccountRenewalEvent, CachedAccountsState};
use launcher_ui::notification;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::webview_sign_in;

pub const REPAINT_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug)]
pub enum AuthUiStatus {
    Idle,
    RefreshingCachedSession,
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
    active_avatar_png: Option<Vec<u8>>,
    flow: Option<Receiver<AuthFlowEvent>>,
    startup_renewal: Option<Receiver<Result<CachedAccountsState, String>>>,
    status: AuthUiStatus,
}

impl AuthState {
    pub fn load() -> Self {
        let (accounts_state, mut status) = match auth::load_cached_accounts() {
            Ok(state) => (state, AuthUiStatus::Idle),
            Err(err) => (
                CachedAccountsState::default(),
                AuthUiStatus::Error(format!("Failed to load cached account state: {err}")),
            ),
        };
        let mut startup_renewal = None;
        if !accounts_state.accounts.is_empty() {
            match microsoft_client_id() {
                Ok(client_id) => {
                    let (tx, rx) = mpsc::channel::<Result<CachedAccountsState, String>>();
                    std::thread::spawn(move || {
                        let result =
                            auth::renew_cached_accounts_tokens_with_callback(&client_id, |event| {
                                emit_cached_account_renewal_notification(event);
                            })
                            .map_err(|err| err.to_string());
                        let _ = tx.send(result);
                    });
                    startup_renewal = Some(rx);
                    status = AuthUiStatus::RefreshingCachedSession;
                }
                Err(err) => {
                    status = AuthUiStatus::Error(format!(
                        "Loaded cached accounts, but token renewal was skipped: {err}"
                    ));
                }
            }
        }

        let active_avatar_png = accounts_state
            .active_account()
            .and_then(CachedAccount::avatar_png_bytes);

        Self {
            accounts_state,
            active_avatar_png,
            flow: None,
            startup_renewal,
            status,
        }
    }

    pub fn poll(&mut self) {
        if let Some(startup_renewal) = self.startup_renewal.as_mut() {
            match startup_renewal.try_recv() {
                Ok(Ok(renewed)) => {
                    let previous_active = self.accounts_state.active_profile_id.clone();
                    self.accounts_state = renewed;
                    if let Some(active_id) = previous_active.as_deref() {
                        let _ = self.accounts_state.set_active_profile_id(active_id);
                    }
                    self.active_avatar_png = self
                        .accounts_state
                        .active_account()
                        .and_then(CachedAccount::avatar_png_bytes);
                    self.status = AuthUiStatus::Idle;
                    self.startup_renewal = None;
                }
                Ok(Err(err)) => {
                    self.status = AuthUiStatus::Error(format!(
                        "Loaded cached accounts, but token renewal failed: {err}"
                    ));
                    self.startup_renewal = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = AuthUiStatus::Error(
                        "Loaded cached accounts, but token renewal worker stopped unexpectedly."
                            .to_owned(),
                    );
                    self.startup_renewal = None;
                }
            }
        }

        let mut flow_finished = false;

        if let Some(flow) = self.flow.as_mut() {
            loop {
                match flow.try_recv() {
                    Ok(event) => match event {
                        AuthFlowEvent::AwaitingBrowser => {
                            self.status = AuthUiStatus::AwaitingBrowser;
                        }
                        AuthFlowEvent::WaitingForAuthorization => {
                            self.status = AuthUiStatus::WaitingForAuthorization;
                        }
                        AuthFlowEvent::Completed(account) => {
                            self.accounts_state.upsert_and_activate(account);
                            self.active_avatar_png = self
                                .accounts_state
                                .active_account()
                                .and_then(CachedAccount::avatar_png_bytes);
                            self.status = AuthUiStatus::Idle;

                            if let Err(err) = auth::save_cached_accounts(&self.accounts_state) {
                                self.status = AuthUiStatus::Error(format!(
                                    "Sign-in succeeded, but failed to cache account state: {err}",
                                ));
                            }

                            flow_finished = true;
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
        }

        if flow_finished {
            self.flow = None;
        }
    }

    pub fn start_sign_in(&mut self) {
        if self.flow.is_some() {
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
        self.active_avatar_png = self
            .accounts_state
            .active_account()
            .and_then(CachedAccount::avatar_png_bytes);
        self.status = AuthUiStatus::Idle;

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

        self.active_avatar_png = self
            .accounts_state
            .active_account()
            .and_then(CachedAccount::avatar_png_bytes);
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

        match auth::renew_cached_accounts_tokens_with_callback(&client_id, |event| {
            emit_cached_account_renewal_notification(event);
        }) {
            Ok(renewed) => {
                let previous_active = self.accounts_state.active_profile_id.clone();
                self.accounts_state = renewed;
                if let Some(active_id) = previous_active.as_deref() {
                    let _ = self.accounts_state.set_active_profile_id(active_id);
                }
                let refreshed_has_token = self
                    .accounts_state
                    .active_account()
                    .and_then(|account| account.minecraft_access_token.as_deref())
                    .map(str::trim)
                    .is_some_and(|token| !token.is_empty());
                if !refreshed_has_token {
                    self.status = AuthUiStatus::Error(
                        "Active account does not have a renewable session. Sign in again."
                            .to_owned(),
                    );
                }
            }
            Err(err) => {
                self.status =
                    AuthUiStatus::Error(format!("Failed to renew active account token: {err}",));
            }
        }
    }

    pub fn should_request_repaint(&self) -> bool {
        self.flow.is_some() || self.startup_renewal.is_some()
    }

    pub fn sign_in_in_progress(&self) -> bool {
        self.flow.is_some()
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
                avatar_png: account.avatar_png_bytes(),
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
}

fn emit_cached_account_renewal_notification(event: CachedAccountRenewalEvent) {
    match event {
        CachedAccountRenewalEvent::Started {
            profile_id,
            display_name,
        } => {
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
                format!("Error in attepting to renew login for {display_name}"),
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
    ) {
        Ok(callback_url) => callback_url,
        Err(err) => {
            let _ = sender.send(AuthFlowEvent::Failed(err));
            return;
        }
    };

    let _ = sender.send(AuthFlowEvent::WaitingForAuthorization);

    match auth::login_finish_from_redirect(&callback_url, flow) {
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
