//! Helpers that keep sign-in secrets (codes, states, tokens, URLs with query strings) out of the log.

use std::hash::{DefaultHasher, Hash, Hasher};

const MAX_LOG_MESSAGE_CHARS: usize = 240;

pub(super) fn fingerprint_for_log(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) use log_redaction::sanitize_url_for_log;

/// Sanitizes a message for logging (URLs, bearer tokens and secrets removed).
pub(super) fn sanitize_message_for_log(value: &str) -> String {
    log_redaction::sanitize_message_for_log(value, MAX_LOG_MESSAGE_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_url_for_log_only_keeps_parameter_names() {
        let actual = sanitize_url_for_log(
            "https://login.live.com/oauth20_desktop.srf?code=abc&state=secret&error=none#frag",
        );
        assert_eq!(
            actual,
            "https://login.live.com/oauth20_desktop.srf?params=code,error,state#fragment"
        );
    }

    #[test]
    fn sanitize_message_for_log_redacts_embedded_url_and_bearer_token() {
        let actual = sanitize_message_for_log(
            "saw https://login.live.com/oauth20_desktop.srf?code=abc&state=secret and Bearer super-secret-token",
        );
        assert_eq!(
            actual,
            "saw https://login.live.com/oauth20_desktop.srf?params=code,state and Bearer [redacted]"
        );
    }

    #[test]
    fn sanitize_message_for_log_redacts_sensitive_key_values() {
        let actual = sanitize_message_for_log(
            r#"callback payload {"code":"abc","state":"secret"} refresh_token=refresh access_token='access'"#,
        );
        assert_eq!(
            actual,
            r#"callback payload {"code":[redacted],"state":[redacted]} refresh_token=[redacted] access_token=[redacted]"#
        );
    }
}
