use super::*;

pub(super) fn draw_character(
    ui: &Ui,
    painter: &egui::Painter,
    rect: Rect,
    skin_texture: &TextureHandle,
    cape_texture: Option<&TextureHandle>,
    default_elytra_texture: Option<&TextureHandle>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
    cape_uv: FaceUvs,
    yaw: f32,
    yaw_velocity: f32,
    preview_pose: PreviewPose,
    variant: MinecraftSkinVariant,
    show_elytra: bool,
    expressions_enabled: bool,
    expression_layout: Option<DetectedExpressionsLayout>,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
    preview_motion_blur_enabled: bool,
    preview_motion_blur_amount: f32,
    preview_motion_blur_shutter_frames: f32,
    preview_motion_blur_sample_count: usize,
    preview_3d_layers_enabled: bool,
    preview_texture: &mut Option<TextureHandle>,
    preview_history: &mut Option<PreviewHistory>,
) {
    let cape_render_texture = if show_elytra {
        cape_texture.or(default_elytra_texture)
    } else {
        cape_texture
    };
    let scene = build_character_scene(
        rect,
        cape_uv,
        yaw,
        preview_pose,
        variant,
        preview_3d_layers_enabled,
        show_elytra,
        expressions_enabled,
        expression_layout,
        skin_sample.clone(),
        cape_sample.clone(),
        default_elytra_sample.clone(),
    );

    if preview_motion_blur_enabled {
        if let (Some(target_format), Some(skin_sample)) = (wgpu_target_format, skin_sample.as_ref())
        {
            let motion_blur_samples = build_motion_blur_scene_samples(
                rect,
                cape_uv,
                yaw,
                yaw_velocity,
                preview_pose,
                preview_motion_blur_shutter_frames,
                preview_motion_blur_sample_count,
                variant,
                preview_3d_layers_enabled,
                show_elytra,
                expressions_enabled,
                expression_layout,
                Some(Arc::clone(skin_sample)),
                cape_sample,
                default_elytra_sample,
                preview_motion_blur_amount,
            );
            if !motion_blur_samples.is_empty() {
                render_motion_blur_wgpu_scene(
                    ui,
                    rect,
                    &motion_blur_samples,
                    Arc::clone(skin_sample),
                    scene.cape_render_sample.clone(),
                    target_format,
                    if preview_aa_mode == SkinPreviewAaMode::Msaa {
                        preview_msaa_samples.max(1)
                    } else {
                        1
                    },
                    preview_msaa_samples.max(1),
                    preview_aa_mode,
                    preview_texel_aa_mode,
                );
                return;
            }
        }
    }

    render_depth_buffered_scene(
        ui,
        painter,
        rect,
        &scene.triangles,
        skin_texture,
        cape_render_texture,
        skin_sample,
        scene.cape_render_sample,
        wgpu_target_format,
        preview_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
        preview_texture,
        preview_history,
    );
}

struct FaceExpressionPose {
    look_x: f32,
    look_y: f32,
    brow_raise_left: f32,
    brow_raise_right: f32,
    brow_squeeze: f32,
    upper_lid_left: f32,
    upper_lid_right: f32,
    lower_lid: f32,
}

struct BuiltCharacterScene {
    triangles: Vec<RenderTriangle>,
    cape_render_sample: Option<Arc<RgbaImage>>,
}

pub(super) struct WeightedPreviewScene {
    pub(super) weight: f32,
    pub(super) triangles: Vec<RenderTriangle>,
}

#[derive(Clone, Copy)]
enum OverlayVoxelFace {
    Top,
    Bottom,
    Left,
    Right,
    Front,
    Back,
}

#[derive(Clone, Copy)]
struct OverlayRegionSpec {
    face: OverlayVoxelFace,
    tex_x: u32,
    tex_y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct OverlayPartSpec {
    size: Vec3,
    pivot_top_center: Vec3,
    rotate_x: f32,
    rotate_z: f32,
}

fn add_voxel_overlay_layer(
    out: &mut Vec<RenderTriangle>,
    image: &RgbaImage,
    part: OverlayPartSpec,
    regions: &[OverlayRegionSpec],
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
) {
    const VOXEL_THICKNESS: f32 = 0.92;
    const VOXEL_GAP: f32 = 0.08;

    for region in regions {
        for row in 0..region.height {
            for col in 0..region.width {
                let tex_x = region.tex_x + col;
                let tex_y = region.tex_y + row;
                if tex_x >= image.width() || tex_y >= image.height() {
                    continue;
                }
                if image.get_pixel(tex_x, tex_y).0[3] == 0 {
                    continue;
                }

                let uv =
                    uv_rect_with_inset([image.width(), image.height()], tex_x, tex_y, 1, 1, 0.02);
                let voxel_uv = FaceUvs {
                    top: uv,
                    bottom: uv,
                    left: uv,
                    right: uv,
                    front: uv,
                    back: uv,
                };
                let (size, local_center) = overlay_voxel_geometry(
                    part.size,
                    region.face,
                    col,
                    row,
                    region.width,
                    region.height,
                    VOXEL_THICKNESS,
                    VOXEL_GAP,
                );

                add_cuboid_triangles_with_y(
                    out,
                    TriangleTexture::Skin,
                    CuboidSpec {
                        size,
                        pivot_top_center: part.pivot_top_center,
                        rotate_x: part.rotate_x,
                        rotate_z: part.rotate_z,
                        uv: voxel_uv,
                        cull_backfaces: false,
                    },
                    camera,
                    projection,
                    rect,
                    light_dir,
                    0.0,
                    local_center,
                );
            }
        }
    }
}

fn overlay_voxel_geometry(
    part_size: Vec3,
    face: OverlayVoxelFace,
    col: u32,
    row: u32,
    _width: u32,
    _height: u32,
    thickness: f32,
    gap: f32,
) -> (Vec3, Vec3) {
    let half_w = part_size.x * 0.5;
    let half_d = part_size.z * 0.5;
    let half_t = thickness * 0.5;
    match face {
        OverlayVoxelFace::Front => (
            Vec3::new(1.0, 1.0, thickness),
            Vec3::new(
                -half_w + col as f32 + 0.5,
                -(row as f32) - 0.5,
                half_d + gap + half_t,
            ),
        ),
        OverlayVoxelFace::Back => (
            Vec3::new(1.0, 1.0, thickness),
            Vec3::new(
                half_w - col as f32 - 0.5,
                -(row as f32) - 0.5,
                -half_d - gap - half_t,
            ),
        ),
        OverlayVoxelFace::Left => (
            Vec3::new(thickness, 1.0, 1.0),
            Vec3::new(
                -half_w - gap - half_t,
                -(row as f32) - 0.5,
                -half_d + col as f32 + 0.5,
            ),
        ),
        OverlayVoxelFace::Right => (
            Vec3::new(thickness, 1.0, 1.0),
            Vec3::new(
                half_w + gap + half_t,
                -(row as f32) - 0.5,
                half_d - col as f32 - 0.5,
            ),
        ),
        OverlayVoxelFace::Top => (
            Vec3::new(1.0, thickness, 1.0),
            Vec3::new(
                -half_w + col as f32 + 0.5,
                gap + half_t,
                -half_d + row as f32 + 0.5,
            ),
        ),
        OverlayVoxelFace::Bottom => (
            Vec3::new(1.0, thickness, 1.0),
            Vec3::new(
                -half_w + col as f32 + 0.5,
                -part_size.y - gap - half_t,
                half_d - row as f32 - 0.5,
            ),
        ),
    }
}

fn build_character_scene(
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
    let arm_width = if variant == MinecraftSkinVariant::Slim {
        3.0
    } else {
        4.0
    };
    let idle_sway = preview_pose.idle_cycle;
    let walk_phase = preview_pose.walk_cycle;
    let locomotion_blend = preview_pose.locomotion_blend;
    let stride_phase = walk_phase * locomotion_blend;
    let bob_idle = 0.08 + (idle_sway * 0.5 + 0.5) * 0.12;
    let bob_walk = stride_phase.abs() * 0.58
        + (preview_pose.time_seconds * 6.6).cos().abs() * 0.04 * locomotion_blend;
    let bob = egui::lerp(bob_idle..=bob_walk, locomotion_blend);
    let leg_swing = stride_phase * 0.72;
    let arm_idle = (preview_pose.time_seconds * 1.35).sin() * 0.055;
    let arm_swing = (-stride_phase * 0.82) + arm_idle * (1.0 - locomotion_blend * 0.45);
    let torso_idle_tilt = idle_sway * 0.035 * (1.0 - locomotion_blend * 0.6) + stride_phase * 0.05;
    let head_idle_tilt = idle_sway * 0.055 * (1.0 - locomotion_blend * 0.55) - stride_phase * 0.035;
    let cape_walk_phase = stride_phase;

    let target = Vec3::new(0.0, 19.5 + bob, 0.0);
    let camera_radius = 56.0;
    let camera_pos = Vec3::new(
        target.x + yaw.cos() * camera_radius,
        target.y + 25.0,
        target.z + yaw.sin() * camera_radius,
    );
    let camera = Camera::look_at(camera_pos, target, Vec3::new(0.0, 1.0, 0.0));
    let projection = Projection {
        fov_y_radians: 36.0_f32.to_radians(),
        near: 1.5,
    };

    let mut base_tris = Vec::with_capacity(180);
    let mut overlay_tris = Vec::with_capacity(140);
    let model_offset = Vec3::new(0.0, bob, 0.0);
    let light_dir = Vec3::new(0.35, 1.0, 0.2).normalized();

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

    let (right_arm_uv, left_arm_uv, right_arm_overlay_uv, left_arm_overlay_uv) =
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
        };

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

