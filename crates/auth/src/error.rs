use std::io::{self, Read};

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
        ureq::Error::Status(code, response) => {
            if !auth_verbose_errors_enabled() {
                return AuthError::Http(format!("HTTP status {code}"));
            }

            let mut snippet = String::new();
            let _ = response
                .into_reader()
                .take(1024)
                .read_to_string(&mut snippet);

            if snippet.trim().is_empty() {
                AuthError::Http(format!("HTTP status {code}"))
            } else {
                AuthError::Http(format!("HTTP status {code}: {}", snippet.trim()))
            }
        }
        ureq::Error::Transport(transport) => AuthError::Http(transport.to_string()),
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

pub(crate) fn oauth_error_with_guidance(error: &str, description: &str, tenant: &str) -> AuthError {
    if description.contains("AADSTS9002346")
        || description.contains("configured for use by Microsoft Accounts users only")
    {
        return AuthError::OAuth(format!(
            "{error}: {description}. This app is Microsoft-accounts-only, so use the \\
`consumers` endpoint. Set VERTEX_MSA_TENANT=consumers or set \\
auth::BUILTIN_MICROSOFT_TENANT=\"consumers\" in crates/auth/src/lib.rs (current tenant: '{tenant}')."
        ));
    }

    if description.contains("AADSTS70002")
        || description.contains("must be marked as 'mobile'")
        || description.contains("not supported for this feature")
    {
        return AuthError::OAuth(format!(
            "{error}: {description}. Device-code flow requires a public/native client. In Azure \\
Portal, open your app registration -> Authentication, set 'Allow public client flows' to Yes, \\
and add a 'Mobile and desktop applications' platform (native client)."
        ));
    }

    AuthError::OAuth(format!("{error}: {description}"))
}

fn auth_verbose_errors_enabled() -> bool {
    std::env::var("VERTEX_AUTH_VERBOSE_ERRORS")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}
