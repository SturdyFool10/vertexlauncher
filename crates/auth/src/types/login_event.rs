use crate::types::{CachedAccount, DeviceCodePrompt};

/// Events produced by async device-code login polling.
#[derive(Debug, Clone)]
pub enum LoginEvent {
    /// Device code prompt received - user must enter the code.
    DeviceCode(DeviceCodePrompt),
    /// Waiting for user to complete authorization on their device.
    WaitingForAuthorization,
    /// Login completed successfully with the cached account data.
    Completed(CachedAccount),
    /// Login failed with the provided error message.
    Failed(String),
}
