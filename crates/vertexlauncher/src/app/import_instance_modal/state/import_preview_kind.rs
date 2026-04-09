use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportPreviewKind {
    Manifest(ImportPackageKind),
    Launcher(LauncherKind),
}

impl ImportPreviewKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ImportPreviewKind::Manifest(kind) => kind.label(),
            ImportPreviewKind::Launcher(kind) => kind.label(),
        }
    }
}
