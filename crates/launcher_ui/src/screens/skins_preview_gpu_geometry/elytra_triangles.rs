use super::*;

pub(in super::super::super) fn add_elytra_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    model_offset: Vec3,
    _time_seconds: f32,
    walk_phase: f32,
    wing_uvs: ElytraWingUvs,
    light_dir: Vec3,
) {
    let neutral_pose = VANILLA_ELYTRA_POSE_STANDING;
    let _ = VANILLA_ELYTRA_POSE_SNEAKING;
    let _ = VANILLA_ELYTRA_POSE_GLIDE_OPEN;
    let left_leg_phase = walk_phase;
    let right_leg_phase = -walk_phase;
    let left_flap = neutral_pose.left.rotate_x + left_leg_phase * 0.10;
    let right_flap = neutral_pose.right.rotate_x + right_leg_phase * 0.10;
    let left_yaw = neutral_pose.left.rotate_y + left_leg_phase * 0.045;
    let right_yaw = neutral_pose.right.rotate_y + right_leg_phase * 0.045;
    let left_fold_z = neutral_pose.left.rotate_z;
    let right_fold_z = neutral_pose.right.rotate_z;

    let left_hinge_pivot =
        Vec3::new(-5.0, 24.1 + neutral_pose.left.pivot_y_offset, -2.1) + model_offset;
    let right_hinge_pivot =
        Vec3::new(5.0, 24.1 + neutral_pose.right.pivot_y_offset, -2.1) + model_offset;

    add_cuboid_triangles_with_y(
        out,
        texture,
        CuboidSpec {
            size: Vec3::new(10.0, 20.0, 2.0),
            pivot_top_center: left_hinge_pivot,
            rotate_x: left_flap,
            rotate_z: left_fold_z,
            uv: wing_uvs.left,
            cull_backfaces: false,
        },
        camera,
        projection,
        rect,
        light_dir,
        left_yaw,
        Vec3::new(5.0, 0.0, 0.0),
    );
    add_cuboid_triangles_with_y(
        out,
        texture,
        CuboidSpec {
            size: Vec3::new(10.0, 20.0, 2.0),
            pivot_top_center: right_hinge_pivot,
            rotate_x: right_flap,
            rotate_z: right_fold_z,
            uv: wing_uvs.right,
            cull_backfaces: false,
        },
        camera,
        projection,
        rect,
        light_dir,
        right_yaw,
        Vec3::new(-5.0, 0.0, 0.0),
    );
}
