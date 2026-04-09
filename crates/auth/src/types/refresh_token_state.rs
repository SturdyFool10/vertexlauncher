/// State of refresh token for cached account renewal tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefreshTokenState {
    /// Token state is unknown - not yet determined.
    #[default]
    Unknown,
    /// No refresh token available for this account.
    Missing,
    /// Refresh token is present and can be used for renewal.
    Present,
    /// Refresh token exists but is unavailable (e.g., expired or invalid).
    Unavailable,
}
