use keyring::{Entry, Error as KeyringError};

use crate::error::AuthError;

const REFRESH_TOKEN_SERVICE: &str = "vertexlauncher.microsoft_refresh_token";

fn refresh_token_entry(profile_id: &str) -> Result<Entry, AuthError> {
    Entry::new(REFRESH_TOKEN_SERVICE, profile_id).map_err(|err| {
        AuthError::SecureStorage(format!(
            "Failed to open refresh-token secure storage entry for profile '{profile_id}': {err}",
        ))
    })
}

pub(crate) fn load_refresh_token(profile_id: &str) -> Result<Option<String>, AuthError> {
    let entry = refresh_token_entry(profile_id)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to load refresh token for profile '{profile_id}': {err}",
        ))),
    }
}

pub(crate) fn store_refresh_token(profile_id: &str, refresh_token: &str) -> Result<(), AuthError> {
    let entry = refresh_token_entry(profile_id)?;
    entry.set_password(refresh_token).map_err(|err| {
        AuthError::SecureStorage(format!(
            "Failed to store refresh token for profile '{profile_id}': {err}",
        ))
    })
}

pub(crate) fn delete_refresh_token(profile_id: &str) -> Result<(), AuthError> {
    let entry = refresh_token_entry(profile_id)?;
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) => Err(AuthError::SecureStorage(format!(
            "Failed to delete refresh token for profile '{profile_id}': {err}",
        ))),
    }
}
