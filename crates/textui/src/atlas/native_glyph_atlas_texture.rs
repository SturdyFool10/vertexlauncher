use super::*;

pub(super) struct NativeGlyphAtlasTexture {
    pub(super) id: TextureId,
    pub(super) texture: wgpu::Texture,
}
