use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use url::Url;

use crate::constants::{AUTH_REQUEST_MIN_INTERVAL, MAX_LOG_MESSAGE_CHARS};
use crate::error::AuthError;

fn auth_request_gate() -> &'static Mutex<Option<Instant>> {
    static GATE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn build_http_agent() -> ureq::Agent {
    ureq::Agent::new_with_defaults()
}

pub(crate) fn wait_for_auth_request_slot(operation: &str) {
    let mut gate = match auth_request_gate().lock() {
        Ok(gate) => gate,
        Err(poisoned) => {
            tracing::warn!(
                target: "vertexlauncher/auth/rate_limit",
                operation,
                "auth request gate mutex was poisoned; recovering lock state"
            );
            poisoned.into_inner()
        }
    };
    let now = Instant::now();

    if let Some(next_allowed_at) = *gate {
        if now < next_allowed_at {
            let delay = next_allowed_at.saturating_duration_since(now);
            tracing::debug!(
                target: "vertexlauncher/auth/rate_limit",
                operation,
                delay_ms = delay.as_millis(),
                "delaying auth request to avoid upstream rate limits"
            );
            thread::sleep(delay);
        }
    }

    *gate = Some(Instant::now() + AUTH_REQUEST_MIN_INTERVAL);
}

pub(crate) fn generate_pkce_verifier() -> String {
    generate_random_token(64)
}

pub(crate) fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub(crate) fn generate_random_token(length: usize) -> String {
    let mut out = String::with_capacity(length + 64);
    let mut rng = rand::thread_rng();
    while out.len() < length {
        let mut chunk = [0_u8; 48];
        rng.fill_bytes(&mut chunk);
        out.push_str(&URL_SAFE_NO_PAD.encode(chunk));
    }
    out.truncate(length);
    out
}

pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    BASE64_STANDARD.encode(bytes)
}

pub(crate) fn truncate_for_log(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut chars = value.chars();

    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return value.to_owned();
        };
        out.push(ch);
    }

    if chars.next().is_some() {
        out.push_str("...");
    }

    out
}

pub(crate) fn sanitize_url_for_log(value: &str) -> String {
    let Ok(parsed) = Url::parse(value) else {
        return truncate_for_log(value, 160);
    };

    let mut out = String::new();
    out.push_str(parsed.scheme());
    out.push_str("://");
    out.push_str(parsed.host_str().unwrap_or("<no-host>"));
    out.push_str(parsed.path());

    let query_keys: BTreeSet<String> = parsed.query_pairs().map(|(key, _)| key.into()).collect();
    if !query_keys.is_empty() {
        out.push_str("?params=");
        for (i, key) in query_keys.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&key);
        }
    }

    if parsed.fragment().is_some() {
        out.push_str("#fragment");
    }

    out
}

pub(crate) fn sanitize_message_for_log(value: &str) -> String {
    let sanitized_urls = sanitize_urls_in_text(value);
    let sanitized_bearer = redact_bearer_tokens(&sanitized_urls);
    let sanitized_pairs = redact_sensitive_key_values(&sanitized_bearer);
    truncate_for_log(&sanitized_pairs, MAX_LOG_MESSAGE_CHARS)
}

pub(crate) fn fingerprint_for_log(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn decode_base64(raw: &str) -> Result<Vec<u8>, AuthError> {
    BASE64_STANDARD
        .decode(raw)
        .map_err(|err| AuthError::OAuth(format!("Base64 decode failed: {err}")))
}

pub(crate) fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) trait UreqResponseExt {
    fn into_json<T: DeserializeOwned>(self) -> Result<T, AuthError>;
    fn into_string(self) -> Result<String, io::Error>;
}

impl UreqResponseExt for ureq::http::Response<ureq::Body> {
    fn into_json<T: DeserializeOwned>(mut self) -> Result<T, AuthError> {
        self.body_mut()
            .read_json::<T>()
            .map_err(|err| AuthError::Http(err.to_string()))
    }

    fn into_string(mut self) -> Result<String, io::Error> {
        let mut raw = String::new();
        self.body_mut().as_reader().read_to_string(&mut raw)?;
        Ok(raw)
    }
}

fn sanitize_urls_in_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(url_start) = find_next_url_start(remaining) {
        out.push_str(&remaining[..url_start]);
        let url_candidate = &remaining[url_start..];
        let url_end = find_url_end(url_candidate);
        let (url, trailing) = split_trailing_url_punctuation(&url_candidate[..url_end]);
        out.push_str(&sanitize_url_for_log(url));
        out.push_str(trailing);
        remaining = &url_candidate[url_end..];
    }

    out.push_str(remaining);
    out
}

fn find_next_url_start(value: &str) -> Option<usize> {
    ["https://", "http://", "file://"]
        .into_iter()
        .filter_map(|prefix| value.find(prefix))
        .min()
}

