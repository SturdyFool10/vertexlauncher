use std::sync::mpsc::Sender;

use crate::error::{AuthError, prefix_auth_error};
use crate::minecraft::complete_minecraft_login;
use crate::oauth::{device_code_credentials, poll_for_microsoft_token, request_device_code};
use crate::types::{DeviceCodePrompt, LoginEvent};
use crate::util::build_http_agent;

pub(crate) fn run_device_code_login(
    _client_id: String,
    sender: &Sender<LoginEvent>,
) -> Result<(), AuthError> {
    tracing::info!(
        target: "vertexlauncher/auth/device_code",
        "starting device-code login worker"
    );
    let agent = build_http_agent();
    let (client_id, tenant) = device_code_credentials();

    let device_code = request_device_code(&agent, &client_id, &tenant)
        .map_err(|err| prefix_auth_error("RequestDeviceCode", err))?;

    let prompt = DeviceCodePrompt {
        user_code: device_code.user_code.clone(),
        verification_uri: device_code.verification_uri.clone(),
        verification_uri_complete: device_code.verification_uri_complete.clone(),
        expires_in_secs: device_code.expires_in,
        poll_interval_secs: device_code.interval.max(1),
        message: device_code.message.clone(),
    };
    let _ = sender.send(LoginEvent::DeviceCode(prompt));

    let microsoft_token =
        poll_for_microsoft_token(&agent, client_id.as_str(), &tenant, &device_code, sender)
            .map_err(|err| prefix_auth_error("PollForMicrosoftToken", err))?;

    let mut account = complete_minecraft_login(
        &agent,
        &microsoft_token.access_token,
        microsoft_token.refresh_token.as_deref(),
    )?;
    account.microsoft_client_id = Some(client_id.clone());
    account.microsoft_token_uri = Some(format!(
        "{}/{}/oauth2/v2.0/token",
        crate::constants::OAUTH_BASE_URL,
        tenant
    ));
    account.microsoft_scope = Some(crate::constants::DEVICE_CODE_SCOPE.to_owned());
    if account
        .microsoft_refresh_token
        .as_deref()
        .map(str::trim)
        .is_none_or(|value| value.is_empty())
    {
        tracing::warn!(
            target: "vertexlauncher/auth/device_code",
            "device-code login completed without a Microsoft refresh token; cached session will not be renewable"
        );
    }
    let _ = sender.send(LoginEvent::Completed(account));
    tracing::info!(
        target: "vertexlauncher/auth/device_code",
        "device-code login worker completed successfully"
    );

    Ok(())
}
