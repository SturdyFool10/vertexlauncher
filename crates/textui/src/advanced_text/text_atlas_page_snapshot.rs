use super::*;

#[derive(Clone, Debug)]
pub struct TextAtlasPageSnapshot {
    pub page_index: usize,
    pub size_px: [usize; 2],
    pub rgba8: Vec<u8>,
}

impl TextAtlasPageSnapshot {
    pub fn to_rgba8(&self) -> TextAtlasPageData {
        TextAtlasPageData {
            page_index: self.page_index,
            size_px: self.size_px,
            rgba8: self.rgba8.clone(),
        }
    }
}
