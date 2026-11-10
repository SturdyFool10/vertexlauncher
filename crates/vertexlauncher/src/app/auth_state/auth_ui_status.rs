use vertex_i18n::tr;
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
    pub(super) fn status_message(&self) -> Option<std::borrow::Cow<'_, str>> {
        match self {
            AuthUiStatus::Idle => None,
            AuthUiStatus::RefreshingCachedSession => Some(tr!("Refreshing cached account session...")),
            AuthUiStatus::RefreshingActiveSession => Some(tr!("Refreshing account token...")),
            AuthUiStatus::Starting => Some(tr!("Preparing Microsoft sign-in...")),
            AuthUiStatus::AwaitingBrowser => {
                Some(tr!("Complete sign-in in the Microsoft webview window..."))
            }
            AuthUiStatus::AwaitingExternalBrowser => {
                Some(tr!("Complete sign-in in your default browser..."))
            }
            AuthUiStatus::AwaitingDeviceCode(message) => Some(message.as_str().into()),
            AuthUiStatus::WaitingForAuthorization => Some(tr!("Finalizing sign-in...")),
            AuthUiStatus::Error(message) => Some(message.as_str().into()),
        }
    }
}
