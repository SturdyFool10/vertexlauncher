use super::*;

/// Decodes a Minecraft skin PNG into RGBA pixels.
///
/// Valid skins must decode successfully as PNG and have dimensions `64x64` or `64x32`.
/// Returns `None` for invalid image bytes or unsupported dimensions. This function does not panic.
pub(super) fn decode_skin_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    if w == 64 && (h == 64 || h == 32) {
        Some(image)
    } else {
        None
    }
}

/// Decodes any supported raster image bytes into RGBA pixels.
///
/// Returns `None` when the bytes are not a supported image payload. This function does not panic.
pub(super) fn decode_generic_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| image.to_rgba8())
}

/// Reads image dimensions without enforcing a specific texture layout.
///
/// Returns `None` when the bytes cannot be decoded as an image. This function does not panic.
pub(super) fn decode_image_dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    let image = image::load_from_memory(bytes).ok()?;
    Some([image.width(), image.height()])
}
