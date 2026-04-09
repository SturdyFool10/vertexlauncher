use super::*;

#[allow(dead_code)]
#[derive(Clone)]
pub(in super::super) struct PreviewHistory {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

#[allow(dead_code)]
pub(super) fn render_cpu_post_aa_scene(
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
