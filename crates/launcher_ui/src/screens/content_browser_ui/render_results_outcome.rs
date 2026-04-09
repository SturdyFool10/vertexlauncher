use super::*;

#[derive(Default)]
pub(crate) struct RenderResultsOutcome {
    pub(crate) requested_page: Option<u32>,
    pub(crate) open_entry: Option<BrowserProjectEntry>,
}
