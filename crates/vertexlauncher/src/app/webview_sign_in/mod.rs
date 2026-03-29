use std::{
    collections::BTreeSet,
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use url::Url;

mod desktop;
mod ipc;
mod validation;

const HELPER_FLAG: &str = "--vertex-webview-signin";
const RETRYABLE_HELPER_ERROR_MARKER: &str = "__VERTEX_WEBVIEW_RETRYABLE__:";
const DEFAULT_HELPER_TIMEOUT_SECS: u64 = 15 * 60;
const HELPER_TIMEOUT_ENV: &str = "VERTEX_WEBVIEW_SIGNIN_HELPER_TIMEOUT_SECS";
const MAX_LOG_MESSAGE_CHARS: usize = 240;

#[derive(Clone, Copy, Debug)]
enum HelperProfile {
    Default,
    #[cfg(target_os = "linux")]
    LinuxDisableCompositing,
}

impl HelperProfile {
    fn log_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            #[cfg(target_os = "linux")]
            Self::LinuxDisableCompositing => "linux-disable-compositing",
        }
    }

    fn apply_to_command(self, command: &mut Command) {
        command.env("VERTEX_WEBVIEW_SIGNIN_PROFILE", self.log_name());

        #[cfg(target_os = "linux")]
        if matches!(self, Self::LinuxDisableCompositing) {
            command.env("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }
}

#[derive(Debug)]
struct HelperAttemptError {
    retryable: bool,
    message: String,
}

pub(super) fn truncate_for_log(value: &str, max_chars: usize) -> String {
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

pub(super) fn sanitize_url_for_log(value: &str) -> String {
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
        out.push_str(&query_keys.into_iter().collect::<Vec<_>>().join(","));
    }

    if parsed.fragment().is_some() {
        out.push_str("#fragment");
    }

    out
}

pub(super) fn sanitize_message_for_log(value: &str) -> String {
    let sanitized_urls = sanitize_urls_in_text(value);
    let sanitized_bearer = redact_bearer_tokens(&sanitized_urls);
    let sanitized_pairs = redact_sensitive_key_values(&sanitized_bearer);
    truncate_for_log(&sanitized_pairs, MAX_LOG_MESSAGE_CHARS)
}

pub(super) fn fingerprint_for_log(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) fn mark_retryable_helper_error(message: &str) -> String {
    format!("{RETRYABLE_HELPER_ERROR_MARKER} {message}")
}

fn decode_helper_error_message(raw: &str) -> HelperAttemptError {
    if let Some(message) = raw.strip_prefix(RETRYABLE_HELPER_ERROR_MARKER) {
        return HelperAttemptError {
            retryable: true,
            message: message.trim().to_owned(),
        };
    }

    HelperAttemptError {
        retryable: false,
        message: raw.trim().to_owned(),
    }
}

fn helper_profiles() -> Vec<HelperProfile> {
    let profiles = vec![HelperProfile::Default];
    #[cfg(target_os = "linux")]
    {
        let mut profiles = profiles;
        profiles.push(HelperProfile::LinuxDisableCompositing);
        profiles
    }
    #[cfg(not(target_os = "linux"))]
    {
        profiles
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

fn helper_timeout() -> Duration {
    match std::env::var(HELPER_TIMEOUT_ENV) {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(seconds) if seconds > 0 => Duration::from_secs(seconds),
            _ => {
                tracing::warn!(
                    target: "vertexlauncher/auth/webview",
                    env = HELPER_TIMEOUT_ENV,
                    value = %raw,
                    "Invalid helper timeout override; using default."
                );
                Duration::from_secs(DEFAULT_HELPER_TIMEOUT_SECS)
            }
        },
        Err(_) => Duration::from_secs(DEFAULT_HELPER_TIMEOUT_SECS),
    }
}

fn read_helper_stderr(
    profile: HelperProfile,
    stderr: impl Read + Send + 'static,
) -> thread::JoinHandle<Vec<String>> {
    thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let sanitized = sanitize_message_for_log(trimmed);
                    tracing::info!(
                        target: "vertexlauncher/auth/webview/helper",
                        profile = profile.log_name(),
                        "{}",
                        sanitized
                    );
                    lines.push(trimmed.to_owned());
                }
                Err(err) => {
                    tracing::warn!(
                        target: "vertexlauncher/auth/webview",
                        profile = profile.log_name(),
                        error = %sanitize_message_for_log(&err.to_string()),
                        "Failed reading webview helper stderr."
                    );
                    break;
                }
            }
        }
        lines
    })
}

