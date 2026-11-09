use super::*;

pub(super) fn add_back_attachment_scene_parts(
    scene_tris: &mut Vec<RenderTriangle>,
    context: &CharacterSceneBuildContext,
) -> Option<Arc<RgbaImage>> {
    let mut cape_render_sample = context.cape_sample.clone();

    let pose = context.motion.pose;
    if cape_render_sample.is_some() && !context.show_elytra {
        // The cape hangs from the body's neck pivot so it stays on the shoulders as the
        // torso leans and bobs: the hinge copies the body pitch (and yaw while standing),
        // then the cape swings out behind the back.
        let cape_hinge = ModelTransform::from_pivot_rotation(
            pose.body.transform(context.model_offset).origin,
            pose.cape_hinge_rx,
            -pose.cape_hinge_ry,
            0.0,
        );
        let cape = cape_hinge.then(ModelTransform::from_pivot_rotation(
            Vec3::new(0.0, 0.0, -2.0),
            pose.cape_rx,
            0.0,
            -pose.cape_rz,
        ));
        capture_part(PartTag::Cape);
        add_cape_triangles(
            scene_tris,
            TriangleTexture::Cape,
            &context.camera,
            context.projection,
            context.rect,
            cape,
            context.cape_uv,
            context.light_dir,
        );
    }

    if context.show_elytra {
        if cape_render_sample.is_none() {
            cape_render_sample = context.default_elytra_sample.clone();
        }
        add_elytra_triangles(
            scene_tris,
            TriangleTexture::Cape,
            &context.camera,
            context.projection,
            context.rect,
            pose.body.transform(context.model_offset),
            ElytraMotion {
                wings_rx: pose.wings_rx,
                wing_ry: pose.wing_ry,
                right_wing_rz: pose.right_wing_rz,
                left_wing_rz: pose.left_wing_rz,
            },
            context.light_dir,
        );
    }

    cape_render_sample
}
