#[derive(Debug, Clone)]
pub struct LaunchAuthContext {
    pub account_key: String,
    pub player_name: String,
    pub player_uuid: String,
    pub access_token: Option<String>,
    pub xuid: Option<String>,
    pub user_type: String,
}
