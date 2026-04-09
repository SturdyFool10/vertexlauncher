use super::*;

#[derive(Clone, Debug)]
pub(crate) enum SearchUpdate {
    Snapshot {
        request: BrowserSearchRequest,
        snapshot: BrowserSearchSnapshot,
        completed_tasks: usize,
        total_tasks: usize,
        finished: bool,
    },
    Failed {
        request: BrowserSearchRequest,
        error: String,
    },
}
