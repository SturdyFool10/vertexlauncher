use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use auth::{CachedAccount, MinecraftProfileState, MinecraftSkinVariant};
use bytemuck::{Pod, Zeroable};
use config::{SkinPreviewAaMode, SkinPreviewTexelAaMode};
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui};
use image::{RgbaImage, imageops::FilterType};
use launcher_runtime as tokio_runtime;
use textui::TextUi;
use textui_egui::{gamepad_scroll, prelude::*};

use super::LaunchAuthContext;
use crate::{
    notification, privacy,
    ui::{components::image_textures, style},
};

#[path = "skins_expressions.rs"]
mod skins_expressions;
#[path = "skins_preview.rs"]
mod skins_preview;
#[path = "skins_preview_gpu.rs"]
mod skins_preview_gpu;
#[path = "skins_state.rs"]
mod skins_state;
use self::skins_expressions::*;
use self::skins_preview::*;
use self::skins_preview_gpu::*;
use self::skins_state::*;

const PREVIEW_ORBIT_SECONDS: f64 = 45.0;
const PREVIEW_TARGET_FPS: f32 = 60.0;
const PREVIEW_HEIGHT: f32 = 460.0;
const CAMERA_DRAG_SENSITIVITY_RAD_PER_POINT: f32 = 0.0046;
const GAMEPAD_ORBIT_MAX_RAD_PER_SEC: f32 = 3.4;
const GAMEPAD_ORBIT_DEADZONE: f32 = 0.18;
const CAMERA_INERTIA_FRICTION_PER_SEC: f32 = 2.0;
const CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC: f32 = 0.015;
const UV_EDGE_INSET_BASE_TEXELS: f32 = 0.08;
const UV_EDGE_INSET_OVERLAY_TEXELS: f32 = 0.5;
const CAPE_TILE_WIDTH_MIN: f32 = 132.0;
const CAPE_TILE_HEIGHT: f32 = 186.0;
const SKIN_PREVIEW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SKIN_PREVIEW_NEAR: f32 = 1.5;
const SKIN_PREVIEW_ANISOTROPY_CLAMP: u16 = 16;
const MOTION_BLUR_MIN_ANGULAR_SPAN: f32 = 0.015;
const FORCE_MOTION_FOCUS_ID: &str = "skins_force_motion_focus";
const FORCE_MODEL_FOCUS_ID: &str = "skins_force_model_focus";
const CLASSIC_MODEL_BUTTON_ID_KEY: &str = "skins_classic_model_button_id";
const SLIM_MODEL_BUTTON_ID_KEY: &str = "skins_slim_model_button_id";

pub fn purge_inactive_state(ctx: &egui::Context) {
    let state_id = egui::Id::new("skins_screen_state");
    ctx.data_mut(|data| data.insert_temp(state_id, SkinManagerState::default()));
}

pub fn set_gamepad_orbit_input(ctx: &egui::Context, input: f32) {
    let input_id = egui::Id::new("skins_screen_gamepad_orbit_input");
    ctx.data_mut(|data| data.insert_temp(input_id, input.clamp(-1.0, 1.0)));
}

pub fn request_motion_focus(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(FORCE_MOTION_FOCUS_ID), true));
}

pub fn request_model_focus(ctx: &egui::Context, variant: MinecraftSkinVariant) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(FORCE_MODEL_FOCUS_ID), variant));
}

pub fn classic_model_button_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(CLASSIC_MODEL_BUTTON_ID_KEY)))
}

pub fn slim_model_button_id(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::new(SLIM_MODEL_BUTTON_ID_KEY)))
}

fn take_model_focus_request(ctx: &egui::Context) -> Option<MinecraftSkinVariant> {
    ctx.data_mut(|data| {
        let key = egui::Id::new(FORCE_MODEL_FOCUS_ID);
        let value = data.get_temp::<MinecraftSkinVariant>(key);
        if value.is_some() {
            data.remove::<MinecraftSkinVariant>(key);
        }
        value
    })
}

