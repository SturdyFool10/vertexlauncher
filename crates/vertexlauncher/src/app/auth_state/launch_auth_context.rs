#[derive(Clone)]
pub struct LaunchAuthContext {
    pub account_key: String,
    pub player_name: String,
    pub player_uuid: String,
    pub access_token: Option<String>,
    pub xuid: Option<String>,
    pub user_type: String,
}

impl std::fmt::Debug for LaunchAuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchAuthContext")
            .field("account_key", &self.account_key)
            .field("player_name", &self.player_name)
            .field("player_uuid", &self.player_uuid)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[redacted]"),
            )
            .field("xuid", &self.xuid)
            .field("user_type", &self.user_type)
            .finish()
    }
}
