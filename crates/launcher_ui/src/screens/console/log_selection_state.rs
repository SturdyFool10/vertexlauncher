use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct LogSelectionState {
    pub(super) anchor: Option<LogSelectionCursor>,
    pub(super) head: Option<LogSelectionCursor>,
    pub(super) dragging: bool,
}

impl LogSelectionState {
    pub(super) fn normalized(&self) -> Option<(LogSelectionCursor, LogSelectionCursor)> {
        let anchor = self.anchor?;
        let head = self.head.unwrap_or(anchor);
        if anchor <= head {
            Some((anchor, head))
        } else {
            Some((head, anchor))
        }
    }

    pub(super) fn has_selection(&self) -> bool {
        matches!(self.normalized(), Some((start, end)) if start != end)
    }

    pub(super) fn clear(&mut self) {
        self.anchor = None;
        self.head = None;
        self.dragging = false;
    }
}