pub fn render(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    _selected_instance_id: Option<&str>,
    active_launch_auth: Option<&LaunchAuthContext>,
    skin_manager_opened: bool,
    skin_manager_account_switched: bool,
    streamer_mode: bool,
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
    let state_id = ui.make_persistent_id("skins_screen_state");
    let mut state = ui
        .ctx()
        .data_mut(|data| data.get_temp::<SkinManagerState>(state_id))
        .unwrap_or_default();

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
        state.preview_texture = None;
        state.preview_history = None;
        state.last_preview_aa_mode = state.preview_aa_mode;
        state.last_preview_texel_aa_mode = state.preview_texel_aa_mode;
        state.last_preview_motion_blur_enabled = state.preview_motion_blur_enabled;
        state.last_preview_motion_blur_amount = state.preview_motion_blur_amount;
        state.last_preview_motion_blur_shutter_frames = state.preview_motion_blur_shutter_frames;
        state.last_preview_motion_blur_sample_count = state.preview_motion_blur_sample_count;
        state.last_preview_3d_layers_enabled = state.preview_3d_layers_enabled;
        state.last_expressions_enabled = state.expressions_enabled;
    }
    state.poll_worker(ui.ctx());
    state.poll_pick_skin_result(ui.ctx());
    state.try_consume_open_refresh();
    state.ensure_skin_texture(ui.ctx());
    state.ensure_default_elytra_texture(ui.ctx());
    state.ensure_cape_texture(ui.ctx());
    ui.ctx()
        .request_repaint_after(Duration::from_secs_f32(1.0 / PREVIEW_TARGET_FPS));
    if state.pick_skin_in_progress {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    gamepad_scroll(
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        ui,
        |ui| render_contents(ui, text_ui, &mut state, streamer_mode),
    );

    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
}

