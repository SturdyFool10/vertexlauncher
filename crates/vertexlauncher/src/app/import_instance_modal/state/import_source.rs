use super::*;

#[derive(Clone, Debug)]
pub enum ImportSource {
    ManifestFile(PathBuf),
    LauncherDirectory {
        path: PathBuf,
        launcher: Option<LauncherKind>,
    },
}
