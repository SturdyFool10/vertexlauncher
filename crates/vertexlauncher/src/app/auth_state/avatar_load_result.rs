pub(super) struct AvatarLoadResult {
    pub(super) profile_id: String,
    pub(super) avatar_png: Option<Vec<u8>>,
    pub(super) error: Option<String>,
}
