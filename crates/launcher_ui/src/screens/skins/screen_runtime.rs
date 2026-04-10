use super::*;

pub(super) fn apply_skin_screen_runtime_settings(
    state: &mut SkinManagerState,
    active_launch_auth: Option<&LaunchAuthContext>,
    skin_manager_opened: bool,
    skin_manager_account_switched: bool,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    skin_preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
    preview_motion_blur_enabled: bool,
    preview_motion_blur_amount: f32,
    preview_motion_blur_shutter_frames: f32,
    preview_motion_blur_sample_count: i32,
    preview_3d_layers_enabled: bool,
    expressions_enabled: bool,
) {
    state.sync_active_account(active_launch_auth);
    if skin_manager_opened {
        state.refresh_on_open_pending = true;
        tracing::info!(
            target: "vertexlauncher/skins",
            "Skin manager opened; queuing active profile refresh."
        );
    }
    if skin_manager_account_switched {
        state.refresh_on_open_pending = true;
        tracing::info!(
            target: "vertexlauncher/skins",
            "Active account changed while skin manager is open; queuing profile refresh."
        );
    }
    state.wgpu_target_format = wgpu_target_format;
    state.preview_msaa_samples = skin_preview_msaa_samples.max(1);
    state.preview_aa_mode = preview_aa_mode;
    state.preview_texel_aa_mode = preview_texel_aa_mode;
    state.preview_motion_blur_enabled = preview_motion_blur_enabled;
    state.preview_motion_blur_amount = preview_motion_blur_amount.clamp(0.0, 1.0);
    state.preview_motion_blur_shutter_frames = preview_motion_blur_shutter_frames.max(0.0);
    state.preview_motion_blur_sample_count = preview_motion_blur_sample_count.max(1) as usize;
    state.preview_3d_layers_enabled = preview_3d_layers_enabled;
    state.expressions_enabled = expressions_enabled;
    if state.last_preview_aa_mode != state.preview_aa_mode
        || state.last_preview_texel_aa_mode != state.preview_texel_aa_mode
        || state.last_preview_motion_blur_enabled != state.preview_motion_blur_enabled
        || (state.last_preview_motion_blur_amount - state.preview_motion_blur_amount).abs()
            > f32::EPSILON
        || (state.last_preview_motion_blur_shutter_frames
            - state.preview_motion_blur_shutter_frames)
            .abs()
            > f32::EPSILON
        || state.last_preview_motion_blur_sample_count != state.preview_motion_blur_sample_count
        || state.last_preview_3d_layers_enabled != state.preview_3d_layers_enabled
        || state.last_expressions_enabled != state.expressions_enabled
    {
        state.last_preview_aa_mode = state.preview_aa_mode;
        state.last_preview_texel_aa_mode = state.preview_texel_aa_mode;
        state.last_preview_motion_blur_enabled = state.preview_motion_blur_enabled;
        state.last_preview_motion_blur_amount = state.preview_motion_blur_amount;
        state.last_preview_motion_blur_shutter_frames = state.preview_motion_blur_shutter_frames;
        state.last_preview_motion_blur_sample_count = state.preview_motion_blur_sample_count;
        state.last_preview_3d_layers_enabled = state.preview_3d_layers_enabled;
        state.last_expressions_enabled = state.expressions_enabled;
    }
}

pub(super) fn prepare_skin_screen_frame(state: &mut SkinManagerState, ctx: &egui::Context) {
    state.poll_worker(ctx);
    state.poll_pick_skin_result(ctx);
    state.try_consume_open_refresh();
    state.ensure_skin_texture(ctx);
    state.ensure_default_elytra_texture(ctx);
    state.ensure_cape_texture(ctx);
}

pub(super) fn schedule_skin_screen_repaint(state: &SkinManagerState, ctx: &egui::Context) {
    ctx.request_repaint_after(Duration::from_secs_f32(1.0 / PREVIEW_TARGET_FPS));
    if state.pick_skin_in_progress {
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}
