use super::*;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TextAtlasPageSnapshot {
    pub page_index: usize,
    pub size_px: [usize; 2],
    pub rgba8: Vec<u8>,
}

impl TextAtlasPageSnapshot {
    pub fn to_rgba8(&self) -> TextAtlasPageData {
        let mut hasher = rustc_hash::FxHasher::default();
        self.page_index.hash(&mut hasher);
        self.size_px.hash(&mut hasher);
        hasher.write(&self.rgba8);
        TextAtlasPageData {
            page_index: self.page_index,
            size_px: self.size_px,
            content_hash: hasher.finish(),
            rgba8: Arc::from(self.rgba8.as_slice()),
        }
    }
}
