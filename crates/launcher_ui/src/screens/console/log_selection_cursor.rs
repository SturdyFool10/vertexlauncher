#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct LogSelectionCursor {
    pub(super) line: usize,
    pub(super) char_index: usize,
}
