use super::*;

#[derive(Clone, Debug)]
pub(super) enum WorkerEvent {
    Refreshed(Result<(String, LoadedProfile), String>),
    Saved(Result<(String, LoadedProfile), String>),
}
