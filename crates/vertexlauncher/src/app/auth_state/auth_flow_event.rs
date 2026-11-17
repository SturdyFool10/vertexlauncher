use super::*;

pub(super) enum AuthFlowEvent {
    AwaitingExternalBrowser,
    WaitingForAuthorization,
    Completed(CachedAccount),
    Failed(String),
}
