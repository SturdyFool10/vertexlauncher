use vertex_i18n::tr;
use std::sync::Arc;

use image::{
    ColorType, DynamicImage, ImageEncoder, ImageFormat, RgbaImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

use crate::app::tokio_runtime;

pub async fn load_image_path_for_memory(path: std::path::PathBuf) -> Result<Arc<[u8]>, String> {
    let path_label = path.display().to_string();
    let bytes = launcher_runtime::fs::read(path.as_path())
        .await
        .map_err(|err| tr!("failed to read '{path_label}': {err}", path_label = path_label, err = err).into_owned())?;

    if should_recompress_for_memory(bytes.as_slice()) {
        let task =
            tokio_runtime::spawn_blocking(move || prepare_owned_image_bytes_for_memory(bytes));
        task.await
            .map_err(|err| tr!("failed to optimize '{path_label}' for memory: {err}", path_label = path_label, err = err).into_owned())
    } else {
        Ok(Arc::<[u8]>::from(bytes.into_boxed_slice()))
    }
}

pub fn prepare_owned_image_bytes_for_memory(bytes: Vec<u8>) -> Arc<[u8]> {
    if !should_recompress_for_memory(bytes.as_slice()) {
        return Arc::<[u8]>::from(bytes.into_boxed_slice());
    }

    let Ok(decoded) = image::load_from_memory(bytes.as_slice()) else {
        return Arc::<[u8]>::from(bytes.into_boxed_slice());
    };

    Arc::<[u8]>::from(compress_dynamic_image_for_memory(decoded, Some(bytes)).into_boxed_slice())
}

pub fn should_recompress_for_memory(bytes: &[u8]) -> bool {
    matches!(
        image::guess_format(bytes).ok(),
        Some(ImageFormat::Bmp)
            | Some(ImageFormat::Tga)
            | Some(ImageFormat::Pnm)
            | Some(ImageFormat::Farbfeld)
    )
}

pub fn compress_dynamic_image_for_memory(
    image: DynamicImage,
    original_bytes: Option<Vec<u8>>,
) -> Vec<u8> {
    compress_rgba_image_for_memory(image.to_rgba8(), original_bytes)
}

pub fn compress_rgba_image_for_memory(
    image: RgbaImage,
    original_bytes: Option<Vec<u8>>,
) -> Vec<u8> {
    let width = image.width();
    let height = image.height();
    let pixels = image.as_raw();

    let mut best = original_bytes.unwrap_or_default();
    let best_len = if best.is_empty() {
        usize::MAX
    } else {
        best.len()
    };

    if let Some(png) = encode_png_best(width, height, pixels)
        && png.len() < best_len
    {
        best = png;
    }

    if best.is_empty() {
        encode_png_best(width, height, pixels).unwrap_or_else(|| pixels.clone())
    } else {
        best
    }
}

fn encode_png_best(width: u32, height: u32, pixels: &[u8]) -> Option<Vec<u8>> {
    let mut encoded = Vec::new();
    PngEncoder::new_with_quality(&mut encoded, CompressionType::Best, FilterType::Adaptive)
        .write_image(pixels, width, height, ColorType::Rgba8)
        .ok()?;
    Some(encoded)
}
