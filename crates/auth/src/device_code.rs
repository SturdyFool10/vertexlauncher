use std::sync::mpsc::Sender;

use crate::error::{AuthError, prefix_auth_error};
use crate::minecraft::complete_minecraft_login;
use crate::oauth::{oauth_tenant, poll_for_microsoft_token, request_device_code};
use crate::types::{DeviceCodePrompt, LoginEvent};
use crate::util::build_http_agent;

pub(crate) fn run_device_code_login(
    client_id: String,
    sender: &Sender<LoginEvent>,
) -> Result<(), AuthError> {
    tracing::info!(
        target: "vertexlauncher/auth/device_code",
        "starting device-code login worker"
    );
    let agent = build_http_agent();
    let tenant = oauth_tenant();

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
        poll_for_microsoft_token(&agent, &client_id, &tenant, &device_code, sender)
            .map_err(|err| prefix_auth_error("PollForMicrosoftToken", err))?;

    let account = complete_minecraft_login(&agent, &microsoft_token.access_token)?;
    let _ = sender.send(LoginEvent::Completed(account));
    tracing::info!(
        target: "vertexlauncher/auth/device_code",
        "device-code login worker completed successfully"
    );

    Ok(())
}