fn render_contents(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    state: &mut SkinManagerState,
    streamer_mode: bool,
) {
    let body = style::body(ui);
    let muted = style::muted(ui);

    if let Some(name) = state.active_player_name.as_deref() {
        let _ = text_ui.label(
            ui,
            "skins_active_user",
            &format!(
                "Active account: {}",
                privacy::redact_account_label(streamer_mode, name)
            ),
            &body,
        );
    } else {
        let _ = text_ui.label(
            ui,
            "skins_no_active_user",
            "Sign in with a Minecraft account to manage skins and capes.",
            &muted,
        );
        return;
    }

    ui.add_space(style::SPACE_MD);
    if render_preview(ui, text_ui, state) {
        state.show_elytra = !state.show_elytra;
    }
    ui.add_space(style::SPACE_LG);

    let button_style =
        style::neutral_button_with_min_size(ui, egui::vec2(160.0, style::CONTROL_HEIGHT));
    let viewport_width = ui.clip_rect().width().max(1.0);
    let full_width = ui.available_width().min(viewport_width).max(1.0);
    let mut full_width_button_style = button_style.clone();
    full_width_button_style.min_size = egui::vec2(full_width, style::CONTROL_HEIGHT);

    if text_ui
        .button(
            ui,
            "skins_refresh_profile",
            "Refresh profile",
            &full_width_button_style,
        )
        .clicked()
    {
        state.start_refresh();
    }

    ui.add_space(style::SPACE_LG);
    let _ = text_ui.label(
        ui,
        "skins_picker_heading",
        "Skin Image",
        &style::section_heading(ui),
    );

    render_skin_drop_zone(ui, text_ui, state);

    if state.pick_skin_in_progress {
        ui.add_space(style::SPACE_XS);
        ui.horizontal(|ui| {
            ui.spinner();
            let _ = text_ui.label(
                ui,
                "skins_pick_file_loading",
                "Loading selected skin in the background...",
                &muted,
            );
        });
    }

    if let Some(path) = state.pending_skin_path.as_deref() {
        ui.add_space(style::SPACE_XS);
        let _ = text_ui.label(
            ui,
            "skins_selected_path",
            path.as_os_str().to_string_lossy().as_ref(),
            &muted,
        );
    }

    ui.add_space(style::SPACE_SM);
    let mut model_button_style = button_style.clone();
    let model_button_gap = style::SPACE_XS;
    let half_width = ((ui.available_width() - model_button_gap) * 0.5).max(1.0);
    model_button_style.min_size = egui::vec2(half_width, style::CONTROL_HEIGHT);
    model_button_style.fill = ui.visuals().widgets.inactive.weak_bg_fill;
    model_button_style.fill_hovered = ui.visuals().widgets.hovered.bg_fill.gamma_multiply(1.05);
    model_button_style.fill_active = ui.visuals().selection.bg_fill.gamma_multiply(0.92);
    model_button_style.fill_selected = ui.visuals().selection.bg_fill.gamma_multiply(0.78);
    model_button_style.stroke = ui.visuals().widgets.hovered.bg_stroke;
    let _ = text_ui.label(ui, "skins_model_label", "Model:", &body);
    ui.add_space(style::SPACE_XS);
    let model_focus_request = take_model_focus_request(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(model_button_gap, style::SPACE_XS);
        let classic_response = text_ui.selectable_button(
            ui,
            "skins_model_classic",
            "Classic",
            state.pending_variant == MinecraftSkinVariant::Classic,
            &model_button_style,
        );
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new(CLASSIC_MODEL_BUTTON_ID_KEY),
                classic_response.id,
            )
        });
        if model_focus_request == Some(MinecraftSkinVariant::Classic) {
            classic_response.request_focus();
        }
        if classic_response.clicked() {
            state.pending_variant = MinecraftSkinVariant::Classic;
        }
        let slim_response = text_ui.selectable_button(
            ui,
            "skins_model_slim",
            "Slim (Alex)",
            state.pending_variant == MinecraftSkinVariant::Slim,
            &model_button_style,
        );
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new(SLIM_MODEL_BUTTON_ID_KEY), slim_response.id)
        });
        if model_focus_request == Some(MinecraftSkinVariant::Slim) {
            slim_response.request_focus();
        }
        if slim_response.clicked() {
            state.pending_variant = MinecraftSkinVariant::Slim;
        }
    });

    ui.add_space(style::SPACE_MD);
    let _ = text_ui.label(
        ui,
        "skins_cape_heading",
        "Cape",
        &style::section_heading(ui),
    );
    ui.add_space(style::SPACE_XS);

    render_cape_grid(ui, text_ui, state);

    ui.add_space(style::SPACE_MD);
    let mut save_style = button_style.clone();
    let save_width = ui.available_width().min(viewport_width).max(1.0);
    save_style.min_size = egui::vec2(save_width, style::CONTROL_HEIGHT_LG);
    save_style.fill = ui.visuals().selection.bg_fill;
    save_style.fill_hovered = ui.visuals().selection.bg_fill.gamma_multiply(1.15);
    save_style.fill_active = ui.visuals().selection.bg_fill.gamma_multiply(0.92);
    save_style.text_color = ui.visuals().strong_text_color();

    let can_save = state.can_save();
    let response = ui.add_enabled_ui(can_save && !state.save_in_progress, |ui| {
        text_ui.button(ui, "skins_save", "Save", &save_style)
    });
    if response.inner.clicked() {
        state.start_save();
    }
}

