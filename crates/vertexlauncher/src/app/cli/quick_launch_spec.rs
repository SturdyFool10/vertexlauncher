use super::*;

#[derive(Debug)]
pub(super) struct QuickLaunchSpec {
    pub(super) mode: QuickLaunchMode,
    pub(super) instance: String,
    pub(super) user: String,
    pub(super) world: Option<String>,
    pub(super) server: Option<String>,
}
