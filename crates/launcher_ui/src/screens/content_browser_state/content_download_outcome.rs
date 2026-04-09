#[derive(Clone, Debug)]
pub(crate) struct ContentDownloadOutcome {
    pub(crate) project_name: String,
    pub(crate) added_files: Vec<String>,
    pub(crate) removed_files: Vec<String>,
}