fn render_preview(ui: &mut Ui, text_ui: &mut TextUi, state: &mut SkinManagerState) -> bool {
    let viewport_width = ui.clip_rect().width().max(1.0);
    let desired = egui::vec2(
        ui.available_width().min(viewport_width).max(1.0),
        PREVIEW_HEIGHT.min(ui.available_height().max(280.0)),
    );
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    paint_preview_background(ui, &painter, rect);

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

    let skin_texture = state.skin_texture.as_ref();
    let cape_texture = state.cape_texture.as_ref();
    let default_elytra_texture = state.default_elytra_texture.as_ref();
    let skin_sample = state.skin_sample.as_ref().cloned();
    let cape_sample = state.cape_sample.as_ref().cloned();
    let default_elytra_sample = state.default_elytra_sample.as_ref().cloned();
    let cape_uv = state.cape_uv;
    let variant = state.pending_variant;
    let show_elytra = state.show_elytra;
    let wgpu_target_format = state.wgpu_target_format;
    let preview_msaa_samples = state.preview_msaa_samples;
    let preview_aa_mode = state.preview_aa_mode;
    let preview_motion_blur_enabled = state.preview_motion_blur_enabled;
    let preview_motion_blur_amount = state.preview_motion_blur_amount;
    let preview_motion_blur_shutter_frames = state.preview_motion_blur_shutter_frames;
    let preview_motion_blur_sample_count = state.preview_motion_blur_sample_count;
    let preview_3d_layers_enabled = state.preview_3d_layers_enabled;
    let preview_texel_aa_mode = state.preview_texel_aa_mode;
    let preview_texture = &mut state.preview_texture;
    let preview_history = &mut state.preview_history;

    if let Some(skin_texture) = skin_texture {
        draw_character(
            ui,
            &painter,
            rect,
            skin_texture,
            cape_texture,
            default_elytra_texture,
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
            preview_texture,
            preview_history,
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

    let button_size = egui::vec2(154.0, 32.0);
    let button_gap = 8.0;
    let base_x = rect.left() + 14.0;
    let base_y = rect.bottom() - 46.0;

    let mut button_clicked = false;

    let motion_rect = Rect::from_min_size(
        egui::pos2(base_x, base_y - (button_size.y + button_gap)),
        button_size,
    );
    let toggle_rect = Rect::from_min_size(egui::pos2(base_x, base_y), button_size);

    let motion_text = match state.preview_motion_mode {
        PreviewMotionMode::Idle => "Motion: Idle",
        PreviewMotionMode::Walk => "Motion: Walk",
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(motion_rect), |ui| {
        let mut toggle_style = style::neutral_button(ui);
        toggle_style.min_size = motion_rect.size();
        let response = text_ui.button(ui, "skins_toggle_motion_mode", motion_text, &toggle_style);
        let should_force_focus = ui.ctx().data_mut(|data| {
            data.get_temp::<bool>(egui::Id::new(FORCE_MOTION_FOCUS_ID))
                .unwrap_or(false)
        });
        if should_force_focus {
            response.request_focus();
            ui.ctx()
                .data_mut(|data| data.remove::<bool>(egui::Id::new(FORCE_MOTION_FOCUS_ID)));
        }
        if response.clicked() {
            state.preview_motion_mode = match state.preview_motion_mode {
                PreviewMotionMode::Idle => PreviewMotionMode::Walk,
                PreviewMotionMode::Walk => PreviewMotionMode::Idle,
            };
        }
    });

    let toggle_text = if state.show_elytra {
        "Elytra: On"
    } else {
        "Elytra: Off"
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(toggle_rect), |ui| {
        let mut toggle_style = style::neutral_button(ui);
        toggle_style.min_size = toggle_rect.size();
        let response = text_ui.button(
            ui,
            "skins_toggle_elytra_overlay",
            toggle_text,
            &toggle_style,
        );
        button_clicked = response.clicked();
    });
    button_clicked
}

fn render_skin_drop_zone(ui: &mut Ui, text_ui: &mut TextUi, state: &mut SkinManagerState) {
    let width = ui
        .available_width()
        .min(ui.clip_rect().width().max(1.0))
        .max(1.0);
    let height = style::CONTROL_HEIGHT_LG * 3.4;
    let drop_zone_id = ui.make_persistent_id("skins_drop_zone");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let response = ui.interact(rect, drop_zone_id, Sense::click());
    let hovered_files = ui.input(|input| input.raw.hovered_files.clone());
    let dropped_files = ui.input(|input| input.raw.dropped_files.clone());
    let hovering_drop = !hovered_files.is_empty()
        && ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|pointer| rect.contains(pointer));
    let received_drop = !dropped_files.is_empty()
        && ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|pointer| rect.contains(pointer));
    let focused = response.has_focus();
    let pressed = response.is_pointer_button_down_on();
    let fill = if hovering_drop {
        ui.visuals().selection.bg_fill.gamma_multiply(0.22)
    } else if pressed {
        ui.visuals().widgets.active.bg_fill.gamma_multiply(0.95)
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.92)
    } else if focused {
        ui.visuals().selection.bg_fill.gamma_multiply(0.12)
    } else {
        ui.visuals()
            .widgets
            .inactive
            .weak_bg_fill
            .gamma_multiply(0.7)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(14), fill);
    paint_dotted_drop_zone_stroke(
        ui,
        rect.shrink(1.5),
        if hovering_drop || focused {
            ui.visuals().selection.stroke.color
        } else if response.hovered() {
            ui.visuals().widgets.hovered.bg_stroke.color
        } else {
            ui.visuals().weak_text_color()
        },
    );
    if focused {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(16),
            Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }

    let mut choose_style =
        style::neutral_button_with_min_size(ui, egui::vec2(220.0, style::CONTROL_HEIGHT));
    choose_style.fill = ui.visuals().widgets.inactive.bg_fill;
    choose_style.fill_hovered = ui.visuals().widgets.hovered.bg_fill;
    choose_style.fill_active = ui.visuals().widgets.active.bg_fill;
    choose_style.fill_selected = ui.visuals().selection.bg_fill.gamma_multiply(0.7);
    let content_rect = rect.shrink2(egui::vec2(18.0, 18.0));
    let title_style = style::section_heading(ui);
    let muted = style::muted(ui);
    let button_label_style = LabelOptions {
        font_size: choose_style.font_size,
        line_height: choose_style.line_height,
        color: choose_style.text_color,
        wrap: false,
        ..style::body(ui)
    };
    let title_size = text_ui.measure_text_size(ui, "Drag Skin Image here", &title_style);
    let or_size = text_ui.measure_text_size(ui, "or", &muted);
    let button_text_size = text_ui.measure_text_size(ui, "Choose Skin Image", &button_label_style);
    let button_size = egui::vec2(
        (button_text_size.x + choose_style.padding.x * 2.0).max(choose_style.min_size.x),
        (button_text_size.y + choose_style.padding.y * 2.0).max(choose_style.min_size.y),
    );
    let gap = style::SPACE_XS;
    let total_height = title_size.y + gap + or_size.y + gap + button_size.y;
    let mut current_y = content_rect.center().y - total_height * 0.5;

    let title_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.center().x - title_size.x * 0.5, current_y),
        title_size,
    );
    current_y += title_size.y + gap;
    let or_width = (or_size.x + 8.0).min(content_rect.width());
    let or_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.center().x - or_width * 0.5, current_y),
        egui::vec2(or_width, or_size.y),
    );
    current_y += or_size.y + gap;
    let button_rect = egui::Rect::from_min_size(
        egui::pos2(content_rect.center().x - button_size.x * 0.5, current_y),
        button_size,
    );
    let button_text_rect = egui::Rect::from_min_size(
        egui::pos2(
            button_rect.center().x - button_text_size.x * 0.5,
            button_rect.center().y - button_text_size.y * 0.5,
        ),
        button_text_size,
    );

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let _ = text_ui.label(
                ui,
                "skins_drop_prompt",
                "Drag Skin Image here",
                &title_style,
            );
        },
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(or_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let _ = text_ui.label(ui, "skins_drop_prompt_or", "or", &muted);
        },
    );
    let button_fill = if pressed {
        choose_style.fill_active
    } else if response.hovered() {
        choose_style.fill_hovered
    } else if focused {
        choose_style.fill_selected
    } else {
        choose_style.fill
    };
    let button_stroke = if focused {
        ui.visuals().selection.stroke
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_stroke
    } else {
        choose_style.stroke
    };
    ui.painter().rect_filled(
        button_rect,
        CornerRadius::same(choose_style.corner_radius),
        button_fill,
    );
    ui.painter().rect_stroke(
        button_rect,
        CornerRadius::same(choose_style.corner_radius),
        button_stroke,
        egui::StrokeKind::Inside,
    );
    if focused {
        ui.painter().rect_stroke(
            button_rect.expand(2.0),
            CornerRadius::same(choose_style.corner_radius.saturating_add(2)),
            Stroke::new(
                (ui.visuals().selection.stroke.width + 1.0).max(2.0),
                ui.visuals().selection.stroke.color,
            ),
            egui::StrokeKind::Outside,
        );
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(button_text_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let _ = text_ui.label(
                ui,
                "skins_pick_file_visual",
                "Choose Skin Image",
                &button_label_style,
            );
        },
    );
    if response.clicked() && !state.pick_skin_in_progress && !received_drop {
        state.pick_skin_file();
    }

    if received_drop && !state.pick_skin_in_progress {
        if let Some(file) = dropped_files.into_iter().next() {
            if let Some(path) = file.path {
                state.begin_loading_skin_from_path(path);
            } else if let Some(bytes) = file.bytes {
                let name = if file.name.trim().is_empty() {
                    "Dropped skin.png".to_owned()
                } else {
                    file.name
                };
                state.begin_loading_skin_from_bytes(PathBuf::from(name), bytes.as_ref().to_vec());
            } else {
                notification::error!(
                    "skin_manager",
                    "Dropped file did not include readable image data."
                );
            }
        }
    }
}