    add_cuboid_triangles(
        &mut base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(8.0, 12.0, 4.0),
            pivot_top_center: Vec3::new(0.0, 24.0, 0.0) + model_offset,
            rotate_x: torso_idle_tilt,
            rotate_z: 0.0,
            uv: torso_uv,
            cull_backfaces: true,
        },
        &camera,
        projection,
        rect,
        light_dir,
    );
    if preview_3d_layers_enabled {
        if let Some(skin_image) = skin_sample.as_ref() {
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
                &mut overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(8.0, 12.0, 4.0),
                    pivot_top_center: Vec3::new(0.0, 24.0, 0.0) + model_offset,
                    rotate_x: torso_idle_tilt,
                    rotate_z: 0.0,
                },
                &torso_regions,
                &camera,
                projection,
                rect,
                light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(8.6, 12.6, 4.6),
                pivot_top_center: Vec3::new(0.0, 24.2, 0.0) + model_offset,
                rotate_x: torso_idle_tilt,
                rotate_z: 0.0,
                uv: torso_overlay_uv,
                cull_backfaces: false,
            },
            &camera,
            projection,
            rect,
            light_dir,
        );
    }

    add_cuboid_triangles(
        &mut base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(8.0, 8.0, 8.0),
            pivot_top_center: Vec3::new(0.0, 32.0, 0.0) + model_offset,
            rotate_x: head_idle_tilt,
            rotate_z: 0.0,
            uv: head_uv,
            cull_backfaces: true,
        },
        &camera,
        projection,
        rect,
        light_dir,
    );
    if preview_3d_layers_enabled {
        if let Some(skin_image) = skin_sample.as_ref() {
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
                &mut overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(8.0, 8.0, 8.0),
                    pivot_top_center: Vec3::new(0.0, 32.0, 0.0) + model_offset,
                    rotate_x: head_idle_tilt,
                    rotate_z: 0.0,
                },
                &head_regions,
                &camera,
                projection,
                rect,
                light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(8.8, 8.8, 8.8),
                pivot_top_center: Vec3::new(0.0, 32.4, 0.0) + model_offset,
                rotate_x: head_idle_tilt,
                rotate_z: 0.0,
                uv: head_overlay_uv,
                cull_backfaces: false,
            },
            &camera,
            projection,
            rect,
            light_dir,
        );
    }

    if expressions_enabled {
        if let (Some(layout), Some(skin_image)) = (expression_layout, skin_sample.as_ref()) {
            let expression_pose = compute_expression_pose(
                preview_pose.time_seconds,
                hash_rgba_image(skin_image),
                locomotion_blend,
            );
            add_expression_triangles(
                &mut overlay_tris,
                &camera,
                projection,
                rect,
                model_offset,
                head_idle_tilt,
                light_dir,
                layout,
                expression_pose,
            );
        }
    }

    let shoulder_x = 4.0 + arm_width * 0.5;
    add_cuboid_triangles(
        &mut base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(arm_width, 12.0, 4.0),
            pivot_top_center: Vec3::new(-shoulder_x, 24.0, 0.0) + model_offset,
            rotate_x: arm_swing,
            rotate_z: 0.0,
            uv: left_arm_uv,
            cull_backfaces: true,
        },
        &camera,
        projection,
        rect,
        light_dir,
    );
    if preview_3d_layers_enabled {
        if let Some(skin_image) = skin_sample.as_ref() {
            let left_arm_regions = [
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Top,
                    tex_x: if variant == MinecraftSkinVariant::Slim {
                        52
                    } else {
                        52
                    },
                    tex_y: 48,
                    width: arm_width as u32,
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
                    width: arm_width as u32,
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
                    width: arm_width as u32,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 48,
                    tex_y: 52,
                    width: arm_width as u32,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Front,
                    tex_x: 52,
                    tex_y: 52,
                    width: arm_width as u32,
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
                    width: arm_width as u32,
                    height: 12,
                },
            ];
            add_voxel_overlay_layer(
                &mut overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(arm_width, 12.0, 4.0),
                    pivot_top_center: Vec3::new(-shoulder_x, 24.0, 0.0) + model_offset,
                    rotate_x: arm_swing,
                    rotate_z: 0.0,
                },
                &left_arm_regions,
                &camera,
                projection,
                rect,
                light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(arm_width + 0.55, 12.55, 4.55),
                pivot_top_center: Vec3::new(-shoulder_x, 24.15, 0.0) + model_offset,
                rotate_x: arm_swing,
                rotate_z: 0.0,
                uv: left_arm_overlay_uv,
                cull_backfaces: false,
            },
            &camera,
            projection,
            rect,
            light_dir,
        );
    }
    add_cuboid_triangles(
        &mut base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(arm_width, 12.0, 4.0),
            pivot_top_center: Vec3::new(shoulder_x, 24.0, 0.0) + model_offset,
            rotate_x: -arm_swing,
            rotate_z: 0.0,
            uv: right_arm_uv,
            cull_backfaces: true,
        },
        &camera,
        projection,
        rect,
        light_dir,
    );
    if preview_3d_layers_enabled {
        if let Some(skin_image) = skin_sample.as_ref() {
            let right_arm_regions = [
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Top,
                    tex_x: 44,
                    tex_y: 32,
                    width: arm_width as u32,
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
                    width: arm_width as u32,
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
                    width: arm_width as u32,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Right,
                    tex_x: 40,
                    tex_y: 36,
                    width: arm_width as u32,
                    height: 12,
                },
                OverlayRegionSpec {
                    face: OverlayVoxelFace::Front,
                    tex_x: 44,
                    tex_y: 36,
                    width: arm_width as u32,
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
                    width: arm_width as u32,
                    height: 12,
                },
            ];
            add_voxel_overlay_layer(
                &mut overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(arm_width, 12.0, 4.0),
                    pivot_top_center: Vec3::new(shoulder_x, 24.0, 0.0) + model_offset,
                    rotate_x: -arm_swing,
                    rotate_z: 0.0,
                },
                &right_arm_regions,
                &camera,
                projection,
                rect,
                light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(arm_width + 0.55, 12.55, 4.55),
                pivot_top_center: Vec3::new(shoulder_x, 24.15, 0.0) + model_offset,
                rotate_x: -arm_swing,
                rotate_z: 0.0,
                uv: right_arm_overlay_uv,
                cull_backfaces: false,
            },
            &camera,
            projection,
            rect,
            light_dir,
        );
    }

    add_cuboid_triangles(
        &mut base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(4.0, 12.0, 4.0),
            pivot_top_center: Vec3::new(-2.0, 12.0, 0.0) + model_offset,
            rotate_x: leg_swing,
            rotate_z: 0.0,
            uv: left_leg_uv,
            cull_backfaces: true,
        },
        &camera,
        projection,
        rect,
        light_dir,
    );
    if preview_3d_layers_enabled {
        if let Some(skin_image) = skin_sample.as_ref() {
            let left_leg_regions = [
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
                &mut overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(4.0, 12.0, 4.0),
                    pivot_top_center: Vec3::new(-2.0, 12.0, 0.0) + model_offset,
                    rotate_x: leg_swing,
                    rotate_z: 0.0,
                },
                &left_leg_regions,
                &camera,
                projection,
                rect,
                light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(4.55, 12.55, 4.55),
                pivot_top_center: Vec3::new(-2.0, 12.15, 0.0) + model_offset,
                rotate_x: leg_swing,
                rotate_z: 0.0,
                uv: leg_overlay_uv,
                cull_backfaces: false,
            },
            &camera,
            projection,
            rect,
            light_dir,
        );
    }

    add_cuboid_triangles(
        &mut base_tris,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(4.0, 12.0, 4.0),
            pivot_top_center: Vec3::new(2.0, 12.0, 0.0) + model_offset,
            rotate_x: -leg_swing,
            rotate_z: 0.0,
            uv: right_leg_uv,
            cull_backfaces: true,
        },
        &camera,
        projection,
        rect,
        light_dir,
    );
    if preview_3d_layers_enabled {
        if let Some(skin_image) = skin_sample.as_ref() {
            let right_leg_regions = [
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
                &mut overlay_tris,
                skin_image,
                OverlayPartSpec {
                    size: Vec3::new(4.0, 12.0, 4.0),
                    pivot_top_center: Vec3::new(2.0, 12.0, 0.0) + model_offset,
                    rotate_x: -leg_swing,
                    rotate_z: 0.0,
                },
                &right_leg_regions,
                &camera,
                projection,
                rect,
                light_dir,
            );
        }
    } else {
        add_cuboid_triangles(
            &mut overlay_tris,
            TriangleTexture::Skin,
            CuboidSpec {
                size: Vec3::new(4.55, 12.55, 4.55),
                pivot_top_center: Vec3::new(2.0, 12.15, 0.0) + model_offset,
                rotate_x: -leg_swing,
                rotate_z: 0.0,
                uv: leg_overlay_uv,
                cull_backfaces: false,
            },
            &camera,
            projection,
            rect,
            light_dir,
        );
    }

    let mut scene_tris = base_tris;
    scene_tris.extend(overlay_tris);

    let mut cape_render_sample = cape_sample;

    if cape_render_sample.is_some() && !show_elytra {
        add_cape_triangles(
            &mut scene_tris,
            TriangleTexture::Cape,
            &camera,
            projection,
            rect,
            model_offset,
            cape_walk_phase,
            cape_uv,
            light_dir,
        );
    }

    if show_elytra {
        if cape_render_sample.is_none() {
            cape_render_sample = default_elytra_sample.clone();
        }
        let elytra_sample = cape_render_sample.as_ref();

        let uv_layout = elytra_sample
            .map(|image| [image.width(), image.height()])
            .and_then(elytra_wing_uvs)
            .unwrap_or_else(default_elytra_wing_uvs);
        add_elytra_triangles(
            &mut scene_tris,
            TriangleTexture::Cape,
            &camera,
            projection,
            rect,
            model_offset,
            preview_pose.time_seconds,
            cape_walk_phase,
            uv_layout,
            light_dir,
        );
    }

    BuiltCharacterScene {
        triangles: scene_tris,
        cape_render_sample,
    }
}

