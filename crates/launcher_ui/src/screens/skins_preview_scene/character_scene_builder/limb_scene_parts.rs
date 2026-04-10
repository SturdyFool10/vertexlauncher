use super::*;

pub(super) fn add_limb_scene_parts(context: &mut CharacterSceneBuildContext) {
    add_arm_scene_parts(context);
    add_leg_scene_parts(context);
}

fn add_arm_scene_parts(context: &mut CharacterSceneBuildContext) {
    let (right_arm_uv, left_arm_uv, right_arm_overlay_uv, left_arm_overlay_uv) =
        build_arm_uvs(context.variant);
    let shoulder_x = 4.0 + context.motion.arm_width * 0.5;

    add_arm(
        context,
        -shoulder_x,
        context.motion.arm_swing,
        left_arm_uv,
        left_arm_overlay_uv,
        true,
    );
    add_arm(
        context,
        shoulder_x,
        -context.motion.arm_swing,
        right_arm_uv,
        right_arm_overlay_uv,
        false,
    );
}

fn add_leg_scene_parts(context: &mut CharacterSceneBuildContext) {
    let right_leg_uv = FaceUvs {
        top: uv_rect(4, 16, 4, 4),
        bottom: uv_rect(8, 16, 4, 4),
        left: uv_rect(8, 20, 4, 12),
        right: uv_rect(0, 20, 4, 12),
        front: uv_rect(4, 20, 4, 12),
        back: uv_rect(12, 20, 4, 12),
    };
    let left_leg_uv = FaceUvs {
        top: uv_rect(20, 48, 4, 4),
        bottom: uv_rect(24, 48, 4, 4),
        left: uv_rect(24, 52, 4, 12),
        right: uv_rect(16, 52, 4, 12),
        front: uv_rect(20, 52, 4, 12),
        back: uv_rect(28, 52, 4, 12),
    };
    let leg_overlay_uv = FaceUvs {
        top: uv_rect_overlay(4, 48, 4, 4),
        bottom: uv_rect_overlay(8, 48, 4, 4),
        left: uv_rect_overlay(8, 52, 4, 12),
        right: uv_rect_overlay(0, 52, 4, 12),
        front: uv_rect_overlay(4, 52, 4, 12),
        back: uv_rect_overlay(12, 52, 4, 12),
    };

    add_leg(
        context,
        -2.0,
        context.motion.leg_swing,
        left_leg_uv,
        leg_overlay_uv,
    );
    add_leg(
        context,
        2.0,
        -context.motion.leg_swing,
        right_leg_uv,
        leg_overlay_uv,
    );
}

fn add_arm(
    context: &mut CharacterSceneBuildContext,
    x: f32,
    rotate_x: f32,
    base_uv: FaceUvs,
    overlay_uv: FaceUvs,
    is_left: bool,
) {
    let pivot_top_center = Vec3::new(x, 24.0, 0.0) + context.model_offset;
    let arm_width = context.motion.arm_width;
    add_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(arm_width, 12.0, 4.0),
            pivot_top_center,
            rotate_x,
            rotate_z: 0.0,
            uv: base_uv,
            cull_backfaces: true,
        },
        &context.camera,
        context.projection,
        context.rect,
        context.light_dir,
    );

    if context.preview_3d_layers_enabled {
        if let Some(skin_image) = context.skin_sample.as_ref() {
            let regions = arm_overlay_regions(context.variant, arm_width as u32, is_left);
            add_voxel_overlay_layer(
                &mut context.overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(arm_width, 12.0, 4.0),
                    pivot_top_center,
                    rotate_x,
                    rotate_z: 0.0,
                },
                &regions,
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
                size: Vec3::new(arm_width + 0.55, 12.55, 4.55),
                pivot_top_center: Vec3::new(x, 24.15, 0.0) + context.model_offset,
                rotate_x,
                rotate_z: 0.0,
                uv: overlay_uv,
                cull_backfaces: false,
            },
            &context.camera,
            context.projection,
            context.rect,
            context.light_dir,
        );
    }
}

fn add_leg(
    context: &mut CharacterSceneBuildContext,
    x: f32,
    rotate_x: f32,
    base_uv: FaceUvs,
    overlay_uv: FaceUvs,
) {
    let pivot_top_center = Vec3::new(x, 12.0, 0.0) + context.model_offset;
    add_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(4.0, 12.0, 4.0),
            pivot_top_center,
            rotate_x,
            rotate_z: 0.0,
            uv: base_uv,
            cull_backfaces: true,
        },
        &context.camera,
        context.projection,
        context.rect,
        context.light_dir,
    );

    if context.preview_3d_layers_enabled {
        if let Some(skin_image) = context.skin_sample.as_ref() {
            let regions = [
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Top,
                    tex_x: 4,
                    tex_y: 48,
                    width: 4,
                    height: 4,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Bottom,
                    tex_x: 8,
                    tex_y: 48,
                    width: 4,
                    height: 4,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Left,
                    tex_x: 8,
                    tex_y: 52,
                    width: 4,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 0,
                    tex_y: 52,
                    width: 4,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Front,
                    tex_x: 4,
                    tex_y: 52,
                    width: 4,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Back,
                    tex_x: 12,
                    tex_y: 52,
                    width: 4,
                    height: 12,
                },
            ];
            add_voxel_overlay_layer(
                &mut context.overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(4.0, 12.0, 4.0),
                    pivot_top_center,
                    rotate_x,
                    rotate_z: 0.0,
                },
                &regions,
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
                size: Vec3::new(4.55, 12.55, 4.55),
                pivot_top_center: Vec3::new(x, 12.15, 0.0) + context.model_offset,
                rotate_x,
                rotate_z: 0.0,
                uv: overlay_uv,
                cull_backfaces: false,
            },
            &context.camera,
            context.projection,
            context.rect,
            context.light_dir,
        );
    }
}

