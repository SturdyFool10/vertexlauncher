//! Authentication API for Microsoft -> Xbox -> Minecraft account login flows.

mod cache;
mod constants;
mod device_code;
mod error;
mod minecraft;
mod oauth;
mod runtime;
mod secret_store;
mod types;
mod util;

use std::sync::mpsc;

use tokio::runtime::Handle;

pub use constants::{BUILTIN_MICROSOFT_CLIENT_ID, BUILTIN_MICROSOFT_TENANT};
pub use error::AuthError;
pub use types::{
    CachedAccount, CachedAccountsState, DeviceCodeLoginFlow, DeviceCodePrompt, LoginEvent,
    MinecraftCapeState, MinecraftLoginFlow, MinecraftProfileState, MinecraftSkinState,
    MinecraftSkinVariant,
};

#[derive(Debug, Clone)]
pub enum CachedAccountRenewalEvent {
    Started {
        profile_id: String,
        display_name: String,
    },
    Succeeded {
        profile_id: String,
        display_name: String,
    },
    Failed {
        profile_id: String,
        display_name: String,
        error: String,
    },
}

/// Returns the built-in Microsoft OAuth client id if configured.
///
/// Empty compile-time values are treated as missing and return `None`.
pub fn builtin_client_id() -> Option<&'static str> {
    let value = BUILTIN_MICROSOFT_CLIENT_ID.trim();
    if value.is_empty() { None } else { Some(value) }
}

/// Returns the redirect URI expected for browser/device OAuth callbacks.
pub fn oauth_redirect_uri() -> &'static str {
    constants::LIVE_REDIRECT_URI
}

/// Extracts and validates an OAuth auth code from the Microsoft callback URL.
pub fn validate_oauth_callback_code(
    callback_url: &str,
    expected_state: &str,
) -> Result<String, AuthError> {
    oauth::extract_authorization_code(callback_url, expected_state)
}

/// Starts the device-code login flow on the current Tokio runtime if available.
///
/// Falls back to the internal auth runtime when called from non-runtime threads.
pub fn start_device_code_login(client_id: impl Into<String>) -> DeviceCodeLoginFlow {
    tracing::debug!(
        target: "vertexlauncher/auth",
        "starting device-code login flow"
    );
    if let Ok(handle) = Handle::try_current() {
        return start_device_code_login_with_handle(client_id, &handle);
    }

    start_device_code_login_with_handle(client_id, runtime::auth_runtime_handle())
}

/// Starts device-code login flow using the provided Tokio runtime handle.
///
/// The returned flow must be polled via [`DeviceCodeLoginFlow::poll_events`].
pub fn start_device_code_login_with_handle(
    client_id: impl Into<String>,
    handle: &Handle,
) -> DeviceCodeLoginFlow {
    let client_id = client_id.into();
    let (sender, receiver) = mpsc::channel();
    let sender_for_task = sender.clone();

    // Run the blocking device-code polling flow on the runtime's blocking pool
    // so UI threads stay responsive.
    handle.spawn_blocking(move || {
        if let Err(err) = device_code::run_device_code_login(client_id, &sender_for_task) {
            let _ = sender_for_task.send(LoginEvent::Failed(err.to_string()));
        }
    });

    DeviceCodeLoginFlow {
        receiver,
        finished: false,
    }
}

/// Starts interactive browser OAuth by generating PKCE + auth request URL.
pub fn login_begin(client_id: impl Into<String>) -> Result<MinecraftLoginFlow, AuthError> {
    tracing::debug!(target: "vertexlauncher/auth", "starting browser OAuth flow");
    oauth::login_begin(client_id.into())
}

/// Completes login from OAuth authorization code.
///
/// This exchanges code -> Microsoft token and then completes Xbox/XSTS/Minecraft
/// service authentication, returning a normalized cached account.
pub fn login_finish(code: &str, flow: MinecraftLoginFlow) -> Result<CachedAccount, AuthError> {
    let agent = util::build_http_agent();

    // Exchange the browser auth code for a Microsoft OAuth access token first.
    let microsoft_token = oauth::exchange_auth_code_for_microsoft_token(&agent, code, &flow)
        .map_err(|err| error::prefix_auth_error("GetOAuthToken", err))?;

    // Continue through Xbox -> XSTS -> Minecraft service token chain.
    minecraft::complete_minecraft_login(
        &agent,
        &microsoft_token.access_token,
        microsoft_token.refresh_token.as_deref(),
    )
}

/// Renews cached account sessions using stored Microsoft refresh tokens.
///
/// Accounts without refresh tokens are left unchanged.
pub fn renew_cached_accounts_tokens(client_id: &str) -> Result<CachedAccountsState, AuthError> {
    renew_cached_accounts_tokens_with_callback(client_id, |_| {})
}

