use keyring::{Entry, Error as KeyringError};

use crate::error::AuthError;

const ACCOUNTS_STATE_SERVICE: &str = "vertexlauncher.accounts_state.v1";
const ACCOUNTS_STATE_ACCOUNT: &str = "cached_accounts";
const REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token.v2";
const LEGACY_REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token";

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
    let entry = accounts_state_entry()?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) if is_corrupt_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                error = %err,
                "ignoring unreadable cached accounts state entry"
            );
            let _ = delete_accounts_state();
            Ok(None)
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load cached accounts state from secure storage: {err}",
        ))),
    }
}

pub(crate) fn store_accounts_state(serialized_state: &str) -> Result<(), AuthError> {
    let entry = accounts_state_entry()?;
    entry.set_password(serialized_state).map_err(|err| {
        AuthError::SecureStorage(format!(
            "Failed to store cached accounts state in secure storage: {err}",
        ))
    })
}

pub(crate) fn delete_accounts_state() -> Result<(), AuthError> {
    let entry = accounts_state_entry()?;
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to delete cached accounts state from secure storage: {err}",
        ))),
    }
}

pub(crate) fn load_refresh_token(profile_id: &str) -> Result<Option<String>, AuthError> {
    let entry = refresh_token_entry(REFRESH_TOKEN_SERVICE, profile_id)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => load_legacy_refresh_token(profile_id),
        Err(err) if is_corrupt_secure_storage_error(&err) => Err(AuthError::SecureStorage(
            format!("Failed to load refresh token for profile '{profile_id}': {err}",),
        )),
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load refresh token for profile '{profile_id}': {err}",
        ))),
    }
}

pub(crate) fn store_refresh_token(profile_id: &str, refresh_token: &str) -> Result<(), AuthError> {
    let entry = refresh_token_entry(REFRESH_TOKEN_SERVICE, profile_id)?;
    entry.set_password(refresh_token).map_err(|err| {
        AuthError::SecureStorage(format!(
            "Failed to store refresh token for profile '{profile_id}': {err}",
        ))
    })?;
    let _ = delete_refresh_token_for_service(LEGACY_REFRESH_TOKEN_SERVICE, profile_id);
    Ok(())
}

pub(crate) fn delete_refresh_token(profile_id: &str) -> Result<(), AuthError> {
    delete_refresh_token_for_service(REFRESH_TOKEN_SERVICE, profile_id)?;
    delete_refresh_token_for_service(LEGACY_REFRESH_TOKEN_SERVICE, profile_id)?;
    Ok(())
}

fn load_legacy_refresh_token(profile_id: &str) -> Result<Option<String>, AuthError> {
    let legacy_entry = refresh_token_entry(LEGACY_REFRESH_TOKEN_SERVICE, profile_id)?;
    match legacy_entry.get_password() {
        Ok(value) => {
            store_refresh_token(profile_id, &value)?;
            Ok(Some(value))
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) if is_corrupt_secure_storage_error(&err) => {
            tracing::warn!(
                target: "vertexlauncher/auth/secret_store",
                profile_id,
                error = %err,
                "ignoring unreadable legacy refresh-token entry"
            );
            let _ = delete_refresh_token_for_service(LEGACY_REFRESH_TOKEN_SERVICE, profile_id);
            Ok(None)
        }
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load refresh token for profile '{profile_id}' from legacy secure storage: {err}",
        ))),
    }
}

fn delete_refresh_token_for_service(service: &str, profile_id: &str) -> Result<(), AuthError> {
    let entry = refresh_token_entry(service, profile_id)?;
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to delete refresh token for profile '{profile_id}': {err}",
        ))),
    }
}

fn is_corrupt_secure_storage_error(err: &KeyringError) -> bool {
    let error_text = err.to_string();
    error_text.contains("Crypto error")
        || error_text.contains("Unpad Error")
        || error_text.contains("Platform secure storage failure")
}
