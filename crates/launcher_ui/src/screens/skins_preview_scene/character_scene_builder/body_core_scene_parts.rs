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
        left: uv_rect(28, 20, 4, 12),
        right: uv_rect(16, 20, 4, 12),
        front: uv_rect(20, 20, 8, 12),
        back: uv_rect(32, 20, 8, 12),
    };
    let torso_overlay_uv = FaceUvs {
        top: uv_rect_overlay(20, 32, 8, 4),
        bottom: uv_rect_overlay(28, 32, 8, 4),
        left: uv_rect_overlay(28, 36, 4, 12),
        right: uv_rect_overlay(16, 36, 4, 12),
        front: uv_rect_overlay(20, 36, 8, 12),
        back: uv_rect_overlay(32, 36, 8, 12),
    };

    let pivot_top_center = Vec3::new(0.0, 24.0, 0.0) + context.model_offset;
    add_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(8.0, 12.0, 4.0),
            pivot_top_center,
            rotate_x: context.motion.torso_idle_tilt,
            rotate_z: 0.0,
            uv: torso_uv,
            cull_backfaces: true,
        },
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
                    tex_x: 28,
                    tex_y: 36,
                    width: 4,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 16,
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
                    pivot_top_center,
                    rotate_x: context.motion.torso_idle_tilt,
                    rotate_z: 0.0,
                },
                &torso_regions,
                &context.camera,
                context.projection,
                context.rect,
                context.light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut context.overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(8.6, 12.6, 4.6),
                pivot_top_center: Vec3::new(0.0, 24.2, 0.0) + context.model_offset,
                rotate_x: context.motion.torso_idle_tilt,
                rotate_z: 0.0,
                uv: torso_overlay_uv,
                cull_backfaces: false,
            },
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
        left: uv_rect(16, 8, 8, 8),
        right: uv_rect(0, 8, 8, 8),
        front: uv_rect(8, 8, 8, 8),
        back: uv_rect(24, 8, 8, 8),
    };
    let head_overlay_uv = FaceUvs {
        top: uv_rect_overlay(40, 0, 8, 8),
        bottom: uv_rect_overlay(48, 0, 8, 8),
        left: uv_rect_overlay(48, 8, 8, 8),
        right: uv_rect_overlay(32, 8, 8, 8),
        front: uv_rect_overlay(40, 8, 8, 8),
        back: uv_rect_overlay(56, 8, 8, 8),
    };
    let pivot_top_center = Vec3::new(0.0, 32.0, 0.0) + context.model_offset;
    add_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(8.0, 8.0, 8.0),
            pivot_top_center,
            rotate_x: context.motion.head_idle_tilt,
            rotate_z: 0.0,
            uv: head_uv,
            cull_backfaces: true,
        },
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
                    tex_x: 48,
                    tex_y: 8,
                    width: 8,
                    height: 8,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 32,
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
                    pivot_top_center,
                    rotate_x: context.motion.head_idle_tilt,
                    rotate_z: 0.0,
                },
                &head_regions,
                &context.camera,
                context.projection,
                context.rect,
                context.light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut context.overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(8.8, 8.8, 8.8),
                pivot_top_center: Vec3::new(0.0, 32.4, 0.0) + context.model_offset,
                rotate_x: context.motion.head_idle_tilt,
                rotate_z: 0.0,
                uv: head_overlay_uv,
                cull_backfaces: false,
            },
            &context.camera,
            context.projection,
            context.rect,
            context.light_dir,
        );
    }
}

fn add_expression_scene_parts(context: &mut CharacterSceneBuildContext) {
    if !context.expressions_enabled {
        return;
    }
    if let (Some(layout), Some(skin_image)) =
        (context.expression_layout, context.skin_sample.as_ref())
    {
        let expression_pose = compute_expression_pose(
            context.preview_pose.time_seconds,
            hash_preview_image(skin_image),
            context.motion.locomotion_blend,
        );
        add_expression_triangles(
            &mut context.overlay_tris,
            &context.camera,
            context.projection,
            context.rect,
            context.model_offset,
            context.motion.head_idle_tilt,
            context.light_dir,
            layout,
            expression_pose,
        );
    }
}