/// Renews cached account sessions using stored Microsoft refresh tokens and
/// emits per-account lifecycle events through the provided callback.
pub fn renew_cached_accounts_tokens_with_callback<F>(
    client_id: &str,
    mut on_event: F,
) -> Result<CachedAccountsState, AuthError>
where
    F: FnMut(CachedAccountRenewalEvent),
{
    let mut state = cache::load_cached_accounts()?;
    if state.accounts.is_empty() {
        return Ok(state);
    }

    let agent = util::build_http_agent();
    let mut any_updated = false;

    for account in &mut state.accounts {
        let Some(refresh_token) = account
            .microsoft_refresh_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            tracing::info!(
                target: "vertexlauncher/auth/renew",
                display_name = %account.minecraft_profile.name,
                "Skipping token renewal: no Microsoft refresh token cached."
            );
            continue;
        };

        on_event(CachedAccountRenewalEvent::Started {
            profile_id: account.minecraft_profile.id.clone(),
            display_name: account.minecraft_profile.name.clone(),
        });

        match oauth::refresh_microsoft_token(&agent, client_id, refresh_token).and_then(
            |microsoft_token| {
                minecraft::complete_minecraft_login(
                    &agent,
                    &microsoft_token.access_token,
                    microsoft_token.refresh_token.as_deref(),
                )
            },
        ) {
            Ok(renewed) => {
                on_event(CachedAccountRenewalEvent::Succeeded {
                    profile_id: renewed.minecraft_profile.id.clone(),
                    display_name: renewed.minecraft_profile.name.clone(),
                });
                tracing::info!(
                    target: "vertexlauncher/auth/renew",
                    display_name = %renewed.minecraft_profile.name,
                    "Renewed cached account session."
                );
                *account = renewed;
                any_updated = true;
            }
            Err(err) => {
                on_event(CachedAccountRenewalEvent::Failed {
                    profile_id: account.minecraft_profile.id.clone(),
                    display_name: account.minecraft_profile.name.clone(),
                    error: err.to_string(),
                });
                tracing::warn!(
                    target: "vertexlauncher/auth/renew",
                    display_name = %account.minecraft_profile.name,
                    error = %err,
                    "Failed to renew cached account session; keeping existing cached token."
                );
            }
        }
    }

    state = state.normalize();
    if any_updated {
        cache::save_cached_accounts(&state)?;
    }
    Ok(state)
}

/// Completes login from a full callback URL by extracting and validating `code`.
pub fn login_finish_from_redirect(
    callback_url: &str,
    flow: MinecraftLoginFlow,
) -> Result<CachedAccount, AuthError> {
    let code = oauth::extract_authorization_code(callback_url, &flow.state)?;
    login_finish(&code, flow)
}

/// Fetches the active user's latest Minecraft profile, including decoded texture payloads.
pub fn fetch_minecraft_profile(access_token: &str) -> Result<MinecraftProfileState, AuthError> {
    let agent = util::build_http_agent();
    minecraft::fetch_profile_state_with_textures(&agent, access_token)
}

/// Resolves a cached account avatar using stored bytes or the saved source skin URL.
pub fn resolve_cached_account_avatar(
    account: &CachedAccount,
) -> Result<Option<Vec<u8>>, AuthError> {
    minecraft::resolve_cached_account_avatar(account)
}

/// Uploads and activates a new skin texture for the active profile.
pub fn upload_minecraft_skin(
    access_token: &str,
    skin_png_bytes: &[u8],
    variant: MinecraftSkinVariant,
) -> Result<MinecraftProfileState, AuthError> {
    let agent = util::build_http_agent();
    minecraft::upload_profile_skin(&agent, access_token, skin_png_bytes, variant)
}

/// Activates one owned cape id for the active profile.
pub fn set_active_minecraft_cape(
    access_token: &str,
    cape_id: &str,
) -> Result<MinecraftProfileState, AuthError> {
    let agent = util::build_http_agent();
    minecraft::set_active_profile_cape(&agent, access_token, cape_id)
}

/// Clears the active cape for the active profile.
pub fn clear_active_minecraft_cape(access_token: &str) -> Result<MinecraftProfileState, AuthError> {
    let agent = util::build_http_agent();
    minecraft::clear_active_profile_cape(&agent, access_token)
}

/// Loads all cached accounts from the persistent auth cache file.
pub fn load_cached_accounts() -> Result<CachedAccountsState, AuthError> {
    cache::load_cached_accounts()
}

/// Saves all cached accounts to persistent auth cache storage.
pub fn save_cached_accounts(state: &CachedAccountsState) -> Result<(), AuthError> {
    cache::save_cached_accounts(state)
}

/// Clears all cached accounts from persistent auth cache storage.
pub fn clear_cached_accounts() -> Result<(), AuthError> {
    cache::clear_cached_accounts()
}

/// Loads the active cached account, if any.
pub fn load_cached_account() -> Result<Option<CachedAccount>, AuthError> {
    cache::load_cached_account()
}

/// Saves one account and sets it as active.
pub fn save_cached_account(account: &CachedAccount) -> Result<(), AuthError> {
    cache::save_cached_account(account)
}

/// Clears the active cached account.
///
/// Current behavior clears the full cache file, matching prior single-account
/// storage semantics.
pub fn clear_cached_account() -> Result<(), AuthError> {
    cache::clear_cached_account()
}
