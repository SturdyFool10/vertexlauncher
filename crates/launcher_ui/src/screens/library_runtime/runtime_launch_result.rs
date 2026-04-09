use super::*;

#[derive(Debug, Clone)]
pub(super) struct RuntimeLaunchResult {
    pub(super) instance_id: String,
    pub(super) result: Result<RuntimeLaunchOutcome, String>,
}
