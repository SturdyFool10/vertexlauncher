use super::voxel_overlay_layer::add_voxel_overlay_layer;
use super::*;

#[path = "character_scene_builder/back_attachment_scene_parts.rs"]
mod back_attachment_scene_parts;
#[path = "character_scene_builder/body_core_scene_parts.rs"]
mod body_core_scene_parts;
#[path = "character_scene_builder/character_scene_build_context.rs"]
mod character_scene_build_context;
#[path = "character_scene_builder/character_scene_motion.rs"]
mod character_scene_motion;
#[path = "character_scene_builder/limb_scene_parts.rs"]
mod limb_scene_parts;
#[path = "character_scene_builder/scene_camera.rs"]
mod scene_camera;

use self::back_attachment_scene_parts::add_back_attachment_scene_parts;
use self::body_core_scene_parts::add_body_core_scene_parts;
use self::character_scene_build_context::CharacterSceneBuildContext;
use self::character_scene_motion::CharacterSceneMotion;
use self::limb_scene_parts::add_limb_scene_parts;
use self::scene_camera::build_character_scene_camera;

/// Builds the full preview scene for the current pose and render options.
///
/// The image samples may be absent; in that case texture-driven overlay features are
/// skipped where necessary. This function accepts any finite `yaw` and uses the
/// provided pose as-is.
///
/// This function does not panic.
pub(in super::super) fn build_character_scene(
    rect: Rect,
    cape_uv: FaceUvs,
    yaw: f32,
    preview_pose: PreviewPose,
    variant: MinecraftSkinVariant,
    preview_3d_layers_enabled: bool,
    show_elytra: bool,
    expressions_enabled: bool,
    expression_layout: Option<DetectedExpressionsLayout>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
) -> BuiltCharacterScene {
    let motion = CharacterSceneMotion::new(preview_pose, variant);
    let (camera, projection, model_offset, light_dir) =
        build_character_scene_camera(yaw, motion.bob);
    let mut context = CharacterSceneBuildContext {
        rect,
        variant,
        cape_uv,
        preview_pose,
        preview_3d_layers_enabled,
        show_elytra,
        expressions_enabled,
        expression_layout,
        skin_sample,
        cape_sample,
        default_elytra_sample,
        camera,
        projection,
        model_offset,
        light_dir,
        motion,
        base_tris: Vec::with_capacity(180),
        overlay_tris: Vec::with_capacity(140),
    };

    add_body_core_scene_parts(&mut context);
    add_limb_scene_parts(&mut context);

    let mut scene_tris = std::mem::take(&mut context.base_tris);
    scene_tris.extend(std::mem::take(&mut context.overlay_tris));
    let cape_render_sample = add_back_attachment_scene_parts(&mut scene_tris, &context);

    BuiltCharacterScene {
        triangles: scene_tris,
        cape_render_sample,
    }
}