fn paint_dotted_drop_zone_stroke(ui: &Ui, rect: Rect, color: Color32) {
    let dash_step = 10.0;
    let dash_len = 4.0;
    let stroke = Stroke::new(1.5, color);

    let mut x = rect.left();
    while x <= rect.right() {
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.top()),
                egui::pos2((x + dash_len).min(rect.right()), rect.top()),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(x, rect.bottom()),
                egui::pos2((x + dash_len).min(rect.right()), rect.bottom()),
            ],
            stroke,
        );
        x += dash_step;
    }

    let mut y = rect.top();
    while y <= rect.bottom() {
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), y),
                egui::pos2(rect.left(), (y + dash_len).min(rect.bottom())),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(rect.right(), y),
                egui::pos2(rect.right(), (y + dash_len).min(rect.bottom())),
            ],
            stroke,
        );
        y += dash_step;
    }
}

fn paint_preview_background(ui: &Ui, painter: &egui::Painter, rect: Rect) {
    let fill = ui.visuals().faint_bg_color;

    painter.rect_filled(rect, CornerRadius::same(8), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Outside,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewMotionMode {
    Idle,
    Walk,
}

#[derive(Clone, Copy)]
struct PreviewPose {
    time_seconds: f32,
    idle_cycle: f32,
    walk_cycle: f32,
    locomotion_blend: f32,
}

fn fetch_and_cache_profile(
    profile_id: String,
    access_token: &str,
    display_name: &str,
) -> Result<LoadedProfile, String> {
    tracing::info!(
        target: "vertexlauncher/skins",
        display_name,
        "Fetching latest skin profile."
    );
    let profile = auth::fetch_minecraft_profile(access_token)
        .map_err(|err| format!("Failed to fetch latest profile: {err}"))?;
    tracing::info!(
        target: "vertexlauncher/skins",
        display_name,
        skins = profile.skins.len(),
        capes = profile.capes.len(),
        "Fetched latest skin profile."
    );
    update_cached_profile(profile_id.as_str(), &profile, display_name)?;
    Ok(LoadedProfile::from_profile(profile))
}

fn update_cached_profile(
    profile_id: &str,
    profile: &MinecraftProfileState,
    display_name: &str,
) -> Result<(), String> {
    tracing::info!(
        target: "vertexlauncher/skins",
        display_name,
        "Updating cached account profile snapshot."
    );
    let mut cache =
        auth::load_cached_accounts().map_err(|err| format!("Cache read failed: {err}"))?;
    let mut changed = false;

    for account in &mut cache.accounts {
        if account
            .minecraft_profile
            .id
            .eq_ignore_ascii_case(profile_id)
        {
            account.minecraft_profile = profile.clone();
            account.cached_at_unix_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            changed = true;
            break;
        }
    }

    if changed {
        auth::save_cached_accounts(&cache).map_err(|err| format!("Cache write failed: {err}"))?;
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name,
            "Cached account profile snapshot updated."
        );
    } else {
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name,
            "No matching cached account found to update."
        );
    }

    Ok(())
}

fn _absolute_path_string(path: &PathBuf) -> String {
    path.display().to_string()
}
