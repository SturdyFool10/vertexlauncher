use super::*;

#[derive(Clone)]
pub(super) struct CachedConsoleLineLayout {
    pub(super) fingerprint: u64,
    pub(super) galley: Arc<egui::Galley>,
    pub(super) line_len_chars: usize,
    pub(super) last_used_frame: u64,
}
