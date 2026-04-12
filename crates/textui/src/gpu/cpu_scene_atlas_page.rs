use super::*;

pub(crate) struct CpuSceneAtlasPage {
    pub(crate) allocator: AtlasAllocator,
    /// Raw RGBA8 pixel bytes, Arc-owned so TextAtlasPageData can share without copying.
    pub(crate) rgba8: Arc<[u8]>,
    pub(crate) size: [usize; 2],
}

impl CpuSceneAtlasPage {
    pub(crate) fn new_for_size(side: usize) -> Self {
        let byte_count = side * side * 4;
        Self {
            allocator: AtlasAllocator::new(size2(side as i32, side as i32)),
            rgba8: Arc::from(vec![0u8; byte_count].as_slice()),
            size: [side, side],
        }
    }

    /// Reset for reuse from the pool. Zeroes pixels in-place if the Arc is exclusively owned
    /// and the size matches; otherwise allocates a fresh buffer.
    pub(crate) fn reset_for_size(&mut self, side: usize) {
        self.allocator = AtlasAllocator::new(size2(side as i32, side as i32));
        self.size = [side, side];
        let byte_count = side * side * 4;
        if self.rgba8.len() == byte_count {
            if let Some(bytes) = Arc::get_mut(&mut self.rgba8) {
                bytes.fill(0);
                return;
            }
        }
        // Wrong size or still referenced by a cached TextAtlasPageData — create fresh.
        self.rgba8 = Arc::from(vec![0u8; byte_count].as_slice());
    }
}