fn build_arm_uvs(variant: MinecraftSkinVariant) -> (FaceUvs, FaceUvs, FaceUvs, FaceUvs) {
    if variant == MinecraftSkinVariant::Slim {
        (
            FaceUvs {
                top: uv_rect(44, 16, 3, 4),
                bottom: uv_rect(47, 16, 3, 4),
                left: uv_rect(47, 20, 3, 12),
                right: uv_rect(40, 20, 3, 12),
                front: uv_rect(44, 20, 3, 12),
                back: uv_rect(51, 20, 3, 12),
            },
            FaceUvs {
                top: uv_rect(36, 48, 3, 4),
                bottom: uv_rect(39, 48, 3, 4),
                left: uv_rect(39, 52, 3, 12),
                right: uv_rect(32, 52, 3, 12),
                front: uv_rect(36, 52, 3, 12),
                back: uv_rect(43, 52, 3, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(44, 32, 3, 4),
                bottom: uv_rect_overlay(47, 32, 3, 4),
                left: uv_rect_overlay(47, 36, 3, 12),
                right: uv_rect_overlay(40, 36, 3, 12),
                front: uv_rect_overlay(44, 36, 3, 12),
                back: uv_rect_overlay(51, 36, 3, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(52, 48, 3, 4),
                bottom: uv_rect_overlay(55, 48, 3, 4),
                left: uv_rect_overlay(55, 52, 3, 12),
                right: uv_rect_overlay(48, 52, 3, 12),
                front: uv_rect_overlay(52, 52, 3, 12),
                back: uv_rect_overlay(59, 52, 3, 12),
            },
        )
    } else {
        (
            FaceUvs {
                top: uv_rect(44, 16, 4, 4),
                bottom: uv_rect(48, 16, 4, 4),
                left: uv_rect(48, 20, 4, 12),
                right: uv_rect(40, 20, 4, 12),
                front: uv_rect(44, 20, 4, 12),
                back: uv_rect(52, 20, 4, 12),
            },
            FaceUvs {
                top: uv_rect(36, 48, 4, 4),
                bottom: uv_rect(40, 48, 4, 4),
                left: uv_rect(40, 52, 4, 12),
                right: uv_rect(32, 52, 4, 12),
                front: uv_rect(36, 52, 4, 12),
                back: uv_rect(44, 52, 4, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(44, 32, 4, 4),
                bottom: uv_rect_overlay(48, 32, 4, 4),
                left: uv_rect_overlay(48, 36, 4, 12),
                right: uv_rect_overlay(40, 36, 4, 12),
                front: uv_rect_overlay(44, 36, 4, 12),
                back: uv_rect_overlay(52, 36, 4, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(52, 48, 4, 4),
                bottom: uv_rect_overlay(56, 48, 4, 4),
                left: uv_rect_overlay(56, 52, 4, 12),
                right: uv_rect_overlay(48, 52, 4, 12),
                front: uv_rect_overlay(52, 52, 4, 12),
                back: uv_rect_overlay(60, 52, 4, 12),
            },
        )
    }
}

fn arm_overlay_regions(
    variant: MinecraftSkinVariant,
    arm_width: u32,
    is_left: bool,
) -> [OverlayRegionSpec; 6] {
    if is_left {
        [
            OverlayRegionSpec {
                face: OverlayVoxelFace::Top,
                tex_x: 52,
                tex_y: 48,
                width: arm_width,
                height: 4,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Bottom,
                tex_x: if variant == MinecraftSkinVariant::Slim {
                    55
                } else {
                    56
                },
                tex_y: 48,
                width: arm_width,
                height: 4,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Left,
                tex_x: if variant == MinecraftSkinVariant::Slim {
                    55
                } else {
                    56
                },
                tex_y: 52,
                width: arm_width,
                height: 12,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Right,
                tex_x: 48,
                tex_y: 52,
                width: arm_width,
                height: 12,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Front,
                tex_x: 52,
                tex_y: 52,
                width: arm_width,
                height: 12,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Back,
                tex_x: if variant == MinecraftSkinVariant::Slim {
                    59
                } else {
                    60
                },
                tex_y: 52,
                width: arm_width,
                height: 12,
            },
        ]
    } else {
        [
            OverlayRegionSpec {
                face: OverlayVoxelFace::Top,
                tex_x: 44,
                tex_y: 32,
                width: arm_width,
                height: 4,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Bottom,
                tex_x: if variant == MinecraftSkinVariant::Slim {
                    47
                } else {
                    48
                },
                tex_y: 32,
                width: arm_width,
                height: 4,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Left,
                tex_x: if variant == MinecraftSkinVariant::Slim {
                    47
                } else {
                    48
                },
                tex_y: 36,
                width: arm_width,
                height: 12,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Right,
                tex_x: 40,
                tex_y: 36,
                width: arm_width,
                height: 12,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Front,
                tex_x: 44,
                tex_y: 36,
                width: arm_width,
                height: 12,
            },
            OverlayRegionSpec {
                face: OverlayVoxelFace::Back,
                tex_x: if variant == MinecraftSkinVariant::Slim {
                    51
                } else {
                    52
                },
                tex_y: 36,
                width: arm_width,
                height: 12,
            },
        ]
    }
}
