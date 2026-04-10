use super::*;

/// Returns the full normalized texture rectangle.
///
/// The result always spans the inclusive `[0.0, 1.0]` UV range. This function does not panic.
pub(super) fn full_uv_rect() -> Rect {
    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

fn full_face_uvs() -> FaceUvs {
    let full = full_uv_rect();
    FaceUvs {
        top: full,
        bottom: full,
        left: full,
        right: full,
        front: full,
        back: full,
    }
}

/// Returns the default cape UV layout for the vanilla `64x32` cape texture.
///
/// Falls back to full-face UVs if the built-in layout cannot be constructed. This function does not panic.
pub(super) fn default_cape_uv_layout() -> FaceUvs {
    cape_uv_layout([64, 32]).unwrap_or_else(full_face_uvs)
}

/// Returns the default elytra wing UV layout for the vanilla `64x32` texture.
///
/// Falls back to full-face UVs for both wings if the built-in layout cannot be constructed.
/// This function does not panic.
pub(super) fn default_elytra_wing_uvs() -> ElytraWingUvs {
    elytra_wing_uvs([64, 32]).unwrap_or(ElytraWingUvs {
        left: full_face_uvs(),
        right: full_face_uvs(),
    })
}

/// Builds the left and right wing UVs for an elytra texture.
///
/// `texture_size` must be at least `[46, 22]` pixels to contain the vanilla elytra layout.
/// Returns `None` for smaller textures. This function does not panic.
pub(super) fn elytra_wing_uvs(texture_size: [u32; 2]) -> Option<ElytraWingUvs> {
    if texture_size[0] < 46 || texture_size[1] < 22 {
        return None;
    }
    let inset = 0.0;
    let left = FaceUvs {
        top: flip_uv_rect_x(uv_rect_with_inset(texture_size, 24, 0, 10, 2, inset)),
        bottom: flip_uv_rect_x(uv_rect_with_inset(texture_size, 34, 1, 10, 2, inset)),
        left: flip_uv_rect_x(uv_rect_with_inset(texture_size, 34, 2, 2, 20, inset)),
        right: flip_uv_rect_x(uv_rect_with_inset(texture_size, 22, 2, 2, 20, inset)),
        front: flip_uv_rect_x(uv_rect_with_inset(texture_size, 24, 2, 10, 20, inset)),
        back: flip_uv_rect_x(uv_rect_with_inset(texture_size, 36, 2, 10, 20, inset)),
    };
    let right = FaceUvs {
        top: uv_rect_with_inset(texture_size, 24, 0, 10, 2, inset),
        bottom: uv_rect_with_inset(texture_size, 34, 1, 10, 2, inset),
        left: uv_rect_with_inset(texture_size, 22, 2, 2, 20, inset),
        right: uv_rect_with_inset(texture_size, 34, 2, 2, 20, inset),
        front: uv_rect_with_inset(texture_size, 24, 2, 10, 20, inset),
        back: uv_rect_with_inset(texture_size, 36, 2, 10, 20, inset),
    };
    Some(ElytraWingUvs { left, right })
}

/// Returns the outer back-face UV for a vanilla cape layout.
///
/// `texture_size` must be at least `[22, 17]` pixels. Returns `None` for smaller textures.
/// This function does not panic.
pub(super) fn cape_outer_face_uv(texture_size: [u32; 2]) -> Option<Rect> {
    if texture_size[0] < 22 || texture_size[1] < 17 {
        return None;
    }
    Some(uv_rect_with_inset(
        texture_size,
        1,
        1,
        10,
        16,
        UV_EDGE_INSET_BASE_TEXELS,
    ))
}

/// Builds the six-face UV layout for a vanilla-style cape texture.
///
/// `texture_size` must be at least `[22, 17]` pixels. Returns `None` for smaller textures.
/// This function does not panic.
pub(super) fn cape_uv_layout(texture_size: [u32; 2]) -> Option<FaceUvs> {
    let outer = cape_outer_face_uv(texture_size)?;
    let inner = uv_rect_with_inset(texture_size, 12, 1, 10, 16, UV_EDGE_INSET_BASE_TEXELS);
    Some(FaceUvs {
        top: uv_rect_with_inset(texture_size, 1, 0, 10, 1, UV_EDGE_INSET_BASE_TEXELS),
        bottom: uv_rect_with_inset(texture_size, 11, 0, 10, 1, UV_EDGE_INSET_BASE_TEXELS),
        left: uv_rect_with_inset(texture_size, 0, 1, 1, 16, UV_EDGE_INSET_BASE_TEXELS),
        right: uv_rect_with_inset(texture_size, 11, 1, 1, 16, UV_EDGE_INSET_BASE_TEXELS),
        front: inner,
        back: outer,
    })
}
