use super::*;

// Preview space keeps Minecraft's handedness: the player's right side is -x, so the "left"
// face of each cuboid (at -x) takes the texture's right-side region and vice versa.

pub(super) fn add_limb_scene_parts(context: &mut CharacterSceneBuildContext) {
    add_arm_scene_parts(context);
    add_leg_scene_parts(context);
}

fn add_arm_scene_parts(context: &mut CharacterSceneBuildContext) {
    let (right_arm_uv, left_arm_uv, right_arm_overlay_uv, left_arm_overlay_uv) =
        build_arm_uvs(context.variant);
    // Vanilla arm boxes hang from a pivot 1px inside the shoulder edge, 2px below the neck.
    let arm_center_x = context.motion.arm_width * 0.5 - 1.0;

    add_arm(
        context,
        context.motion.pose.right_arm,
        -arm_center_x,
        right_arm_uv,
        right_arm_overlay_uv,
        false,
    );
    add_arm(
        context,
        context.motion.pose.left_arm,
        arm_center_x,
        left_arm_uv,
        left_arm_overlay_uv,
        true,
    );
}

fn add_leg_scene_parts(context: &mut CharacterSceneBuildContext) {
    let right_leg_uv = FaceUvs {
        top: uv_rect(4, 16, 4, 4),
        bottom: uv_rect(8, 16, 4, 4),
        left: uv_rect(0, 20, 4, 12),
        right: uv_rect(8, 20, 4, 12),
        front: uv_rect(4, 20, 4, 12),
        back: uv_rect(12, 20, 4, 12),
    };
    let left_leg_uv = FaceUvs {
        top: uv_rect(20, 48, 4, 4),
        bottom: uv_rect(24, 48, 4, 4),
        left: uv_rect(16, 52, 4, 12),
        right: uv_rect(24, 52, 4, 12),
        front: uv_rect(20, 52, 4, 12),
        back: uv_rect(28, 52, 4, 12),
    };

    capture_part(PartTag::RightLeg);
    add_leg(
        context,
        context.motion.pose.right_leg,
        right_leg_uv,
        leg_overlay_uv(0, 32),
        leg_overlay_regions(0, 32),
    );
    capture_part(PartTag::LeftLeg);
    add_leg(
        context,
        context.motion.pose.left_leg,
        left_leg_uv,
        leg_overlay_uv(0, 48),
        leg_overlay_regions(0, 48),
    );
}

fn leg_overlay_uv(tex_x: u32, tex_y: u32) -> FaceUvs {
    FaceUvs {
        top: uv_rect_overlay(tex_x + 4, tex_y, 4, 4),
        bottom: uv_rect_overlay(tex_x + 8, tex_y, 4, 4),
        left: uv_rect_overlay(tex_x, tex_y + 4, 4, 12),
        right: uv_rect_overlay(tex_x + 8, tex_y + 4, 4, 12),
        front: uv_rect_overlay(tex_x + 4, tex_y + 4, 4, 12),
        back: uv_rect_overlay(tex_x + 12, tex_y + 4, 4, 12),
    }
}

fn leg_overlay_regions(tex_x: u32, tex_y: u32) -> [OverlayRegionSpec; 6] {
    [
        OverlayRegionSpec {
            face: OverlayVoxelFace::Top,
            tex_x: tex_x + 4,
            tex_y,
            width: 4,
            height: 4,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Bottom,
            tex_x: tex_x + 8,
            tex_y,
            width: 4,
            height: 4,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Left,
            tex_x,
            tex_y: tex_y + 4,
            width: 4,
            height: 12,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Right,
            tex_x: tex_x + 8,
            tex_y: tex_y + 4,
            width: 4,
            height: 12,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Front,
            tex_x: tex_x + 4,
            tex_y: tex_y + 4,
            width: 4,
            height: 12,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Back,
            tex_x: tex_x + 12,
            tex_y: tex_y + 4,
            width: 4,
            height: 12,
        },
    ]
}

fn add_arm(
    context: &mut CharacterSceneBuildContext,
    pose: McPartPose,
    center_x: f32,
    base_uv: FaceUvs,
    overlay_uv: FaceUvs,
    is_left: bool,
) {
    capture_part(if is_left { PartTag::LeftArm } else { PartTag::RightArm });
    let transform = pose.transform(context.model_offset);
    let local_top_center = Vec3::new(center_x, 2.0, 0.0);
    let arm_width = context.motion.arm_width;
    add_transformed_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        transform,
        Vec3::new(arm_width, 12.0, 4.0),
        local_top_center,
        base_uv,
        true,
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
                    transform,
                    local_top_center,
                },
                &regions,
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
            Vec3::new(arm_width + 0.55, 12.55, 4.55),
            local_top_center + Vec3::new(0.0, 0.15, 0.0),
            overlay_uv,
            false,
            &context.camera,
            context.projection,
            context.rect,
            context.light_dir,
        );
    }
}

