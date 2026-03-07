use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tracing::{debug, warn};
use url::Url;

use crate::constants::{
    BUILTIN_MICROSOFT_TENANT, DEVICE_CODE_SCOPE, LIVE_AUTHORIZE_URL, LIVE_REDIRECT_URI, LIVE_SCOPE,
    LIVE_TOKEN_URL, OAUTH_BASE_URL,
};
use crate::error::{AuthError, map_http_error, oauth_error_with_guidance};
use crate::types::{LoginEvent, MinecraftLoginFlow};
use crate::util::{generate_pkce_verifier, generate_random_token, pkce_challenge};

#[derive(Debug, Deserialize)]
pub(crate) struct DeviceCodeResponse {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    #[serde(default)]
    pub(crate) verification_uri_complete: Option<String>,
    pub(crate) expires_in: u64,
    #[serde(default = "default_poll_interval")]
    pub(crate) interval: u64,
    #[serde(default)]
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MicrosoftTokenResponse {
    pub(crate) access_token: String,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

pub(crate) fn login_begin(client_id: String) -> Result<MinecraftLoginFlow, AuthError> {
    debug!(
        target: "vertexlauncher/auth/oauth",
        "building Microsoft OAuth authorization URL"
    );
    let verifier = generate_pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let state = generate_random_token(24);
    let session_id = generate_random_token(16);

    let mut auth_url = Url::parse(LIVE_AUTHORIZE_URL)
        .map_err(|err| AuthError::OAuth(format!("Failed to build authorize URL: {err}")))?;

    {
        let mut query = auth_url.query_pairs_mut();
        query.append_pair("client_id", &client_id);
        query.append_pair("response_type", "code");
        query.append_pair("redirect_uri", LIVE_REDIRECT_URI);
        query.append_pair("scope", LIVE_SCOPE);
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", &state);
        query.append_pair("prompt", "select_account");
    }

    Ok(MinecraftLoginFlow {
        verifier,
        challenge,
        session_id,
        auth_request_uri: auth_url.to_string(),
        state,
        client_id,
    })
}

pub(crate) fn exchange_auth_code_for_microsoft_token(
    agent: &ureq::Agent,
    code: &str,
    flow: &MinecraftLoginFlow,
) -> Result<MicrosoftTokenResponse, AuthError> {
    let response = agent
        .post(LIVE_TOKEN_URL)
        .set("Accept", "application/json")
        .send_form(&[
            ("client_id", flow.client_id.as_str()),
            ("code", code),
            ("code_verifier", flow.verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", LIVE_REDIRECT_URI),
            ("scope", LIVE_SCOPE),
        ]);

    match response {
        Ok(ok) => Ok(ok.into_json::<MicrosoftTokenResponse>()?),
        Err(ureq::Error::Status(_, err_response)) => {
            if let Ok(oauth_error) = err_response.into_json::<OAuthErrorResponse>() {
                let description = oauth_error
                    .error_description
                    .unwrap_or_else(|| "No details provided".to_owned());
                return Err(AuthError::OAuth(format!(
                    "{}: {}",
                    oauth_error.error, description
                )));
            }

            Err(AuthError::OAuth(
                "Authorization-code exchange failed with an unknown response".to_owned(),
            ))
        }
        Err(err) => Err(map_http_error(err)),
    }
}

pub(crate) fn extract_authorization_code(
    callback_url: &str,
    expected_state: &str,
) -> Result<String, AuthError> {
    let parsed = Url::parse(callback_url)
        .map_err(|err| AuthError::OAuth(format!("Failed to parse callback URL: {err}")))?;

    if parsed.scheme() != "https"
        || parsed.host_str() != Some("login.live.com")
        || parsed.path() != "/oauth20_desktop.srf"
    {
        return Err(AuthError::OAuth(
            "Microsoft callback URL did not match the expected redirect URI".to_owned(),
        ));
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;

    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(error) = error {
        let description = error_description.unwrap_or_else(|| "No details provided".to_owned());
        return Err(AuthError::OAuth(format!(
            "Microsoft sign-in failed: {error}: {description}"
        )));
    }

    let returned_state = state.ok_or_else(|| {
        AuthError::OAuth("Microsoft callback was missing the OAuth state".to_owned())
    })?;

    if returned_state != expected_state {
        return Err(AuthError::OAuth(
            "Microsoft callback state did not match the login session".to_owned(),
        ));
    }

    code.ok_or_else(|| {
        AuthError::OAuth("Microsoft callback did not include an auth code".to_owned())
    })
}

pub(crate) fn request_device_code(
    agent: &ureq::Agent,
    client_id: &str,
    tenant: &str,
) -> Result<DeviceCodeResponse, AuthError> {
    debug!(
        target: "vertexlauncher/auth/oauth",
        client_id,
        tenant,
        "requesting Microsoft device code"
    );
    let url = device_code_url(tenant);
    let response = agent
        .post(&url)
        .set("Accept", "application/json")
        .send_form(&[("client_id", client_id), ("scope", DEVICE_CODE_SCOPE)]);

    match response {
        Ok(ok) => Ok(ok.into_json::<DeviceCodeResponse>()?),
        Err(ureq::Error::Status(_, err_response)) => {
            if let Ok(oauth_error) = err_response.into_json::<OAuthErrorResponse>() {
                warn!(
                    target: "vertexlauncher/auth/oauth",
                    client_id,
                    tenant,
                    error = oauth_error.error.as_str(),
                    "device-code request returned OAuth error"
                );
                if oauth_error.error == "unauthorized_client" {
                    return Err(AuthError::OAuth(format!(
                        "unauthorized_client for client id '{client_id}' on tenant '{tenant}'. \
Set VERTEX_MSA_CLIENT_ID to your app id and ensure the app supports personal Microsoft accounts \
plus public client flows. If your app is multi-tenant/AAD, try VERTEX_MSA_TENANT=common or \
set auth::BUILTIN_MICROSOFT_TENANT in crates/auth/src/lib.rs.",
                    )));
                }

                let description = oauth_error
                    .error_description
                    .unwrap_or_else(|| "No details provided".to_owned());
                return Err(oauth_error_with_guidance(
                    &oauth_error.error,
                    &description,
                    tenant,
                ));
            }

            Err(AuthError::Http(
                "HTTP status error while requesting device code".to_owned(),
            ))
        }
        Err(err) => Err(map_http_error(err)),
    }
}

pub(crate) fn poll_for_microsoft_token(
    agent: &ureq::Agent,
    client_id: &str,
    tenant: &str,
    device_code: &DeviceCodeResponse,
    sender: &Sender<LoginEvent>,
) -> Result<MicrosoftTokenResponse, AuthError> {
    debug!(
        target: "vertexlauncher/auth/oauth",
        tenant,
        expires_in_secs = device_code.expires_in,
        initial_interval_secs = device_code.interval.max(1),
        "starting Microsoft device-code token polling"
    );
    let expires_after = Duration::from_secs(device_code.expires_in);
    let started_at = Instant::now();
    let mut poll_interval_secs = device_code.interval.max(1);
    let mut sent_waiting = false;

    loop {
        if started_at.elapsed() >= expires_after {
            return Err(AuthError::DeviceCodeExpired);
        }

        let response = agent
            .post(&token_url(tenant))
            .set("Accept", "application/json")
            .send_form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", device_code.device_code.as_str()),
            ]);

        match response {
            Ok(ok) => {
                let parsed = ok.into_json::<MicrosoftTokenResponse>()?;
                return Ok(parsed);
            }
            Err(ureq::Error::Status(_, err_response)) => {
                let oauth_error = err_response.into_json::<OAuthErrorResponse>().ok();
                let Some(oauth_error) = oauth_error else {
                    return Err(AuthError::OAuth(
                        "Token polling failed with unknown response".to_owned(),
                    ));
                };

                match oauth_error.error.as_str() {
                    "authorization_pending" => {
                        if !sent_waiting {
                            let _ = sender.send(LoginEvent::WaitingForAuthorization);
                            sent_waiting = true;
                        }
                    }
                    "slow_down" => {
                        poll_interval_secs = (poll_interval_secs + 5).min(30);
                        debug!(
                            target: "vertexlauncher/auth/oauth",
                            poll_interval_secs,
                            "device-code polling asked to slow down"
                        );
                        if !sent_waiting {
                            let _ = sender.send(LoginEvent::WaitingForAuthorization);
                            sent_waiting = true;
                        }
                    }
                    "authorization_declined" => return Err(AuthError::AuthorizationDeclined),
                    "expired_token" | "bad_verification_code" => {
                        warn!(
                            target: "vertexlauncher/auth/oauth",
                            error = oauth_error.error.as_str(),
                            "device-code polling expired or invalidated"
                        );
                        return Err(AuthError::DeviceCodeExpired);
                    }
                    other => {
                        let description = oauth_error
                            .error_description
                            .unwrap_or_else(|| "No details provided".to_owned());
                        return Err(oauth_error_with_guidance(other, &description, tenant));
                    }
                }
            }
            Err(other) => return Err(map_http_error(other)),
        }

        thread::sleep(Duration::from_secs(poll_interval_secs));
    }
}

pub(crate) fn oauth_tenant() -> String {
    std::env::var("VERTEX_MSA_TENANT")
        .ok()
        .map(|tenant| tenant.trim().to_owned())
        .filter(|tenant| !tenant.is_empty())
        .unwrap_or_else(|| {
            let builtin = BUILTIN_MICROSOFT_TENANT.trim();
            if builtin.is_empty() {
                "common".to_owned()
            } else {
                builtin.to_owned()
            }
        })
}

fn device_code_url(tenant: &str) -> String {
    format!("{OAUTH_BASE_URL}/{tenant}/oauth2/v2.0/devicecode")
}

fn token_url(tenant: &str) -> String {
    format!("{OAUTH_BASE_URL}/{tenant}/oauth2/v2.0/token")
}

const fn default_poll_interval() -> u64 {
    5
}
