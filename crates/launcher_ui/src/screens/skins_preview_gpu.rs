use super::*;

pub(super) fn render_motion_blur_wgpu_scene(
    ui: &Ui,
    rect: Rect,
    scenes: &[WeightedPreviewScene],
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
) {
    let callback = SkinPreviewPostProcessWgpuCallback::from_weighted_scenes(
        scenes,
        skin_sample,
        cape_sample,
        target_format,
        scene_msaa_samples,
        present_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
    );
    let callback_shape = egui_wgpu::Callback::new_paint_callback(rect, callback);
    ui.painter().add(callback_shape);
}

pub(super) fn add_cape_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    model_offset: Vec3,
    walk_phase: f32,
    cape_uv: FaceUvs,
    light_dir: Vec3,
) {
    let pivot = Vec3::new(0.0, 24.0, -2.55) + model_offset;
    let cape_tilt = 0.12 + walk_phase.abs() * 0.10;
    add_cuboid_triangles(
        out,
        texture,
        CuboidSpec {
            size: Vec3::new(10.0, 16.0, 1.0),
            pivot_top_center: pivot,
            rotate_x: cape_tilt,
            rotate_z: 0.0,
            uv: cape_uv,
            cull_backfaces: true,
        },
        camera,
        projection,
        rect,
        light_dir,
    );
}

#[derive(Clone, Copy)]
pub(super) struct ElytraWingUvs {
    pub(super) left: FaceUvs,
    pub(super) right: FaceUvs,
}

#[derive(Clone, Copy)]
struct ElytraWingPose {
    rotate_x: f32,
    rotate_y: f32,
    rotate_z: f32,
    pivot_y_offset: f32,
}

#[derive(Clone, Copy)]
struct ElytraPose {
    left: ElytraWingPose,
    right: ElytraWingPose,
}

const VANILLA_ELYTRA_POSE_STANDING: ElytraPose = ElytraPose {
    left: ElytraWingPose {
        rotate_x: 0.261_799_4,
        rotate_y: -0.087_266_46,
        rotate_z: -0.261_799_4,
        pivot_y_offset: 0.0,
    },
    right: ElytraWingPose {
        rotate_x: 0.261_799_4,
        rotate_y: 0.087_266_46,
        rotate_z: 0.261_799_4,
        pivot_y_offset: 0.0,
    },
};

const VANILLA_ELYTRA_POSE_SNEAKING: ElytraPose = ElytraPose {
    left: ElytraWingPose {
        rotate_x: 0.698_131_7,
        rotate_y: 0.087_266_46,
        rotate_z: -0.785_398_2,
        pivot_y_offset: 3.0,
    },
    right: ElytraWingPose {
        rotate_x: 0.698_131_7,
        rotate_y: -0.087_266_46,
        rotate_z: 0.785_398_2,
        pivot_y_offset: 3.0,
    },
};

const VANILLA_ELYTRA_POSE_GLIDE_OPEN: ElytraPose = ElytraPose {
    left: ElytraWingPose {
        rotate_x: 0.349_065_84,
        rotate_y: 0.0,
        rotate_z: -std::f32::consts::FRAC_PI_2,
        pivot_y_offset: 0.0,
    },
    right: ElytraWingPose {
        rotate_x: 0.349_065_84,
        rotate_y: 0.0,
        rotate_z: std::f32::consts::FRAC_PI_2,
        pivot_y_offset: 0.0,
    },
};

pub(super) fn add_elytra_triangles(
    out: &mut Vec<RenderTriangle>,
    texture: TriangleTexture,
    camera: &Camera,
    projection: Projection,
    rect: Rect,
    model_offset: Vec3,
    _time_seconds: f32,
    walk_phase: f32,
    wing_uvs: ElytraWingUvs,
    light_dir: Vec3,
) {
    let neutral_pose = VANILLA_ELYTRA_POSE_STANDING;
    let _ = VANILLA_ELYTRA_POSE_SNEAKING;
    let _ = VANILLA_ELYTRA_POSE_GLIDE_OPEN;
    // Couple each wing to the leg on the same side, similar to cape walk response.
    let left_leg_phase = walk_phase;
    let right_leg_phase = -walk_phase;
    let left_flap = neutral_pose.left.rotate_x + left_leg_phase * 0.10;
    let right_flap = neutral_pose.right.rotate_x + right_leg_phase * 0.10;
    let left_yaw = neutral_pose.left.rotate_y + left_leg_phase * 0.045;
    let right_yaw = neutral_pose.right.rotate_y + right_leg_phase * 0.045;
    let left_fold_z = neutral_pose.left.rotate_z;
    let right_fold_z = neutral_pose.right.rotate_z;

    let left_hinge_pivot =
        Vec3::new(-5.0, 24.1 + neutral_pose.left.pivot_y_offset, -2.1) + model_offset;
    let right_hinge_pivot =
        Vec3::new(5.0, 24.1 + neutral_pose.right.pivot_y_offset, -2.1) + model_offset;

    add_cuboid_triangles_with_y(
        out,
        texture,
        CuboidSpec {
            size: Vec3::new(10.0, 20.0, 2.0),
            pivot_top_center: left_hinge_pivot,
            rotate_x: left_flap,
            rotate_z: left_fold_z,
            uv: wing_uvs.left,
            cull_backfaces: false,
        },
        camera,
        projection,
        rect,
        light_dir,
        left_yaw,
        Vec3::new(5.0, 0.0, 0.0),
    );
    add_cuboid_triangles_with_y(
        out,
        texture,
        CuboidSpec {
            size: Vec3::new(10.0, 20.0, 2.0),
            pivot_top_center: right_hinge_pivot,
            rotate_x: right_flap,
            rotate_z: right_fold_z,
            uv: wing_uvs.right,
            cull_backfaces: false,
        },
        camera,
        projection,
        rect,
        light_dir,
        right_yaw,
        Vec3::new(-5.0, 0.0, 0.0),
    );
}

