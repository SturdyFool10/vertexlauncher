use std::io;

use thiserror::Error;

/// Error type for authentication and account cache operations.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Secure storage error: {0}")]
    SecureStorage(String),
    #[error("Failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Failed to decode image data: {0}")]
    Image(#[from] image::ImageError),
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("Device-code authorization timed out")]
    DeviceCodeExpired,
    #[error("Microsoft authorization was declined")]
    AuthorizationDeclined,
    #[error("Minecraft Java profile is unavailable for this account")]
    MinecraftProfileUnavailable,
    #[error("OAuth error: {0}")]
    OAuth(String),
}

pub(crate) fn map_http_error(error: ureq::Error) -> AuthError {
    match error {
        ureq::Error::StatusCode(code) => AuthError::Http(format!("HTTP status {code}")),
        other => AuthError::Http(other.to_string()),
    }
}

pub(crate) fn prefix_auth_error(step: &str, error: AuthError) -> AuthError {
    match error {
        AuthError::Http(message) => AuthError::Http(format!("{step}: {message}")),
        AuthError::OAuth(message) => AuthError::OAuth(format!("{step}: {message}")),
        AuthError::SecureStorage(message) => AuthError::SecureStorage(format!("{step}: {message}")),
        other => AuthError::OAuth(format!("{step}: {other}")),
    }
}
