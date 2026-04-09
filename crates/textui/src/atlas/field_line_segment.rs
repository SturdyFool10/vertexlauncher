#[derive(Clone, Copy)]
pub(super) struct FieldLineSegment {
    pub(super) a: [f32; 2],
    pub(super) b: [f32; 2],
    pub(super) color_mask: u8,
}
