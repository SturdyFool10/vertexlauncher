#[derive(Clone, Debug)]
pub struct AccountUiEntry {
    pub profile_id: String,
    pub display_name: String,
    pub is_active: bool,
    pub is_failed: bool,
}
