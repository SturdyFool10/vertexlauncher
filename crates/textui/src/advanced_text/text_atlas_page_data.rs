use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TextAtlasPageData {
    pub page_index: usize,
    pub size_px: [usize; 2],
    pub content_hash: u64,
    pub rgba8: Arc<[u8]>,
}
