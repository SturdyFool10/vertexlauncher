use super::*;

/// Adds a voxelized overlay layer for the provided skin regions.
///
/// Regions that fall outside the image bounds or point at fully transparent texels are
/// skipped. `regions` may be empty.
///
/// This function does not panic.
pub(super) fn add_voxel_overlay_layer(
    out: &mut Vec<RenderTriangle>,
    image: &RgbaImage,
    part: OverlayPartSpec,
    regions: &[OverlayRegionSpec],
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
) {
    const VOXEL_THICKNESS: f32 = 0.92;
    const VOXEL_GAP: f32 = 0.08;

    for region in regions {
        for row in 0..region.height {
            for col in 0..region.width {
                let tex_x = region.tex_x + col;
                let tex_y = region.tex_y + row;
                if tex_x >= image.width() || tex_y >= image.height() {
                    continue;
                }
                if image.get_pixel(tex_x, tex_y).0[3] == 0 {
                    continue;
                }

                let uv =
                    uv_rect_with_inset([image.width(), image.height()], tex_x, tex_y, 1, 1, 0.02);
                let voxel_uv = FaceUvs {
                    top: uv,
                    bottom: uv,
                    left: uv,
                    right: uv,
                    front: uv,
                    back: uv,
                };
                let (size, local_center) = overlay_voxel_geometry(
                    part.size,
                    region.face,
                    col,
                    row,
                    VOXEL_THICKNESS,
                    VOXEL_GAP,
                );

                add_cuboid_triangles_with_y(
                    out,
                    TriangleTexture::Skin,
                    CuboidSpec {
                        size,
                        pivot_top_center: part.pivot_top_center,
                        rotate_x: part.rotate_x,
                        rotate_z: part.rotate_z,
                        uv: voxel_uv,
                        cull_backfaces: false,
                    },
                    camera,
                    projection,
                    rect,
                    light_dir,
                    0.0,
                    local_center,
                );
            }
        }
    }
}

fn overlay_voxel_geometry(
    part_size: Vec3,
    face: OverlayVoxelFace,
    col: u32,
    row: u32,
    thickness: f32,
    gap: f32,
) -> (Vec3, Vec3) {
    let half_w = part_size.x * 0.5;
    let half_d = part_size.z * 0.5;
    let half_t = thickness * 0.5;
    match face {
        OverlayVoxelFace::Front => (
            Vec3::new(1.0, 1.0, thickness),
            Vec3::new(
                -half_w + col as f32 + 0.5,
                -(row as f32) - 0.5,
                half_d + gap + half_t,
            ),
        ),
        OverlayVoxelFace::Back => (
            Vec3::new(1.0, 1.0, thickness),
            Vec3::new(
                half_w - col as f32 - 0.5,
                -(row as f32) - 0.5,
                -half_d - gap - half_t,
            ),
        ),
        OverlayVoxelFace::Left => (
            Vec3::new(thickness, 1.0, 1.0),
            Vec3::new(
                -half_w - gap - half_t,
                -(row as f32) - 0.5,
                -half_d + col as f32 + 0.5,
            ),
        ),
        OverlayVoxelFace::Right => (
            Vec3::new(thickness, 1.0, 1.0),
            Vec3::new(
                half_w + gap + half_t,
                -(row as f32) - 0.5,
                half_d - col as f32 - 0.5,
            ),
        ),
        OverlayVoxelFace::Top => (
            Vec3::new(1.0, thickness, 1.0),
            Vec3::new(
                -half_w + col as f32 + 0.5,
                gap + half_t,
                -half_d + row as f32 + 0.5,
            ),
        ),
        OverlayVoxelFace::Bottom => (
            Vec3::new(1.0, thickness, 1.0),
            Vec3::new(
                -half_w + col as f32 + 0.5,
                -part_size.y - gap - half_t,
                half_d - row as f32 - 0.5,
            ),
        ),
    }
}
