#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveContentDownload {
    pub(crate) dedupe_key: String,
    pub(crate) version_id: Option<String>,
}
