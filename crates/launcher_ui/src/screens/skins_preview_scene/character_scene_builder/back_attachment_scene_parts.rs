use super::*;

pub(super) fn add_back_attachment_scene_parts(
    scene_tris: &mut Vec<RenderTriangle>,
    context: &CharacterSceneBuildContext,
) -> Option<Arc<RgbaImage>> {
    let mut cape_render_sample = context.cape_sample.clone();

    if cape_render_sample.is_some() && !context.show_elytra {
        add_cape_triangles(
            scene_tris,
            TriangleTexture::Cape,
            &context.camera,
            context.projection,
            context.rect,
            context.model_offset,
            context.motion.cape_walk_phase,
            context.cape_uv,
            context.light_dir,
        );
    }

    if context.show_elytra {
        if cape_render_sample.is_none() {
            cape_render_sample = context.default_elytra_sample.clone();
        }
        let uv_layout = cape_render_sample
            .as_ref()
            .map(|image| [image.width(), image.height()])
            .and_then(elytra_wing_uvs)
            .unwrap_or_else(default_elytra_wing_uvs);
        add_elytra_triangles(
            scene_tris,
            TriangleTexture::Cape,
            &context.camera,
            context.projection,
            context.rect,
            context.model_offset,
            context.preview_pose.time_seconds,
            context.motion.cape_walk_phase,
            uv_layout,
            context.light_dir,
        );
    }

    cape_render_sample
}
