use super::*;

pub(super) enum ScreenshotScanMessage {
    EntryLoaded {
        request_id: u64,
        entry: ScreenshotEntry,
    },
    TaskDone {
        request_id: u64,
    },
}
