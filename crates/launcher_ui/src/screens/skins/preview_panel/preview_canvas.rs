use super::super::*;

pub(super) fn render_preview_canvas(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
) -> (Rect, bool) {
    let viewport_width = ui.clip_rect().width().max(1.0);
    let desired = egui::vec2(
        ui.available_width().min(viewport_width).max(1.0),
        PREVIEW_HEIGHT.min(ui.available_height().max(280.0)),
    );
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    paint_preview_panel_background(ui, &painter, rect);

    let now = ui.input(|i| i.time);
    let dt = state.consume_frame_dt(now);
    let gamepad_orbit_input = ui
        .ctx()
        .data_mut(|data| data.get_temp::<f32>(egui::Id::new("skins_screen_gamepad_orbit_input")))
        .unwrap_or(0.0);

    if response.drag_started() {
        state.begin_manual_camera_control(now);
        state.camera_drag_active = true;
        state.camera_drag_velocity = 0.0;
        state.camera_inertial_velocity = 0.0;
    }
    if state.camera_drag_active && response.dragged() {
        let drag_step_x = ui.input(|i| i.pointer.delta().x);
        let yaw_step = drag_step_x * CAMERA_DRAG_SENSITIVITY_RAD_PER_POINT;
        state.camera_yaw_offset += yaw_step;
        if dt > 0.000_1 && yaw_step.abs() > 0.0 {
            state.camera_drag_velocity = yaw_step / dt;
        }
    }
    if state.camera_drag_active && response.drag_stopped() {
        state.camera_drag_active = false;
        state.camera_inertial_velocity = state.camera_drag_velocity;
        if state.camera_inertial_velocity.abs() < CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC {
            state.camera_inertial_velocity = 0.0;
            state.finish_manual_camera_control(now);
        }
    }

    if !state.camera_drag_active && state.orbit_pause_started_at.is_some() {
        state.camera_yaw_offset += state.camera_inertial_velocity * dt;
        if dt > 0.0 {
            let friction = (-CAMERA_INERTIA_FRICTION_PER_SEC * dt).exp();
            state.camera_inertial_velocity *= friction;
        }
        if state.camera_inertial_velocity.abs() < CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC {
            state.camera_inertial_velocity = 0.0;
            state.finish_manual_camera_control(now);
        }
    }

    if !state.camera_drag_active {
        let orbit_strength = if gamepad_orbit_input.abs() > GAMEPAD_ORBIT_DEADZONE {
            let normalized = (gamepad_orbit_input.abs() - GAMEPAD_ORBIT_DEADZONE)
                / (1.0 - GAMEPAD_ORBIT_DEADZONE);
            normalized.clamp(0.0, 1.0) * gamepad_orbit_input.signum()
        } else {
            0.0
        };
        if orbit_strength.abs() > 0.0 {
            state.begin_manual_camera_control(now);
            state.camera_inertial_velocity = orbit_strength * GAMEPAD_ORBIT_MAX_RAD_PER_SEC;
            state.camera_yaw_offset += state.camera_inertial_velocity * dt;
        }
    }

    let orbit_time = state.effective_orbit_time(now);
    let yaw = ((orbit_time / PREVIEW_ORBIT_SECONDS) as f32) * std::f32::consts::TAU
        + state.camera_yaw_offset;
    let yaw_velocity = if state.camera_drag_active {
        state.camera_drag_velocity
    } else if state.orbit_pause_started_at.is_some() {
        state.camera_inertial_velocity
    } else {
        std::f32::consts::TAU / PREVIEW_ORBIT_SECONDS as f32
    };

    let blend_target = match state.preview_motion_mode {
        PreviewMotionMode::Idle => 0.0,
        PreviewMotionMode::Walk => 1.0,
    };
    let blend_speed = if blend_target > state.preview_motion_blend {
        5.4
    } else {
        4.2
    };
    let blend_alpha = 1.0 - (-blend_speed * dt.max(0.0)).exp();
    state.preview_motion_blend += (blend_target - state.preview_motion_blend) * blend_alpha;
    state.preview_motion_blend = state.preview_motion_blend.clamp(0.0, 1.0);
    let preview_pose = PreviewPose {
        time_seconds: now as f32,
        idle_cycle: (now as f32 * 1.15).sin(),
        walk_cycle: (now as f32 * 3.3).sin(),
        locomotion_blend: state.preview_motion_blend,
    };

    state.refresh_expression_layout_cache();

    let skin_sample = state.skin_sample.as_ref().cloned();
    let cape_sample = state.cape_sample.as_ref().cloned();
    let default_elytra_sample = state.default_elytra_sample.as_ref().cloned();
    let cape_uv = state.cape_uv;
    let variant = state.pending_variant;
    let show_elytra = state.show_elytra;
    let Some(wgpu_target_format) = state.wgpu_target_format else {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    let mut muted = style::muted(ui);
                    muted.wrap = true;
                    let _ = text_ui.label(
                        ui,
                        "skins_preview_renderer_initializing",
                        "Skin preview is waiting for the GPU renderer to become ready.",
                        &muted,
                    );
                },
            );
        });
        return (rect, true);
    };
    let preview_msaa_samples = state.preview_msaa_samples;
    let preview_aa_mode = state.preview_aa_mode;
    let preview_motion_blur_enabled = state.preview_motion_blur_enabled;
    let preview_motion_blur_amount = state.preview_motion_blur_amount;
    let preview_motion_blur_shutter_frames = state.preview_motion_blur_shutter_frames;
    let preview_motion_blur_sample_count = state.preview_motion_blur_sample_count;
    let preview_3d_layers_enabled = state.preview_3d_layers_enabled;
    let preview_texel_aa_mode = state.preview_texel_aa_mode;
    if state.skin_texture.is_some() {
        render_preview_character(
            ui,
            rect,
            skin_sample,
            cape_sample,
            default_elytra_sample,
            cape_uv,
            yaw,
            yaw_velocity,
            preview_pose,
            variant,
            show_elytra,
            state.expressions_enabled,
            state.cached_expression_layout,
            wgpu_target_format,
            preview_msaa_samples,
            preview_aa_mode,
            preview_texel_aa_mode,
            preview_motion_blur_enabled,
            preview_motion_blur_amount,
            preview_motion_blur_shutter_frames,
            preview_motion_blur_sample_count,
            preview_3d_layers_enabled,
        );
    } else {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    let mut muted = style::muted(ui);
                    muted.wrap = false;
                    let _ = text_ui.label(ui, "skins_preview_no_skin", "No skin loaded", &muted);
                },
            );
        });
    }

    (rect, true)
}

fn paint_preview_panel_background(ui: &Ui, painter: &egui::Painter, rect: Rect) {
    let fill = ui.visuals().faint_bg_color;

    painter.rect_filled(rect, CornerRadius::same(8), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Outside,
    );
}
