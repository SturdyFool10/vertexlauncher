#[derive(Clone, Debug)]
pub struct AccountUiEntry {
    pub profile_id: String,
    pub display_name: String,
    pub is_active: bool,
    pub is_failed: bool,
    /// Where the account's skin can be downloaded from, if known (the head shown beside the name is drawn from it).
    pub skin_url: Option<String>,
}
