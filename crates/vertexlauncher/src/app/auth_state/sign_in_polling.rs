use vertex_i18n::tr;
use super::*;

impl AuthState {
    /// Drains events from the interactive (webview / system browser) sign-in worker.
    pub(super) fn poll_interactive_flow(&mut self) {
        let mut flow_finished = false;

        while self.flow.is_some() {
            let next_event = {
                let Some(flow) = self.flow.as_mut() else {
                    break;
                };
                flow.try_recv()
            };
            match next_event {
                Ok(event) => match event {
                    AuthFlowEvent::AwaitingBrowser => {
                        self.status = AuthUiStatus::AwaitingBrowser;
                    }
                    AuthFlowEvent::AwaitingExternalBrowser => {
                        self.status = AuthUiStatus::AwaitingExternalBrowser;
                    }
                    AuthFlowEvent::WaitingForAuthorization => {
                        self.status = AuthUiStatus::WaitingForAuthorization;
                    }
                    AuthFlowEvent::Completed(account) => {
                        self.failed_account_errors
                            .remove(&account.minecraft_profile.id);
                        self.accounts_state.upsert_and_activate(account);
                        self.sync_account_avatar_cache();
                        self.rebuild_active_avatar_png();
                        self.status = AuthUiStatus::Idle;

                        self.save_accounts_in_background(
                            "Sign-in succeeded, but failed to cache account state",
                        );

                        flow_finished = true;
                        self.schedule_missing_avatars();
                    }
                    AuthFlowEvent::Failed(err) => {
                        tracing::error!(
                            target: "vertexlauncher/auth_state",
                            error = %err,
                            "Interactive sign-in flow failed."
                        );
                        notification::error!("auth", "{err}", err = err);
                        self.status = AuthUiStatus::Error(err);
                        flow_finished = true;
                    }
                },
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !flow_finished {
                        tracing::error!(
                            target: "vertexlauncher/auth_state",
                            "Interactive sign-in worker disconnected before completion."
                        );
                        notification::error!("auth", "Sign-in stopped unexpectedly before completion");
                        self.status = AuthUiStatus::Error(
                            tr!("Sign-in stopped unexpectedly before completion").into_owned(),
                        );
                    }
                    flow_finished = true;
                    break;
                }
            }
        }

        if flow_finished {
            self.flow = None;
        }
    }

    /// Advances the device-code flow, restarting it when its code expires.
    pub(super) fn poll_device_code_flow(&mut self) {
        // Proactively restart device code flow when the code expires
        if self.device_code_flow.is_some()
            && self
                .device_code_expiry
                .is_some_and(|exp| exp <= Instant::now())
        {
            self.device_code_flow = None;
            self.device_code_prompt = None;
            self.device_code_expiry = None;
            self.restart_device_code_sign_in();
        }

        if let Some(device_flow) = self.device_code_flow.as_mut() {
            let mut device_flow_finished = false;
            for event in device_flow.poll_events() {
                match event {
                    LoginEvent::DeviceCode(prompt) => {
                        self.status = AuthUiStatus::AwaitingDeviceCode(tr!("Go to {verification_uri} and enter code: {user_code}", verification_uri = prompt.verification_uri, user_code = prompt.user_code).into_owned());
                        self.device_code_expiry =
                            Some(Instant::now() + Duration::from_secs(prompt.expires_in_secs));
                        self.device_code_prompt = Some(prompt);
                    }
                    LoginEvent::WaitingForAuthorization => {
                        self.status = AuthUiStatus::WaitingForAuthorization;
                    }
                    LoginEvent::Completed(account) => {
                        self.failed_account_errors
                            .remove(&account.minecraft_profile.id);
                        self.accounts_state.upsert_and_activate(account);
                        self.sync_account_avatar_cache();
                        self.rebuild_active_avatar_png();
                        self.status = AuthUiStatus::Idle;

                        self.save_accounts_in_background(
                            "Sign-in succeeded, but failed to cache account state",
                        );

                        device_flow_finished = true;
                        self.schedule_missing_avatars();
                    }
                    LoginEvent::Failed(err) => {
                        let expired = self
                            .device_code_expiry
                            .is_some_and(|exp| exp <= Instant::now());
                        if expired {
                            // Code timed out — restart silently with a fresh code
                            device_flow_finished = true;
                        } else {
                            notification::error!("auth", "{err}", err = err);
                            self.status = AuthUiStatus::Error(err);
                            device_flow_finished = true;
                        }
                    }
                }
            }
            if device_flow_finished {
                let should_restart = self
                    .device_code_expiry
                    .is_some_and(|exp| exp <= Instant::now());
                self.device_code_flow = None;
                self.device_code_prompt = None;
                self.device_code_expiry = None;
                if should_restart {
                    self.restart_device_code_sign_in();
                }
            }
        }
    }
}
