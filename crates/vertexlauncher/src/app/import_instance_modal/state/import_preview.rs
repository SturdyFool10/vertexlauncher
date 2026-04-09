use super::*;

#[derive(Clone, Debug)]
pub(crate) struct ImportPreview {
    pub(crate) kind: ImportPreviewKind,
    pub(crate) detected_name: String,
    pub(crate) game_version: String,
    pub(crate) modloader: String,
    pub(crate) modloader_version: String,
    pub(crate) summary: String,
}
