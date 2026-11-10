use vertex_i18n::tr;
use auth::{
    CachedAccount, CachedAccountRenewalEvent, CachedAccountsState, DeviceCodeLoginFlow,
    DeviceCodePrompt, LoginEvent,
};
use launcher_runtime as tokio_runtime;
use launcher_ui::notification;
use std::collections::{HashMap, HashSet};
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
#[path = "auth_state/avatars.rs"]
mod avatars;
#[path = "auth_state/client_id.rs"]
mod client_id;
#[path = "auth_state/error_classification.rs"]
mod error_classification;
#[path = "auth_state/launch_auth_context.rs"]
mod launch_auth_context;
#[path = "auth_state/renewal.rs"]
mod renewal;
#[path = "auth_state/renewal_notifications.rs"]
mod renewal_notifications;
#[path = "auth_state/renewal_result.rs"]
mod renewal_result;
#[path = "auth_state/sign_in_polling.rs"]
mod sign_in_polling;
#[path = "auth_state/sign_in_workers.rs"]
mod sign_in_workers;

pub use self::account_ui_entry::AccountUiEntry;
use self::auth_flow_event::AuthFlowEvent;
use self::auth_ui_status::AuthUiStatus;
use self::avatar_load_result::AvatarLoadResult;
use self::avatars::{active_avatar_from_map, decoded_cached_avatars};
use self::client_id::microsoft_client_id;
use self::error_classification::{is_http_auth_error, is_nonfatal_account_cache_error};
pub use self::launch_auth_context::LaunchAuthContext;
use self::renewal_notifications::emit_cached_account_renewal_notification;
use self::renewal_result::RenewalResult;
use self::sign_in_workers::{run_sign_in_flow, run_system_browser_sign_in_flow};

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
    status: AuthUiStatus,
    /// The saved accounts are being read on a worker (the OS keyring can be slow); `poll` swaps them in.
    pending_load: Option<Receiver<Box<AuthState>>>,
    /// Failures of background cache writes, as messages for `poll` to report.
    cache_save_tx: Sender<String>,
    cache_save_rx: Receiver<String>,
    /// "Lazy Account Refresh": nothing is renewed at startup; the active account is renewed when something needs it.
    lazy_refresh: bool,
    /// Profile ids whose renewal has finished this session (successfully or not), so each is renewed once.
    refreshed: HashSet<String>,
}

