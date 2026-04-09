use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct BrowserSearchSnapshot {
    pub(crate) entries: Vec<BrowserProjectEntry>,
    pub(crate) warnings: Vec<String>,
}
