use super::*;

#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub source: ImportSource,
    pub instance_name: String,
    pub manual_curseforge_files: HashMap<u64, PathBuf>,
    pub manual_curseforge_staging_dir: Option<PathBuf>,
    pub max_concurrent_downloads: u32,
}
