/// Browser/device-code OAuth session values required to complete login.
#[derive(Debug, Clone)]
pub struct MinecraftLoginFlow {
    pub verifier: String,
    pub auth_request_uri: String,
    pub redirect_uri: String,
    pub token_uri: String,
    pub scope: String,
    pub(crate) state: String,
    pub(crate) client_id: String,
}

impl MinecraftLoginFlow {
    /// Returns the expected OAuth state value for validation.
    pub fn expected_state(&self) -> &str {
        &self.state
    }
}
