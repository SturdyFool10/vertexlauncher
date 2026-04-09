use super::*;

/// Snap a point-space width to the nearest WIDTH_BIN_PX device-pixel boundary.
#[inline]
pub(crate) fn snap_width_to_bin(width_points: f32, scale: f32) -> f32 {
    let w_px = (width_points * scale).round();
    let snapped_px = (w_px / WIDTH_BIN_PX).floor() * WIDTH_BIN_PX;
    (snapped_px / scale).max(1.0)
}

/// Snap a paint rect to the physical device-pixel grid so already-antialiased
/// glyph textures are not blurred again by fractional placement.
#[inline]
pub(crate) fn snap_rect_to_pixel_grid(rect: Rect, pixels_per_point: f32) -> Rect {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return rect;
    }

    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;

    Rect::from_min_max(
        Pos2::new(snap(rect.min.x), snap(rect.min.y)),
        Pos2::new(snap(rect.max.x), snap(rect.max.y)),
    )
}

pub(crate) fn egui_vec_from_text(vector: TextVector) -> Vec2 {
    vector.into()
}

pub(crate) fn egui_rect_from_text(rect: TextRect) -> Rect {
    rect.into()
}

pub(crate) fn egui_point_from_text(point: TextPoint) -> Pos2 {
    point.into()
}
