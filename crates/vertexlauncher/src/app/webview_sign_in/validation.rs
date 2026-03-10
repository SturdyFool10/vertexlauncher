use url::Url;

pub(super) fn validate_sign_in_urls(
    auth_request_uri: &str,
    redirect_uri: &str,
    expected_state: &str,
) -> Result<(), String> {
    if expected_state.trim().is_empty() {
        return Err("Expected OAuth state must not be empty".to_owned());
    }

    let auth_url = Url::parse(auth_request_uri)
        .map_err(|err| format!("Auth URL could not be parsed: {err}"))?;
    if auth_url.scheme() != "https"
        || auth_url.host_str() != Some("login.live.com")
        || auth_url.path() != "/oauth20_authorize.srf"
    {
        return Err("Auth URL is not an expected Microsoft OAuth authorize endpoint".to_owned());
    }

    let redirect = Url::parse(redirect_uri)
        .map_err(|err| format!("Redirect URL could not be parsed: {err}"))?;
    if redirect.scheme() != "https"
        || redirect.host_str() != Some("login.live.com")
        || redirect.path() != "/oauth20_desktop.srf"
    {
        return Err("Redirect URL is not an expected Microsoft desktop OAuth endpoint".to_owned());
    }

    Ok(())
}
