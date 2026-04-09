#[derive(Clone, Debug)]
pub enum AuthUiStatus {
    Idle,
    RefreshingCachedSession,
    RefreshingActiveSession,
    Starting,
    AwaitingBrowser,
    AwaitingExternalBrowser,
    AwaitingDeviceCode(String),
    WaitingForAuthorization,
    Error(String),
}

impl AuthUiStatus {
    pub(super) fn status_message(&self) -> Option<&str> {
        match self {
            AuthUiStatus::Idle => None,
            AuthUiStatus::RefreshingCachedSession => Some("Refreshing cached account session..."),
            AuthUiStatus::RefreshingActiveSession => Some("Refreshing account token..."),
            AuthUiStatus::Starting => Some("Preparing Microsoft sign-in..."),
            AuthUiStatus::AwaitingBrowser => {
                Some("Complete sign-in in the Microsoft webview window...")
            }
            AuthUiStatus::AwaitingExternalBrowser => {
                Some("Complete sign-in in your default browser...")
            }
            AuthUiStatus::AwaitingDeviceCode(message) => Some(message.as_str()),
            AuthUiStatus::WaitingForAuthorization => Some("Finalizing sign-in..."),
            AuthUiStatus::Error(message) => Some(message.as_str()),
        }
    }
}