fn compute_expression_pose(
    time_seconds: f32,
    skin_hash: u64,
    locomotion_blend: f32,
) -> FaceExpressionPose {
    let seed_a = hash_to_unit(skin_hash ^ 0x14f2_35a7_9bcd_e011);
    let seed_b = hash_to_unit(skin_hash ^ 0xa611_7cc3_52ef_91d5);
    let seed_c = hash_to_unit(skin_hash ^ 0x3d84_2f61_8cbe_7201);
    let seed_d = hash_to_unit(skin_hash ^ 0xff02_6a99_311c_4e73);

    let blink_window = 5.0 + seed_a * 4.8;
    let local_time = time_seconds + seed_b * blink_window;
    let blink_index = (local_time / blink_window).floor().max(0.0) as u64;
    let blink_seed = skin_hash ^ blink_index.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let blink_duration = 0.24 + hash_to_unit(blink_seed ^ 0x19e3) * 0.18;
    let blink_start =
        0.55 + hash_to_unit(blink_seed ^ 0x74c1) * (blink_window - blink_duration - 0.85).max(0.15);
    let blink_chance = 0.72 + locomotion_blend * 0.08;
    let local_phase = local_time - blink_index as f32 * blink_window;
    let mut blink = if hash_to_unit(blink_seed ^ 0xba51) <= blink_chance {
        smooth_blink_pulse(local_phase, blink_start, blink_duration)
    } else {
        0.0
    };
    if blink > 0.0 && hash_to_unit(blink_seed ^ 0xd00b) > 0.78 {
        let second_gap = 0.09 + hash_to_unit(blink_seed ^ 0x52a9) * 0.08;
        let second_duration = blink_duration * (0.72 + hash_to_unit(blink_seed ^ 0x6ef4) * 0.28);
        blink = blink.max(smooth_blink_pulse(
            local_phase,
            blink_start + blink_duration + second_gap,
            second_duration,
        ));
    }

    let micro_blink = ((time_seconds * (0.45 + seed_c * 0.35) + seed_d * 2.1).sin() * 0.5 + 0.5)
        * (0.015 + locomotion_blend * 0.02);

    let gaze_dampen = 1.0 - locomotion_blend * 0.15;
    let mut look_x = (((time_seconds * (0.33 + seed_c * 0.22)).sin() * 0.54)
        + ((time_seconds * (0.84 + seed_a * 0.27) + 1.4).sin() * 0.27)
        + ((time_seconds * (1.47 + seed_d * 0.31) + 0.35).cos() * 0.09))
        * gaze_dampen;
    let mut look_y = (((time_seconds * (0.26 + seed_b * 0.13) + 0.65).sin() * 0.34)
        + ((time_seconds * (0.72 + seed_d * 0.22)).cos() * 0.12))
        * (1.0 - locomotion_blend * 0.1);

    let emotive_wave = (time_seconds * (0.28 + seed_a * 0.18) + seed_b * 2.2).sin();
    let emotive_wave_b = (time_seconds * (0.47 + seed_d * 0.21) + seed_c * 1.7).cos();
    let brow_raise_left = emotive_wave * 0.62 + emotive_wave_b * 0.26;
    let brow_raise_right = emotive_wave * -0.24 + emotive_wave_b * 0.57;
    let brow_squeeze = (((time_seconds * (0.52 + seed_b * 0.29)).sin() * 0.5) + 0.5) * 0.24
        + locomotion_blend * 0.11;
    let action_window = 4.2 + seed_d * 3.1;
    let action_time = time_seconds + seed_a * action_window;
    let action_index = (action_time / action_window).floor().max(0.0) as u64;
    let action_seed = skin_hash ^ action_index.wrapping_mul(0xd1b5_4a32_d192_ed03);
    let action_duration = 0.9 + hash_to_unit(action_seed ^ 0x8123) * 1.6;
    let action_start = 0.35
        + hash_to_unit(action_seed ^ 0x16af) * (action_window - action_duration - 0.95).max(0.15);
    let action_local_time = action_time - action_index as f32 * action_window;
    let action_strength = if hash_to_unit(action_seed ^ 0xb44d) < 0.82 {
        smooth_window_envelope(action_local_time, action_start, action_duration)
    } else {
        0.0
    };
    let action_kind = ((hash_to_unit(action_seed ^ 0x3e91) * 3.0).floor() as i32).clamp(0, 2);
    let action_phase = if action_strength > 0.0 {
        ((action_local_time - action_start) / action_duration).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let mut upper_lid_left = micro_blink;
    let mut upper_lid_right = micro_blink;
    let mut lower_lid = micro_blink * 0.18 + look_y.max(0.0) * 0.2;

    if action_strength > 0.0 {
        match action_kind {
            0 => {
                let first_dir = if hash_to_unit(action_seed ^ 0xa102) > 0.5 {
                    -1.0
                } else {
                    1.0
                };
                let sweep = if action_phase < 0.5 {
                    -1.0 + action_phase / 0.5 * 2.0
                } else {
                    1.0 - (action_phase - 0.5) / 0.5 * 2.0
                };
                look_x += first_dir * sweep * 0.78 * action_strength;
                look_y -= 0.04 * action_strength;
            }
            1 => {
                let dir = if hash_to_unit(action_seed ^ 0x7ac4) > 0.5 {
                    -1.0
                } else {
                    1.0
                };
                look_x += dir * 0.86 * action_strength;
                look_y -= 0.02 * action_strength;
                if dir < 0.0 {
                    upper_lid_left += 0.16 * action_strength;
                    upper_lid_right += 0.34 * action_strength;
                } else {
                    upper_lid_left += 0.34 * action_strength;
                    upper_lid_right += 0.16 * action_strength;
                }
            }
            _ => {
                let scan = if hash_to_unit(action_seed ^ 0xcf17) > 0.46 {
                    (action_phase * std::f32::consts::TAU).sin() * 0.38
                } else {
                    0.0
                };
                look_x += scan * action_strength;
                look_y -= 0.05 * action_strength;
                upper_lid_left += 0.28 + 0.16 * action_strength;
                upper_lid_right += 0.28 + 0.16 * action_strength;
                lower_lid += 0.08 * action_strength;
            }
        }
    }

    upper_lid_left = (upper_lid_left + blink).clamp(0.0, 1.0);
    upper_lid_right = (upper_lid_right + blink).clamp(0.0, 1.0);
    lower_lid = (lower_lid + blink * 0.36).clamp(0.0, 0.82);

    FaceExpressionPose {
        look_x: look_x.clamp(-0.72, 0.72),
        look_y: look_y.clamp(-0.62, 0.62),
        brow_raise_left: brow_raise_left.clamp(-1.05, 1.05),
        brow_raise_right: brow_raise_right.clamp(-1.05, 1.05),
        brow_squeeze: brow_squeeze.clamp(0.0, 0.52),
        upper_lid_left,
        upper_lid_right,
        lower_lid: lower_lid.clamp(0.0, 0.82),
    }
}

fn uv_rect_from_texel_rect(rect: TextureRectU32) -> Rect {
    uv_rect_with_inset(
        [64, 64],
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        UV_EDGE_INSET_OVERLAY_TEXELS,
    )
}

fn uv_rect_from_eyelid_texel_rect(rect: TextureRectU32) -> Rect {
    uv_rect_with_inset(
        [64, 64],
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        UV_EDGE_INSET_OVERLAY_TEXELS,
    )
}

pub(super) fn eye_lid_rects(spec: EyeExpressionSpec) -> (TextureRectU32, TextureRectU32) {
    if let Some(right_lid) = spec.blink {
        let left_dx = spec.left_eye.x as i32 - spec.right_eye.x as i32;
        let left_x = (right_lid.x as i32 + left_dx).max(0) as u32;
        (
            right_lid,
            TextureRectU32 {
                x: left_x,
                y: right_lid.y,
                w: right_lid.w,
                h: right_lid.h,
            },
        )
    } else {
        (spec.right_eye, spec.left_eye)
    }
}

fn hash_mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn hash_to_unit(value: u64) -> f32 {
    let mixed = hash_mix64(value);
    ((mixed >> 40) as f32) / ((1u64 << 24) as f32)
}

fn smooth_blink_pulse(time_seconds: f32, start: f32, duration: f32) -> f32 {
    if duration <= 0.0 || time_seconds < start || time_seconds >= start + duration {
        return 0.0;
    }
    let phase = ((time_seconds - start) / duration).clamp(0.0, 1.0);
    (std::f32::consts::PI * phase)
        .sin()
        .powf(0.65)
        .clamp(0.0, 1.0)
}

fn smooth_window_envelope(time_seconds: f32, start: f32, duration: f32) -> f32 {
    if duration <= 0.0 || time_seconds < start || time_seconds >= start + duration {
        return 0.0;
    }
    let phase = ((time_seconds - start) / duration).clamp(0.0, 1.0);
    let rise = (phase / 0.22).clamp(0.0, 1.0);
    let fall = ((1.0 - phase) / 0.22).clamp(0.0, 1.0);
    let edge = rise.min(fall);
    edge * edge * (3.0 - 2.0 * edge)
}

pub(super) fn compatibility_score(eye: EyeExpressionSpec, brow: BrowExpressionSpec) -> i32 {
    let mut score = 0;
    if eye.offset == brow.offset {
        score += 10;
    }
    match (eye.family, brow.kind) {
        (EyeFamily::Spread, BrowKind::Spread) => score += 6,
        (EyeFamily::ThreeByOne, BrowKind::Villager) => score += 3,
        (_, BrowKind::Standard) => score += 2,
        _ => {}
    }
    score - ((eye.center_y - brow.center_y).abs() * 4.0) as i32
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FacePixelRect {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

impl FacePixelRect {
    pub(super) fn center_x(self) -> f32 {
        4.0 - (self.left + self.width * 0.5)
    }

    pub(super) fn top_y(self) -> f32 {
        self.top
    }

    pub(super) fn bottom_y(self) -> f32 {
        self.top + self.height
    }
}

fn face_left_from_center(center_x: f32, width: f32) -> f32 {
    4.0 - center_x - width * 0.5
}

fn face_top_from_center(center_y: f32, height: f32) -> f32 {
    32.0 - center_y - height * 0.5
}

pub(super) fn eye_face_rects(spec: EyeExpressionSpec) -> (FacePixelRect, FacePixelRect) {
    (
        FacePixelRect {
            left: face_left_from_center(spec.right_center_x, spec.width),
            top: face_top_from_center(spec.center_y, spec.height) - 1.0,
            width: spec.width,
            height: spec.height,
        },
        FacePixelRect {
            left: face_left_from_center(spec.left_center_x, spec.width),
            top: face_top_from_center(spec.center_y, spec.height) - 1.0,
            width: spec.width,
            height: spec.height,
        },
    )
}

fn brow_face_rects(spec: BrowExpressionSpec) -> (FacePixelRect, Option<FacePixelRect>) {
    match spec.kind {
        BrowKind::Spread => (
            FacePixelRect {
                left: face_left_from_center(spec.center_x, spec.width),
                top: face_top_from_center(spec.center_y, spec.height),
                width: spec.width,
                height: spec.height,
            },
            Some(FacePixelRect {
                left: face_left_from_center(-spec.center_x, spec.width),
                top: face_top_from_center(spec.center_y, spec.height),
                width: spec.width,
                height: spec.height,
            }),
        ),
        BrowKind::Villager => (
            FacePixelRect {
                left: face_left_from_center(spec.center_x, spec.width),
                top: face_top_from_center(spec.center_y, spec.height),
                width: spec.width,
                height: spec.height,
            },
            None,
        ),
        BrowKind::Standard | BrowKind::Hat => (
            FacePixelRect {
                left: face_left_from_center(spec.center_x, spec.width),
                top: face_top_from_center(spec.center_y, spec.height),
                width: spec.width,
                height: spec.height,
            },
            spec.left_brow.map(|_| FacePixelRect {
                left: face_left_from_center(-spec.center_x, spec.width),
                top: face_top_from_center(spec.center_y, spec.height),
                width: spec.width,
                height: spec.height,
            }),
        ),
    }
}

#[derive(Clone, Copy)]
enum FaceCoverBias {
    Above,
    Below,
}

fn face_cover_texel(rect: FacePixelRect, bias: FaceCoverBias) -> TextureRectU32 {
    let face_min_x = 8;
    let face_max_x = 15;
    let face_min_y = 8;
    let face_max_y = 15;
    let center_x = (8 + rect.left.round() as i32 + (rect.width.max(1.0).round() as i32 - 1) / 2)
        .clamp(face_min_x, face_max_x);
    let center_y = (8 + rect.top.round() as i32 + (rect.height.max(1.0).round() as i32 - 1) / 2)
        .clamp(face_min_y, face_max_y);
    let top = (8 + rect.top.round() as i32).clamp(face_min_y, face_max_y);
    let bottom = (8 + rect.bottom_y().round() as i32).clamp(face_min_y, face_max_y);
    let left = (8 + rect.left.round() as i32 - 1).clamp(face_min_x, face_max_x);
    let right = (8 + (rect.left + rect.width).round() as i32).clamp(face_min_x, face_max_x);

    let candidates = match bias {
        FaceCoverBias::Above => [
            (center_x, top - 1),
            (left, center_y),
            (right, center_y),
            (center_x, bottom),
        ],
        FaceCoverBias::Below => [
            (center_x, bottom),
            (left, center_y),
            (right, center_y),
            (center_x, top - 1),
        ],
    };

    let (x, y) = candidates
        .into_iter()
        .map(|(x, y)| {
            (
                x.clamp(face_min_x, face_max_x),
                y.clamp(face_min_y, face_max_y),
            )
        })
        .next()
        .unwrap_or((center_x, center_y));

    TextureRectU32 {
        x: x as u32,
        y: y as u32,
        w: 1,
        h: 1,
    }
}

fn expression_eye_plane_z() -> f32 {
    4.06
}

fn expression_hat_plane_z() -> f32 {
    4.46
}

fn add_expression_panel_with_uv(
    out: &mut Vec<RenderTriangle>,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
    head_pivot: Vec3,
    head_rotate_x: f32,
    x_center: f32,
    y_top: f32,
    z_front: f32,
    width: f32,
    height: f32,
    uv: Rect,
) {
    if width <= 0.01 || height <= 0.01 {
        return;
    }

    add_cuboid_triangles_with_y(
        out,
        TriangleTexture::Skin,
        CuboidSpec {
            size: Vec3::new(width, height, 0.055),
            pivot_top_center: head_pivot,
            rotate_x: head_rotate_x,
            rotate_z: 0.0,
            uv: FaceUvs {
                top: uv,
                bottom: uv,
                left: uv,
                right: uv,
                front: uv,
                back: uv,
            },
            cull_backfaces: false,
        },
        camera,
        projection,
        rect,
        light_dir,
        0.0,
        Vec3::new(x_center, -y_top - height * 0.5, z_front),
    );
}

fn add_expression_panel(
    out: &mut Vec<RenderTriangle>,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
    head_pivot: Vec3,
    head_rotate_x: f32,
    x_center: f32,
    y_top: f32,
    z_front: f32,
    width: f32,
    height: f32,
    uv: Rect,
) {
    add_expression_panel_with_uv(
        out,
        camera,
        projection,
        rect,
        light_dir,
        head_pivot,
        head_rotate_x,
        x_center,
        y_top,
        z_front,
        width,
        height,
        uv,
    );
}

fn add_expression_triangles(
    out: &mut Vec<RenderTriangle>,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    model_offset: Vec3,
    head_rotate_x: f32,
    light_dir: Vec3,
    layout: DetectedExpressionsLayout,
    pose: FaceExpressionPose,
) {
    let head_pivot = Vec3::new(0.0, 32.0, 0.0) + model_offset;
    let eye = layout.eye;
    let (right_eye_rect, left_eye_rect) = eye_face_rects(eye);
    let eye_plane_z = expression_eye_plane_z();
    let face_cover_z = eye_plane_z - 0.018;
    let eye_white_z = eye_plane_z + 0.012;
    let pupil_z = eye_plane_z + 0.024;
    let upper_lid_z = eye_plane_z + 0.036;
    let lower_lid_z = eye_plane_z + 0.032;
    let upper_mask_z = eye_plane_z + 0.028;
    let lower_mask_z = eye_plane_z + 0.026;

    add_expression_panel(
        out,
        camera,
        projection,
        rect,
        light_dir,
        head_pivot,
        head_rotate_x,
        right_eye_rect.center_x(),
        right_eye_rect.top_y(),
        face_cover_z,
        right_eye_rect.width,
        right_eye_rect.height,
        uv_rect_from_texel_rect(face_cover_texel(right_eye_rect, FaceCoverBias::Below)),
    );
    add_expression_panel(
        out,
        camera,
        projection,
        rect,
        light_dir,
        head_pivot,
        head_rotate_x,
        left_eye_rect.center_x(),
        left_eye_rect.top_y(),
        face_cover_z,
        left_eye_rect.width,
        left_eye_rect.height,
        uv_rect_from_texel_rect(face_cover_texel(left_eye_rect, FaceCoverBias::Below)),
    );

    add_expression_panel(
        out,
        camera,
        projection,
        rect,
        light_dir,
        head_pivot,
        head_rotate_x,
        right_eye_rect.center_x(),
        right_eye_rect.top_y(),
        eye_plane_z,
        right_eye_rect.width,
        right_eye_rect.height,
        uv_rect_from_texel_rect(eye.right_eye),
    );
    add_expression_panel(
        out,
        camera,
        projection,
        rect,
        light_dir,
        head_pivot,
        head_rotate_x,
        left_eye_rect.center_x(),
        left_eye_rect.top_y(),
        eye_plane_z,
        left_eye_rect.width,
        left_eye_rect.height,
        uv_rect_from_texel_rect(eye.left_eye),
    );

    if let (Some(right_white), Some(left_white), Some(right_pupil), Some(left_pupil)) = (
        eye.right_white,
        eye.left_white,
        eye.right_pupil,
        eye.left_pupil,
    ) {
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_eye_rect.center_x(),
            right_eye_rect.top_y(),
            eye_white_z,
            right_eye_rect.width,
            right_eye_rect.height,
            uv_rect_from_texel_rect(right_white),
        );
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            left_eye_rect.center_x(),
            left_eye_rect.top_y(),
            eye_white_z,
            left_eye_rect.width,
            left_eye_rect.height,
            uv_rect_from_texel_rect(left_white),
        );

        let right_pupil_width = eye.pupil_width.min(right_eye_rect.width);
        let right_pupil_height = eye.pupil_height.min(right_eye_rect.height);
        let left_pupil_width = eye.pupil_width.min(left_eye_rect.width);
        let left_pupil_height = eye.pupil_height.min(left_eye_rect.height);
        let right_gaze_limit_x = eye
            .gaze_scale_x
            .min(((right_eye_rect.width - right_pupil_width) * 0.5).max(0.0));
        let left_gaze_limit_x = eye
            .gaze_scale_x
            .min(((left_eye_rect.width - left_pupil_width) * 0.5).max(0.0));
        let right_gaze_limit_y = eye
            .gaze_scale_y
            .min(((right_eye_rect.height - right_pupil_height) * 0.5).max(0.0));
        let left_gaze_limit_y = eye
            .gaze_scale_y
            .min(((left_eye_rect.height - left_pupil_height) * 0.5).max(0.0));
        let right_pupil_top = right_eye_rect.top
            + (right_eye_rect.height - right_pupil_height) * 0.5
            - pose.look_y * right_gaze_limit_y;
        let left_pupil_top = left_eye_rect.top + (left_eye_rect.height - left_pupil_height) * 0.5
            - pose.look_y * left_gaze_limit_y;

        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_eye_rect.center_x() + pose.look_x * right_gaze_limit_x,
            right_pupil_top,
            pupil_z,
            right_pupil_width,
            right_pupil_height,
            uv_rect_from_texel_rect(right_pupil),
        );
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            left_eye_rect.center_x() + pose.look_x * left_gaze_limit_x,
            left_pupil_top,
            pupil_z,
            left_pupil_width,
            left_pupil_height,
            uv_rect_from_texel_rect(left_pupil),
        );
    }

    let (right_lid_rect, left_lid_rect) = eye_lid_rects(eye);
    let right_lid_uv = uv_rect_from_eyelid_texel_rect(right_lid_rect);
    let left_lid_uv = uv_rect_from_eyelid_texel_rect(left_lid_rect);
    let right_upper_top = right_eye_rect.top_y() - 1.0;
    let left_upper_top = left_eye_rect.top_y() - 1.0;
    let right_lid_h = right_lid_rect.h as f32;
    let left_lid_h = left_lid_rect.h as f32;
    // World-space Y of the lid top: fixed at the open-state position.
    // The panel function center-anchors (local_offset = -y_top - h/2), so:
    //   panel_world_top = 32 - y_top - h/2  =>  W_top = 32 - right_upper_top - lid_h/2
    let right_lid_world_top = 32.0 - right_upper_top - right_lid_h * 0.5;
    let left_lid_world_top = 32.0 - left_upper_top - left_lid_h * 0.5;
    // World-space Y of the eye's visible bottom edge (= where the eye cover panel ends)
    let right_eye_world_bottom = 32.0 - right_eye_rect.top_y() - right_eye_rect.height * 1.5;
    let left_eye_world_bottom = 32.0 - left_eye_rect.top_y() - left_eye_rect.height * 1.5;
    // Lid world bottom: open = world_top - lid_h, closed = eye_world_bottom
    let right_lid_world_bottom = (right_lid_world_top - right_lid_h)
        + pose.upper_lid_right.clamp(0.0, 1.0)
            * (right_eye_world_bottom - (right_lid_world_top - right_lid_h));
    let left_lid_world_bottom = (left_lid_world_top - left_lid_h)
        + pose.upper_lid_left.clamp(0.0, 1.0)
            * (left_eye_world_bottom - (left_lid_world_top - left_lid_h));
    // Back to face-space params: y_top = 32 - W_top - height/2
    let right_upper_draw_h = (right_lid_world_top - right_lid_world_bottom).max(0.0);
    let left_upper_draw_h = (left_lid_world_top - left_lid_world_bottom).max(0.0);
    let right_upper_panel_y_top = 32.0 - right_lid_world_top - right_upper_draw_h * 0.5;
    let left_upper_panel_y_top = 32.0 - left_lid_world_top - left_upper_draw_h * 0.5;
    let upper_lid_inset = 0.12;
    let upper_lid_width = right_eye_rect.width + upper_lid_inset;
    let right_upper_center_x = right_eye_rect.center_x() - upper_lid_inset * 0.5;
    let left_upper_center_x = left_eye_rect.center_x() + upper_lid_inset * 0.5;
    if right_upper_draw_h > 0.01 {
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_upper_center_x,
            right_upper_panel_y_top,
            upper_mask_z,
            upper_lid_width,
            right_upper_draw_h,
            uv_rect_from_texel_rect(face_cover_texel(right_eye_rect, FaceCoverBias::Above)),
        );
        add_expression_panel_with_uv(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_upper_center_x,
            right_upper_panel_y_top,
            upper_lid_z,
            upper_lid_width,
            right_upper_draw_h,
            right_lid_uv,
        );
    }
    if left_upper_draw_h > 0.01 {
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            left_upper_center_x,
            left_upper_panel_y_top,
            upper_mask_z,
            upper_lid_width,
            left_upper_draw_h,
            uv_rect_from_texel_rect(face_cover_texel(left_eye_rect, FaceCoverBias::Above)),
        );
        add_expression_panel_with_uv(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            left_upper_center_x,
            left_upper_panel_y_top,
            upper_lid_z,
            upper_lid_width,
            left_upper_draw_h,
            left_lid_uv,
        );
    }

    let lower = pose.lower_lid.clamp(0.0, 1.0);
    let right_lower_h = (lower * (right_eye_rect.height * 0.28)).clamp(0.0, right_eye_rect.height);
    let left_lower_h = (lower * (left_eye_rect.height * 0.28)).clamp(0.0, left_eye_rect.height);
    let right_lower_y = right_eye_rect.top_y() + (right_eye_rect.height - right_lower_h);
    let left_lower_y = left_eye_rect.top_y() + (left_eye_rect.height - left_lower_h);

    if right_lower_h > 0.01 {
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_eye_rect.center_x(),
            right_lower_y,
            lower_mask_z,
            right_eye_rect.width,
            right_lower_h,
            uv_rect_from_texel_rect(face_cover_texel(right_eye_rect, FaceCoverBias::Below)),
        );
        add_expression_panel_with_uv(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_eye_rect.center_x(),
            right_lower_y,
            lower_lid_z,
            right_eye_rect.width,
            right_lower_h,
            right_lid_uv,
        );
    }
    if left_lower_h > 0.01 {
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            left_eye_rect.center_x(),
            left_lower_y,
            lower_mask_z,
            left_eye_rect.width,
            left_lower_h,
            uv_rect_from_texel_rect(face_cover_texel(left_eye_rect, FaceCoverBias::Below)),
        );
        add_expression_panel_with_uv(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            left_eye_rect.center_x(),
            left_lower_y,
            lower_lid_z,
            left_eye_rect.width,
            left_lower_h,
            left_lid_uv,
        );
    }

    if let Some(brow) = layout.brow {
        let (right_brow_rect, left_brow_rect) = brow_face_rects(brow);
        let brow_drop = pose.brow_squeeze * 0.22;
        let brow_plane_z = if brow.kind == BrowKind::Hat {
            expression_hat_plane_z()
        } else {
            eye_plane_z + 0.048
        };
        let brow_cover_z = if brow.kind == BrowKind::Hat {
            expression_hat_plane_z() - 0.018
        } else {
            face_cover_z
        };
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_brow_rect.center_x(),
            right_brow_rect.top_y(),
            brow_cover_z,
            right_brow_rect.width,
            right_brow_rect.height,
            uv_rect_from_texel_rect(face_cover_texel(right_brow_rect, FaceCoverBias::Above)),
        );
        add_expression_panel(
            out,
            camera,
            projection,
            rect,
            light_dir,
            head_pivot,
            head_rotate_x,
            right_brow_rect.center_x(),
            right_brow_rect.top_y() - pose.brow_raise_right * 0.48 + brow_drop,
            brow_plane_z,
            right_brow_rect.width,
            right_brow_rect.height,
            uv_rect_from_texel_rect(brow.right_brow),
        );
        if let (Some(left_brow), Some(left_rect)) = (brow.left_brow, left_brow_rect) {
            add_expression_panel(
                out,
                camera,
                projection,
                rect,
                light_dir,
                head_pivot,
                head_rotate_x,
                left_rect.center_x(),
                left_rect.top_y(),
                brow_cover_z,
                left_rect.width,
                left_rect.height,
                uv_rect_from_texel_rect(face_cover_texel(left_rect, FaceCoverBias::Above)),
            );
            add_expression_panel(
                out,
                camera,
                projection,
                rect,
                light_dir,
                head_pivot,
                head_rotate_x,
                left_rect.center_x(),
                left_rect.top_y() - pose.brow_raise_left * 0.48 + brow_drop,
                brow_plane_z,
                left_rect.width,
                left_rect.height,
                uv_rect_from_texel_rect(left_brow),
            );
        }
    }
}

