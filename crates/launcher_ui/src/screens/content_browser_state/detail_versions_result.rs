use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DetailVersionsResult {
    pub(crate) project_key: String,
    pub(crate) versions: Result<Vec<BrowserVersionEntry>, String>,
}
