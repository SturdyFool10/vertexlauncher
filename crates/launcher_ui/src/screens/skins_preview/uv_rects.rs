use super::*;

pub(in super::super) fn uv_rect(x: u32, y: u32, w: u32, h: u32) -> Rect {
    uv_rect_with_inset([64, 64], x, y, w, h, UV_EDGE_INSET_BASE_TEXELS)
}

pub(in super::super) fn uv_rect_overlay(x: u32, y: u32, w: u32, h: u32) -> Rect {
    uv_rect_with_inset([64, 64], x, y, w, h, UV_EDGE_INSET_OVERLAY_TEXELS)
}

/// Builds a normalized UV rectangle and shrinks its edges inward to avoid texel bleed.
///
/// Zero values in `texture_size` are clamped to `1`. `w` and `h` should describe a valid
/// texture region width and height in texels.
///
/// This function does not panic.
pub(in super::super) fn uv_rect_with_inset(
    texture_size: [u32; 2],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    inset_texels: f32,
) -> Rect {
    let tex_w = texture_size[0].max(1) as f32;
    let tex_h = texture_size[1].max(1) as f32;
    let max_inset_x = ((w as f32) * 0.49) / tex_w;
    let max_inset_y = ((h as f32) * 0.49) / tex_h;
    let inset_x = (inset_texels / tex_w).min(max_inset_x);
    let inset_y = (inset_texels / tex_h).min(max_inset_y);
    let min_x = (x as f32 / tex_w) + inset_x;
    let min_y = (y as f32 / tex_h) + inset_y;
    let max_x = ((x + w) as f32 / tex_w) - inset_x;
    let max_y = ((y + h) as f32 / tex_h) - inset_y;
    Rect::from_min_max(egui::pos2(min_x, min_y), egui::pos2(max_x, max_y))
}

pub(in super::super) fn flip_uv_rect_x(rect: Rect) -> Rect {
    Rect::from_min_max(
        egui::pos2(rect.max.x, rect.min.y),
        egui::pos2(rect.min.x, rect.max.y),
    )
}
