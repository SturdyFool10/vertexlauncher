use super::*;

#[path = "skins_preview_gpu_geometry/cape_triangles.rs"]
mod cape_triangles;
#[path = "skins_preview_gpu_geometry/elytra_pose.rs"]
mod elytra_pose;
#[path = "skins_preview_gpu_geometry/elytra_triangles.rs"]
mod elytra_triangles;
#[path = "skins_preview_gpu_geometry/elytra_wing_pose.rs"]
mod elytra_wing_pose;
#[path = "skins_preview_gpu_geometry/elytra_wing_uvs.rs"]
mod elytra_wing_uvs;

pub(in super::super) use self::cape_triangles::add_cape_triangles;
use self::elytra_pose::ElytraPose;
pub(in super::super) use self::elytra_triangles::add_elytra_triangles;
use self::elytra_wing_pose::ElytraWingPose;
pub(in super::super) use self::elytra_wing_uvs::ElytraWingUvs;

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