fn read_helper_stdout(
    stdout: impl Read + Send + 'static,
) -> thread::JoinHandle<Result<String, String>> {
    thread::spawn(move || {
        let mut output = String::new();
        BufReader::new(stdout)
            .read_to_string(&mut output)
            .map_err(|err| format!("Failed reading webview helper stdout: {err}"))?;
        Ok(output)
    })
}

fn join_helper_stderr(handle: thread::JoinHandle<Vec<String>>) -> Vec<String> {
    match handle.join() {
        Ok(lines) => lines,
        Err(_) => {
            tracing::warn!(
                target: "vertexlauncher/auth/webview",
                "Webview helper stderr reader thread panicked unexpectedly."
            );
            Vec::new()
        }
    }
}

fn join_helper_stdout(
    handle: thread::JoinHandle<Result<String, String>>,
) -> Result<String, String> {
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err("Webview helper stdout reader thread panicked unexpectedly".to_owned()),
    }
}

fn run_helper_attempt(
    current_exe: &std::path::Path,
    profile: HelperProfile,
    auth_request_uri: &str,
    redirect_uri: &str,
    expected_state: &str,
) -> Result<String, HelperAttemptError> {
    tracing::info!(
        target: "vertexlauncher/auth/webview",
        profile = profile.log_name(),
        auth_url = %sanitize_url_for_log(auth_request_uri),
        redirect_url = %sanitize_url_for_log(redirect_uri),
        expected_state = %fingerprint_for_log(expected_state),
        "Starting Microsoft sign-in helper process."
    );

    let mut command = Command::new(current_exe);
    command
        .arg(HELPER_FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    profile.apply_to_command(&mut command);

    let mut child = command.spawn().map_err(|err| HelperAttemptError {
        retryable: false,
        message: format!("Failed to start webview helper process: {err}"),
    })?;

    let mut stdin = child.stdin.take().ok_or_else(|| HelperAttemptError {
        retryable: false,
        message: "Webview sign-in helper stdin was unavailable".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| HelperAttemptError {
        retryable: false,
        message: "Webview sign-in helper stdout was unavailable".to_owned(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| HelperAttemptError {
        retryable: false,
        message: "Webview sign-in helper stderr was unavailable".to_owned(),
    })?;

    let stdout_handle = read_helper_stdout(stdout);
    let stderr_handle = read_helper_stderr(profile, stderr);

    if let Err(err) = ipc::write_helper_request_to_stdin(
        &mut stdin,
        auth_request_uri,
        redirect_uri,
        expected_state,
    ) {
        let _ = child.kill();
        let _ = child.wait();
        let _ = join_helper_stdout(stdout_handle);
        let _ = join_helper_stderr(stderr_handle);
        return Err(HelperAttemptError {
            retryable: false,
            message: err,
        });
    }
    drop(stdin);

    let timeout = helper_timeout();
    let started_at = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started_at.elapsed() >= timeout {
                    tracing::error!(
                        target: "vertexlauncher/auth/webview",
                        profile = profile.log_name(),
                        timeout_secs = timeout.as_secs(),
                        "Microsoft sign-in helper timed out; terminating helper process."
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = join_helper_stdout(stdout_handle);
                    let _ = join_helper_stderr(stderr_handle);
                    return Err(HelperAttemptError {
                        retryable: false,
                        message: format!(
                            "Microsoft sign-in helper exceeded {} seconds without completing",
                            timeout.as_secs()
                        ),
                    });
                }
                thread::sleep(Duration::from_millis(200));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_helper_stdout(stdout_handle);
                let _ = join_helper_stderr(stderr_handle);
                return Err(HelperAttemptError {
                    retryable: false,
                    message: format!("Failed waiting for webview helper process: {err}"),
                });
            }
        }
    };

    let stdout = join_helper_stdout(stdout_handle).map_err(|message| HelperAttemptError {
        retryable: false,
        message,
    })?;
    let stderr_lines = join_helper_stderr(stderr_handle);

    if !status.success() {
        let raw_error = stderr_lines
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "Webview sign-in helper failed without an error message".to_owned());
        let error = decode_helper_error_message(&raw_error);
        tracing::error!(
            target: "vertexlauncher/auth/webview",
            profile = profile.log_name(),
            retryable = error.retryable,
            exit_code = status.code(),
            error = %sanitize_message_for_log(&error.message),
            "Microsoft sign-in helper exited with failure."
        );
        return Err(error);
    }

    let auth_code = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .ok_or_else(|| HelperAttemptError {
            retryable: false,
            message: "Webview sign-in helper returned no auth code".to_owned(),
        })?;

    tracing::info!(
        target: "vertexlauncher/auth/webview",
        profile = profile.log_name(),
        "Microsoft sign-in helper completed successfully."
    );
    Ok(auth_code.to_owned())
}

pub fn maybe_run_helper_from_args() -> Result<bool, String> {
    let mut args = std::env::args();
    let _ = args.next();

    let Some(flag) = args.next() else {
        return Ok(false);
    };
    if flag != HELPER_FLAG {
        return Ok(false);
    }

    if args.next().is_some() {
        return Err("Unexpected extra arguments for webview helper".to_owned());
    }

    let (auth_request_uri, redirect_uri, expected_state) = ipc::read_helper_request_from_stdin()?;
    validation::validate_sign_in_urls(&auth_request_uri, &redirect_uri, &expected_state)?;
    tracing::info!(
        target: "vertexlauncher/auth/webview/helper",
        profile = %std::env::var("VERTEX_WEBVIEW_SIGNIN_PROFILE").unwrap_or_else(|_| "default".to_owned()),
        auth_url = %sanitize_url_for_log(&auth_request_uri),
        redirect_url = %sanitize_url_for_log(&redirect_uri),
        expected_state = %fingerprint_for_log(&expected_state),
        "Webview sign-in helper initialized."
    );

    let callback_url = desktop::run_webview_window(&auth_request_uri, &redirect_uri)?;
    tracing::info!(
        target: "vertexlauncher/auth/webview/helper",
        callback_url = %sanitize_url_for_log(&callback_url),
        "Webview sign-in helper received callback URL."
    );
    let auth_code =
        auth::validate_oauth_callback_code(&callback_url, &redirect_uri, &expected_state)
            .map_err(|err| format!("Failed to validate Microsoft callback in helper: {err}"))?;
    ipc::write_helper_response_to_stdout(&mut std::io::stdout(), &auth_code)?;

    tracing::info!(
        target: "vertexlauncher/auth/webview/helper",
        "Webview sign-in helper validated callback and wrote auth code."
    );

    Ok(true)
}

pub fn open_microsoft_sign_in(
    auth_request_uri: &str,
    redirect_uri: &str,
    expected_state: &str,
) -> Result<String, String> {
    validation::validate_sign_in_urls(auth_request_uri, redirect_uri, expected_state)?;
    tracing::info!(
        target: "vertexlauncher/auth/webview",
        auth_url = %sanitize_url_for_log(auth_request_uri),
        redirect_url = %sanitize_url_for_log(redirect_uri),
        expected_state = %fingerprint_for_log(expected_state),
        "Opening embedded Microsoft sign-in helper."
    );

    let current_exe = std::env::current_exe()
        .map_err(|err| format!("Failed to resolve launcher executable path: {err}"))?;
    let profiles = helper_profiles();

    for (index, profile) in profiles.iter().copied().enumerate() {
        match run_helper_attempt(
            &current_exe,
            profile,
            auth_request_uri,
            redirect_uri,
            expected_state,
        ) {
            Ok(auth_code) => return Ok(auth_code),
            Err(err) => {
                if err.retryable && index + 1 < profiles.len() {
                    tracing::warn!(
                        target: "vertexlauncher/auth/webview",
                        current_profile = profile.log_name(),
                        next_profile = profiles[index + 1].log_name(),
                        error = %sanitize_message_for_log(&err.message),
                        "Retrying embedded Microsoft sign-in helper with fallback profile."
                    );
                    continue;
                }

                return Err(err.message);
            }
        }
    }

    Err("Webview sign-in helper failed unexpectedly".to_owned())
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
    fn retryable_marker_round_trips() {
        let marked = mark_retryable_helper_error("blank webview");
        let decoded = decode_helper_error_message(&marked);
        assert!(decoded.retryable);
        assert_eq!(decoded.message, "blank webview");
    }

    #[test]
    fn sanitize_message_for_log_redacts_embedded_url_and_bearer_token() {
        let actual = sanitize_message_for_log(
            "helper saw https://login.live.com/oauth20_desktop.srf?code=abc&state=secret and Bearer super-secret-token",
        );
        assert_eq!(
            actual,
            "helper saw https://login.live.com/oauth20_desktop.srf?params=code,state and Bearer [redacted]"
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