fn build_motion_blur_scene_samples(
    rect: Rect,
    cape_uv: FaceUvs,
    yaw: f32,
    yaw_velocity: f32,
    preview_pose: PreviewPose,
    shutter_frames: f32,
    sample_count: usize,
    variant: MinecraftSkinVariant,
    preview_3d_layers_enabled: bool,
    show_elytra: bool,
    expressions_enabled: bool,
    expression_layout: Option<DetectedExpressionsLayout>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
    amount: f32,
) -> Vec<WeightedPreviewScene> {
    let amount = amount.clamp(0.0, 1.0);
    if amount <= 0.001 {
        return Vec::new();
    }

    let sample_count = sample_count.max(2);
    let shutter_seconds = motion_blur_shutter_seconds(shutter_frames);
    if shutter_seconds * yaw_velocity.abs() <= MOTION_BLUR_MIN_ANGULAR_SPAN {
        return Vec::new();
    }
    let center = (sample_count.saturating_sub(1)) as f32 * 0.5;
    let mut weights = Vec::with_capacity(sample_count);
    let mut total_weight = 0.0;

    for index in 0..sample_count {
        let distance = (index as f32 - center).abs();
        let normalized_distance = if center <= f32::EPSILON {
            0.0
        } else {
            distance / center
        };
        let falloff = egui::lerp(4.8..=1.35, amount);
        let edge_floor = egui::lerp(0.0..=0.08, amount * amount);
        let weight = (1.0 - normalized_distance * normalized_distance)
            .max(0.0)
            .powf(falloff)
            .max(edge_floor)
            .max(0.02);
        weights.push(weight);
        total_weight += weight;
    }

    let total_weight = total_weight.max(f32::EPSILON);
    let mut scenes = Vec::with_capacity(sample_count);
    for (index, raw_weight) in weights.into_iter().enumerate() {
        let sample_t = if sample_count <= 1 {
            0.5
        } else {
            index as f32 / (sample_count - 1) as f32
        };
        let time_offset = (sample_t - 0.5) * shutter_seconds;
        let sample_yaw = yaw + time_offset * yaw_velocity;
        let sample_pose = PreviewPose {
            time_seconds: preview_pose.time_seconds + time_offset,
            idle_cycle: ((preview_pose.time_seconds + time_offset) * 1.15).sin(),
            walk_cycle: ((preview_pose.time_seconds + time_offset) * 3.3).sin(),
            locomotion_blend: preview_pose.locomotion_blend,
        };
        let scene = build_character_scene(
            rect,
            cape_uv,
            sample_yaw,
            sample_pose,
            variant,
            preview_3d_layers_enabled,
            show_elytra,
            expressions_enabled,
            expression_layout,
            skin_sample.clone(),
            cape_sample.clone(),
            default_elytra_sample.clone(),
        );
        scenes.push(WeightedPreviewScene {
            weight: raw_weight / total_weight,
            triangles: scene.triangles,
        });
    }

    scenes
}

