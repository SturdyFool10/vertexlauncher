use super::*;

#[derive(Debug, Clone)]
pub(super) struct RuntimeLaunchOutcome {
    pub(super) launch: LaunchResult,
    pub(super) downloaded_files: u32,
    pub(super) resolved_modloader_version: Option<String>,
    pub(super) configured_java: Option<(u8, String)>,
}