pub(super) fn render_depth_buffered_scene(
    ui: &Ui,
    painter: &egui::Painter,
    rect: Rect,
    triangles: &[RenderTriangle],
    skin_texture: &TextureHandle,
    cape_texture: Option<&TextureHandle>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
    preview_texture: &mut Option<TextureHandle>,
    preview_history: &mut Option<PreviewHistory>,
) {
    let Some(target_format) = wgpu_target_format else {
        // WGPU target format can be absent on non-wgpu renderers.
        paint_scene_fallback_mesh(painter, triangles, skin_texture, cape_texture);
        return;
    };
    let Some(skin_sample) = skin_sample else {
        paint_scene_fallback_mesh(painter, triangles, skin_texture, cape_texture);
        return;
    };

    let callback = SkinPreviewPostProcessWgpuCallback::from_scene(
        triangles,
        skin_sample,
        cape_sample,
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
    let callback_shape = egui_wgpu::Callback::new_paint_callback(rect, callback);
    ui.painter().add(callback_shape);
    let _ = (preview_texture, preview_history);
}

fn paint_scene_fallback_mesh(
    painter: &egui::Painter,
    triangles: &[RenderTriangle],
    skin_texture: &TextureHandle,
    cape_texture: Option<&TextureHandle>,
) {
    for tri in triangles {
        let texture_id = match tri.texture {
            TriangleTexture::Skin => skin_texture.id(),
            TriangleTexture::Cape => match cape_texture {
                Some(texture) => texture.id(),
                None => continue,
            },
        };
        let mut mesh = egui::epaint::Mesh::with_texture(texture_id);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: tri.pos[0],
            uv: tri.uv[0],
            color: tri.color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: tri.pos[1],
            uv: tri.uv[1],
            color: tri.color,
        });
        mesh.vertices.push(egui::epaint::Vertex {
            pos: tri.pos[2],
            uv: tri.uv[2],
            color: tri.color,
        });
        mesh.indices.extend_from_slice(&[0, 1, 2]);
        painter.add(egui::Shape::mesh(mesh));
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct PreviewHistory {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[allow(dead_code)]
fn render_cpu_post_aa_scene(
    ctx: &egui::Context,
    painter: &egui::Painter,
    rect: Rect,
    triangles: &[RenderTriangle],
    skin_image: &RgbaImage,
    cape_image: Option<&RgbaImage>,
    mode: SkinPreviewAaMode,
    preview_texture: &mut Option<TextureHandle>,
    preview_history: &mut Option<PreviewHistory>,
) {
    let width = rect.width().round().max(1.0) as usize;
    let height = rect.height().round().max(1.0) as usize;
    let mut color = vec![0u8; width * height * 4];
    let mut depth = vec![f32::INFINITY; width * height];

    for tri in triangles {
        let texture = match tri.texture {
            TriangleTexture::Skin => skin_image,
            TriangleTexture::Cape => match cape_image {
                Some(image) => image,
                None => continue,
            },
        };
        rasterize_triangle_depth_tested(&mut color, &mut depth, width, height, rect, tri, texture);
    }

    match mode {
        SkinPreviewAaMode::Smaa => apply_fxaa_rgba(&mut color, width, height),
        SkinPreviewAaMode::Fxaa => apply_fxaa_rgba(&mut color, width, height),
        SkinPreviewAaMode::Taa => apply_taa_rgba(&mut color, width, height, preview_history, 0.35),
        SkinPreviewAaMode::FxaaTaa => {
            apply_taa_rgba(&mut color, width, height, preview_history, 0.22);
            apply_fxaa_rgba(&mut color, width, height);
        }
        _ => {}
    }

    let color_image = egui::ColorImage::from_rgba_unmultiplied([width, height], &color);
    if let Some(texture) = preview_texture.as_mut() {
        texture.set(color_image, TextureOptions::NEAREST);
    } else {
        *preview_texture = Some(ctx.load_texture(
            "skins/preview/post-aa-frame",
            color_image,
            TextureOptions::NEAREST,
        ));
    }
    if let Some(texture) = preview_texture.as_ref() {
        painter.image(texture.id(), rect, full_uv_rect(), Color32::WHITE);
    }
}

#[allow(dead_code)]
fn rasterize_triangle_depth_tested(
    color_buffer: &mut [u8],
    depth_buffer: &mut [f32],
    width: usize,
    height: usize,
    rect: Rect,
    tri: &RenderTriangle,
    texture: &RgbaImage,
) {
    let p0 = Pos2::new(tri.pos[0].x - rect.left(), tri.pos[0].y - rect.top());
    let p1 = Pos2::new(tri.pos[1].x - rect.left(), tri.pos[1].y - rect.top());
    let p2 = Pos2::new(tri.pos[2].x - rect.left(), tri.pos[2].y - rect.top());
    let area = edge_function(p0, p1, p2);
    if area.abs() <= 0.000_01 {
        return;
    }

    let min_x = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as i32;
    let min_y = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as i32;
    let max_x = p0.x.max(p1.x).max(p2.x).ceil().min(width as f32 - 1.0) as i32;
    let max_y = p0.y.max(p1.y).max(p2.y).ceil().min(height as f32 - 1.0) as i32;
    if min_x > max_x || min_y > max_y {
        return;
    }

    let inv_area = 1.0 / area;
    let inv_z0 = 1.0 / tri.depth[0].max(0.000_1);
    let inv_z1 = 1.0 / tri.depth[1].max(0.000_1);
    let inv_z2 = 1.0 / tri.depth[2].max(0.000_1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let sample = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge_function(p1, p2, sample) * inv_area;
            let w1 = edge_function(p2, p0, sample) * inv_area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -0.000_1 || w1 < -0.000_1 || w2 < -0.000_1 {
                continue;
            }

            let pixel_index = (y as usize) * width + (x as usize);
            let depth = w0 * tri.depth[0] + w1 * tri.depth[1] + w2 * tri.depth[2];
            if depth >= depth_buffer[pixel_index] {
                continue;
            }

            let inv_z = w0 * inv_z0 + w1 * inv_z1 + w2 * inv_z2;
            if inv_z <= 0.0 {
                continue;
            }
            let u =
                (w0 * tri.uv[0].x * inv_z0 + w1 * tri.uv[1].x * inv_z1 + w2 * tri.uv[2].x * inv_z2)
                    / inv_z;
            let v =
                (w0 * tri.uv[0].y * inv_z0 + w1 * tri.uv[1].y * inv_z1 + w2 * tri.uv[2].y * inv_z2)
                    / inv_z;
            let texel = sample_texture_nearest(texture, u, v);
            let tinted = tint_rgba(texel, tri.color);
            if tinted[3] == 0 {
                continue;
            }

            blend_rgba_over(color_buffer, pixel_index * 4, tinted);
            depth_buffer[pixel_index] = depth;
        }
    }
}

#[allow(dead_code)]
fn sample_texture_nearest(texture: &RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let width = texture.width() as usize;
    let height = texture.height() as usize;
    if width == 0 || height == 0 {
        return [0, 0, 0, 0];
    }
    let x = (u.clamp(0.0, 1.0) * width as f32).floor() as usize;
    let y = (v.clamp(0.0, 1.0) * height as f32).floor() as usize;
    let x = x.min(width - 1);
    let y = y.min(height - 1);
    let idx = (y * width + x) * 4;
    let raw = texture.as_raw();
    [raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]]
}

#[allow(dead_code)]
fn tint_rgba(color: [u8; 4], tint: Color32) -> [u8; 4] {
    [
        ((color[0] as u16 * tint.r() as u16) / 255) as u8,
        ((color[1] as u16 * tint.g() as u16) / 255) as u8,
        ((color[2] as u16 * tint.b() as u16) / 255) as u8,
        ((color[3] as u16 * tint.a() as u16) / 255) as u8,
    ]
}

#[allow(dead_code)]
fn blend_rgba_over(buffer: &mut [u8], base: usize, src: [u8; 4]) {
    let src_a = src[3] as f32 / 255.0;
    if src_a <= 0.0 {
        return;
    }
    let dst_r = buffer[base] as f32 / 255.0;
    let dst_g = buffer[base + 1] as f32 / 255.0;
    let dst_b = buffer[base + 2] as f32 / 255.0;
    let dst_a = buffer[base + 3] as f32 / 255.0;

    let src_r = src[0] as f32 / 255.0;
    let src_g = src[1] as f32 / 255.0;
    let src_b = src[2] as f32 / 255.0;

    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    let out_r = (src_r * src_a + dst_r * dst_a * (1.0 - src_a)) / out_a;
    let out_g = (src_g * src_a + dst_g * dst_a * (1.0 - src_a)) / out_a;
    let out_b = (src_b * src_a + dst_b * dst_a * (1.0 - src_a)) / out_a;

    buffer[base] = (out_r * 255.0).round() as u8;
    buffer[base + 1] = (out_g * 255.0).round() as u8;
    buffer[base + 2] = (out_b * 255.0).round() as u8;
    buffer[base + 3] = (out_a * 255.0).round() as u8;
}