fn motion_blur_shutter_seconds(shutter_frames: f32) -> f32 {
    let frame = 1.0 / PREVIEW_TARGET_FPS;
    frame * shutter_frames.max(0.0)
}

#[derive(Clone, Copy)]
pub(super) struct Vec3 {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) z: f32,
}

impl Vec3 {
    pub(super) fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    fn normalized(self) -> Self {
        let len = self.length();
        if len <= 0.000_1 {
            Self::new(0.0, 0.0, 0.0)
        } else {
            self * (1.0 / len)
        }
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Camera {
    pub(super) position: Vec3,
    pub(super) right: Vec3,
    pub(super) up: Vec3,
    pub(super) forward: Vec3,
}

impl Camera {
    fn look_at(position: Vec3, target: Vec3, world_up: Vec3) -> Self {
        let forward = (target - position).normalized();
        let right = forward.cross(world_up).normalized();
        let up = right.cross(forward).normalized();
        Self {
            position,
            right,
            up,
            forward,
        }
    }

    fn world_to_camera(self, world: Vec3) -> Vec3 {
        let rel = world - self.position;
        Vec3::new(rel.dot(self.right), rel.dot(self.up), rel.dot(self.forward))
    }
}

#[derive(Clone, Copy)]
pub(super) struct Projection {
    pub(super) fov_y_radians: f32,
    pub(super) near: f32,
}

#[derive(Clone, Copy)]
pub(super) struct FaceUvs {
    pub(super) top: Rect,
    pub(super) bottom: Rect,
    pub(super) left: Rect,
    pub(super) right: Rect,
    pub(super) front: Rect,
    pub(super) back: Rect,
}

#[derive(Clone, Copy)]
pub(super) struct CuboidSpec {
    pub(super) size: Vec3,
    pub(super) pivot_top_center: Vec3,
    pub(super) rotate_x: f32,
    pub(super) rotate_z: f32,
    pub(super) uv: FaceUvs,
    pub(super) cull_backfaces: bool,
}

#[derive(Clone, Copy)]
pub(super) enum TriangleTexture {
    Skin,
    Cape,
}

pub(super) struct RenderTriangle {
    pub(super) texture: TriangleTexture,
    pub(super) pos: [Pos2; 3],
    pub(super) uv: [Pos2; 3],
    pub(super) depth: [f32; 3],
    pub(super) color: Color32,
}

fn rotate_x(point: Vec3, radians: f32) -> Vec3 {
    let (sin, cos) = radians.sin_cos();
    Vec3::new(
        point.x,
        point.y * cos - point.z * sin,
        point.y * sin + point.z * cos,
    )
}

fn rotate_y(point: Vec3, radians: f32) -> Vec3 {
    let (sin, cos) = radians.sin_cos();
    Vec3::new(
        point.x * cos + point.z * sin,
        point.y,
        -point.x * sin + point.z * cos,
    )
}

fn rotate_z(point: Vec3, radians: f32) -> Vec3 {
    let (sin, cos) = radians.sin_cos();
    Vec3::new(
        point.x * cos - point.y * sin,
        point.x * sin + point.y * cos,
        point.z,
    )
}

fn project_point(camera_space: Vec3, projection: Projection, rect: Rect) -> Option<Pos2> {
    if camera_space.z <= projection.near {
        return None;
    }

    let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
    let tan_half_fov = (projection.fov_y_radians * 0.5).tan().max(0.01);
    let x_ndc = camera_space.x / (camera_space.z * tan_half_fov * aspect);
    let y_ndc = camera_space.y / (camera_space.z * tan_half_fov);
    let x = rect.center().x + x_ndc * (rect.width() * 0.5);
    let y = rect.center().y - y_ndc * (rect.height() * 0.5);
    Some(Pos2::new(x, y))
}

fn color_with_brightness(base: Color32, brightness: f32) -> Color32 {
    let b = brightness.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied(
        ((base.r() as f32) * b).round() as u8,
        ((base.g() as f32) * b).round() as u8,
        ((base.b() as f32) * b).round() as u8,
        base.a(),
    )
}

pub(super) fn add_cuboid_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    spec: CuboidSpec,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
) {
    add_cuboid_triangles_with_y(
        out,
        texture,
        spec,
        camera,
        projection,
        rect,
        light_dir,
        0.0,
        Vec3::new(0.0, 0.0, 0.0),
    );
}

pub(super) fn add_cuboid_triangles_with_y(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    spec: CuboidSpec,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    light_dir: Vec3,
    rotate_y_radians: f32,
    local_offset: Vec3,
) {
    let w = spec.size.x;
    let h = spec.size.y;
    let d = spec.size.z;
    let x0 = -w * 0.5;
    let x1 = w * 0.5;
    let y0 = 0.0;
    let y1 = -h;
    let z0 = -d * 0.5;
    let z1 = d * 0.5;

    let faces = [
        (
            [
                Vec3::new(x0, y0, z1),
                Vec3::new(x1, y0, z1),
                Vec3::new(x1, y1, z1),
                Vec3::new(x0, y1, z1),
            ],
            spec.uv.front,
            Vec3::new(0.0, 0.0, 1.0),
        ),
        (
            [
                Vec3::new(x1, y0, z0),
                Vec3::new(x0, y0, z0),
                Vec3::new(x0, y1, z0),
                Vec3::new(x1, y1, z0),
            ],
            spec.uv.back,
            Vec3::new(0.0, 0.0, -1.0),
        ),
        (
            [
                Vec3::new(x0, y0, z0),
                Vec3::new(x0, y0, z1),
                Vec3::new(x0, y1, z1),
                Vec3::new(x0, y1, z0),
            ],
            spec.uv.left,
            Vec3::new(-1.0, 0.0, 0.0),
        ),
        (
            [
                Vec3::new(x1, y0, z1),
                Vec3::new(x1, y0, z0),
                Vec3::new(x1, y1, z0),
                Vec3::new(x1, y1, z1),
            ],
            spec.uv.right,
            Vec3::new(1.0, 0.0, 0.0),
        ),
        (
            [
                Vec3::new(x0, y0, z0),
                Vec3::new(x1, y0, z0),
                Vec3::new(x1, y0, z1),
                Vec3::new(x0, y0, z1),
            ],
            spec.uv.top,
            Vec3::new(0.0, 1.0, 0.0),
        ),
        (
            [
                Vec3::new(x0, y1, z1),
                Vec3::new(x1, y1, z1),
                Vec3::new(x1, y1, z0),
                Vec3::new(x0, y1, z0),
            ],
            spec.uv.bottom,
            Vec3::new(0.0, -1.0, 0.0),
        ),
    ];

    for (quad, uv_rect, normal) in faces {
        let world_normal = rotate_z(
            rotate_y(rotate_x(normal, spec.rotate_x), rotate_y_radians),
            spec.rotate_z,
        )
        .normalized();
        let brightness = 0.58 + world_normal.dot(light_dir).max(0.0) * 0.42;
        let tint = color_with_brightness(Color32::WHITE, brightness);

        let transformed = quad.map(|vertex| {
            let vertex = vertex + local_offset;
            rotate_z(
                rotate_y(rotate_x(vertex, spec.rotate_x), rotate_y_radians),
                spec.rotate_z,
            ) + spec.pivot_top_center
        });
        let camera_vertices = transformed.map(|v| camera.world_to_camera(v));
        if camera_vertices.iter().any(|v| v.z <= projection.near) {
            continue;
        }
        if spec.cull_backfaces {
            // Cull faces that point away from the camera to reduce overdraw artifacts.
            let normal_camera = Vec3::new(
                world_normal.dot(camera.right),
                world_normal.dot(camera.up),
                world_normal.dot(camera.forward),
            );
            let center_camera =
                (camera_vertices[0] + camera_vertices[1] + camera_vertices[2] + camera_vertices[3])
                    * 0.25;
            let to_camera = (Vec3::new(0.0, 0.0, 0.0) - center_camera).normalized();
            if normal_camera.dot(to_camera) <= 0.0 {
                continue;
            }
        }
        let projected = camera_vertices.map(|v| project_point(v, projection, rect));
        if projected.iter().any(Option::is_none) {
            continue;
        }
        let projected = projected.map(Option::unwrap);

        let uv0 = uv_rect.left_top();
        let uv1 = uv_rect.right_top();
        let uv2 = uv_rect.right_bottom();
        let uv3 = uv_rect.left_bottom();
        out.push(RenderTriangle {
            texture,
            pos: [projected[0], projected[1], projected[2]],
            uv: [uv0, uv1, uv2],
            depth: [
                camera_vertices[0].z,
                camera_vertices[1].z,
                camera_vertices[2].z,
            ],
            color: tint,
        });
        out.push(RenderTriangle {
            texture,
            pos: [projected[0], projected[2], projected[3]],
            uv: [uv0, uv2, uv3],
            depth: [
                camera_vertices[0].z,
                camera_vertices[2].z,
                camera_vertices[3].z,
            ],
            color: tint,
        });
    }
}

fn uv_rect(x: u32, y: u32, w: u32, h: u32) -> Rect {
    uv_rect_with_inset([64, 64], x, y, w, h, UV_EDGE_INSET_BASE_TEXELS)
}

fn uv_rect_overlay(x: u32, y: u32, w: u32, h: u32) -> Rect {
    uv_rect_with_inset([64, 64], x, y, w, h, UV_EDGE_INSET_OVERLAY_TEXELS)
}

pub(super) fn uv_rect_with_inset(
    texture_size: [u32; 2],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    inset_texels: f32,
) -> Rect {
    let tex_w = texture_size[0].max(1) as f32;
    let tex_h = texture_size[1].max(1) as f32;
    let max_inset_x = ((w as f32) * 0.49) / tex_w;
    let max_inset_y = ((h as f32) * 0.49) / tex_h;
    let inset_x = (inset_texels / tex_w).min(max_inset_x);
    let inset_y = (inset_texels / tex_h).min(max_inset_y);
    let min_x = (x as f32 / tex_w) + inset_x;
    let min_y = (y as f32 / tex_h) + inset_y;
    let max_x = ((x + w) as f32 / tex_w) - inset_x;
    let max_y = ((y + h) as f32 / tex_h) - inset_y;
    Rect::from_min_max(egui::pos2(min_x, min_y), egui::pos2(max_x, max_y))
}

pub(super) fn flip_uv_rect_x(rect: Rect) -> Rect {
    Rect::from_min_max(
        egui::pos2(rect.max.x, rect.min.y),
        egui::pos2(rect.min.x, rect.max.y),
    )
}
