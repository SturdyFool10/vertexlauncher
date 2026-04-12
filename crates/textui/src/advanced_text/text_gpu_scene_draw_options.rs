use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGpuSceneDrawOptions {
    pub offset: TextPoint,
    pub scale: TextVector,
    pub tint: TextColor,
}

impl TextGpuSceneDrawOptions {
    pub const IDENTITY: Self = Self {
        offset: TextPoint::ZERO,
        scale: TextVector::splat(1.0),
        tint: TextColor::WHITE,
    };
}

impl Default for TextGpuSceneDrawOptions {
    fn default() -> Self {
        Self::IDENTITY
    }
}
