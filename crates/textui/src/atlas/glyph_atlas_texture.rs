use super::*;

pub(super) enum GlyphAtlasTexture {
    Egui(TextureHandle),
    Wgpu(NativeGlyphAtlasTexture),
}

impl GlyphAtlasTexture {
    pub(super) fn id(&self) -> TextureId {
        match self {
            Self::Egui(texture) => texture.id(),
            Self::Wgpu(texture) => texture.id,
        }
    }
}
