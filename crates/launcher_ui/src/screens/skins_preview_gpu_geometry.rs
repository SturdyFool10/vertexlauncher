use super::*;

#[path = "skins_preview_gpu_geometry/elytra_pose.rs"]
mod elytra_pose;
#[path = "skins_preview_gpu_geometry/elytra_wing_pose.rs"]
mod elytra_wing_pose;

use self::elytra_pose::ElytraPose;
use self::elytra_wing_pose::ElytraWingPose;

#[derive(Clone, Copy)]
pub(in super::super) struct ElytraWingUvs {
    pub(in super::super) left: FaceUvs,
    pub(in super::super) right: FaceUvs,
}

const VANILLA_ELYTRA_POSE_STANDING: ElytraPose = ElytraPose {
    left: ElytraWingPose {
        rotate_x: 0.261_799_4,
        rotate_y: -0.087_266_46,
        rotate_z: -0.261_799_4,
        pivot_y_offset: 0.0,
    },
    right: ElytraWingPose {
        rotate_x: 0.261_799_4,
        rotate_y: 0.087_266_46,
        rotate_z: 0.261_799_4,
        pivot_y_offset: 0.0,
    },
};

const VANILLA_ELYTRA_POSE_SNEAKING: ElytraPose = ElytraPose {
    left: ElytraWingPose {
        rotate_x: 0.698_131_7,
        rotate_y: 0.087_266_46,
        rotate_z: -0.785_398_2,
        pivot_y_offset: 3.0,
    },
    right: ElytraWingPose {
        rotate_x: 0.698_131_7,
        rotate_y: -0.087_266_46,
        rotate_z: 0.785_398_2,
        pivot_y_offset: 3.0,
    },
};

const VANILLA_ELYTRA_POSE_GLIDE_OPEN: ElytraPose = ElytraPose {
    left: ElytraWingPose {
        rotate_x: 0.349_065_84,
        rotate_y: 0.0,
        rotate_z: -std::f32::consts::FRAC_PI_2,
        pivot_y_offset: 0.0,
    },
    right: ElytraWingPose {
        rotate_x: 0.349_065_84,
        rotate_y: 0.0,
        rotate_z: std::f32::consts::FRAC_PI_2,
        pivot_y_offset: 0.0,
    },
};

pub(in super::super) fn add_cape_triangles(
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

pub(in super::super) fn add_elytra_triangles(
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