fn find_url_end(value: &str) -> usize {
    value
        .char_indices()
        .find_map(|(index, ch)| {
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
                )
            {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(value.len())
}

fn split_trailing_url_punctuation(value: &str) -> (&str, &str) {
    let trimmed = value.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    let trailing = &value[trimmed.len()..];
    (trimmed, trailing)
}

fn redact_bearer_tokens(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let lower = value.to_ascii_lowercase();
    let mut cursor = 0;

    while let Some(relative_start) = lower[cursor..].find("bearer ") {
        let start = cursor + relative_start;
        let token_start = start + "bearer ".len();
        out.push_str(&value[cursor..token_start]);

        let token_end = value[token_start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                if ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | ')' | ']' | '}') {
                    Some(token_start + offset)
                } else {
                    None
                }
            })
            .unwrap_or(value.len());

        if token_end > token_start {
            out.push_str("[redacted]");
        }
        cursor = token_end;
    }

    out.push_str(&value[cursor..]);
    out
}

fn redact_sensitive_key_values(value: &str) -> String {
    const SENSITIVE_KEYS: [&str; 9] = [
        "authorization_code",
        "access_token",
        "refresh_token",
        "client_secret",
        "code_verifier",
        "authorization",
        "id_token",
        "state",
        "code",
    ];

    let lower = value.to_ascii_lowercase();
    let mut out = String::with_capacity(value.len());
    let mut cursor = 0;

    while let Some((value_start, value_end)) =
        find_next_sensitive_value_range(value, &lower, cursor, &SENSITIVE_KEYS)
    {
        out.push_str(&value[cursor..value_start]);
        out.push_str("[redacted]");
        cursor = value_end;
    }

    out.push_str(&value[cursor..]);
    out
}

fn find_next_sensitive_value_range(
    value: &str,
    lower: &str,
    cursor: usize,
    sensitive_keys: &[&str],
) -> Option<(usize, usize)> {
    let bytes = value.as_bytes();
    let lower_bytes = lower.as_bytes();
    let mut index = cursor;

    while index < value.len() {
        if !value.is_char_boundary(index) {
            index += 1;
            continue;
        }

        for key in sensitive_keys {
            let key_bytes = key.as_bytes();
            let key_end = index + key_bytes.len();
            if key_end > value.len() || lower_bytes[index..key_end] != *key_bytes {
                continue;
            }

            if index > 0 {
                let previous = bytes[index - 1];
                if previous.is_ascii_alphanumeric() || previous == b'_' {
                    continue;
                }
            }

            if key_end < value.len() {
                let next = bytes[key_end];
                if next.is_ascii_alphanumeric() || next == b'_' {
                    continue;
                }
            }

            let mut separator_index = key_end;
            if separator_index < value.len() && matches!(bytes[separator_index], b'"' | b'\'') {
                separator_index += 1;
            }
            while separator_index < value.len() && bytes[separator_index].is_ascii_whitespace() {
                separator_index += 1;
            }
            if separator_index >= value.len() || !matches!(bytes[separator_index], b'=' | b':') {
                continue;
            }

            let mut value_start = separator_index + 1;
            while value_start < value.len() && bytes[value_start].is_ascii_whitespace() {
                value_start += 1;
            }
            if value_start >= value.len() {
                continue;
            }

            let value_end = find_sensitive_value_end(bytes, value_start);
            if value_end > value_start {
                return Some((value_start, value_end));
            }
        }

        index += 1;
    }

    None
}

fn find_sensitive_value_end(value: &[u8], value_start: usize) -> usize {
    if value_start >= value.len() {
        return value_start;
    }

    if matches!(value[value_start], b'"' | b'\'') {
        let quote = value[value_start];
        for index in value_start + 1..value.len() {
            if value[index] == quote {
                return index + 1;
            }
        }
        return value.len();
    }

    let mut index = value_start;
    while index < value.len() {
        let ch = value[index];
        if ch.is_ascii_whitespace()
            || matches!(ch, b'&' | b',' | b';' | b')' | b']' | b'}' | b'"' | b'\'')
        {
            break;
        }
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::{AUTH_REQUEST_MIN_INTERVAL, sanitize_message_for_log};

    #[test]
    fn auth_request_min_interval_is_nonzero() {
        assert!(AUTH_REQUEST_MIN_INTERVAL > std::time::Duration::ZERO);
    }

    #[test]
    fn sanitize_message_for_log_redacts_urls_and_tokens() {
        let actual = sanitize_message_for_log(
            r#"failed callback https://login.live.com/oauth20_desktop.srf?code=abc&state=secret refresh_token=refresh Bearer access"#,
        );
        assert_eq!(
            actual,
            "failed callback https://login.live.com/oauth20_desktop.srf?params=code,state refresh_token=[redacted] Bearer [redacted]"
        );
    }
}
