use super::*;

/// Hashes the full RGBA payload and dimensions of a preview image for cache reuse.
///
/// Any `RgbaImage` dimensions are accepted. Equal dimensions and identical raw bytes
/// produce identical hashes within the current hashing implementation.
///
/// This function does not panic.
pub(in super::super) fn hash_preview_image(image: &RgbaImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.width().hash(&mut hasher);
    image.height().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}
