use super::*;

pub(crate) struct CpuSceneAtlasPage {
    pub(crate) allocator: AtlasAllocator,
    /// Raw RGBA8 pixel bytes stored as Vec so reset_for_size can zero in-place without
    /// Arc refcount contention.
    pub(crate) rgba8: Vec<u8>,
    pub(crate) size: [usize; 2],
}

impl CpuSceneAtlasPage {
    pub(crate) fn new_for_size(side: usize) -> Self {
        let byte_count = side * side * 4;
        Self {
            allocator: AtlasAllocator::new(size2(side as i32, side as i32)),
            rgba8: vec![0u8; byte_count],
            size: [side, side],
        }
    }

    /// Reset for reuse from the pool. Zeroes pixels in-place when the size matches (0 allocs);
    /// resizes the Vec only when the page side changes.
    pub(crate) fn reset_for_size(&mut self, side: usize) {
        self.allocator = AtlasAllocator::new(size2(side as i32, side as i32));
        self.size = [side, side];
        let byte_count = side * side * 4;
        if self.rgba8.len() == byte_count {
            self.rgba8.fill(0);
        } else {
            self.rgba8.clear();
            self.rgba8.resize(byte_count, 0);
        }
    }
}
