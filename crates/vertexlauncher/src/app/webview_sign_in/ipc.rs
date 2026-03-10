use std::io::{self, BufRead, Write};

pub(super) fn write_helper_request_to_stdin(
    stdin: &mut impl Write,
    auth_request_uri: &str,
    redirect_uri: &str,
    expected_state: &str,
) -> Result<(), String> {
    // Keep auth payload out of process args to reduce local disclosure surface.
    writeln!(stdin, "{auth_request_uri}")
        .map_err(|err| format!("Failed writing auth URL to helper stdin: {err}"))?;
    writeln!(stdin, "{redirect_uri}")
        .map_err(|err| format!("Failed writing redirect URL to helper stdin: {err}"))?;
    writeln!(stdin, "{expected_state}")
        .map_err(|err| format!("Failed writing expected OAuth state to helper stdin: {err}"))?;
    stdin
        .flush()
        .map_err(|err| format!("Failed flushing helper stdin payload: {err}"))
}

pub(super) fn read_helper_request_from_stdin() -> Result<(String, String, String), String> {
    let mut lines = io::BufReader::new(io::stdin()).lines();
    let auth_request_uri = lines
        .next()
        .transpose()
        .map_err(|err| format!("Failed reading auth URL from helper stdin: {err}"))?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing auth request URL for webview helper".to_owned())?;

    let redirect_uri = lines
        .next()
        .transpose()
        .map_err(|err| format!("Failed reading redirect URL from helper stdin: {err}"))?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing redirect URI for webview helper".to_owned())?;

    let expected_state = lines
        .next()
        .transpose()
        .map_err(|err| format!("Failed reading expected OAuth state from helper stdin: {err}"))?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Missing expected OAuth state for webview helper".to_owned())?;

    Ok((auth_request_uri, redirect_uri, expected_state))
}

pub(super) fn write_helper_response_to_stdout(
    stdout: &mut impl Write,
    auth_code: &str,
) -> Result<(), String> {
    writeln!(stdout, "{auth_code}")
        .map_err(|err| format!("Failed writing auth code from helper stdout: {err}"))?;
    stdout
        .flush()
        .map_err(|err| format!("Failed flushing helper auth code: {err}"))
}
