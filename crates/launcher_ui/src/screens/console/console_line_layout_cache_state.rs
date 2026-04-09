use super::*;

#[derive(Default)]
pub(super) struct ConsoleLineLayoutCacheState {
    pub(super) entries: HashMap<egui::Id, CachedConsoleLineLayout>,
    pub(super) last_eviction_frame: u64,
}
