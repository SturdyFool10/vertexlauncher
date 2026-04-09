use super::*;

#[derive(Clone, Copy)]
pub(super) struct FaceExpressionPose {
    look_x: f32,
    look_y: f32,
    brow_raise_left: f32,
    brow_raise_right: f32,
    brow_squeeze: f32,
    upper_lid_left: f32,
    upper_lid_right: f32,
    lower_lid: f32,
}

pub(super) fn compute_expression_pose(
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

pub(in super::super) fn eye_lid_rects(spec: EyeExpressionSpec) -> (TextureRectU32, TextureRectU32) {
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

pub(in super::super) fn compatibility_score(
    eye: EyeExpressionSpec,
    brow: BrowExpressionSpec,
) -> i32 {
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
pub(in super::super) struct FacePixelRect {
    pub(in super::super) left: f32,
    pub(in super::super) top: f32,
    pub(in super::super) width: f32,
    pub(in super::super) height: f32,
}

impl FacePixelRect {
    pub(in super::super) fn center_x(self) -> f32 {
        4.0 - (self.left + self.width * 0.5)
    }

    pub(in super::super) fn top_y(self) -> f32 {
        self.top
    }

    pub(in super::super) fn bottom_y(self) -> f32 {
        self.top + self.height
    }
}

fn face_left_from_center(center_x: f32, width: f32) -> f32 {
    4.0 - center_x - width * 0.5
}

fn face_top_from_center(center_y: f32, height: f32) -> f32 {
    32.0 - center_y - height * 0.5
}

pub(in super::super) fn eye_face_rects(spec: EyeExpressionSpec) -> (FacePixelRect, FacePixelRect) {
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

pub(super) fn add_expression_triangles(
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
    let right_lid_world_top = 32.0 - right_upper_top - right_lid_h * 0.5;
    let left_lid_world_top = 32.0 - left_upper_top - left_lid_h * 0.5;
    let right_eye_world_bottom = 32.0 - right_eye_rect.top_y() - right_eye_rect.height * 1.5;
    let left_eye_world_bottom = 32.0 - left_eye_rect.top_y() - left_eye_rect.height * 1.5;
    let right_lid_world_bottom = (right_lid_world_top - right_lid_h)
        + pose.upper_lid_right.clamp(0.0, 1.0)
            * (right_eye_world_bottom - (right_lid_world_top - right_lid_h));
    let left_lid_world_bottom = (left_lid_world_top - left_lid_h)
        + pose.upper_lid_left.clamp(0.0, 1.0)
            * (left_eye_world_bottom - (left_lid_world_top - left_lid_h));
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