fn add_leg(
    context: &mut CharacterSceneBuildContext,
    pose: McPartPose,
    base_uv: FaceUvs,
    overlay_uv: FaceUvs,
    overlay_regions: [OverlayRegionSpec; 6],
) {
    let transform = pose.transform(context.model_offset);
    let local_top_center = Vec3::new(0.0, 0.0, 0.0);
    add_transformed_cuboid_triangles(
        &mut context.base_tris,
        TriangleTexture::Skin,
        transform,
        Vec3::new(4.0, 12.0, 4.0),
        local_top_center,
        base_uv,
        true,
        &context.camera,
        context.projection,
        context.rect,
        context.light_dir,
    );

    if context.preview_3d_layers_enabled {
        if let Some(skin_image) = context.skin_sample.as_ref() {
            add_voxel_overlay_layer(
                &mut context.overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(4.0, 12.0, 4.0),
                    transform,
                    local_top_center,
                },
                &overlay_regions,
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
            Vec3::new(4.55, 12.55, 4.55),
            local_top_center + Vec3::new(0.0, 0.15, 0.0),
            overlay_uv,
            false,
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
                left: uv_rect(40, 20, 4, 12),
                right: uv_rect(47, 20, 4, 12),
                front: uv_rect(44, 20, 3, 12),
                back: uv_rect(51, 20, 3, 12),
            },
            FaceUvs {
                top: uv_rect(36, 48, 3, 4),
                bottom: uv_rect(39, 48, 3, 4),
                left: uv_rect(32, 52, 4, 12),
                right: uv_rect(39, 52, 4, 12),
                front: uv_rect(36, 52, 3, 12),
                back: uv_rect(43, 52, 3, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(44, 32, 3, 4),
                bottom: uv_rect_overlay(47, 32, 3, 4),
                left: uv_rect_overlay(40, 36, 4, 12),
                right: uv_rect_overlay(47, 36, 4, 12),
                front: uv_rect_overlay(44, 36, 3, 12),
                back: uv_rect_overlay(51, 36, 3, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(52, 48, 3, 4),
                bottom: uv_rect_overlay(55, 48, 3, 4),
                left: uv_rect_overlay(48, 52, 4, 12),
                right: uv_rect_overlay(55, 52, 4, 12),
                front: uv_rect_overlay(52, 52, 3, 12),
                back: uv_rect_overlay(59, 52, 3, 12),
            },
        )
    } else {
        (
            FaceUvs {
                top: uv_rect(44, 16, 4, 4),
                bottom: uv_rect(48, 16, 4, 4),
                left: uv_rect(40, 20, 4, 12),
                right: uv_rect(48, 20, 4, 12),
                front: uv_rect(44, 20, 4, 12),
                back: uv_rect(52, 20, 4, 12),
            },
            FaceUvs {
                top: uv_rect(36, 48, 4, 4),
                bottom: uv_rect(40, 48, 4, 4),
                left: uv_rect(32, 52, 4, 12),
                right: uv_rect(40, 52, 4, 12),
                front: uv_rect(36, 52, 4, 12),
                back: uv_rect(44, 52, 4, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(44, 32, 4, 4),
                bottom: uv_rect_overlay(48, 32, 4, 4),
                left: uv_rect_overlay(40, 36, 4, 12),
                right: uv_rect_overlay(48, 36, 4, 12),
                front: uv_rect_overlay(44, 36, 4, 12),
                back: uv_rect_overlay(52, 36, 4, 12),
            },
            FaceUvs {
                top: uv_rect_overlay(52, 48, 4, 4),
                bottom: uv_rect_overlay(56, 48, 4, 4),
                left: uv_rect_overlay(48, 52, 4, 12),
                right: uv_rect_overlay(56, 52, 4, 12),
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
    let (base_x, base_y) = if is_left { (48, 48) } else { (40, 32) };
    let slim_shift = u32::from(variant == MinecraftSkinVariant::Slim);
    [
        OverlayRegionSpec {
            face: OverlayVoxelFace::Top,
            tex_x: base_x + 4,
            tex_y: base_y,
            width: arm_width,
            height: 4,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Bottom,
            tex_x: base_x + 8 - slim_shift,
            tex_y: base_y,
            width: arm_width,
            height: 4,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Left,
            tex_x: base_x,
            tex_y: base_y + 4,
            width: 4,
            height: 12,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Right,
            tex_x: base_x + 8 - slim_shift,
            tex_y: base_y + 4,
            width: 4,
            height: 12,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Front,
            tex_x: base_x + 4,
            tex_y: base_y + 4,
            width: arm_width,
            height: 12,
        },
        OverlayRegionSpec {
            face: OverlayVoxelFace::Back,
            tex_x: base_x + 12 - slim_shift,
            tex_y: base_y + 4,
            width: arm_width,
            height: 12,
        },
    ]
}