impl AuthState {
    /// Starts with no accounts and reads the saved ones on a worker, so a slow or stuck keyring never holds up the
    /// window. `poll` swaps them in; until then the account list is empty and sign-in actions wait.
    pub fn load(lazy_refresh: bool) -> Self {
        let mut state = Self::empty();
        let (tx, rx) = mpsc::channel();
        state.pending_load = Some(rx);
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            let _ = tx.send(Box::new(Self::load_blocking(lazy_refresh)));
        });
        state
    }

    fn empty() -> Self {
        let (avatar_result_tx, avatar_result_rx) = mpsc::channel();
        let (cache_save_tx, cache_save_rx) = mpsc::channel();
        Self {
            accounts_state: CachedAccountsState::default(),
            account_avatars: HashMap::new(),
            failed_account_errors: HashMap::new(),
            avatar_result_tx,
            avatar_result_rx,
            avatar_loads_in_flight: HashSet::new(),
            active_avatar_png: None,
            flow: None,
            device_code_flow: None,
            device_code_prompt: None,
            device_code_expiry: None,
            renewal: None,
            status: AuthUiStatus::Idle,
            pending_load: None,
            cache_save_tx,
            cache_save_rx,
            lazy_refresh: false,
            refreshed: HashSet::new(),
        }
    }

    /// Writes the account cache on a worker (it goes through the OS keyring, which can stall). Writes go one at a
    /// time and a newer request makes an older one that has not started yet skip itself. A failure comes back
    /// through `cache_save_rx` as `"{failure}: {error}"`.
    pub(super) fn save_accounts_in_background(&self, failure: &'static str) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        static WRITTEN: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed) + 1;
        let state = self.accounts_state.clone();
        let tx = self.cache_save_tx.clone();
        let _ = tokio_runtime::spawn_blocking_detached(move || {
            let mut written = WRITTEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if sequence < *written {
                return;
            }
            *written = sequence;
            if let Err(err) = auth::save_cached_accounts(&state) {
                tracing::warn!(
                    target: "vertexlauncher/auth_state",
                    error = %err,
                    "Persisting cached account state failed."
                );
                let _ = tx.send(tr!("{failure}: {err}", failure = failure, err = err).into_owned());
            }
        });
    }

    fn load_blocking(lazy_refresh: bool) -> Self {
        let (avatar_result_tx, avatar_result_rx) = mpsc::channel();
        let (cache_save_tx, cache_save_rx) = mpsc::channel();
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
                    AuthUiStatus::Error(tr!("Failed to load cached account state: {err}", err = err).into_owned()),
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
            status,
            pending_load: None,
            cache_save_tx,
            cache_save_rx,
            lazy_refresh,
            refreshed: HashSet::new(),
        };

        // Lazy mode renews nothing here: the active account is renewed when something needs it.
        if !lazy_refresh && !auth_state.accounts_state.accounts.is_empty() {
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
                    auth_state.status = AuthUiStatus::Error(tr!("Loaded cached accounts, but token renewal was skipped: {err}", err = err).into_owned());
                }
            }
        }

        auth_state.schedule_missing_avatars();
        auth_state
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.pending_load {
            match rx.try_recv() {
                Ok(loaded) => *self = *loaded,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.pending_load = None,
            }
        }
        while let Ok(message) = self.cache_save_rx.try_recv() {
            self.report_cache_save_failure(message);
        }
        self.poll_avatar_loads();
        self.poll_renewal();
        self.poll_interactive_flow();
        self.poll_device_code_flow();
    }

    pub fn start_webview_sign_in(&mut self) {
        if self.pending_load.is_some() || self.flow.is_some() || self.device_code_flow.is_some() || self.renewal.is_some() {
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
        if self.pending_load.is_some() || self.flow.is_some() || self.device_code_flow.is_some() || self.renewal.is_some() {
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

    pub fn start_system_browser_sign_in(&mut self, theme: &vertex_theme::Theme) {
        if self.pending_load.is_some() || self.flow.is_some() || self.renewal.is_some() {
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
        if self.pending_load.is_some() || !self.accounts_state.set_active_profile_id(profile_id) {
            return;
        }

        // Lazy mode renews the account when something needs it instead of the moment it is picked.
        if !self.lazy_refresh {
            self.ensure_active_account_token_ready();
        }
        self.rebuild_active_avatar_png();
        if !self.auth_busy() && !self.failed_account_errors.contains_key(profile_id) {
            self.status = AuthUiStatus::Idle;
        }

        self.save_accounts_in_background("Switched account in memory, but failed to cache account state");
    }

    pub fn remove_account(&mut self, profile_id: &str) {
        if self.pending_load.is_some() || !self.accounts_state.remove_by_profile_id(profile_id) {
            return;
        }

        self.failed_account_errors.remove(profile_id);
        self.sync_account_avatar_cache();
        self.rebuild_active_avatar_png();
        self.status = AuthUiStatus::Idle;

        self.save_accounts_in_background("Removed account in memory, but failed to cache account state");
    }

    pub fn refresh_account_token(&mut self, profile_id: &str) {
        if self.pending_load.is_some() || self.auth_busy() {
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
            let message = tr!("Account '{name}' has no renewable Microsoft refresh token.", name = account.minecraft_profile.name).into_owned();
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

    /// Reports a failure to persist the account cache: non-fatal secure-storage/session-bus
    /// problems only warn, everything else becomes an error status.
    pub(super) fn report_cache_save_failure(&mut self, message: String) {
        if is_nonfatal_account_cache_error(message.as_str()) {
            notification::warn!("auth", "{message}", message = message);
        } else {
            notification::error!("auth", "{message}", message = message);
            self.status = AuthUiStatus::Error(message);
        }
    }

    pub fn sign_in_in_progress(&self) -> bool {
        self.flow.is_some() || self.device_code_flow.is_some()
    }

    pub fn auth_busy(&self) -> bool {
        self.flow.is_some() || self.device_code_flow.is_some() || self.renewal.is_some()
    }

    /// Follows the "Lazy Account Refresh" setting.
    pub fn set_lazy_refresh(&mut self, enabled: bool) {
        self.lazy_refresh = enabled;
    }

    /// Whether the active account can be used for something that needs a signed-in session (launching, the skin
    /// preview). Always true unless "Lazy Account Refresh" is on. When it is on and the active account has not been
    /// renewed yet this session, this starts renewing that one account (never the others) and returns `false`; call it
    /// again later (every frame is fine) until it returns `true`. The attempt counts even when it fails, so a failure
    /// falls back to the cached session instead of retrying forever.
    pub fn ensure_active_fresh(&mut self) -> bool {
        if !self.lazy_refresh {
            return true;
        }
        if self.pending_load.is_some() {
            return false;
        }
        let Some(profile_id) = self.active_profile_needing_refresh() else {
            // Nothing to renew; just let a renewal that is already running (a manual one) finish first.
            return self.renewal.is_none();
        };
        if self.renewal.is_some() || self.flow.is_some() || self.device_code_flow.is_some() {
            return false;
        }
        match microsoft_client_id() {
            Ok(client_id) => self.spawn_single_account_renewal_worker(client_id, profile_id),
            Err(err) => {
                notification::warn!("auth", "Could not renew the active account before use: {err}. Continuing with the cached session.", err = err);
                self.refreshed.insert(profile_id);
                return true;
            }
        }
        false
    }

    /// Whether the active account is still waiting for its first renewal of the session (lazy mode only).
    pub fn active_refresh_pending(&self) -> bool {
        self.lazy_refresh && (self.pending_load.is_some() || self.active_profile_needing_refresh().is_some())
    }

    /// The active account's profile id when it has a refresh token and has not been renewed this session.
    fn active_profile_needing_refresh(&self) -> Option<String> {
        let account = self.accounts_state.active_account()?;
        let id = account.minecraft_profile.id.as_str();
        let renewable = account
            .microsoft_refresh_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        (renewable && !self.refreshed.contains(id)).then(|| id.to_owned())
    }

    pub fn token_refresh_in_progress(&self) -> bool {
        self.renewal.is_some()
    }

    /// The Minecraft name of every saved account, for hiding them all in streamer mode.
    pub fn account_names(&self) -> impl Iterator<Item = &str> {
        self.accounts_state
            .accounts
            .iter()
            .map(|account| account.minecraft_profile.name.as_str())
    }

    /// (profile id, Minecraft name) of every saved account, so a hidden name can still be drawn in its account's colour.
    pub fn account_identities(&self) -> impl Iterator<Item = (&str, &str)> {
        self.accounts_state.accounts.iter().map(|account| {
            (
                account.minecraft_profile.id.as_str(),
                account.minecraft_profile.name.as_str(),
            )
        })
    }

    pub fn display_name(&self) -> Option<&str> {
        self.accounts_state
            .active_account()
            .map(|account| account.minecraft_profile.name.as_str())
    }

    /// Lowercased profile id of the active account, matching `LaunchAuthContext::account_key`.
    pub fn active_account_key(&self) -> Option<String> {
        let account = self.accounts_state.active_account()?;
        let player_name = account.minecraft_profile.name.trim();
        let player_uuid = account.minecraft_profile.id.trim();
        if player_name.is_empty() || player_uuid.is_empty() {
            return None;
        }
        Some(player_uuid.to_ascii_lowercase())
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

    /// Where the active account's skin can be downloaded from, if known.
    pub fn active_skin_url(&self) -> Option<String> {
        self.accounts_state
            .active_account()?
            .avatar_source_skin_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
    }

    pub fn avatar_png(&self) -> Option<&[u8]> {
        self.active_avatar_png.as_deref()
    }

    pub fn account_avatars_by_key(&self) -> &HashMap<String, Vec<u8>> {
        &self.account_avatars
    }

    pub fn status_message(&self) -> Option<std::borrow::Cow<'_, str>> {
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
                skin_url: account
                    .avatar_source_skin_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned),
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

#[cfg(test)]
mod tests {
    use super::{AuthState, is_nonfatal_account_cache_error};

    fn state_with_accounts(ids: &[&str], lazy: bool) -> AuthState {
        let mut state = AuthState::empty();
        for id in ids {
            let account: auth::CachedAccount = serde_json::from_value(serde_json::json!({
                "minecraft_profile": { "id": id, "name": format!("name-{id}") },
                "microsoft_refresh_token": "refresh",
                "cached_at_unix_secs": 1,
            }))
            .expect("account json");
            state.accounts_state.upsert_and_activate(account);
        }
        state.lazy_refresh = lazy;
        state
    }

    #[test]
    fn eager_mode_never_waits_for_a_refresh() {
        let mut state = state_with_accounts(&["a"], false);
        assert!(state.ensure_active_fresh());
        assert!(!state.active_refresh_pending());
        assert!(state.renewal.is_none());
    }

    #[test]
    fn lazy_mode_wants_only_the_active_account_and_only_once() {
        let mut state = state_with_accounts(&["a", "b"], true);
        // `b` was added last, so it is the active one; `a` is never asked about.
        assert_eq!(state.active_profile_needing_refresh().as_deref(), Some("b"));
        assert!(state.active_refresh_pending());

        state.refreshed.insert("b".to_owned());
        assert_eq!(state.active_profile_needing_refresh(), None);
        assert!(state.ensure_active_fresh());

        // Switching to the other account makes that one pending, the first stays done.
        assert!(state.accounts_state.set_active_profile_id("a"));
        assert_eq!(state.active_profile_needing_refresh().as_deref(), Some("a"));
    }

    #[test]
    fn lazy_mode_without_accounts_has_nothing_to_wait_for() {
        let mut state = state_with_accounts(&[], true);
        assert!(state.ensure_active_fresh());
        assert!(!state.active_refresh_pending());
    }

    #[test]
    fn lazy_mode_waits_while_accounts_are_still_loading() {
        let mut state = state_with_accounts(&[], true);
        let (_tx, rx) = std::sync::mpsc::channel();
        state.pending_load = Some(rx);
        assert!(!state.ensure_active_fresh());
        assert!(state.active_refresh_pending());
    }

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
