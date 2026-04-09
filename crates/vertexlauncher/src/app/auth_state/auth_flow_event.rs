use super::*;

pub(super) enum AuthFlowEvent {
    AwaitingBrowser,
    AwaitingExternalBrowser,
    WaitingForAuthorization,
    Completed(CachedAccount),
    Failed(String),
}
