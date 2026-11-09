use super::*;

pub(super) fn add_body_core_scene_parts(context: &mut CharacterSceneBuildContext) {
    add_torso_scene_parts(context);
    add_head_scene_parts(context);
    add_expression_scene_parts(context);
}

fn add_torso_scene_parts(context: &mut CharacterSceneBuildContext) {
    let torso_uv = FaceUvs {
        top: uv_rect(20, 16, 8, 4),
        bottom: uv_rect(28, 16, 8, 4),
        left: uv_rect(16, 20, 4, 12),
        right: uv_rect(28, 20, 4, 12),
        front: uv_rect(20, 20, 8, 12),
        back: uv_rect(32, 20, 8, 12),
    };
    let torso_overlay_uv = FaceUvs {
        top: uv_rect_overlay(20, 32, 8, 4),
        bottom: uv_rect_overlay(28, 32, 8, 4),
        left: uv_rect_overlay(16, 36, 4, 12),
        right: uv_rect_overlay(28, 36, 4, 12),
        front: uv_rect_overlay(20, 36, 8, 12),
        back: uv_rect_overlay(32, 36, 8, 12),
    };

    capture_part(PartTag::Body);
    let transform = context.motion.pose.body.transform(context.model_offset);
    let local_top_center = Vec3::new(0.0, 0.0, 0.0);
    add_transformed_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        transform,
        Vec3::new(8.0, 12.0, 4.0),
        local_top_center,
        torso_uv,
        true,
        &context.camera,
        context.projection,
        context.rect,
        context.light_dir,
    );

    if context.preview_3d_layers_enabled {
        if let Some(skin_image) = context.skin_sample.as_ref() {
            let torso_regions = [
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Top,
                    tex_x: 20,
                    tex_y: 32,
                    width: 8,
                    height: 4,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Bottom,
                    tex_x: 28,
                    tex_y: 32,
                    width: 8,
                    height: 4,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Left,
                    tex_x: 16,
                    tex_y: 36,
                    width: 4,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 28,
                    tex_y: 36,
                    width: 4,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Front,
                    tex_x: 20,
                    tex_y: 36,
                    width: 8,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Back,
                    tex_x: 32,
                    tex_y: 36,
                    width: 8,
                    height: 12,
                },
            ];
            add_voxel_overlay_layer(
                &mut context.overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(8.0, 12.0, 4.0),
                    transform,
                    local_top_center,
                },
                &torso_regions,
                &context.camera,
                context.projection,
                context.rect,
                context.light_dir,
            );
        }
    } else {
        add_transformed_cuboid_triangles(
            &mut context.overlay_tris,
            TriangleTexture::Skin,
            transform,
            Vec3::new(8.6, 12.6, 4.6),
            local_top_center + Vec3::new(0.0, 0.2, 0.0),
            torso_overlay_uv,
            false,
            &context.camera,
            context.projection,
            context.rect,
            context.light_dir,
        );
    }
}

fn add_head_scene_parts(context: &mut CharacterSceneBuildContext) {
    let head_uv = FaceUvs {
        top: uv_rect(8, 0, 8, 8),
        bottom: uv_rect(16, 0, 8, 8),
        left: uv_rect(0, 8, 8, 8),
        right: uv_rect(16, 8, 8, 8),
        // Expression panels line up with individual face texels, so keep the face's texel
        // grid exact instead of shrinking it like the other faces.
        front: uv_rect_with_inset([64, 64], 8, 8, 8, 8, FACE_TEXEL_EXACT_INSET_TEXELS),
        back: uv_rect(24, 8, 8, 8),
    };
    let head_overlay_uv = FaceUvs {
        top: uv_rect_overlay(40, 0, 8, 8),
        bottom: uv_rect_overlay(48, 0, 8, 8),
        left: uv_rect_overlay(32, 8, 8, 8),
        right: uv_rect_overlay(48, 8, 8, 8),
        front: uv_rect_overlay(40, 8, 8, 8),
        back: uv_rect_overlay(56, 8, 8, 8),
    };
    capture_part(PartTag::Head);
    let transform = context.motion.pose.head.transform(context.model_offset);
    let local_top_center = Vec3::new(0.0, 8.0, 0.0);
    add_transformed_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        transform,
        Vec3::new(8.0, 8.0, 8.0),
        local_top_center,
        head_uv,
        true,
        &context.camera,
        context.projection,
        context.rect,
        context.light_dir,
    );
    if context.preview_3d_layers_enabled {
        if let Some(skin_image) = context.skin_sample.as_ref() {
            let head_regions = [
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Top,
                    tex_x: 40,
                    tex_y: 0,
                    width: 8,
                    height: 8,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Bottom,
                    tex_x: 48,
                    tex_y: 0,
                    width: 8,
                    height: 8,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Left,
                    tex_x: 32,
                    tex_y: 8,
                    width: 8,
                    height: 8,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 48,
                    tex_y: 8,
                    width: 8,
                    height: 8,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Front,
                    tex_x: 40,
                    tex_y: 8,
                    width: 8,
                    height: 8,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Back,
                    tex_x: 56,
                    tex_y: 8,
                    width: 8,
                    height: 8,
                },
            ];
            add_voxel_overlay_layer(
                &mut context.overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(8.0, 8.0, 8.0),
                    transform,
                    local_top_center,
                },
                &head_regions,
                &context.camera,
                context.projection,
                context.rect,
                context.light_dir,
            );
        }
    } else {
        add_transformed_cuboid_triangles(
            &mut context.overlay_tris,
            TriangleTexture::Skin,
            transform,
            Vec3::new(8.8, 8.8, 8.8),
            local_top_center + Vec3::new(0.0, 0.4, 0.0),
            head_overlay_uv,
            false,
            &context.camera,
            context.projection,
            context.rect,
            context.light_dir,
        );
    }
}

fn add_expression_scene_parts(context: &mut CharacterSceneBuildContext) {
    if !context.expressions_enabled || context.skin_sample.is_none() {
        return;
    }
    let Some(layout) = context.expression_layout else {
        return;
    };
    capture_part(PartTag::Face);
    let head_transform = context.motion.pose.head.transform(context.model_offset);
    let face_pose = compute_face_pose(FaceAnimationInput {
        age_ticks: context.motion.age_ticks,
        limb_speed: context.motion.limb_speed,
        entity_random: context.motion.entity_random,
        camera_distance_blocks: (context.camera.position - head_transform.origin).length() / 16.0,
    });
    add_expression_triangles(
        &mut context.overlay_tris,
        &context.camera,
        context.projection,
        context.rect,
        head_transform,
        context.light_dir,
        layout,
        face_pose,
    );
}
