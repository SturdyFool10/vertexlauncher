use super::*;

pub(in super::super::super) fn add_cape_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    model_offset: Vec3,
    walk_phase: f32,
    cape_uv: FaceUvs,
    light_dir: Vec3,
) {
    let pivot = Vec3::new(0.0, 24.0, -2.55) + model_offset;
    let cape_tilt = 0.12 + walk_phase.abs() * 0.10;
    add_cuboid_triangles(
        out,
        texture,
        CuboidSpec {
            size: Vec3::new(10.0, 16.0, 1.0),
            pivot_top_center: pivot,
            rotate_x: cape_tilt,
            rotate_z: 0.0,
            uv: cape_uv,
            cull_backfaces: true,
        },
        camera,
        projection,
        rect,
        light_dir,
    );
}