#[allow(dead_code)]
fn apply_taa_rgba(
    color: &mut [u8],
    width: usize,
    height: usize,
    history: &mut Option<PreviewHistory>,
    alpha: f32,
) {
    if width == 0 || height == 0 {
        return;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let len = width * height * 4;
    if history
        .as_ref()
        .is_none_or(|h| h.width != width || h.height != height || h.rgba.len() != len)
    {
        *history = Some(PreviewHistory {
            width,
            height,
            rgba: color.to_vec(),
        });
        return;
    }
    let Some(hist) = history.as_mut() else {
        return;
    };
    for i in (0..len).step_by(4) {
        let curr_a = color[i + 3] as f32 / 255.0;
        if curr_a <= 0.001 {
            continue;
        }
        for c in 0..3 {
            let curr = color[i + c] as f32;
            let prev = hist.rgba[i + c] as f32;
            let mixed = curr * alpha + prev * (1.0 - alpha);
            color[i + c] = mixed.round().clamp(0.0, 255.0) as u8;
        }
    }
    hist.rgba.copy_from_slice(color);
}

#[allow(dead_code)]
fn apply_fxaa_rgba(buffer: &mut [u8], width: usize, height: usize) {
    if width < 3 || height < 3 {
        return;
    }

    const EDGE_THRESHOLD: f32 = 1.0 / 8.0;
    const EDGE_THRESHOLD_MIN: f32 = 1.0 / 16.0;
    const FXAA_REDUCE_MIN: f32 = 1.0 / 128.0;
    const FXAA_REDUCE_MUL: f32 = 1.0 / 8.0;
    const FXAA_SPAN_MAX: f32 = 8.0;

    let src = buffer.to_vec();
    for y in 1..(height - 1) {
        for x in 1..(width - 1) {
            let luma_nw = luma_at(&src, width, x - 1, y - 1);
            let luma_ne = luma_at(&src, width, x + 1, y - 1);
            let luma_sw = luma_at(&src, width, x - 1, y + 1);
            let luma_se = luma_at(&src, width, x + 1, y + 1);
            let luma_m = luma_at(&src, width, x, y);

            let luma_min = luma_m.min(luma_nw.min(luma_ne).min(luma_sw).min(luma_se));
            let luma_max = luma_m.max(luma_nw.max(luma_ne).max(luma_sw).max(luma_se));
            let luma_range = luma_max - luma_min;
            let threshold = EDGE_THRESHOLD_MIN.max(luma_max * EDGE_THRESHOLD);
            if luma_range < threshold {
                continue;
            }

            let mut dir_x = -((luma_nw + luma_ne) - (luma_sw + luma_se));
            let mut dir_y = (luma_nw + luma_sw) - (luma_ne + luma_se);

            let dir_reduce = ((luma_nw + luma_ne + luma_sw + luma_se) * 0.25 * FXAA_REDUCE_MUL)
                .max(FXAA_REDUCE_MIN);
            let rcp_dir_min = 1.0 / (dir_x.abs().min(dir_y.abs()) + dir_reduce);
            dir_x = (dir_x * rcp_dir_min).clamp(-FXAA_SPAN_MAX, FXAA_SPAN_MAX);
            dir_y = (dir_y * rcp_dir_min).clamp(-FXAA_SPAN_MAX, FXAA_SPAN_MAX);

            let px = x as f32;
            let py = y as f32;
            let rgb_a = {
                let s0 = sample_rgb_linear(
                    &src,
                    width,
                    height,
                    px + dir_x * (1.0 / 3.0 - 0.5),
                    py + dir_y * (1.0 / 3.0 - 0.5),
                );
                let s1 = sample_rgb_linear(
                    &src,
                    width,
                    height,
                    px + dir_x * (2.0 / 3.0 - 0.5),
                    py + dir_y * (2.0 / 3.0 - 0.5),
                );
                [
                    (s0[0] + s1[0]) * 0.5,
                    (s0[1] + s1[1]) * 0.5,
                    (s0[2] + s1[2]) * 0.5,
                ]
            };

            let rgb_b = {
                let s0 =
                    sample_rgb_linear(&src, width, height, px + dir_x * -0.5, py + dir_y * -0.5);
                let s1 = sample_rgb_linear(&src, width, height, px + dir_x * 0.5, py + dir_y * 0.5);
                [
                    rgb_a[0] * 0.5 + (s0[0] + s1[0]) * 0.25,
                    rgb_a[1] * 0.5 + (s0[1] + s1[1]) * 0.25,
                    rgb_a[2] * 0.5 + (s0[2] + s1[2]) * 0.25,
                ]
            };

            let luma_b = rgb_luma(rgb_b);
            let rgb = if luma_b < luma_min || luma_b > luma_max {
                rgb_a
            } else {
                rgb_b
            };

            let idx = (y * width + x) * 4;
            buffer[idx] = (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8;
            buffer[idx + 1] = (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8;
            buffer[idx + 2] = (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8;
            buffer[idx + 3] = src[idx + 3];
        }
    }
}

#[allow(dead_code)]
fn luma_at(src: &[u8], width: usize, x: usize, y: usize) -> f32 {
    let idx = (y * width + x) * 4;
    rgb_luma([
        src[idx] as f32 / 255.0,
        src[idx + 1] as f32 / 255.0,
        src[idx + 2] as f32 / 255.0,
    ])
}

#[allow(dead_code)]
fn rgb_luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114
}

#[allow(dead_code)]
fn sample_rgb_linear(src: &[u8], width: usize, height: usize, x: f32, y: f32) -> [f32; 3] {
    let max_x = (width - 1) as f32;
    let max_y = (height - 1) as f32;
    let x = x.clamp(0.0, max_x);
    let y = y.clamp(0.0, max_y);

    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);

    let tx = x - x0 as f32;
    let ty = y - y0 as f32;

    let c00 = rgb_at(src, width, x0, y0);
    let c10 = rgb_at(src, width, x1, y0);
    let c01 = rgb_at(src, width, x0, y1);
    let c11 = rgb_at(src, width, x1, y1);

    let top = [
        c00[0] * (1.0 - tx) + c10[0] * tx,
        c00[1] * (1.0 - tx) + c10[1] * tx,
        c00[2] * (1.0 - tx) + c10[2] * tx,
    ];
    let bottom = [
        c01[0] * (1.0 - tx) + c11[0] * tx,
        c01[1] * (1.0 - tx) + c11[1] * tx,
        c01[2] * (1.0 - tx) + c11[2] * tx,
    ];

    [
        top[0] * (1.0 - ty) + bottom[0] * ty,
        top[1] * (1.0 - ty) + bottom[1] * ty,
        top[2] * (1.0 - ty) + bottom[2] * ty,
    ]
}

#[allow(dead_code)]
fn rgb_at(src: &[u8], width: usize, x: usize, y: usize) -> [f32; 3] {
    let idx = (y * width + x) * 4;
    [
        src[idx] as f32 / 255.0,
        src[idx + 1] as f32 / 255.0,
        src[idx + 2] as f32 / 255.0,
    ]
}

#[allow(dead_code)]
fn edge_function(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuPreviewVertex {
    pos_points: [f32; 2],
    camera_z: f32,
    uv: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuPreviewUniform {
    screen_size_points: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuPreviewScalarUniform {
    value: [f32; 4],
}

struct GpuPreviewSceneBatch {
    weight: f32,
    skin_vertices: Vec<GpuPreviewVertex>,
    skin_indices: Vec<u32>,
    cape_vertices: Vec<GpuPreviewVertex>,
    cape_indices: Vec<u32>,
}

struct PreparedGpuPreviewSceneBatch {
    _weight_buffer: wgpu::Buffer,
    weight_bind_group: wgpu::BindGroup,
    skin_vertex_buffer: wgpu::Buffer,
    skin_index_buffer: wgpu::Buffer,
    cape_vertex_buffer: wgpu::Buffer,
    cape_index_buffer: wgpu::Buffer,
    skin_index_count: u32,
    cape_index_count: u32,
}

fn build_gpu_preview_scene_batch(
    triangles: &[RenderTriangle],
    weight: f32,
) -> GpuPreviewSceneBatch {
    let mut skin_vertices = Vec::with_capacity(triangles.len() * 3);
    let mut skin_indices = Vec::with_capacity(triangles.len() * 3);
    let mut cape_vertices = Vec::new();
    let mut cape_indices = Vec::new();

    for tri in triangles {
        let target = match tri.texture {
            TriangleTexture::Skin => (&mut skin_vertices, &mut skin_indices),
            TriangleTexture::Cape => (&mut cape_vertices, &mut cape_indices),
        };
        let base = target.0.len() as u32;
        for i in 0..3 {
            target.0.push(GpuPreviewVertex {
                pos_points: [tri.pos[i].x, tri.pos[i].y],
                camera_z: tri.depth[i].max(SKIN_PREVIEW_NEAR + 0.000_1),
                uv: [tri.uv[i].x, tri.uv[i].y],
                color: tri.color.to_normalized_gamma_f32(),
            });
        }
        target
            .1
            .extend_from_slice(&[base, base.saturating_add(1), base.saturating_add(2)]);
    }

    GpuPreviewSceneBatch {
        weight,
        skin_vertices,
        skin_indices,
        cape_vertices,
        cape_indices,
    }
}

fn prepare_gpu_preview_scene_batch(
    device: &wgpu::Device,
    scalar_uniform_bind_group_layout: &wgpu::BindGroupLayout,
    batch: &GpuPreviewSceneBatch,
) -> PreparedGpuPreviewSceneBatch {
    let weight_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("skins-preview-batch-weight-buffer"),
        contents: bytemuck::bytes_of(&GpuPreviewScalarUniform {
            value: [batch.weight, 0.0, 0.0, 0.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let weight_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skins-preview-batch-weight-bind-group"),
        layout: scalar_uniform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: weight_buffer.as_entire_binding(),
        }],
    });

    PreparedGpuPreviewSceneBatch {
        _weight_buffer: weight_buffer,
        weight_bind_group,
        skin_vertex_buffer: create_preview_vertex_buffer(
            device,
            "skins-preview-batch-skin-vertex-buffer",
            &batch.skin_vertices,
        ),
        skin_index_buffer: create_preview_index_buffer(
            device,
            "skins-preview-batch-skin-index-buffer",
            &batch.skin_indices,
        ),
        cape_vertex_buffer: create_preview_vertex_buffer(
            device,
            "skins-preview-batch-cape-vertex-buffer",
            &batch.cape_vertices,
        ),
        cape_index_buffer: create_preview_index_buffer(
            device,
            "skins-preview-batch-cape-index-buffer",
            &batch.cape_indices,
        ),
        skin_index_count: batch.skin_indices.len() as u32,
        cape_index_count: batch.cape_indices.len() as u32,
    }
}

struct SkinPreviewPostProcessWgpuCallback {
    scene_batches: Vec<GpuPreviewSceneBatch>,
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    skin_hash: u64,
    cape_hash: Option<u64>,
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    aa_mode: SkinPreviewAaMode,
    texel_aa_mode: SkinPreviewTexelAaMode,
}

impl SkinPreviewPostProcessWgpuCallback {
    fn from_scene(
        triangles: &[RenderTriangle],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
        aa_mode: SkinPreviewAaMode,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) -> Self {
        Self {
            scene_batches: vec![build_gpu_preview_scene_batch(triangles, 1.0)],
            skin_hash: hash_rgba_image(&skin_sample),
            cape_hash: cape_sample
                .as_ref()
                .map(|image| hash_rgba_image(image.as_ref())),
            skin_sample,
            cape_sample,
            target_format,
            scene_msaa_samples,
            present_msaa_samples,
            aa_mode,
            texel_aa_mode,
        }
    }

    fn from_weighted_scenes(
        scenes: &[WeightedPreviewScene],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
        aa_mode: SkinPreviewAaMode,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) -> Self {
        let scene_batches = scenes
            .iter()
            .map(|scene| build_gpu_preview_scene_batch(&scene.triangles, scene.weight))
            .collect();
        Self {
            scene_batches,
            skin_hash: hash_rgba_image(&skin_sample),
            cape_hash: cape_sample
                .as_ref()
                .map(|image| hash_rgba_image(image.as_ref())),
            skin_sample,
            cape_sample,
            target_format,
            scene_msaa_samples,
            present_msaa_samples,
            aa_mode,
            texel_aa_mode,
        }
    }
}

impl egui_wgpu::CallbackTrait for SkinPreviewPostProcessWgpuCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<SkinPreviewPostProcessWgpuResources>()
            .or_insert_with(|| {
                SkinPreviewPostProcessWgpuResources::new(
                    device,
                    self.target_format,
                    self.scene_msaa_samples,
                    self.present_msaa_samples,
                )
            });
        if resources.target_format != self.target_format
            || resources.scene_msaa_samples != self.scene_msaa_samples
            || resources.present_msaa_samples != self.present_msaa_samples
        {
            *resources = SkinPreviewPostProcessWgpuResources::new(
                device,
                self.target_format,
                self.scene_msaa_samples,
                self.present_msaa_samples,
            );
        }

        resources.update_scene_uniform(
            queue,
            [
                screen_descriptor.size_in_pixels[0] as f32 / screen_descriptor.pixels_per_point,
                screen_descriptor.size_in_pixels[1] as f32 / screen_descriptor.pixels_per_point,
            ],
        );
        resources.update_scene_texture_aa_mode(queue, self.texel_aa_mode);
        resources.ensure_render_targets(device, screen_descriptor.size_in_pixels);
        resources.update_texture(
            device,
            queue,
            TextureSlot::Skin,
            self.skin_hash,
            &self.skin_sample,
        );
        if let (Some(cape_hash), Some(cape_sample)) = (self.cape_hash, self.cape_sample.as_ref()) {
            resources.update_texture(device, queue, TextureSlot::Cape, cape_hash, cape_sample);
        } else {
            resources.cape_texture = None;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("skins-preview-post-process-encoder"),
        });

        let use_smaa = self.aa_mode == SkinPreviewAaMode::Smaa;
        let use_fxaa = matches!(
            self.aa_mode,
            SkinPreviewAaMode::Fxaa | SkinPreviewAaMode::FxaaTaa
        );
        let use_taa = matches!(
            self.aa_mode,
            SkinPreviewAaMode::Taa | SkinPreviewAaMode::FxaaTaa
        );
        let use_fxaa_after_taa = self.aa_mode == SkinPreviewAaMode::FxaaTaa;
        resources.present_source = PresentSource::Accumulation;

        for (index, batch) in self.scene_batches.iter().enumerate() {
            let prepared_batch = prepare_gpu_preview_scene_batch(
                device,
                &resources.scalar_uniform_bind_group_layout,
                batch,
            );

            {
                let color_attachment =
                    resources.scene_color_attachment(index == 0 || self.scene_batches.len() == 1);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-scene-pass"),
                    color_attachments: &[Some(color_attachment)],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: resources.scene_depth_view(),
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                resources.paint_scene(&mut pass, &prepared_batch);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-accumulation-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &resources.accumulation_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: if index == 0 {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&resources.accumulate_pipeline);
                pass.set_bind_group(0, &resources.scene_resolve_bind_group, &[]);
                pass.set_bind_group(1, &prepared_batch.weight_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        if use_smaa {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-smaa-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &resources.post_process_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&resources.smaa_pipeline);
                pass.set_bind_group(0, &resources.accumulation_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            resources.present_source = PresentSource::PostProcess;
            resources.taa_history_valid = false;
        } else if use_fxaa && !use_taa {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("skins-preview-fxaa-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &resources.post_process_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&resources.fxaa_pipeline);
                pass.set_bind_group(0, &resources.accumulation_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            resources.present_source = PresentSource::PostProcess;
        } else if use_taa {
            let taa_scalar = if use_fxaa_after_taa { 0.22 } else { 0.35 };
            let mut taa_source = PresentSource::Accumulation;
            if resources.taa_history_valid {
                queue.write_buffer(
                    &resources.scalar_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&GpuPreviewScalarUniform {
                        value: [taa_scalar, 0.0, 0.0, 0.0],
                    }),
                );
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("skins-preview-taa-pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &resources.post_process_view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_pipeline(&resources.taa_pipeline);
                    pass.set_bind_group(0, &resources.accumulation_bind_group, &[]);
                    pass.set_bind_group(1, &resources.taa_history_bind_group, &[]);
                    pass.set_bind_group(2, &resources.scalar_uniform_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &resources.post_process_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &resources.taa_history_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    resources.render_target_extent(),
                );
                taa_source = PresentSource::PostProcess;
            } else {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &resources.accumulation_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &resources.taa_history_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    resources.render_target_extent(),
                );
            }
            resources.taa_history_valid = true;
            if use_fxaa_after_taa {
                let (source_bind_group, target_view, present_source, label) = match taa_source {
                    PresentSource::Accumulation => (
                        &resources.accumulation_bind_group,
                        &resources.post_process_view,
                        PresentSource::PostProcess,
                        "skins-preview-fxaa-after-taa-pass",
                    ),
                    PresentSource::PostProcess => (
                        &resources.post_process_bind_group,
                        &resources.accumulation_view,
                        PresentSource::Accumulation,
                        "skins-preview-fxaa-after-taa-pass",
                    ),
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&resources.fxaa_pipeline);
                pass.set_bind_group(0, source_bind_group, &[]);
                pass.draw(0..3, 0..1);
                resources.present_source = present_source;
            } else {
                resources.present_source = taa_source;
            }
        } else {
            resources.taa_history_valid = false;
        }

        vec![encoder.finish()]
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<SkinPreviewPostProcessWgpuResources>()
        else {
            return;
        };
        let viewport = info.viewport_in_pixels();
        render_pass.set_viewport(
            viewport.left_px as f32,
            viewport.top_px as f32,
            viewport.width_px as f32,
            viewport.height_px as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.present_pipeline);
        let bind_group = match resources.present_source {
            PresentSource::Accumulation => &resources.accumulation_bind_group,
            PresentSource::PostProcess => &resources.post_process_bind_group,
        };
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[derive(Clone, Copy)]
enum PresentSource {
    Accumulation,
    PostProcess,
}

struct SkinPreviewPostProcessWgpuResources {
    scene_pipeline: wgpu::RenderPipeline,
    accumulate_pipeline: wgpu::RenderPipeline,
    smaa_pipeline: wgpu::RenderPipeline,
    fxaa_pipeline: wgpu::RenderPipeline,
    taa_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
    uniform_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    scalar_uniform_bind_group_layout: wgpu::BindGroupLayout,
    scalar_uniform_bind_group: wgpu::BindGroup,
    scalar_uniform_buffer: wgpu::Buffer,
    skin_texture: Option<UploadedPreviewTexture>,
    cape_texture: Option<UploadedPreviewTexture>,
    accumulation_texture: wgpu::Texture,
    accumulation_view: wgpu::TextureView,
    accumulation_bind_group: wgpu::BindGroup,
    scene_resolve_texture: wgpu::Texture,
    scene_resolve_view: wgpu::TextureView,
    scene_resolve_bind_group: wgpu::BindGroup,
    scene_msaa_texture: Option<wgpu::Texture>,
    scene_msaa_view: Option<wgpu::TextureView>,
    scene_depth_texture: wgpu::Texture,
    scene_depth_view: wgpu::TextureView,
    post_process_texture: wgpu::Texture,
    post_process_view: wgpu::TextureView,
    post_process_bind_group: wgpu::BindGroup,
    taa_history_texture: wgpu::Texture,
    taa_history_view: wgpu::TextureView,
    taa_history_bind_group: wgpu::BindGroup,
    taa_history_valid: bool,
    render_target_size: [u32; 2],
    target_format: wgpu::TextureFormat,
    scene_msaa_samples: u32,
    present_msaa_samples: u32,
    present_source: PresentSource,
}

impl SkinPreviewPostProcessWgpuResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        scene_msaa_samples: u32,
        present_msaa_samples: u32,
    ) -> Self {
        const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-post-scene-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_post_scene.wgsl"
            ))),
        });

        let accumulate_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-accumulate-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_accumulate.wgsl"
            ))),
        });

        let fxaa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-fxaa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_fxaa.wgsl"
            ))),
        });

        let smaa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-smaa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_smaa.wgsl"
            ))),
        });

        let taa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-taa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_taa.wgsl"
            ))),
        });

        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-present-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview_present.wgsl"
            ))),
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-post-texture-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let texture_sampler =
            create_skin_preview_sampler(device, "skins-preview-post-texture-sampler");
        let scene_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-post-scene-uniform-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let scalar_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-post-scalar-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-post-scene-uniform-buffer"),
            size: std::mem::size_of::<GpuPreviewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-post-scene-uniform-bind-group"),
            layout: &scene_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let scalar_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-post-scalar-uniform-buffer"),
            size: std::mem::size_of::<GpuPreviewScalarUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scalar_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-post-scalar-bind-group"),
            layout: &scalar_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scalar_uniform_buffer.as_entire_binding(),
            }],
        });

        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("skins-preview-post-scene-layout"),
                bind_group_layouts: &[
                    &texture_bind_group_layout,
                    &scene_uniform_layout,
                    &scalar_uniform_layout,
                ],
                push_constant_ranges: &[],
            });
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-post-scene-pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuPreviewVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32,
                        2 => Float32x2,
                        3 => Float32x4
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SKIN_PREVIEW_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: scene_msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: scene_msaa_samples > 1,
            },
            multiview: None,
            cache: None,
        });

        let fullscreen_vertex = wgpu::VertexState {
            module: &accumulate_shader,
            entry_point: Some("vs_fullscreen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        };

        let accumulate_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-accumulate-layout"),
            bind_group_layouts: &[&texture_bind_group_layout, &scalar_uniform_layout],
            push_constant_ranges: &[],
        });
        let accumulate_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-accumulate-pipeline"),
            layout: Some(&accumulate_layout),
            vertex: fullscreen_vertex.clone(),
            fragment: Some(wgpu::FragmentState {
                module: &accumulate_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let smaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-smaa-layout"),
            bind_group_layouts: &[&texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let smaa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-smaa-pipeline"),
            layout: Some(&smaa_layout),
            vertex: wgpu::VertexState {
                module: &smaa_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &smaa_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let fxaa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-fxaa-layout"),
            bind_group_layouts: &[&texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let fxaa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-fxaa-pipeline"),
            layout: Some(&fxaa_layout),
            vertex: wgpu::VertexState {
                module: &fxaa_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fxaa_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let taa_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-taa-layout"),
            bind_group_layouts: &[
                &texture_bind_group_layout,
                &texture_bind_group_layout,
                &scalar_uniform_layout,
            ],
            push_constant_ranges: &[],
        });
        let taa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-taa-pipeline"),
            layout: Some(&taa_layout),
            vertex: wgpu::VertexState {
                module: &taa_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &taa_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let present_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-present-layout"),
            bind_group_layouts: &[&texture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let present_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-present-pipeline"),
            layout: Some(&present_layout),
            vertex: wgpu::VertexState {
                module: &present_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &present_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SKIN_PREVIEW_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: present_msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let (accumulation_texture, accumulation_view, accumulation_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-accumulation",
            );
        let (scene_resolve_texture, scene_resolve_view, scene_resolve_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-scene-resolve",
            );
        let (post_process_texture, post_process_view, post_process_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-post-process",
            );
        let (taa_history_texture, taa_history_view, taa_history_bind_group) =
            create_preview_render_texture(
                device,
                &texture_bind_group_layout,
                &texture_sampler,
                OFFSCREEN_FORMAT,
                [1, 1],
                1,
                "skins-preview-taa-history",
            );
        let (scene_depth_texture, scene_depth_view) = create_preview_depth_texture(
            device,
            [1, 1],
            scene_msaa_samples.max(1),
            "skins-preview-scene-depth",
        );
        let (scene_msaa_texture, scene_msaa_view) = if scene_msaa_samples > 1 {
            let (texture, view) = create_preview_color_texture(
                device,
                OFFSCREEN_FORMAT,
                [1, 1],
                scene_msaa_samples.max(1),
                "skins-preview-scene-msaa",
            );
            (Some(texture), Some(view))
        } else {
            (None, None)
        };

        Self {
            scene_pipeline,
            accumulate_pipeline,
            smaa_pipeline,
            fxaa_pipeline,
            taa_pipeline,
            present_pipeline,
            texture_bind_group_layout,
            texture_sampler,
            uniform_bind_group,
            uniform_buffer,
            scalar_uniform_bind_group_layout: scalar_uniform_layout,
            scalar_uniform_bind_group,
            scalar_uniform_buffer,
            skin_texture: None,
            cape_texture: None,
            accumulation_texture,
            accumulation_view,
            accumulation_bind_group,
            scene_resolve_texture,
            scene_resolve_view,
            scene_resolve_bind_group,
            scene_msaa_texture,
            scene_msaa_view,
            scene_depth_texture,
            scene_depth_view,
            post_process_texture,
            post_process_view,
            post_process_bind_group,
            taa_history_texture,
            taa_history_view,
            taa_history_bind_group,
            taa_history_valid: false,
            render_target_size: [1, 1],
            target_format,
            scene_msaa_samples: scene_msaa_samples.max(1),
            present_msaa_samples: present_msaa_samples.max(1),
            present_source: PresentSource::Accumulation,
        }
    }

    fn ensure_render_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        if self.render_target_size == size {
            return;
        }
        self.render_target_size = size;
        self.taa_history_valid = false;

        const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let (accumulation_texture, accumulation_view, accumulation_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-accumulation",
            );
        self.accumulation_texture = accumulation_texture;
        self.accumulation_view = accumulation_view;
        self.accumulation_bind_group = accumulation_bind_group;

        let (scene_resolve_texture, scene_resolve_view, scene_resolve_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-scene-resolve",
            );
        self.scene_resolve_texture = scene_resolve_texture;
        self.scene_resolve_view = scene_resolve_view;
        self.scene_resolve_bind_group = scene_resolve_bind_group;

        let (post_process_texture, post_process_view, post_process_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-post-process",
            );
        self.post_process_texture = post_process_texture;
        self.post_process_view = post_process_view;
        self.post_process_bind_group = post_process_bind_group;

        let (taa_history_texture, taa_history_view, taa_history_bind_group) =
            create_preview_render_texture(
                device,
                &self.texture_bind_group_layout,
                &self.texture_sampler,
                OFFSCREEN_FORMAT,
                size,
                1,
                "skins-preview-taa-history",
            );
        self.taa_history_texture = taa_history_texture;
        self.taa_history_view = taa_history_view;
        self.taa_history_bind_group = taa_history_bind_group;

        let (scene_depth_texture, scene_depth_view) = create_preview_depth_texture(
            device,
            size,
            self.scene_msaa_samples.max(1),
            "skins-preview-scene-depth",
        );
        self.scene_depth_texture = scene_depth_texture;
        self.scene_depth_view = scene_depth_view;

        if self.scene_msaa_samples > 1 {
            let (texture, view) = create_preview_color_texture(
                device,
                OFFSCREEN_FORMAT,
                size,
                self.scene_msaa_samples.max(1),
                "skins-preview-scene-msaa",
            );
            self.scene_msaa_texture = Some(texture);
            self.scene_msaa_view = Some(view);
        } else {
            self.scene_msaa_texture = None;
            self.scene_msaa_view = None;
        }
    }

    fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: TextureSlot,
        hash: u64,
        image: &RgbaImage,
    ) {
        let size = [image.width(), image.height()];
        let target = match slot {
            TextureSlot::Skin => &mut self.skin_texture,
            TextureSlot::Cape => &mut self.cape_texture,
        };

        if target
            .as_ref()
            .is_some_and(|uploaded| uploaded.hash == hash && uploaded.size == size)
        {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skins-preview-post-source-texture"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: preview_mip_level_count(size),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        write_preview_texture_mips(queue, &texture, image);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = create_preview_texture_bind_group(
            device,
            &self.texture_bind_group_layout,
            &self.texture_sampler,
            &view,
            "skins-preview-post-source-bind-group",
        );

        *target = Some(UploadedPreviewTexture {
            hash,
            size,
            bind_group,
            _texture: texture,
        });
    }

    fn update_scene_uniform(&self, queue: &wgpu::Queue, screen_size_points: [f32; 2]) {
        let uniform = GpuPreviewUniform {
            screen_size_points,
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    fn update_scene_texture_aa_mode(
        &self,
        queue: &wgpu::Queue,
        texel_aa_mode: SkinPreviewTexelAaMode,
    ) {
        queue.write_buffer(
            &self.scalar_uniform_buffer,
            0,
            bytemuck::bytes_of(&GpuPreviewScalarUniform {
                value: [
                    if texel_aa_mode == SkinPreviewTexelAaMode::TexelBoundary {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                    0.0,
                    0.0,
                ],
            }),
        );
    }

    fn scene_color_attachment(&self, _clear: bool) -> wgpu::RenderPassColorAttachment<'_> {
        if let Some(view) = self.scene_msaa_view.as_ref() {
            wgpu::RenderPassColorAttachment {
                view,
                resolve_target: Some(&self.scene_resolve_view),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }
        } else {
            wgpu::RenderPassColorAttachment {
                view: &self.scene_resolve_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }
        }
    }

    fn scene_depth_view(&self) -> &wgpu::TextureView {
        &self.scene_depth_view
    }

    fn paint_scene(
        &self,
        render_pass: &mut wgpu::RenderPass<'_>,
        batch: &PreparedGpuPreviewSceneBatch,
    ) {
        render_pass.set_pipeline(&self.scene_pipeline);
        render_pass.set_bind_group(1, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(2, &self.scalar_uniform_bind_group, &[]);

        if let Some(texture) = self.skin_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.skin_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(batch.skin_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..batch.skin_index_count, 0, 0..1);
        }
        if let Some(texture) = self.cape_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, batch.cape_vertex_buffer.slice(..));
            render_pass
                .set_index_buffer(batch.cape_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..batch.cape_index_count, 0, 0..1);
        }
    }

    fn render_target_extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.render_target_size[0].max(1),
            height: self.render_target_size[1].max(1),
            depth_or_array_layers: 1,
        }
    }
}

fn create_preview_vertex_buffer(
    device: &wgpu::Device,
    label: &'static str,
    vertices: &[GpuPreviewVertex],
) -> wgpu::Buffer {
    if vertices.is_empty() {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<GpuPreviewVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        })
    } else {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }
}

