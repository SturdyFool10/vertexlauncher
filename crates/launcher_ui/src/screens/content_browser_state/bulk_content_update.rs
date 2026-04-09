use super::*;

#[derive(Clone, Debug)]
pub(crate) struct BulkContentUpdate {
    pub entry: UnifiedContentEntry,
    pub installed_file_path: PathBuf,
    pub version_id: String,
}
