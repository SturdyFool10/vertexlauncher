#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportMode {
    ManifestFile,
    LauncherDirectory,
}

impl ImportMode {
    pub(crate) fn from_index(index: usize) -> Self {
        match index {
            1 => Self::LauncherDirectory,
            _ => Self::ManifestFile,
        }
    }

    pub(crate) fn options() -> [&'static str; 2] {
        ["From manifest file", "Import from another launcher"]
    }
}
