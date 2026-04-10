use super::*;

pub(super) fn preview_mip_level_count(size: [u32; 2]) -> u32 {
    size[0].max(size[1]).max(1).ilog2() + 1
}

/// Uploads a full mip chain for an RGBA preview texture.
///
/// The source image may have any non-zero or zero dimensions accepted by `RgbaImage`;
/// each mip level is clamped to at least `1x1` before upload.
///
/// This function does not panic.
pub(super) fn upload_preview_texture_mips(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    image: &RgbaImage,
) {
    let mut mip_image = image.clone();
    let mip_level_count = preview_mip_level_count([image.width(), image.height()]);

    for mip_level in 0..mip_level_count {
        let width = mip_image.width().max(1);
        let height = mip_image.height().max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            mip_image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if mip_level + 1 < mip_level_count {
            let next_width = (width / 2).max(1);
            let next_height = (height / 2).max(1);
            mip_image = resize_preview_mip_image(&mip_image, next_width, next_height);
        }
    }
}

fn resize_preview_mip_image(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let mut premultiplied = image.clone();
    for pixel in premultiplied.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }

    let mut resized = image::imageops::resize(&premultiplied, width, height, FilterType::Triangle);
    for pixel in resized.pixels_mut() {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            continue;
        }

        let scale = 255.0 / f32::from(alpha);
        pixel[0] = (f32::from(pixel[0]) * scale).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (f32::from(pixel[1]) * scale).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (f32::from(pixel[2]) * scale).round().clamp(0.0, 255.0) as u8;
    }

    resized
}
