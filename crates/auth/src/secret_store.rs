use std::{
    sync::{LazyLock, Mutex},
    thread,
    time::Duration,
};

use keyring::{Entry, Error as KeyringError};

use crate::error::AuthError;

const ACCOUNTS_STATE_SERVICE: &str = "vertexlauncher.accounts_state.v1";
const ACCOUNTS_STATE_ACCOUNT: &str = "cached_accounts";
const REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token.v2";
const LEGACY_REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token";
const SECURE_STORE_RETRY_ATTEMPTS: usize = 5;
const SECURE_STORE_RETRY_DELAY: Duration = Duration::from_millis(75);
const REFRESH_TOKEN_VERIFY_ATTEMPTS: usize = 5;
const REFRESH_TOKEN_STORE_ATTEMPTS: usize = 3;

static SECURE_STORE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(crate) enum RefreshTokenLoadResult {
    Present(String),
    Missing,
    Unavailable,
}

fn accounts_state_entry() -> Result<Entry, AuthError> {
    Entry::new(ACCOUNTS_STATE_SERVICE, ACCOUNTS_STATE_ACCOUNT).map_err(|err| {
        AuthError::SecureStorage(format!(
            "Failed to open secure storage entry for cached accounts state: {err}",
        ))
    })
}

fn refresh_token_entry(service: &str, profile_id: &str) -> Result<Entry, AuthError> {
    Entry::new(service, profile_id).map_err(|err| {
        AuthError::SecureStorage(format!(
            "Failed to open refresh-token secure storage entry for profile '{profile_id}': {err}",
        ))
    })
}

pub(crate) fn load_accounts_state() -> Result<Option<String>, AuthError> {
    with_secure_store_lock(load_accounts_state_unlocked)
}

fn load_accounts_state_unlocked() -> Result<Option<String>, AuthError> {
    let entry = accounts_state_entry()?;
    match retry_keyring_operation(|| entry.get_password()) {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) if is_unavailable_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                error = %err,
                "secure storage unavailable while loading cached accounts state; using empty cache"
            );
            Ok(None)
        }
        Err(err) if is_corrupt_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                error = %err,
                "ignoring unreadable cached accounts state entry"
            );
            let _ = delete_accounts_state_unlocked();
            Ok(None)
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load cached accounts state from secure storage: {err}",
        ))),
    }
}

pub(crate) fn delete_accounts_state() -> Result<(), AuthError> {
    with_secure_store_lock(delete_accounts_state_unlocked)
}

fn delete_accounts_state_unlocked() -> Result<(), AuthError> {
    let entry = accounts_state_entry()?;
    match retry_keyring_operation(|| entry.delete_credential()) {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) if is_unavailable_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                error = %err,
                "secure storage unavailable while deleting cached accounts state; keeping in-memory state cleared"
            );
            Ok(())
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to delete cached accounts state from secure storage: {err}",
        ))),
    }
}

pub(crate) fn load_refresh_token(profile_id: &str) -> Result<RefreshTokenLoadResult, AuthError> {
    with_secure_store_lock(|| load_refresh_token_unlocked(profile_id))
}

fn load_refresh_token_unlocked(profile_id: &str) -> Result<RefreshTokenLoadResult, AuthError> {
    let entry = refresh_token_entry(REFRESH_TOKEN_SERVICE, profile_id)?;
    match retry_keyring_operation(|| entry.get_password()) {
        Ok(value) => Ok(RefreshTokenLoadResult::Present(value)),
        Err(KeyringError::NoEntry) => load_legacy_refresh_token_unlocked(profile_id),
        Err(err) if is_unavailable_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                profile_id,
                error = %err,
                "secure storage unavailable while loading refresh token; continuing without a persisted token"
            );
            Ok(RefreshTokenLoadResult::Unavailable)
        }
        Err(err) if is_corrupt_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                profile_id,
                error = %err,
                "ignoring unreadable refresh-token entry"
            );
            let _ = delete_refresh_token_for_service_unlocked(REFRESH_TOKEN_SERVICE, profile_id);
            load_legacy_refresh_token_unlocked(profile_id)
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load refresh token for profile '{profile_id}': {err}",
        ))),
    }
}

