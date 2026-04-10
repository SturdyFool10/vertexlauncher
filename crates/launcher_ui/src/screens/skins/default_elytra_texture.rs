use super::*;

/// Builds the fallback elytra texture used when no texture asset is available.
///
/// The function first tries the embedded PNG asset and otherwise synthesizes a `64x32`
/// placeholder image. This function does not panic.
pub(super) fn default_elytra_texture_image() -> RgbaImage {
    const DEFAULT_ELYTRA_TEXTURE_PNG: &[u8] = include_bytes!("../../assets/default_elytra.png");
    if let Some(image) = decode_generic_rgba(DEFAULT_ELYTRA_TEXTURE_PNG) {
        return image;
    }
    let mut image = RgbaImage::from_pixel(64, 32, image::Rgba([0, 0, 0, 0]));
    let base = image::Rgba([141, 141, 141, 255]);
    let edge = image::Rgba([112, 112, 112, 255]);
    fill_rect_rgba(&mut image, 22, 0, 24, 22, base);
    fill_rect_rgba(&mut image, 22, 0, 24, 1, edge);
    fill_rect_rgba(&mut image, 22, 21, 24, 1, edge);
    fill_rect_rgba(&mut image, 22, 0, 1, 22, edge);
    fill_rect_rgba(&mut image, 45, 0, 1, 22, edge);
    image
}

fn fill_rect_rgba(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: image::Rgba<u8>,
) {
    let max_x = image.width();
    let max_y = image.height();
    for py in y..y.saturating_add(height).min(max_y) {
        for px in x..x.saturating_add(width).min(max_x) {
            image.put_pixel(px, py, color);
        }
    }
}
