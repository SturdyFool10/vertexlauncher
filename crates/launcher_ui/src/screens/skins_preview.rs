use super::*;

#[path = "skins_preview_expressions.rs"]
mod skins_preview_expressions;
#[path = "skins_preview_scene.rs"]
mod skins_preview_scene;

pub(super) use self::skins_preview_expressions::{
    compatibility_score, eye_face_rects, eye_lid_rects,
};
use self::skins_preview_scene::{build_character_scene, build_motion_blur_scene_samples};

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

pub(super) struct WeightedPreviewScene {
    pub(super) weight: f32,
    pub(super) triangles: Vec<RenderTriangle>,
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