pub(crate) fn store_refresh_token(profile_id: &str, refresh_token: &str) -> Result<(), AuthError> {
    with_secure_store_lock(|| store_refresh_token_unlocked(profile_id, refresh_token))
}

fn store_refresh_token_unlocked(profile_id: &str, refresh_token: &str) -> Result<(), AuthError> {
    for attempt in 0..REFRESH_TOKEN_STORE_ATTEMPTS {
        let _ = delete_refresh_token_for_service_unlocked(REFRESH_TOKEN_SERVICE, profile_id);
        let entry = refresh_token_entry(REFRESH_TOKEN_SERVICE, profile_id)?;
        retry_keyring_operation(|| entry.set_password(refresh_token)).map_err(|err| {
            AuthError::SecureStorage(format!(
                "Failed to store refresh token for profile '{profile_id}': {err}",
            ))
        })?;

        match verify_refresh_token_round_trip_unlocked(profile_id, refresh_token) {
            Ok(()) => {
                let _ = delete_refresh_token_for_service_unlocked(
                    LEGACY_REFRESH_TOKEN_SERVICE,
                    profile_id,
                );
                return Ok(());
            }
            Err(AuthError::SecureStorage(message))
                if attempt + 1 < REFRESH_TOKEN_STORE_ATTEMPTS
                    && is_corrupt_secure_storage_message(&message) =>
            {
                tracing::warn!(
                    target: "vertexlauncher/auth/secret_store",
                    profile_id,
                    attempt = attempt + 1,
                    "refresh-token verification hit corrupt secure storage data after write; deleting entry and retrying"
                );
                let _ =
                    delete_refresh_token_for_service_unlocked(REFRESH_TOKEN_SERVICE, profile_id);
                thread::sleep(SECURE_STORE_RETRY_DELAY);
            }
            Err(err) => return Err(err),
        }
    }

    Err(AuthError::SecureStorage(format!(
        "Failed to store refresh token for profile '{profile_id}' after repeated secure storage rewrite attempts."
    )))
}

pub(crate) fn delete_refresh_token(profile_id: &str) -> Result<(), AuthError> {
    with_secure_store_lock(|| {
        delete_refresh_token_for_service_unlocked(REFRESH_TOKEN_SERVICE, profile_id)?;
        delete_refresh_token_for_service_unlocked(LEGACY_REFRESH_TOKEN_SERVICE, profile_id)?;
        Ok(())
    })
}

fn load_legacy_refresh_token_unlocked(
    profile_id: &str,
) -> Result<RefreshTokenLoadResult, AuthError> {
    let legacy_entry = refresh_token_entry(LEGACY_REFRESH_TOKEN_SERVICE, profile_id)?;
    match retry_keyring_operation(|| legacy_entry.get_password()) {
        Ok(value) => {
            store_refresh_token_unlocked(profile_id, &value)?;
            Ok(RefreshTokenLoadResult::Present(value))
        }
        Err(KeyringError::NoEntry) => Ok(RefreshTokenLoadResult::Missing),
        Err(err) if is_unavailable_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                profile_id,
                error = %err,
                "secure storage unavailable while loading legacy refresh token; continuing without a persisted token"
            );
            Ok(RefreshTokenLoadResult::Unavailable)
        }
        Err(err) if is_corrupt_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                profile_id,
                error = %err,
                "ignoring unreadable legacy refresh-token entry"
            );
            let _ =
                delete_refresh_token_for_service_unlocked(LEGACY_REFRESH_TOKEN_SERVICE, profile_id);
            Ok(RefreshTokenLoadResult::Missing)
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load refresh token for profile '{profile_id}' from legacy secure storage: {err}",
        ))),
    }
}

fn delete_refresh_token_for_service_unlocked(
    service: &str,
    profile_id: &str,
) -> Result<(), AuthError> {
    let entry = refresh_token_entry(service, profile_id)?;
    match retry_keyring_operation(|| entry.delete_credential()) {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) if is_unavailable_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                profile_id,
                error = %err,
                "secure storage unavailable while deleting refresh token; keeping in-memory state cleared"
            );
            Ok(())
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to delete refresh token for profile '{profile_id}': {err}",
        ))),
    }
}