fn create_preview_index_buffer(
    device: &wgpu::Device,
    label: &'static str,
    indices: &[u32],
) -> wgpu::Buffer {
    if indices.is_empty() {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::INDEX,
            mapped_at_creation: false,
        })
    } else {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }
}

fn create_preview_render_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    sample_count: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_count.max(1),
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = create_preview_texture_bind_group(device, layout, sampler, &view, label);
    (texture, view, bind_group)
}

fn create_skin_preview_sampler(device: &wgpu::Device, label: &'static str) -> wgpu::Sampler {
    // Keep the sampler fully linear so wgpu can enable anisotropy; the fragment shaders
    // switch to exact texel loads whenever the skin is magnified to preserve crisp pixels.
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        anisotropy_clamp: SKIN_PREVIEW_ANISOTROPY_CLAMP,
        ..Default::default()
    })
}

fn create_preview_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    label: &'static str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn preview_mip_level_count(size: [u32; 2]) -> u32 {
    size[0].max(size[1]).max(1).ilog2() + 1
}

fn write_preview_texture_mips(queue: &wgpu::Queue, texture: &wgpu::Texture, image: &RgbaImage) {
    let mut mip_image = image.clone();
    let mip_level_count = preview_mip_level_count([image.width(), image.height()]);

    for mip_level in 0..mip_level_count {
        let width = mip_image.width().max(1);
        let height = mip_image.height().max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            mip_image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if mip_level + 1 < mip_level_count {
            let next_width = (width / 2).max(1);
            let next_height = (height / 2).max(1);
            mip_image = resize_preview_mip(&mip_image, next_width, next_height);
        }
    }
}

