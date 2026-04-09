/// Events emitted during cached account token renewal operations.
#[derive(Debug, Clone)]
pub enum CachedAccountRenewalEvent {
    /// Renewal process has started for this account.
    Started {
        profile_id: String,
        display_name: String,
    },
    /// Renewal completed successfully.
    Succeeded {
        profile_id: String,
        display_name: String,
    },
    /// Renewal failed with the provided error message.
    Failed {
        profile_id: String,
        display_name: String,
        error: String,
    },
}