fn verify_refresh_token_round_trip_unlocked(
    profile_id: &str,
    refresh_token: &str,
) -> Result<(), AuthError> {
    for attempt in 0..refresh_token_verify_attempts() {
        let entry = refresh_token_entry(REFRESH_TOKEN_SERVICE, profile_id)?;
        match retry_keyring_operation(|| entry.get_password()) {
            Ok(stored) if stored == refresh_token => return Ok(()),
            Ok(_) | Err(KeyringError::NoEntry) if attempt + 1 < refresh_token_verify_attempts() => {
                sleep_before_refresh_token_retry();
            }
            Err(err)
                if attempt + 1 < refresh_token_verify_attempts()
                    && should_retry_refresh_token_verification(&err) =>
            {
                sleep_before_refresh_token_retry();
            }
            Err(err) => {
                return Err(AuthError::SecureStorage(format!(
                    "Failed to verify refresh token for profile '{profile_id}' after writing it to secure storage: {err}",
                )));
            }
            Ok(_) => {
                return Err(AuthError::SecureStorage(format!(
                    "Refresh token for profile '{profile_id}' did not round-trip correctly through secure storage."
                )));
            }
        }
    }

    Err(AuthError::SecureStorage(format!(
        "Refresh token for profile '{profile_id}' did not round-trip correctly through secure storage."
    )))
}

fn with_secure_store_lock<T>(
    operation: impl FnOnce() -> Result<T, AuthError>,
) -> Result<T, AuthError> {
    let _guard = match SECURE_STORE_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                "secure storage lock was poisoned; recovering lock state"
            );
            poisoned.into_inner()
        }
    };
    operation()
}

fn retry_keyring_operation<T>(
    mut operation: impl FnMut() -> Result<T, KeyringError>,
) -> Result<T, KeyringError> {
    let retry_attempts = SECURE_STORE_RETRY_ATTEMPTS.max(1);
    let mut last_err = None;
    for attempt in 0..retry_attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(err)
                if attempt + 1 < retry_attempts && should_retry_secure_storage_operation(&err) =>
            {
                last_err = Some(err);
                thread::sleep(SECURE_STORE_RETRY_DELAY);
            }
            Err(err) => return Err(err),
        }
    }

    Err(last_err.unwrap_or(KeyringError::NoEntry))
}

fn is_corrupt_secure_storage_error(err: &KeyringError) -> bool {
    let error_text = err.to_string();
    matches!(err, KeyringError::BadEncoding(_)) || is_corrupt_secure_storage_message(&error_text)
}

fn is_corrupt_secure_storage_message(error_text: &str) -> bool {
    error_text.contains("Crypto error") || error_text.contains("Unpad Error")
}

fn is_unavailable_secure_storage_error(err: &KeyringError) -> bool {
    matches!(err, KeyringError::NoStorageAccess(_)) || is_retryable_platform_storage_error(err)
}

fn is_retryable_platform_storage_error(err: &KeyringError) -> bool {
    let KeyringError::PlatformFailure(inner) = err else {
        return false;
    };

    let lowered = inner.to_string().to_ascii_lowercase();
    lowered.contains("dbus error")
        || lowered.contains("can't find session")
        || lowered.contains("secret service")
        || lowered.contains("org.freedesktop.secrets")
        || lowered.contains("no such object path")
        || lowered.contains("no such interface")
        || lowered.contains("keychain")
        || lowered.contains("errsecnotavailable")
        || lowered.contains("errsecreadonly")
        || lowered.contains("errsecnosuchkeychain")
        || lowered.contains("errsecinvalidkeychain")
        || lowered.contains("windows error_no_such_logon_session")
}

fn refresh_token_verify_attempts() -> usize {
    REFRESH_TOKEN_VERIFY_ATTEMPTS
}

fn should_retry_secure_storage_operation(err: &KeyringError) -> bool {
    is_unavailable_secure_storage_error(err)
}

fn should_retry_refresh_token_verification(err: &KeyringError) -> bool {
    should_retry_secure_storage_operation(err)
}

fn sleep_before_refresh_token_retry() {
    thread::sleep(SECURE_STORE_RETRY_DELAY);
}
