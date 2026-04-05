use std::borrow::Cow;
use std::collections::BTreeSet;

pub const REDACTED_ACCOUNT_LABEL: &str = "Hidden Account";
pub const REDACTED_IDENTIFIER_LABEL: &str = "[redacted]";
const MAX_LOG_TEXT_CHARS: usize = 240;

pub fn redact_account_label<'a>(streamer_mode: bool, label: &'a str) -> Cow<'a, str> {
    if streamer_mode {
        Cow::Borrowed(REDACTED_ACCOUNT_LABEL)
    } else {
        Cow::Borrowed(label)
    }
}

pub fn redact_sensitive_text<'a>(streamer_mode: bool, text: &'a str) -> Cow<'a, str> {
    if !streamer_mode {
        return Cow::Borrowed(text);
    }

    let mut changed = false;
    let mut sanitized_parts = Vec::new();
    for part in text.split_whitespace() {
        let redacted = redact_identifier_token(part);
        if redacted != part {
            changed = true;
        }
        sanitized_parts.push(redacted);
    }

    if !changed {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(sanitized_parts.join(" "))
    }
}

pub fn sanitize_text_for_log(text: &str) -> String {
    let sanitized_urls = sanitize_urls_in_text(text);
    let sanitized_bearer = redact_bearer_tokens(&sanitized_urls);
    let sanitized_pairs = redact_sensitive_key_values(&sanitized_bearer);
    truncate_for_log(&sanitized_pairs, MAX_LOG_TEXT_CHARS)
}

fn redact_identifier_token(token: &str) -> String {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return token.to_owned();
    }

    let prefix_len = trimmed
        .find(|ch: char| ch.is_ascii_alphanumeric())
        .unwrap_or(trimmed.len());
    let suffix_len = trimmed
        .chars()
        .rev()
        .take_while(|ch| !ch.is_ascii_alphanumeric())
        .count();
    let core_end = trimmed.len().saturating_sub(suffix_len);
    if prefix_len >= core_end {
        return token.to_owned();
    }

    let prefix = &trimmed[..prefix_len];
    let core = &trimmed[prefix_len..core_end];
    let suffix = &trimmed[core_end..];

    if looks_like_sensitive_identifier(core) {
        format!("{prefix}{REDACTED_IDENTIFIER_LABEL}{suffix}")
    } else {
        token.to_owned()
    }
}

fn looks_like_sensitive_identifier(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("profile_id=")
        || lower.starts_with("player_uuid=")
        || lower.starts_with("xuid=")
        || lower.starts_with("account_key=")
        || lower.starts_with("display_name=")
        || lower.starts_with("player_name=")
    {
        return true;
    }

    is_hex_identifier(&lower) || is_uuid_identifier(&lower)
}

fn truncate_for_log(value: &str, max_chars: usize) -> String {
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

fn sanitize_url_for_log(value: &str) -> String {
    let Some((scheme, rest)) = value.split_once("://") else {
        return truncate_for_log(value, 160);
    };

    let (authority_and_path, fragment) = rest
        .split_once('#')
        .map(|(head, tail)| (head, Some(tail)))
        .unwrap_or((rest, None));
    let (authority_and_path, query) = authority_and_path
        .split_once('?')
        .map(|(head, tail)| (head, Some(tail)))
        .unwrap_or((authority_and_path, None));
    let (host, path) = authority_and_path
        .split_once('/')
        .map(|(host, path)| (host, format!("/{}", path)))
        .unwrap_or((authority_and_path, String::new()));

    let mut out = String::new();
    out.push_str(scheme);
    out.push_str("://");
    out.push_str(if host.is_empty() { "<no-host>" } else { host });
    out.push_str(path.as_str());

    let query_keys: BTreeSet<String> = query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|part| part.split_once('=').map(|(key, _)| key.to_owned()))
        .collect();
    if !query_keys.is_empty() {
        out.push_str("?params=");
        for (i, key) in query_keys.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&key);
        }
    }

    if fragment.is_some() {
        out.push_str("#fragment");
    }

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
    let lower = value.to_ascii_lowercase();
    let mut cursor = 0usize;
    let mut out = String::with_capacity(value.len());

    while let Some(relative_start) = lower[cursor..].find("bearer ") {
        let start = cursor + relative_start;
        let token_start = start + "bearer ".len();
        out.push_str(&value[cursor..token_start]);
        let token_end = value[token_start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                if ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | ')' | ']') {
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
    let sensitive_keys = [
        "authorization_code",
        "access_token",
        "refresh_token",
        "client_secret",
        "authorization",
        "id_token",
        "code",
        "state",
        "token",
        "xuid",
    ];

    let mut out = value.to_owned();
    for key in sensitive_keys {
        out = redact_sensitive_key_value(&out, key);
    }
    out
}

fn redact_sensitive_key_value(value: &str, key: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let needle = format!("{key}=");
    let mut cursor = 0usize;
    let mut out = String::with_capacity(value.len());

    while let Some(relative_start) = lower[cursor..].find(&needle) {
        let start = cursor + relative_start;
        let value_start = start + needle.len();
        out.push_str(&value[cursor..value_start]);
        let value_end = value[value_start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                if ch.is_whitespace() || matches!(ch, '&' | '"' | '\'' | ',' | ';' | ')' | ']') {
                    Some(value_start + offset)
                } else {
                    None
                }
            })
            .unwrap_or(value.len());
        out.push_str("[redacted]");
        cursor = value_end;
    }

    out.push_str(&value[cursor..]);
    out
}

fn is_hex_identifier(value: &str) -> bool {
    value.len() >= 16 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_uuid_identifier(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.len() != 5 {
        return false;
    }
    let expected = [8, 4, 4, 4, 12];
    parts
        .iter()
        .zip(expected)
        .all(|(part, len)| part.len() == len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}
