/// Instructions shown to user during device-code sign-in.
#[derive(Debug, Clone)]
pub struct DeviceCodePrompt {
    /// The code the user must enter on the verification page.
    pub user_code: String,
    /// Base URI for device verification (e.g., "https://microsoft.com/devicelogin").
    pub verification_uri: String,
    /// Complete verification URL including the device code parameter.
    pub verification_uri_complete: Option<String>,
    /// Number of seconds until this device code expires.
    pub expires_in_secs: u64,
    /// Recommended polling interval in seconds for token exchange.
    pub poll_interval_secs: u64,
    /// Human-readable instructions message for the user.
    pub message: String,
}

impl DeviceCodePrompt {
    /// Returns the verification URL to display to the user.
    ///
    /// Prefers `verification_uri_complete` if present and non-empty, otherwise falls back to `verification_uri`.
    pub fn verification_url(&self) -> String {
        self.verification_uri_complete
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(self.verification_uri.as_str())
            .to_owned()
    }
}