fn resize_preview_mip(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let mut premultiplied = image.clone();
    for pixel in premultiplied.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }

    let mut resized = image::imageops::resize(&premultiplied, width, height, FilterType::Triangle);
    for pixel in resized.pixels_mut() {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            continue;
        }

        let scale = 255.0 / f32::from(alpha);
        pixel[0] = (f32::from(pixel[0]) * scale).round().clamp(0.0, 255.0) as u8;
        pixel[1] = (f32::from(pixel[1]) * scale).round().clamp(0.0, 255.0) as u8;
        pixel[2] = (f32::from(pixel[2]) * scale).round().clamp(0.0, 255.0) as u8;
    }

    resized
}

fn create_preview_color_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    sample_count: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_count.max(1),
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_preview_depth_texture(
    device: &wgpu::Device,
    size: [u32; 2],
    sample_count: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_count.max(1),
        dimension: wgpu::TextureDimension::D2,
        format: SKIN_PREVIEW_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[allow(dead_code)]
struct SkinPreviewWgpuCallback {
    skin_vertices: Vec<GpuPreviewVertex>,
    skin_indices: Vec<u32>,
    cape_vertices: Vec<GpuPreviewVertex>,
    cape_indices: Vec<u32>,
    skin_sample: Arc<RgbaImage>,
    cape_sample: Option<Arc<RgbaImage>>,
    skin_hash: u64,
    cape_hash: Option<u64>,
    target_format: wgpu::TextureFormat,
    msaa_samples: u32,
}

#[allow(dead_code)]
impl SkinPreviewWgpuCallback {
    fn from_scene(
        _rect: Rect,
        triangles: &[RenderTriangle],
        skin_sample: Arc<RgbaImage>,
        cape_sample: Option<Arc<RgbaImage>>,
        target_format: wgpu::TextureFormat,
        msaa_samples: u32,
    ) -> Self {
        let mut skin_vertices = Vec::with_capacity(triangles.len() * 3);
        let mut skin_indices = Vec::with_capacity(triangles.len() * 3);
        let mut cape_vertices = Vec::new();
        let mut cape_indices = Vec::new();

        for tri in triangles {
            let target = match tri.texture {
                TriangleTexture::Skin => (&mut skin_vertices, &mut skin_indices),
                TriangleTexture::Cape => {
                    if cape_sample.is_some() {
                        (&mut cape_vertices, &mut cape_indices)
                    } else {
                        continue;
                    }
                }
            };
            let base = target.0.len() as u32;
            for i in 0..3 {
                let color = tri.color.to_normalized_gamma_f32();
                target.0.push(GpuPreviewVertex {
                    pos_points: [tri.pos[i].x, tri.pos[i].y],
                    camera_z: tri.depth[i].max(SKIN_PREVIEW_NEAR + 0.000_1),
                    uv: [tri.uv[i].x, tri.uv[i].y],
                    color,
                });
            }
            target
                .1
                .extend_from_slice(&[base, base.saturating_add(1), base.saturating_add(2)]);
        }

        let skin_hash = hash_rgba_image(&skin_sample);
        let cape_hash = cape_sample
            .as_ref()
            .map(|image| hash_rgba_image(image.as_ref()));

        Self {
            skin_vertices,
            skin_indices,
            cape_vertices,
            cape_indices,
            skin_sample,
            cape_sample,
            skin_hash,
            cape_hash,
            target_format,
            msaa_samples,
        }
    }
}

impl egui_wgpu::CallbackTrait for SkinPreviewWgpuCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources = callback_resources
            .entry::<SkinPreviewWgpuResources>()
            .or_insert_with(|| {
                SkinPreviewWgpuResources::new(device, self.target_format, self.msaa_samples)
            });
        if resources.pipeline.is_none() {
            *resources =
                SkinPreviewWgpuResources::new(device, self.target_format, self.msaa_samples);
        }
        if resources.target_format != self.target_format
            || resources.msaa_samples != self.msaa_samples
        {
            *resources =
                SkinPreviewWgpuResources::new(device, self.target_format, self.msaa_samples);
        }
        if resources.pipeline.is_none() {
            return Vec::new();
        }
        resources.update_uniform(
            queue,
            [
                screen_descriptor.size_in_pixels[0] as f32 / screen_descriptor.pixels_per_point,
                screen_descriptor.size_in_pixels[1] as f32 / screen_descriptor.pixels_per_point,
            ],
        );

        resources.update_texture(
            device,
            queue,
            TextureSlot::Skin,
            self.skin_hash,
            &self.skin_sample,
        );
        if let (Some(cape_hash), Some(cape_sample)) = (self.cape_hash, self.cape_sample.as_ref()) {
            resources.update_texture(device, queue, TextureSlot::Cape, cape_hash, cape_sample);
        } else {
            resources.cape_texture = None;
        }

        resources.update_mesh_buffers(
            device,
            queue,
            &self.skin_vertices,
            &self.skin_indices,
            &self.cape_vertices,
            &self.cape_indices,
        );

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<SkinPreviewWgpuResources>() else {
            return;
        };

        let Some(pipeline) = resources.pipeline.as_ref() else {
            return;
        };
        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(1, &resources.uniform_bind_group, &[]);

        if let Some(texture) = resources.skin_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, resources.skin_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                resources.skin_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..resources.skin_index_count, 0, 0..1);
        }

        if let Some(texture) = resources.cape_texture.as_ref() {
            render_pass.set_bind_group(0, &texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, resources.cape_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                resources.cape_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..resources.cape_index_count, 0, 0..1);
        }
    }
}

