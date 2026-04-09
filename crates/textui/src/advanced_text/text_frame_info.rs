#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextFrameInfo {
    pub frame_number: u64,
    pub max_texture_side_px: usize,
}

impl TextFrameInfo {
    pub const fn new(frame_number: u64, max_texture_side_px: usize) -> Self {
        Self {
            frame_number,
            max_texture_side_px,
        }
    }
}
