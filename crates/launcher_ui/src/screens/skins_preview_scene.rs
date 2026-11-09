use super::skins_preview_expressions::{
    FaceAnimationInput, add_expression_triangles, compute_face_pose,
};
use super::*;

#[path = "skins_preview_scene/built_character_scene.rs"]
mod built_character_scene;
#[path = "skins_preview_scene/character_scene_builder.rs"]
mod character_scene_builder;
#[path = "skins_preview_scene/overlay_part_spec.rs"]
mod overlay_part_spec;
#[path = "skins_preview_scene/overlay_region_spec.rs"]
mod overlay_region_spec;
#[path = "skins_preview_scene/overlay_voxel_face.rs"]
mod overlay_voxel_face;
#[path = "skins_preview_scene/voxel_overlay_layer.rs"]
mod voxel_overlay_layer;

use self::built_character_scene::BuiltCharacterScene;
pub(in crate::screens) use self::character_scene_builder::{build_character_scene, build_character_scene_camera};
use self::overlay_part_spec::OverlayPartSpec;
use self::overlay_region_spec::OverlayRegionSpec;
use self::overlay_voxel_face::OverlayVoxelFace;

#[cfg(test)]
#[path = "skins_preview_scene/character_scene_tests.rs"]
mod character_scene_tests;