enum TextureSlot {
    Skin,
    Cape,
}

struct UploadedPreviewTexture {
    hash: u64,
    size: [u32; 2],
    bind_group: wgpu::BindGroup,
    _texture: wgpu::Texture,
}

#[allow(dead_code)]
struct SkinPreviewWgpuResources {
    pipeline: Option<wgpu::RenderPipeline>,
    texture_bind_group_layout: Option<wgpu::BindGroupLayout>,
    texture_sampler: wgpu::Sampler,
    uniform_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    skin_texture: Option<UploadedPreviewTexture>,
    cape_texture: Option<UploadedPreviewTexture>,
    skin_vertex_buffer: wgpu::Buffer,
    skin_index_buffer: wgpu::Buffer,
    cape_vertex_buffer: wgpu::Buffer,
    cape_index_buffer: wgpu::Buffer,
    skin_vertex_capacity: usize,
    skin_index_capacity: usize,
    cape_vertex_capacity: usize,
    cape_index_capacity: usize,
    skin_index_count: u32,
    cape_index_count: u32,
    target_format: wgpu::TextureFormat,
    msaa_samples: u32,
}

#[allow(dead_code)]
impl SkinPreviewWgpuResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, msaa_samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "shaders/skin_preview.wgsl"
            ))),
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-texture-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let texture_sampler = create_skin_preview_sampler(device, "skins-preview-texture-sampler");

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skins-preview-uniform-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-uniform-buffer"),
            size: std::mem::size_of::<GpuPreviewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skins-preview-uniform-bind-group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skins-preview-pipeline-layout"),
            bind_group_layouts: &[&texture_bind_group_layout, &uniform_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skins-preview-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuPreviewVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32,
                        2 => Float32x2,
                        3 => Float32x4
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SKIN_PREVIEW_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: msaa_samples.max(1),
                mask: !0,
                alpha_to_coverage_enabled: msaa_samples > 1,
            },
            multiview: None,
            cache: None,
        });

        let empty_vertex = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-empty-vertex"),
            size: std::mem::size_of::<GpuPreviewVertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let empty_index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skins-preview-empty-index"),
            size: std::mem::size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline: Some(pipeline),
            texture_bind_group_layout: Some(texture_bind_group_layout),
            texture_sampler,
            uniform_bind_group,
            uniform_buffer,
            skin_texture: None,
            cape_texture: None,
            skin_vertex_buffer: empty_vertex.clone(),
            skin_index_buffer: empty_index.clone(),
            cape_vertex_buffer: empty_vertex,
            cape_index_buffer: empty_index,
            skin_vertex_capacity: 1,
            skin_index_capacity: 1,
            cape_vertex_capacity: 1,
            cape_index_capacity: 1,
            skin_index_count: 0,
            cape_index_count: 0,
            target_format,
            msaa_samples: msaa_samples.max(1),
        }
    }

    fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slot: TextureSlot,
        hash: u64,
        image: &RgbaImage,
    ) {
        let size = [image.width(), image.height()];
        let target = match slot {
            TextureSlot::Skin => &mut self.skin_texture,
            TextureSlot::Cape => &mut self.cape_texture,
        };

        if target
            .as_ref()
            .is_some_and(|uploaded| uploaded.hash == hash && uploaded.size == size)
        {
            return;
        }

        let Some(layout) = self.texture_bind_group_layout.as_ref() else {
            return;
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("skins-preview-texture"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: preview_mip_level_count(size),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        write_preview_texture_mips(queue, &texture, image);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = create_preview_texture_bind_group(
            device,
            layout,
            &self.texture_sampler,
            &view,
            "skins-preview-texture-bind-group",
        );

        *target = Some(UploadedPreviewTexture {
            hash,
            size,
            bind_group,
            _texture: texture,
        });
    }

    fn update_mesh_buffers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        skin_vertices: &[GpuPreviewVertex],
        skin_indices: &[u32],
        cape_vertices: &[GpuPreviewVertex],
        cape_indices: &[u32],
    ) {
        ensure_buffer(
            device,
            &mut self.skin_vertex_buffer,
            &mut self.skin_vertex_capacity,
            skin_vertices.len().max(1),
            std::mem::size_of::<GpuPreviewVertex>(),
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "skins-preview-skin-vertex-buffer",
        );
        ensure_buffer(
            device,
            &mut self.skin_index_buffer,
            &mut self.skin_index_capacity,
            skin_indices.len().max(1),
            std::mem::size_of::<u32>(),
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            "skins-preview-skin-index-buffer",
        );
        ensure_buffer(
            device,
            &mut self.cape_vertex_buffer,
            &mut self.cape_vertex_capacity,
            cape_vertices.len().max(1),
            std::mem::size_of::<GpuPreviewVertex>(),
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "skins-preview-cape-vertex-buffer",
        );
        ensure_buffer(
            device,
            &mut self.cape_index_buffer,
            &mut self.cape_index_capacity,
            cape_indices.len().max(1),
            std::mem::size_of::<u32>(),
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            "skins-preview-cape-index-buffer",
        );

        if !skin_vertices.is_empty() {
            queue.write_buffer(
                &self.skin_vertex_buffer,
                0,
                bytemuck::cast_slice(skin_vertices),
            );
        }
        if !skin_indices.is_empty() {
            queue.write_buffer(
                &self.skin_index_buffer,
                0,
                bytemuck::cast_slice(skin_indices),
            );
        }
        if !cape_vertices.is_empty() {
            queue.write_buffer(
                &self.cape_vertex_buffer,
                0,
                bytemuck::cast_slice(cape_vertices),
            );
        }
        if !cape_indices.is_empty() {
            queue.write_buffer(
                &self.cape_index_buffer,
                0,
                bytemuck::cast_slice(cape_indices),
            );
        }

        self.skin_index_count = skin_indices.len() as u32;
        self.cape_index_count = cape_indices.len() as u32;
    }

    fn update_uniform(&self, queue: &wgpu::Queue, screen_size_points: [f32; 2]) {
        let uniform = GpuPreviewUniform {
            screen_size_points,
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

fn ensure_buffer(
    device: &wgpu::Device,
    existing: &mut wgpu::Buffer,
    capacity: &mut usize,
    desired_items: usize,
    bytes_per_item: usize,
    usage: wgpu::BufferUsages,
    label: &'static str,
) {
    if *capacity >= desired_items {
        return;
    }
    let new_capacity = desired_items.next_power_of_two().max(1);
    *capacity = new_capacity;
    *existing = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (new_capacity * bytes_per_item) as u64,
        usage,
        mapped_at_creation: false,
    });
}

pub(super) fn hash_rgba_image(image: &RgbaImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.width().hash(&mut hasher);
    image.height().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}
