use super::*;

#[derive(Clone)]
pub(super) struct VisibleLogRowHit {
    pub(super) line_index: usize,
    pub(super) rect: egui::Rect,
    pub(super) text_rect: egui::Rect,
    pub(super) galley: Arc<egui::Galley>,
    pub(super) line_len_chars: usize,
}
