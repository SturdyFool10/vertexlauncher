use super::*;

#[derive(Default)]
pub(super) struct ConsoleLogParseCacheState {
    pub(super) entries: HashMap<egui::Id, CachedConsoleLogParse>,
    pub(super) last_eviction_frame: u64,
}
