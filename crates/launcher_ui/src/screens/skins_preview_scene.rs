use super::skins_preview_expressions::{add_expression_triangles, compute_expression_pose};
use super::*;

#[path = "skins_preview_scene/built_character_scene.rs"]
mod built_character_scene;
#[path = "skins_preview_scene/overlay_part_spec.rs"]
mod overlay_part_spec;
#[path = "skins_preview_scene/overlay_region_spec.rs"]
mod overlay_region_spec;
#[path = "skins_preview_scene/overlay_voxel_face.rs"]
mod overlay_voxel_face;

use self::built_character_scene::BuiltCharacterScene;
use self::overlay_part_spec::OverlayPartSpec;
use self::overlay_region_spec::OverlayRegionSpec;
use self::overlay_voxel_face::OverlayVoxelFace;

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

pub(super) fn build_character_scene(
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
                    tex_x: 52,
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

pub(super) fn build_motion_blur_scene_samples(
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
