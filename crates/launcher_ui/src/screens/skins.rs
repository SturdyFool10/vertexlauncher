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

fn draw_character(
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpressionOffset {
    Bottom,
    LowerMid,
    UpperMid,
    Top,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EyeFamily {
    TwoByOne,
    TwoByTwo,
    TwoByThree,
    ThreeByOne,
    OneByOne,
    OneByTwo,
    OneByThree,
    Spread,
    FarOneByOne,
    OneByOneInner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowKind {
    Standard,
    Hat,
    Spread,
    Villager,
}

#[derive(Clone, Copy, Debug)]
struct TextureRectU32 {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

const fn tex(x: u32, y: u32, w: u32, h: u32) -> TextureRectU32 {
    TextureRectU32 { x, y, w, h }
}

#[derive(Clone, Copy, Debug)]
struct EyeExpressionSpec {
    id: &'static str,
    family: EyeFamily,
    offset: ExpressionOffset,
    right_eye: TextureRectU32,
    left_eye: TextureRectU32,
    right_white: Option<TextureRectU32>,
    left_white: Option<TextureRectU32>,
    right_pupil: Option<TextureRectU32>,
    left_pupil: Option<TextureRectU32>,
    blink: Option<TextureRectU32>,
    right_center_x: f32,
    left_center_x: f32,
    center_y: f32,
    #[allow(dead_code)]
    z: f32,
    width: f32,
    height: f32,
    pupil_width: f32,
    pupil_height: f32,
    gaze_scale_x: f32,
    gaze_scale_y: f32,
}

#[derive(Clone, Copy, Debug)]
struct BrowExpressionSpec {
    id: &'static str,
    kind: BrowKind,
    offset: ExpressionOffset,
    right_brow: TextureRectU32,
    left_brow: Option<TextureRectU32>,
    center_x: f32,
    center_y: f32,
    #[allow(dead_code)]
    z: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
struct DetectedExpressionsLayout {
    eye: EyeExpressionSpec,
    brow: Option<BrowExpressionSpec>,
}

#[derive(Clone, Copy)]
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

const SUPPORTED_EYE_SPECS: &[EyeExpressionSpec] = &[
    EyeExpressionSpec {
        id: "eye_16",
        family: EyeFamily::Spread,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(36, 6, 2, 1),
        left_eye: tex(38, 6, 2, 1),
        right_white: Some(tex(36, 7, 1, 1)),
        left_white: Some(tex(39, 7, 1, 1)),
        right_pupil: Some(tex(37, 7, 1, 1)),
        left_pupil: Some(tex(38, 7, 1, 1)),
        blink: Some(tex(36, 6, 2, 1)),
        right_center_x: 2.975,
        left_center_x: -2.975,
        center_y: 27.000,
        z: 3.000,
        width: 2.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_10",
        family: EyeFamily::ThreeByOne,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(32, 2, 3, 1),
        left_eye: tex(37, 2, 3, 1),
        right_white: Some(tex(32, 3, 3, 1)),
        left_white: Some(tex(37, 3, 3, 1)),
        right_pupil: Some(tex(35, 3, 1, 1)),
        left_pupil: Some(tex(36, 3, 1, 1)),
        blink: Some(tex(32, 2, 3, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 4.000,
        width: 3.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.450,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_11",
        family: EyeFamily::ThreeByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(32, 0, 3, 1),
        left_eye: tex(37, 0, 3, 1),
        right_white: Some(tex(32, 1, 3, 1)),
        left_white: Some(tex(37, 1, 3, 1)),
        right_pupil: Some(tex(35, 1, 1, 1)),
        left_pupil: Some(tex(36, 1, 1, 1)),
        blink: Some(tex(32, 0, 3, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 4.000,
        width: 3.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.450,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_9",
        family: EyeFamily::TwoByThree,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(32, 4, 2, 3),
        left_eye: tex(34, 4, 2, 3),
        right_white: Some(tex(32, 5, 1, 3)),
        left_white: Some(tex(35, 5, 1, 3)),
        right_pupil: Some(tex(33, 5, 1, 2)),
        left_pupil: Some(tex(34, 5, 1, 2)),
        blink: Some(tex(32, 4, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.500,
        z: 3.000,
        width: 2.000,
        height: 3.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_5",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::Bottom,
        right_eye: tex(24, 5, 2, 2),
        left_eye: tex(26, 5, 2, 2),
        right_white: Some(tex(24, 6, 1, 2)),
        left_white: Some(tex(27, 6, 1, 2)),
        right_pupil: Some(tex(25, 6, 1, 2)),
        left_pupil: Some(tex(26, 6, 1, 2)),
        blink: Some(tex(24, 5, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_6",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(24, 2, 2, 2),
        left_eye: tex(26, 2, 2, 2),
        right_white: Some(tex(24, 3, 1, 2)),
        left_white: Some(tex(27, 3, 1, 2)),
        right_pupil: Some(tex(25, 3, 1, 2)),
        left_pupil: Some(tex(26, 3, 1, 2)),
        blink: Some(tex(24, 2, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_7",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(28, 5, 2, 2),
        left_eye: tex(30, 5, 2, 2),
        right_white: Some(tex(28, 6, 1, 2)),
        left_white: Some(tex(31, 6, 1, 2)),
        right_pupil: Some(tex(29, 6, 1, 2)),
        left_pupil: Some(tex(30, 6, 1, 2)),
        blink: Some(tex(28, 5, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_8",
        family: EyeFamily::TwoByTwo,
        offset: ExpressionOffset::Top,
        right_eye: tex(28, 2, 2, 2),
        left_eye: tex(30, 2, 2, 2),
        right_white: Some(tex(28, 3, 1, 2)),
        left_white: Some(tex(31, 3, 1, 2)),
        right_pupil: Some(tex(29, 3, 1, 2)),
        left_pupil: Some(tex(30, 3, 1, 2)),
        blink: Some(tex(28, 2, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 29.000,
        z: 3.000,
        width: 2.000,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 2.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_1",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::Bottom,
        right_eye: tex(4, 6, 2, 1),
        left_eye: tex(6, 6, 2, 1),
        right_white: Some(tex(4, 7, 1, 1)),
        left_white: Some(tex(7, 7, 1, 1)),
        right_pupil: Some(tex(5, 7, 1, 1)),
        left_pupil: Some(tex(6, 7, 1, 1)),
        blink: Some(tex(4, 6, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_2",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(4, 4, 2, 1),
        left_eye: tex(6, 4, 2, 1),
        right_white: Some(tex(4, 5, 1, 1)),
        left_white: Some(tex(7, 5, 1, 1)),
        right_pupil: Some(tex(5, 5, 1, 1)),
        left_pupil: Some(tex(6, 5, 1, 1)),
        blink: Some(tex(4, 4, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_3",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(4, 2, 2, 1),
        left_eye: tex(6, 2, 2, 1),
        right_white: Some(tex(4, 3, 1, 1)),
        left_white: Some(tex(7, 3, 1, 1)),
        right_pupil: Some(tex(5, 3, 1, 1)),
        left_pupil: Some(tex(6, 3, 1, 1)),
        blink: Some(tex(4, 2, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_4",
        family: EyeFamily::TwoByOne,
        offset: ExpressionOffset::Top,
        right_eye: tex(4, 0, 2, 1),
        left_eye: tex(6, 0, 2, 1),
        right_white: Some(tex(4, 1, 1, 1)),
        left_white: Some(tex(7, 1, 1, 1)),
        right_pupil: Some(tex(5, 1, 1, 1)),
        left_pupil: Some(tex(6, 1, 1, 1)),
        blink: Some(tex(4, 0, 2, 1)),
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 29.000,
        z: 3.000,
        width: 2.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.380,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_12",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::Bottom,
        right_eye: tex(2, 7, 1, 1),
        left_eye: tex(3, 7, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 7, 1, 1)),
        left_pupil: Some(tex(3, 7, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 26.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_13",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::LowerMid,
        right_eye: tex(2, 5, 1, 1),
        left_eye: tex(3, 5, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 5, 1, 1)),
        left_pupil: Some(tex(3, 5, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 27.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_14",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(2, 3, 1, 1),
        left_eye: tex(3, 3, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 3, 1, 1)),
        left_pupil: Some(tex(3, 3, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 28.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_15",
        family: EyeFamily::OneByOne,
        offset: ExpressionOffset::Top,
        right_eye: tex(2, 1, 1, 1),
        left_eye: tex(3, 1, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: Some(tex(2, 1, 1, 1)),
        left_pupil: Some(tex(3, 1, 1, 1)),
        blink: None,
        right_center_x: 2.000,
        left_center_x: -2.000,
        center_y: 29.000,
        z: 3.000,
        width: 1.000,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_17",
        family: EyeFamily::FarOneByOne,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(0, 7, 1, 1),
        left_eye: tex(1, 7, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 7, 1, 1)),
        right_center_x: 2.475,
        left_center_x: -2.475,
        center_y: 28.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_18",
        family: EyeFamily::FarOneByOne,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 6, 1, 1),
        left_eye: tex(1, 6, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 6, 1, 1)),
        right_center_x: 2.475,
        left_center_x: -2.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_19",
        family: EyeFamily::OneByOneInner,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(0, 5, 1, 1),
        left_eye: tex(1, 5, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 5, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 28.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_20",
        family: EyeFamily::OneByOneInner,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 4, 1, 1),
        left_eye: tex(1, 4, 1, 1),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 4, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 1.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.120,
    },
    EyeExpressionSpec {
        id: "eye_21",
        family: EyeFamily::OneByTwo,
        offset: ExpressionOffset::UpperMid,
        right_eye: tex(0, 3, 1, 2),
        left_eye: tex(1, 3, 1, 2),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 3, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 28.000,
        z: 3.000,
        width: 1.025,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_22",
        family: EyeFamily::OneByTwo,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 2, 1, 2),
        left_eye: tex(1, 2, 1, 2),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 2, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 2.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_23",
        family: EyeFamily::OneByThree,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 1, 1, 3),
        left_eye: tex(1, 1, 1, 3),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 1, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 29.000,
        z: 3.000,
        width: 1.025,
        height: 3.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
    EyeExpressionSpec {
        id: "eye_24",
        family: EyeFamily::OneByThree,
        offset: ExpressionOffset::Top,
        right_eye: tex(0, 0, 1, 3),
        left_eye: tex(1, 0, 1, 3),
        right_white: None,
        left_white: None,
        right_pupil: None,
        left_pupil: None,
        blink: Some(tex(0, 0, 1, 1)),
        right_center_x: 1.475,
        left_center_x: -1.475,
        center_y: 30.000,
        z: 3.000,
        width: 1.025,
        height: 3.000,
        pupil_width: 1.000,
        pupil_height: 1.000,
        gaze_scale_x: 0.250,
        gaze_scale_y: 0.180,
    },
];

const SUPPORTED_BROW_SPECS: &[BrowExpressionSpec] = &[
    BrowExpressionSpec {
        id: "brow_bottom",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::Bottom,
        right_brow: tex(60, 7, 2, 1),
        left_brow: Some(tex(62, 7, 2, 1)),
        center_x: 2.000,
        center_y: 26.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_lower_mid",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::LowerMid,
        right_brow: tex(60, 5, 2, 1),
        left_brow: Some(tex(62, 5, 2, 1)),
        center_x: 2.000,
        center_y: 27.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_upper_mid",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::UpperMid,
        right_brow: tex(60, 3, 2, 1),
        left_brow: Some(tex(62, 3, 2, 1)),
        center_x: 2.000,
        center_y: 28.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_top",
        kind: BrowKind::Standard,
        offset: ExpressionOffset::Top,
        right_brow: tex(60, 1, 2, 1),
        left_brow: Some(tex(62, 1, 2, 1)),
        center_x: 2.000,
        center_y: 29.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_bottom",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::Bottom,
        right_brow: tex(56, 7, 2, 1),
        left_brow: Some(tex(58, 7, 2, 1)),
        center_x: 2.000,
        center_y: 26.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_lower_mid",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::LowerMid,
        right_brow: tex(56, 6, 2, 1),
        left_brow: Some(tex(58, 6, 2, 1)),
        center_x: 2.000,
        center_y: 27.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_upper_mid",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::UpperMid,
        right_brow: tex(56, 5, 2, 1),
        left_brow: Some(tex(58, 5, 2, 1)),
        center_x: 2.000,
        center_y: 28.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_hat_top",
        kind: BrowKind::Hat,
        offset: ExpressionOffset::Top,
        right_brow: tex(56, 4, 2, 1),
        left_brow: Some(tex(58, 4, 2, 1)),
        center_x: 2.000,
        center_y: 29.500,
        z: 4.200,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_spread",
        kind: BrowKind::Spread,
        offset: ExpressionOffset::LowerMid,
        right_brow: tex(36, 5, 2, 1),
        left_brow: Some(tex(38, 5, 2, 1)),
        center_x: 3.000,
        center_y: 27.500,
        z: 4.000,
        width: 2.000,
        height: 1.0,
    },
    BrowExpressionSpec {
        id: "brow_villager",
        kind: BrowKind::Villager,
        offset: ExpressionOffset::UpperMid,
        right_brow: tex(24, 1, 5, 1),
        left_brow: None,
        center_x: 0.000,
        center_y: 28.500,
        z: 4.000,
        width: 6.000,
        height: 1.0,
    },
];

struct BuiltCharacterScene {
    triangles: Vec<RenderTriangle>,
    cape_render_sample: Option<Arc<RgbaImage>>,
}

struct WeightedPreviewScene {
    weight: f32,
    triangles: Vec<RenderTriangle>,
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

fn eye_lid_rects(spec: EyeExpressionSpec) -> (TextureRectU32, TextureRectU32) {
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

fn compatibility_score(eye: EyeExpressionSpec, brow: BrowExpressionSpec) -> i32 {
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
struct FacePixelRect {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl FacePixelRect {
    fn center_x(self) -> f32 {
        4.0 - (self.left + self.width * 0.5)
    }

    fn top_y(self) -> f32 {
        self.top
    }

    fn bottom_y(self) -> f32 {
        self.top + self.height
    }
}

fn face_left_from_center(center_x: f32, width: f32) -> f32 {
    4.0 - center_x - width * 0.5
}

fn face_top_from_center(center_y: f32, height: f32) -> f32 {
    32.0 - center_y - height * 0.5
}

fn eye_face_rects(spec: EyeExpressionSpec) -> (FacePixelRect, FacePixelRect) {
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

fn render_motion_blur_wgpu_scene(
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

fn add_cape_triangles(
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
struct ElytraWingUvs {
    left: FaceUvs,
    right: FaceUvs,
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

fn add_elytra_triangles(
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

fn render_depth_buffered_scene(
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
struct PreviewHistory {
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

fn hash_rgba_image(image: &RgbaImage) -> u64 {
    let mut hasher = DefaultHasher::new();
    image.width().hash(&mut hasher);
    image.height().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

impl Vec3 {
    fn new(x: f32, y: f32, z: f32) -> Self {
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
struct Camera {
    position: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
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
struct Projection {
    fov_y_radians: f32,
    near: f32,
}

#[derive(Clone, Copy)]
struct FaceUvs {
    top: Rect,
    bottom: Rect,
    left: Rect,
    right: Rect,
    front: Rect,
    back: Rect,
}

#[derive(Clone, Copy)]
struct CuboidSpec {
    size: Vec3,
    pivot_top_center: Vec3,
    rotate_x: f32,
    rotate_z: f32,
    uv: FaceUvs,
    cull_backfaces: bool,
}

#[derive(Clone, Copy)]
enum TriangleTexture {
    Skin,
    Cape,
}

struct RenderTriangle {
    texture: TriangleTexture,
    pos: [Pos2; 3],
    uv: [Pos2; 3],
    depth: [f32; 3],
    color: Color32,
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

fn add_cuboid_triangles(
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

fn add_cuboid_triangles_with_y(
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

fn uv_rect_with_inset(
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

fn flip_uv_rect_x(rect: Rect) -> Rect {
    Rect::from_min_max(
        egui::pos2(rect.max.x, rect.min.y),
        egui::pos2(rect.min.x, rect.max.y),
    )
}

fn render_cape_grid(ui: &mut Ui, text_ui: &mut TextUi, state: &mut SkinManagerState) {
    let label_font = egui::TextStyle::Body.resolve(ui.style());
    let label_color = ui.visuals().text_color();
    let mut max_label_width = ui
        .painter()
        .layout_no_wrap("No Cape".to_owned(), label_font.clone(), label_color)
        .size()
        .x;
    for cape in &state.available_capes {
        let width = ui
            .painter()
            .layout_no_wrap(cape.label.clone(), label_font.clone(), label_color)
            .size()
            .x;
        max_label_width = max_label_width.max(width);
    }

    let available_width = ui
        .available_width()
        .min(ui.clip_rect().width().max(1.0))
        .max(1.0);
    let tile_gap = egui::vec2(style::SPACE_MD, style::SPACE_MD);
    let tile_width = (max_label_width + 24.0)
        .max(CAPE_TILE_WIDTH_MIN)
        .min(available_width);
    let columns =
        (((available_width + tile_gap.x) / (tile_width + tile_gap.x)).floor() as usize).max(1);
    let total_items = state.available_capes.len() + 1;
    let mut pending_selection = None;
    for row_start in (0..total_items).step_by(columns) {
        let row_end = (row_start + columns).min(total_items);
        let row_count = row_end.saturating_sub(row_start);
        let fallback_row_width =
            (row_count as f32 * tile_width) + (row_count.saturating_sub(1) as f32 * tile_gap.x);
        let row_width_id = egui::Id::new(("skins_cape_row_width", row_start));
        let measured_row_width = ui
            .ctx()
            .data(|data| data.get_temp::<f32>(row_width_id))
            .unwrap_or(fallback_row_width)
            .min(available_width);
        let (row_rect, _) = ui.allocate_exact_size(
            egui::vec2(available_width, CAPE_TILE_HEIGHT),
            Sense::hover(),
        );
        let box_rect = Rect::from_center_size(
            row_rect.center(),
            egui::vec2(measured_row_width, CAPE_TILE_HEIGHT),
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(box_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.x = tile_gap.x;
                let content = ui.horizontal(|ui| {
                    for item_index in row_start..row_end {
                        if item_index == 0 {
                            let no_cape_selected = state.pending_cape_id.is_none();
                            if draw_cape_tile(
                                ui,
                                text_ui,
                                tile_width,
                                "No Cape",
                                no_cape_selected,
                                true,
                                None,
                                None,
                            ) {
                                pending_selection = Some(None);
                            }
                            continue;
                        }

                        let cape = &state.available_capes[item_index - 1];
                        let selected = state.pending_cape_id.as_deref() == Some(cape.id.as_str());
                        let preview = cape.texture_bytes.as_deref();
                        if draw_cape_tile(
                            ui,
                            text_ui,
                            tile_width,
                            cape.label.as_str(),
                            selected,
                            false,
                            preview,
                            cape.texture_size,
                        ) {
                            pending_selection = Some(Some(cape.id.clone()));
                        }
                    }
                });
                ui.ctx()
                    .data_mut(|data| data.insert_temp(row_width_id, content.response.rect.width()));
            },
        );

        if row_end < total_items {
            ui.add_space(tile_gap.y);
        }
    }

    if let Some(selection) = pending_selection {
        state.pending_cape_id = selection;
    }
}

fn draw_cape_tile(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    tile_width: f32,
    label: &str,
    selected: bool,
    is_no_cape: bool,
    preview_png: Option<&[u8]>,
    preview_texture_size: Option<[u32; 2]>,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(tile_width, CAPE_TILE_HEIGHT), Sense::click());
    let tile_rect = rect.shrink2(egui::vec2(0.0, style::SPACE_XS * 0.5));

    let hover_t = ui
        .ctx()
        .animate_bool(response.id.with("cape_tile_hover"), response.hovered());
    let press_t = ui.ctx().animate_bool(
        response.id.with("cape_tile_press"),
        response.is_pointer_button_down_on(),
    );
    let selected_t = ui
        .ctx()
        .animate_bool(response.id.with("cape_tile_selected"), selected);
    let focused = response.has_focus();

    let fill = if selected || focused {
        ui.visuals().selection.bg_fill.gamma_multiply(
            0.24 + hover_t * 0.06 + press_t * 0.04 + if focused { 0.08 } else { 0.0 },
        )
    } else {
        ui.visuals()
            .widgets
            .inactive
            .bg_fill
            .gamma_multiply(1.0 + hover_t * 0.08 + press_t * 0.04)
    };
    let stroke = if selected || focused {
        let mut stroke = ui.visuals().selection.stroke;
        stroke.width += hover_t * 0.5 + if focused { 0.75 } else { 0.0 };
        stroke
    } else {
        let mut stroke = ui.visuals().widgets.inactive.bg_stroke;
        stroke.color = stroke.color.gamma_multiply(1.0 + hover_t * 0.18);
        stroke
    };

    ui.painter().rect(
        tile_rect,
        CornerRadius::same(10),
        fill,
        stroke,
        egui::StrokeKind::Middle,
    );
    paint_cape_tile_highlight(
        ui,
        tile_rect,
        response.hover_pos().or(response.interact_pointer_pos()),
        hover_t,
        press_t,
        selected_t.max(if focused { 1.0 } else { 0.0 }),
    );

    let preview_rect = Rect::from_min_size(
        egui::pos2(tile_rect.left() + 12.0, tile_rect.top() + 12.0),
        egui::vec2((tile_rect.width() - 24.0).max(0.0), 112.0),
    );

    if is_no_cape {
        ui.painter().rect_stroke(
            preview_rect,
            CornerRadius::same(6),
            Stroke::new(1.5, ui.visuals().weak_text_color()),
            egui::StrokeKind::Middle,
        );
        let dotted_step = 8.0;
        let mut x = preview_rect.left();
        while x <= preview_rect.right() {
            ui.painter().line_segment(
                [
                    egui::pos2(x, preview_rect.top()),
                    egui::pos2((x + 3.0).min(preview_rect.right()), preview_rect.top()),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(x, preview_rect.bottom()),
                    egui::pos2((x + 3.0).min(preview_rect.right()), preview_rect.bottom()),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            x += dotted_step;
        }
        let mut y = preview_rect.top();
        while y <= preview_rect.bottom() {
            ui.painter().line_segment(
                [
                    egui::pos2(preview_rect.left(), y),
                    egui::pos2(preview_rect.left(), (y + 3.0).min(preview_rect.bottom())),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(preview_rect.right(), y),
                    egui::pos2(preview_rect.right(), (y + 3.0).min(preview_rect.bottom())),
                ],
                Stroke::new(1.0, ui.visuals().weak_text_color()),
            );
            y += dotted_step;
        }
    } else if let Some(bytes) = preview_png {
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let uri = format!("bytes://skins/cape/{:016x}.png", hasher.finish());

        if let Some(back_uv) = preview_texture_size.and_then(cape_outer_face_uv) {
            let inner = preview_rect.shrink2(egui::vec2(4.0, 4.0));
            let target_aspect = 10.0 / 16.0;
            let max_h = inner.height();
            let mut face_h = max_h;
            let mut face_w = face_h * target_aspect;
            if face_w > inner.width() {
                face_w = inner.width().max(0.0);
                face_h = face_w / target_aspect;
            }
            let y = inner.center().y - face_h * 0.5;
            let x = inner.center().x - face_w * 0.5;
            let back_rect = Rect::from_min_size(egui::pos2(x, y), egui::vec2(face_w, face_h));

            ui.painter().rect_stroke(
                back_rect,
                CornerRadius::same(4),
                Stroke::new(1.0, ui.visuals().widgets.inactive.bg_stroke.color),
                egui::StrokeKind::Middle,
            );

            if let image_textures::ManagedTextureStatus::Ready(texture) =
                image_textures::request_texture(
                    ui.ctx(),
                    uri.clone(),
                    Arc::<[u8]>::from(bytes.to_vec().into_boxed_slice()),
                    TextureOptions::NEAREST,
                )
            {
                egui::Image::from_texture(&texture)
                    .uv(back_uv)
                    .fit_to_exact_size(back_rect.size())
                    .texture_options(TextureOptions::NEAREST)
                    .paint_at(ui, back_rect);
            }
        } else {
            if let image_textures::ManagedTextureStatus::Ready(texture) =
                image_textures::request_texture(
                    ui.ctx(),
                    uri,
                    Arc::<[u8]>::from(bytes.to_vec().into_boxed_slice()),
                    TextureOptions::NEAREST,
                )
            {
                let image = egui::Image::from_texture(&texture)
                    .fit_to_exact_size(preview_rect.size())
                    .texture_options(TextureOptions::NEAREST);
                image.paint_at(ui, preview_rect);
            }
        }
    } else {
        ui.painter().rect_filled(
            preview_rect,
            CornerRadius::same(6),
            ui.visuals().faint_bg_color,
        );
    }

    let label_rect = Rect::from_min_size(
        Pos2::new(tile_rect.left() + 6.0, tile_rect.bottom() - 44.0),
        egui::vec2(tile_rect.width() - 12.0, 34.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_rect), |ui| {
        ui.set_clip_rect(label_rect);
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                let mut label_style = style::body(ui);
                label_style.wrap = false;
                let _ = text_ui.label(ui, ("skins_cape_label", label), label, &label_style);
            },
        );
    });

    response.clicked()
}

fn paint_cape_tile_highlight(
    ui: &Ui,
    rect: Rect,
    pointer_pos: Option<Pos2>,
    hover_t: f32,
    press_t: f32,
    selected_t: f32,
) {
    let emphasis = (hover_t * 0.95 + press_t * 0.85 + selected_t * 0.55).clamp(0.0, 1.0);
    if emphasis <= 0.01 {
        return;
    }

    let selection = ui.visuals().selection.bg_fill;
    let glow_rect = rect.shrink2(egui::vec2(1.0, 1.0));
    let glow_center = pointer_pos.unwrap_or_else(|| {
        egui::pos2(
            rect.center().x,
            egui::lerp(
                rect.top() + 28.0..=rect.center().y,
                selected_t.max(hover_t * 0.35),
            ),
        )
    });
    let glow_center = egui::pos2(
        glow_center
            .x
            .clamp(glow_rect.left() + 4.0, glow_rect.right() - 4.0),
        glow_center
            .y
            .clamp(glow_rect.top() + 4.0, glow_rect.bottom() - 4.0),
    );
    let glow_radius = rect.width().max(rect.height()) * egui::lerp(0.34..=0.58, emphasis);
    let center_alpha = (32.0 + hover_t * 34.0 + press_t * 22.0 + selected_t * 10.0) / 255.0;
    let ring_specs = [
        (0.0, center_alpha),
        (0.32, center_alpha * 0.52),
        (0.68, center_alpha * 0.16),
        (1.0, 0.0),
    ];

    let mut mesh = egui::epaint::Mesh::default();
    let center_idx = mesh.vertices.len() as u32;
    let center_color: Color32 = egui::Rgba::from(selection).multiply(center_alpha).into();
    mesh.colored_vertex(glow_center, center_color);

    let segments = 40usize;
    let mut previous_ring = Vec::with_capacity(segments);
    for (ring_index, (radius_t, alpha)) in ring_specs.iter().enumerate().skip(1) {
        let color: Color32 = egui::Rgba::from(selection).multiply(*alpha).into();
        let mut current_ring = Vec::with_capacity(segments);
        for segment in 0..segments {
            let angle = std::f32::consts::TAU * (segment as f32 / segments as f32);
            let unit_x = angle.cos();
            let unit_y = angle.sin();
            let vertex = egui::pos2(
                glow_center.x + unit_x * glow_radius * *radius_t,
                glow_center.y + unit_y * glow_radius * *radius_t,
            );
            let vertex_idx = mesh.vertices.len() as u32;
            mesh.colored_vertex(vertex, color);
            current_ring.push(vertex_idx);
        }

        if ring_index == 1 {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                mesh.add_triangle(center_idx, current_ring[segment], current_ring[next]);
            }
        } else {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                mesh.add_triangle(
                    previous_ring[segment],
                    previous_ring[next],
                    current_ring[next],
                );
                mesh.add_triangle(
                    previous_ring[segment],
                    current_ring[next],
                    current_ring[segment],
                );
            }
        }

        previous_ring = current_ring;
    }

    ui.painter()
        .with_clip_rect(glow_rect)
        .add(egui::Shape::mesh(mesh));

    let sheen_rect = Rect::from_min_max(
        glow_rect.min + egui::vec2(0.0, 1.0),
        egui::pos2(glow_rect.max.x, glow_rect.top() + glow_rect.height() * 0.34),
    );
    let sheen_alpha = (14.0 * emphasis) / 255.0;
    ui.painter().rect_filled(
        sheen_rect,
        CornerRadius {
            nw: 10,
            ne: 10,
            sw: 18,
            se: 18,
        },
        egui::Rgba::from_white_alpha(sheen_alpha),
    );
}

#[derive(Clone, Debug, Default)]
struct CapeChoice {
    id: String,
    label: String,
    texture_bytes: Option<Vec<u8>>,
    texture_size: Option<[u32; 2]>,
}

#[derive(Clone)]
struct SkinManagerState {
    active_profile_id: Option<String>,
    active_player_name: Option<String>,
    access_token: Option<String>,
    base_skin_png: Option<Vec<u8>>,
    pending_skin_png: Option<Vec<u8>>,
    pending_skin_path: Option<PathBuf>,
    initial_variant: MinecraftSkinVariant,
    pending_variant: MinecraftSkinVariant,
    available_capes: Vec<CapeChoice>,
    initial_cape_id: Option<String>,
    pending_cape_id: Option<String>,
    show_elytra: bool,
    status_message: Option<String>,
    save_in_progress: bool,
    refresh_in_progress: bool,
    worker_rx: Option<Arc<Mutex<Receiver<WorkerEvent>>>>,
    pick_skin_in_progress: bool,
    pick_skin_results_rx: Option<Arc<Mutex<Receiver<Result<(PathBuf, Vec<u8>), String>>>>>,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    last_preview_aa_mode: SkinPreviewAaMode,
    preview_texel_aa_mode: SkinPreviewTexelAaMode,
    last_preview_texel_aa_mode: SkinPreviewTexelAaMode,
    preview_motion_blur_enabled: bool,
    last_preview_motion_blur_enabled: bool,
    preview_motion_blur_amount: f32,
    last_preview_motion_blur_amount: f32,
    preview_motion_blur_shutter_frames: f32,
    last_preview_motion_blur_shutter_frames: f32,
    preview_motion_blur_sample_count: usize,
    last_preview_motion_blur_sample_count: usize,
    preview_3d_layers_enabled: bool,
    last_preview_3d_layers_enabled: bool,
    expressions_enabled: bool,
    last_expressions_enabled: bool,
    cached_expression_layout_hash: Option<u64>,
    cached_expression_layout: Option<DetectedExpressionsLayout>,
    preview_motion_mode: PreviewMotionMode,
    preview_motion_blend: f32,
    skin_texture_hash: Option<u64>,
    skin_texture: Option<TextureHandle>,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_texture_hash: Option<u64>,
    cape_texture: Option<TextureHandle>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_texture: Option<TextureHandle>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
    preview_texture: Option<TextureHandle>,
    preview_history: Option<PreviewHistory>,
    cape_uv: FaceUvs,
    camera_yaw_offset: f32,
    camera_inertial_velocity: f32,
    camera_drag_velocity: f32,
    camera_drag_active: bool,
    orbit_pause_started_at: Option<f64>,
    orbit_pause_accumulated_secs: f64,
    camera_last_frame_time: Option<f64>,
    refresh_on_open_pending: bool,
}

impl Default for SkinManagerState {
    fn default() -> Self {
        Self {
            active_profile_id: None,
            active_player_name: None,
            access_token: None,
            base_skin_png: None,
            pending_skin_png: None,
            pending_skin_path: None,
            initial_variant: MinecraftSkinVariant::Classic,
            pending_variant: MinecraftSkinVariant::Classic,
            available_capes: Vec::new(),
            initial_cape_id: None,
            pending_cape_id: None,
            show_elytra: false,
            status_message: None,
            save_in_progress: false,
            refresh_in_progress: false,
            worker_rx: None,
            pick_skin_in_progress: false,
            pick_skin_results_rx: None,
            wgpu_target_format: None,
            preview_msaa_samples: 1,
            preview_aa_mode: SkinPreviewAaMode::Msaa,
            last_preview_aa_mode: SkinPreviewAaMode::Msaa,
            preview_texel_aa_mode: SkinPreviewTexelAaMode::Off,
            last_preview_texel_aa_mode: SkinPreviewTexelAaMode::Off,
            preview_motion_blur_enabled: false,
            last_preview_motion_blur_enabled: false,
            preview_motion_blur_amount: 0.15,
            last_preview_motion_blur_amount: 0.15,
            preview_motion_blur_shutter_frames: 0.75,
            last_preview_motion_blur_shutter_frames: 0.75,
            preview_motion_blur_sample_count: 5,
            last_preview_motion_blur_sample_count: 5,
            preview_3d_layers_enabled: false,
            last_preview_3d_layers_enabled: false,
            expressions_enabled: false,
            last_expressions_enabled: false,
            cached_expression_layout_hash: None,
            cached_expression_layout: None,
            preview_motion_mode: PreviewMotionMode::Idle,
            preview_motion_blend: 0.0,
            skin_texture_hash: None,
            skin_texture: None,
            skin_sample: None,
            cape_texture_hash: None,
            cape_texture: None,
            cape_sample: None,
            default_elytra_texture: None,
            default_elytra_sample: None,
            preview_texture: None,
            preview_history: None,
            cape_uv: default_cape_uv_layout(),
            camera_yaw_offset: 0.0,
            camera_inertial_velocity: 0.0,
            camera_drag_velocity: 0.0,
            camera_drag_active: false,
            orbit_pause_started_at: None,
            orbit_pause_accumulated_secs: 0.0,
            camera_last_frame_time: None,
            refresh_on_open_pending: false,
        }
    }
}

impl SkinManagerState {
    fn sync_active_account(&mut self, active_launch_auth: Option<&LaunchAuthContext>) {
        let Some(auth) = active_launch_auth else {
            if self.active_profile_id.is_some() {
                *self = Self::default();
            }
            return;
        };

        let normalized_profile_id = auth.player_uuid.to_ascii_lowercase();
        let profile_changed =
            self.active_profile_id.as_deref() != Some(normalized_profile_id.as_str());
        let token_changed = self.access_token.as_deref() != auth.access_token.as_deref();
        let name_changed = self.active_player_name.as_deref() != Some(auth.player_name.as_str());

        if !profile_changed && !token_changed && !name_changed {
            return;
        }

        if !profile_changed {
            // Same profile can still receive a fresh token after re-auth. Keep current edits intact.
            self.access_token = auth.access_token.clone();
            self.active_player_name = Some(auth.player_name.clone());
            tracing::info!(
                target: "vertexlauncher/skins",
                display_name = auth.player_name.as_str(),
                token_changed,
                name_changed,
                "Updated skin manager auth context for active profile."
            );
            return;
        }

        self.save_in_progress = false;
        self.refresh_in_progress = false;
        self.worker_rx = None;
        self.pick_skin_in_progress = false;
        self.pick_skin_results_rx = None;
        self.status_message = None;
        self.show_elytra = false;
        self.active_profile_id = Some(normalized_profile_id.clone());
        self.active_player_name = Some(auth.player_name.clone());
        self.access_token = auth.access_token.clone();
        self.base_skin_png = None;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.initial_variant = MinecraftSkinVariant::Classic;
        self.pending_variant = MinecraftSkinVariant::Classic;
        self.available_capes.clear();
        self.initial_cape_id = None;
        self.pending_cape_id = None;
        self.skin_texture_hash = None;
        self.skin_texture = None;
        self.skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_texture = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
        self.cached_expression_layout_hash = None;
        self.cached_expression_layout = None;
        self.cape_uv = default_cape_uv_layout();
        self.camera_yaw_offset = 0.0;
        self.camera_inertial_velocity = 0.0;
        self.camera_drag_velocity = 0.0;
        self.camera_drag_active = false;
        self.orbit_pause_started_at = None;
        self.orbit_pause_accumulated_secs = 0.0;
        self.camera_last_frame_time = None;

        self.load_snapshot_from_cache_for_profile(normalized_profile_id.as_str());
        self.start_refresh();
    }

    fn load_snapshot_from_cache_for_profile(&mut self, profile_id: &str) {
        match auth::load_cached_accounts() {
            Ok(accounts) => {
                let profile_id_lower = profile_id.to_ascii_lowercase();
                if let Some(account) = accounts.accounts.iter().find(|account| {
                    account.minecraft_profile.id.to_ascii_lowercase() == profile_id_lower
                }) {
                    self.apply_account_snapshot(account);
                }
            }
            Err(err) => {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Failed to load cached accounts while preparing skin-manager snapshot."
                );
                notification::error!("skin_manager", "Failed to load account cache: {err}");
            }
        }
    }

    fn apply_account_snapshot(&mut self, account: &CachedAccount) {
        self.active_profile_id = Some(account.minecraft_profile.id.to_ascii_lowercase());
        self.active_player_name = Some(account.minecraft_profile.name.clone());

        // Do not trust cached equipped skin/cape state; always load current equip from Mojang.
        self.base_skin_png = None;
        self.initial_variant = MinecraftSkinVariant::Classic;
        self.pending_variant = MinecraftSkinVariant::Classic;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.skin_texture_hash = None;
        self.skin_texture = None;
        self.skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_texture = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
        self.cached_expression_layout_hash = None;
        self.cached_expression_layout = None;
        self.cape_uv = default_cape_uv_layout();

        let mut choices = Vec::with_capacity(account.minecraft_profile.capes.len());
        for cape in &account.minecraft_profile.capes {
            let texture_bytes = cape.texture_png_bytes();
            let texture_size = texture_bytes.as_deref().and_then(decode_image_dimensions);
            choices.push(CapeChoice {
                id: cape.id.clone(),
                label: cape
                    .alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(cape.id.as_str())
                    .to_owned(),
                texture_bytes,
                texture_size,
            });
        }

        self.available_capes = choices;
        self.initial_cape_id = None;
        self.pending_cape_id = None;
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.worker_rx.take() {
            let mut keep_rx = true;
            loop {
                let recv_result = match rx.lock() {
                    Ok(guard) => guard.try_recv(),
                    Err(_) => {
                        self.save_in_progress = false;
                        self.refresh_in_progress = false;
                        notification::error!(
                            "skin_manager",
                            "Background profile task lock was poisoned."
                        );
                        keep_rx = false;
                        break;
                    }
                };
                match recv_result {
                    Ok(WorkerEvent::Refreshed(result)) => {
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            "Skin manager refresh worker completed."
                        );
                        self.refresh_in_progress = false;
                        match result {
                            Ok((profile_id, profile)) => {
                                if self.active_profile_id.as_deref() != Some(profile_id.as_str()) {
                                    tracing::info!(
                                        target: "vertexlauncher/skins",
                                        display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
                                        "Ignoring refresh result for non-active profile."
                                    );
                                    keep_rx = false;
                                    break;
                                }
                                // Keep in-progress edits intact when a late refresh arrives.
                                if self.pending_skin_png.is_some()
                                    || self.pending_variant != self.initial_variant
                                    || self.pending_cape_id != self.initial_cape_id
                                {
                                } else {
                                    self.apply_loaded_profile(profile);
                                }
                            }
                            Err(err) => {
                                tracing::info!(
                                    target: "vertexlauncher/skins",
                                    error = %err,
                                    "Skin manager refresh failed."
                                );
                                notification::error!("skin_manager", "{err}");
                            }
                        }
                        keep_rx = false;
                    }
                    Ok(WorkerEvent::Saved(result)) => {
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            "Skin manager save worker completed."
                        );
                        self.save_in_progress = false;
                        match result {
                            Ok((profile_id, profile)) => {
                                if self.active_profile_id.as_deref() != Some(profile_id.as_str()) {
                                    tracing::info!(
                                        target: "vertexlauncher/skins",
                                        display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
                                        "Ignoring save result for non-active profile."
                                    );
                                    keep_rx = false;
                                    break;
                                }
                                self.apply_loaded_profile(profile);
                                notification::info!("skin_manager", "Saved skin and cape changes.");
                            }
                            Err(err) => {
                                tracing::info!(
                                    target: "vertexlauncher/skins",
                                    error = %err,
                                    "Skin manager save failed."
                                );
                                notification::error!("skin_manager", "{err}");
                            }
                        }
                        keep_rx = false;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.save_in_progress = false;
                        self.refresh_in_progress = false;
                        tracing::error!(
                            target: "vertexlauncher/skins",
                            "Skin manager worker channel disconnected."
                        );
                        keep_rx = false;
                        break;
                    }
                }
            }
            if keep_rx {
                self.worker_rx = Some(rx);
            } else {
                ctx.request_repaint();
            }
        }
    }

    fn poll_pick_skin_result(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.pick_skin_results_rx.as_ref().cloned() else {
            return;
        };

        let Ok(receiver) = rx.lock() else {
            tracing::error!(
                target: "vertexlauncher/skins",
                "Pick-skin result receiver mutex was poisoned."
            );
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };

        self.pick_skin_in_progress = false;
        self.pick_skin_results_rx = None;
        match result {
            Ok((path, bytes)) => {
                self.pending_skin_png = Some(bytes);
                self.pending_skin_path = Some(path);
                self.skin_texture_hash = None;
                self.skin_sample = None;
                self.preview_texture = None;
                self.preview_history = None;
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Pick-skin operation failed."
                );
                notification::error!("skin_manager", "{err}");
            }
        }
        ctx.request_repaint();
    }

    fn ensure_skin_texture(&mut self, ctx: &egui::Context) {
        let active_png = self.preview_skin_png();
        let Some(bytes) = active_png else {
            self.skin_texture = None;
            self.skin_texture_hash = None;
            self.skin_sample = None;
            return;
        };

        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();

        if self.skin_texture_hash == Some(hash) {
            return;
        }

        let Some(image) = decode_skin_rgba(bytes) else {
            self.skin_sample = None;
            return;
        };
        let image = Arc::new(image);

        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ctx.load_texture(
            format!("skins/preview/{hash:016x}"),
            color_image,
            TextureOptions::NEAREST,
        );

        self.skin_texture = Some(texture);
        self.skin_sample = Some(image);
        self.skin_texture_hash = Some(hash);
    }
    fn ensure_default_elytra_texture(&mut self, ctx: &egui::Context) {
        if self.default_elytra_texture.is_some() && self.default_elytra_sample.is_some() {
            return;
        }

        let image = Arc::new(default_elytra_texture_image());
        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture =
            ctx.load_texture("skins/default-elytra", color_image, TextureOptions::NEAREST);
        self.default_elytra_texture = Some(texture);
        self.default_elytra_sample = Some(image);
    }

    fn ensure_cape_texture(&mut self, ctx: &egui::Context) {
        let active_png = self.selected_cape_png();
        let Some(bytes) = active_png else {
            self.cape_texture = None;
            self.cape_texture_hash = None;
            self.cape_sample = None;
            self.cape_uv = default_cape_uv_layout();
            return;
        };

        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let hash = hasher.finish();
        if self.cape_texture_hash == Some(hash) {
            return;
        }

        let Some(image) = decode_generic_rgba(bytes) else {
            self.cape_texture = None;
            self.cape_texture_hash = None;
            self.cape_sample = None;
            self.cape_uv = default_cape_uv_layout();
            return;
        };
        let image = Arc::new(image);

        self.cape_uv =
            cape_uv_layout([image.width(), image.height()]).unwrap_or_else(default_cape_uv_layout);

        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ctx.load_texture(
            format!("skins/cape-preview/{hash:016x}"),
            color_image,
            TextureOptions::NEAREST,
        );

        self.cape_texture = Some(texture);
        self.cape_sample = Some(image);
        self.cape_texture_hash = Some(hash);
    }

    fn preview_skin_png(&self) -> Option<&[u8]> {
        self.pending_skin_png
            .as_deref()
            .or(self.base_skin_png.as_deref())
    }

    fn selected_cape_png(&self) -> Option<&[u8]> {
        let selected = self.pending_cape_id.as_deref()?;
        self.available_capes
            .iter()
            .find(|cape| cape.id == selected)
            .and_then(|cape| cape.texture_bytes.as_deref())
    }

    fn pick_skin_file(&mut self) {
        if self.pick_skin_in_progress {
            return;
        }

        let picked = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_title("Select Minecraft Skin")
            .pick_file();

        let Some(path) = picked else {
            return;
        };

        self.begin_loading_skin_from_path(path);
    }

    fn begin_loading_skin_from_path(&mut self, path: PathBuf) {
        if self.pick_skin_in_progress {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.pick_skin_in_progress = true;
        self.pick_skin_results_rx = Some(Arc::new(Mutex::new(rx)));
        let _ = tokio_runtime::spawn_detached(async move {
            let result =
                tokio::fs::read(path.as_path())
                    .await
                    .map_err(|err| format!("Failed to read image: {err}"))
                    .and_then(|bytes| {
                        if decode_skin_rgba(&bytes).is_none() {
                            Err("Selected image must be a valid PNG skin (expected 64x64 or 64x32)."
                            .to_owned())
                        } else {
                            Ok((path, bytes))
                        }
                    });
            if let Err(err) = tx.send(result) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Failed to deliver picked skin-file result."
                );
            }
        });
    }

    fn begin_loading_skin_from_bytes(&mut self, path: PathBuf, bytes: Vec<u8>) {
        if self.pick_skin_in_progress {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.pick_skin_in_progress = true;
        self.pick_skin_results_rx = Some(Arc::new(Mutex::new(rx)));
        let _ = tokio_runtime::spawn_detached(async move {
            let result = if decode_skin_rgba(&bytes).is_none() {
                Err("Selected image must be a valid PNG skin (expected 64x64 or 64x32).".to_owned())
            } else {
                Ok((path, bytes))
            };
            if let Err(err) = tx.send(result) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    error = %err,
                    "Failed to deliver dropped skin-file result."
                );
            }
        });
    }

    fn can_save(&self) -> bool {
        self.access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty())
            && (self.pending_skin_png.is_some()
                || self.pending_variant != self.initial_variant
                || self.pending_cape_id != self.initial_cape_id)
    }

    fn start_refresh(&mut self) {
        if self.refresh_in_progress || self.save_in_progress {
            tracing::info!(
                target: "vertexlauncher/skins",
                refresh_in_progress = self.refresh_in_progress,
                save_in_progress = self.save_in_progress,
                "Skipping profile refresh because another skin task is active."
            );
            return;
        }
        let Some(profile_id) = self.active_profile_id.clone() else {
            return;
        };
        let Some(token) = self
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
        else {
            notification::error!(
                "skin_manager",
                "Missing Minecraft access token for active account."
            );
            return;
        };

        tracing::info!(
            target: "vertexlauncher/skins",
            display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
            "Starting skin manager profile refresh."
        );
        self.refresh_in_progress = true;
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(Arc::new(Mutex::new(rx)));
        let profile_id_for_result = profile_id.clone();
        let display_name_for_log = self
            .active_player_name
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        tokio_runtime::spawn_blocking_detached(move || {
            let result = fetch_and_cache_profile(profile_id, &token, display_name_for_log.as_str());
            if let Err(err) = tx.send(WorkerEvent::Refreshed(
                result.map(|loaded| (profile_id_for_result, loaded)),
            )) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    display_name = %display_name_for_log,
                    error = %err,
                    "Failed to deliver skin manager refresh result."
                );
            }
        });
    }

    fn try_consume_open_refresh(&mut self) {
        if !self.refresh_on_open_pending {
            return;
        }
        let has_active_profile = self.active_profile_id.is_some();
        let has_token = self
            .access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty());
        if !has_active_profile || !has_token || self.refresh_in_progress || self.save_in_progress {
            return;
        }

        self.refresh_on_open_pending = false;
        tracing::info!(
            target: "vertexlauncher/skins",
            "Running queued skin manager open-refresh."
        );
        self.start_refresh();
    }

    fn start_save(&mut self) {
        if self.save_in_progress || !self.can_save() {
            tracing::info!(
                target: "vertexlauncher/skins",
                save_in_progress = self.save_in_progress,
                can_save = self.can_save(),
                "Skipping skin save request."
            );
            return;
        }
        let Some(profile_id) = self.active_profile_id.clone() else {
            return;
        };
        let Some(token) = self
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
        else {
            notification::error!(
                "skin_manager",
                "Missing Minecraft access token for active account."
            );
            return;
        };

        // Save takes precedence over an in-flight refresh to avoid dropping save completion events.
        if self.refresh_in_progress {
            self.refresh_in_progress = false;
            self.worker_rx = None;
        }

        self.save_in_progress = true;
        let pending_skin = self.pending_skin_png.clone();
        let base_skin = self.base_skin_png.clone();
        let pending_variant = self.pending_variant;
        let initial_variant = self.initial_variant;
        let pending_cape = self.pending_cape_id.clone();
        let initial_cape = self.initial_cape_id.clone();
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
            has_skin_change = pending_skin.is_some() || pending_variant != initial_variant,
            skin_variant = pending_variant.as_api_str(),
            cape_changed = pending_cape != initial_cape,
            cape_selected = pending_cape.is_some(),
            "Starting skin manager save."
        );
        let (tx, rx) = mpsc::channel();
        self.worker_rx = Some(Arc::new(Mutex::new(rx)));
        let profile_id_for_result = profile_id.clone();
        let display_name_for_log = self
            .active_player_name
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        let _ = tokio_runtime::spawn_detached(async move {
            let result: Result<LoadedProfile, String> = (|| {
                let mut latest_profile: Option<MinecraftProfileState> = None;
                let skin_bytes_to_upload = pending_skin
                    .as_deref()
                    .or(base_skin.as_deref())
                    .filter(|_| pending_skin.is_some() || pending_variant != initial_variant);
                if let Some(bytes) = skin_bytes_to_upload {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        png_bytes = bytes.len(),
                        variant = pending_variant.as_api_str(),
                        reused_existing_skin = pending_skin.is_none(),
                        "Uploading skin to Mojang profile API."
                    );
                    latest_profile = Some(
                        auth::upload_minecraft_skin(&token, bytes, pending_variant)
                            .map_err(|err| format_auth_error("upload skin", &err))?,
                    );
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        "Skin upload completed."
                    );
                }

                if pending_cape != initial_cape {
                    if let Some(cape_id) = pending_cape.as_deref() {
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            cape_id_present = !cape_id.is_empty(),
                            "Setting active cape via Mojang profile API."
                        );
                        latest_profile = Some(
                            auth::set_active_minecraft_cape(&token, cape_id)
                                .map_err(|err| format_auth_error("set cape", &err))?,
                        );
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            "Cape activation completed."
                        );
                    } else {
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            "Clearing active cape via Mojang profile API."
                        );
                        latest_profile = Some(
                            auth::clear_active_minecraft_cape(&token)
                                .map_err(|err| format_auth_error("clear cape", &err))?,
                        );
                        tracing::info!(
                            target: "vertexlauncher/skins",
                            display_name = display_name_for_log.as_str(),
                            "Cape clear completed."
                        );
                    }
                }

                if let Some(profile) = latest_profile {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        skins = profile.skins.len(),
                        capes = profile.capes.len(),
                        "Using profile payload returned by Mojang mutation endpoint."
                    );
                    update_cached_profile(
                        profile_id.as_str(),
                        &profile,
                        display_name_for_log.as_str(),
                    )?;
                    Ok(LoadedProfile::from_profile(profile))
                } else {
                    fetch_and_cache_profile(profile_id, &token, display_name_for_log.as_str())
                }
            })();
            if let Err(err) = tx.send(WorkerEvent::Saved(
                result.map(|loaded| (profile_id_for_result, loaded)),
            )) {
                tracing::error!(
                    target: "vertexlauncher/skins",
                    display_name = %display_name_for_log,
                    error = %err,
                    "Failed to deliver skin manager save result."
                );
            }
        });
    }

    fn apply_loaded_profile(&mut self, profile: LoadedProfile) {
        self.active_player_name = Some(profile.player_name);
        self.base_skin_png = profile.active_skin_png;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.initial_variant = profile.skin_variant;
        self.pending_variant = profile.skin_variant;
        self.available_capes = profile.capes;
        self.initial_cape_id = profile.active_cape_id.clone();
        self.pending_cape_id = profile.active_cape_id;
        self.skin_texture_hash = None;
        self.skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
        self.cached_expression_layout_hash = None;
        self.cached_expression_layout = None;
        self.cape_uv = default_cape_uv_layout();
    }

    fn begin_manual_camera_control(&mut self, now: f64) {
        if self.orbit_pause_started_at.is_none() {
            self.orbit_pause_started_at = Some(now);
        }
    }

    fn finish_manual_camera_control(&mut self, now: f64) {
        if let Some(started_at) = self.orbit_pause_started_at.take() {
            self.orbit_pause_accumulated_secs += (now - started_at).max(0.0);
        }
    }

    fn effective_orbit_time(&self, now: f64) -> f64 {
        let paused_now = self
            .orbit_pause_started_at
            .map(|started_at| (now - started_at).max(0.0))
            .unwrap_or(0.0);
        (now - self.orbit_pause_accumulated_secs - paused_now).max(0.0)
    }

    fn consume_frame_dt(&mut self, now: f64) -> f32 {
        let dt = self
            .camera_last_frame_time
            .map(|previous| (now - previous).max(0.0) as f32)
            .unwrap_or(0.0);
        self.camera_last_frame_time = Some(now);
        dt
    }

    fn refresh_expression_layout_cache(&mut self) {
        if !self.expressions_enabled {
            self.cached_expression_layout_hash = None;
            self.cached_expression_layout = None;
            return;
        }
        let Some(sample) = self.skin_sample.as_ref() else {
            self.cached_expression_layout_hash = None;
            self.cached_expression_layout = None;
            return;
        };
        let hash = hash_rgba_image(sample);
        if self.cached_expression_layout_hash == Some(hash) {
            return;
        }
        self.cached_expression_layout_hash = Some(hash);
        self.cached_expression_layout = detect_expression_layout(sample);
        if let Some(layout) = self.cached_expression_layout {
            let (right_eye_rect, left_eye_rect) = eye_face_rects(layout.eye);
            let (right_lid_rect, left_lid_rect) = eye_lid_rects(layout.eye);
            let right_lid_base_h = right_lid_rect.h as f32;
            let left_lid_base_h = left_lid_rect.h as f32;
            let right_upper_travel = (right_eye_rect.height - right_lid_base_h).max(0.0);
            let left_upper_travel = (left_eye_rect.height - left_lid_base_h).max(0.0);
            let right_lower_top_min = right_eye_rect.bottom_y() - right_eye_rect.height;
            let right_lower_top_max = right_eye_rect.bottom_y() - right_lid_base_h;
            let left_lower_top_min = left_eye_rect.bottom_y() - left_eye_rect.height;
            let left_lower_top_max = left_eye_rect.bottom_y() - left_lid_base_h;
            tracing::info!(
                target: "vertexlauncher/skins_expressions",
                eye_id = layout.eye.id,
                eye_family = ?layout.eye.family,
                eye_offset = ?layout.eye.offset,
                eye_width = layout.eye.width,
                eye_height = layout.eye.height,
                eye_center_y = layout.eye.center_y,
                right_eye_center_x = layout.eye.right_center_x,
                left_eye_center_x = layout.eye.left_center_x,
                right_eye_left = right_eye_rect.left,
                right_eye_top = right_eye_rect.top_y(),
                right_eye_bottom = right_eye_rect.bottom_y(),
                left_eye_left = left_eye_rect.left,
                left_eye_top = left_eye_rect.top_y(),
                left_eye_bottom = left_eye_rect.bottom_y(),
                right_upper_lid_top_min = right_eye_rect.top_y(),
                right_upper_lid_top_max = right_eye_rect.top_y(),
                right_upper_lid_height_min = right_lid_base_h,
                right_upper_lid_height_max = right_eye_rect.height,
                right_upper_lid_travel = right_upper_travel,
                right_lid_uv_min_y = uv_rect_from_eyelid_texel_rect(right_lid_rect).min.y,
                right_lid_uv_max_y = uv_rect_from_eyelid_texel_rect(right_lid_rect).max.y,
                left_upper_lid_top_min = left_eye_rect.top_y(),
                left_upper_lid_top_max = left_eye_rect.top_y(),
                left_upper_lid_height_min = left_lid_base_h,
                left_upper_lid_height_max = left_eye_rect.height,
                left_upper_lid_travel = left_upper_travel,
                left_lid_uv_min_y = uv_rect_from_eyelid_texel_rect(left_lid_rect).min.y,
                left_lid_uv_max_y = uv_rect_from_eyelid_texel_rect(left_lid_rect).max.y,
                right_lower_lid_top_min = right_lower_top_min,
                right_lower_lid_top_max = right_lower_top_max,
                right_lower_lid_height_min = right_lid_base_h,
                right_lower_lid_height_max = right_eye_rect.height,
                left_lower_lid_top_min = left_lower_top_min,
                left_lower_lid_top_max = left_lower_top_max,
                left_lower_lid_height_min = left_lid_base_h,
                left_lower_lid_height_max = left_eye_rect.height,
                brow_id = layout.brow.map(|b| b.id).unwrap_or("none"),
                brow_kind = ?layout.brow.map(|b| b.kind),
                brow_offset = ?layout.brow.map(|b| b.offset),
                brow_bounds = ?layout.brow.map(|b| brow_face_rects(b)),
                "Detected expression layout"
            );
        } else {
            tracing::info!(
                target: "vertexlauncher/skins_expressions",
                "No supported expression layout detected for current skin sample"
            );
        }
    }
}

fn detect_expression_layout(image: &RgbaImage) -> Option<DetectedExpressionsLayout> {
    let eye = SUPPORTED_EYE_SPECS
        .iter()
        .copied()
        .filter_map(|spec| eye_layout_score(image, spec).map(|score| (score, spec)))
        .max_by(|(score_a, _), (score_b, _)| score_a.total_cmp(score_b))
        .map(|(_, spec)| spec)?;
    let brow = SUPPORTED_BROW_SPECS
        .iter()
        .copied()
        .filter_map(|spec| brow_layout_score(image, spec).map(|score| (score, spec)))
        .max_by(|(score_a, spec_a), (score_b, spec_b)| {
            score_a.total_cmp(score_b).then_with(|| {
                compatibility_score(eye, *spec_a).cmp(&compatibility_score(eye, *spec_b))
            })
        })
        .map(|(_, spec)| spec);

    Some(DetectedExpressionsLayout { eye, brow })
}

fn eye_layout_score(image: &RgbaImage, spec: EyeExpressionSpec) -> Option<f32> {
    let right_pixels = region_alpha_pixels(image, spec.right_eye);
    let left_pixels = region_alpha_pixels(image, spec.left_eye);
    if right_pixels == 0 || left_pixels == 0 {
        return None;
    }

    let right_coverage = region_alpha_coverage(image, spec.right_eye);
    let left_coverage = region_alpha_coverage(image, spec.left_eye);
    let mut score = right_coverage + left_coverage;
    score += (right_pixels + left_pixels) as f32 * 0.12;
    score -= (right_coverage - left_coverage).abs() * 0.35;

    if let (Some(right_white), Some(left_white)) = (spec.right_white, spec.left_white) {
        let right_white_pixels = region_alpha_pixels(image, right_white);
        let left_white_pixels = region_alpha_pixels(image, left_white);
        if right_white_pixels > 0 && left_white_pixels > 0 {
            score += region_alpha_coverage(image, right_white)
                + region_alpha_coverage(image, left_white);
            score += (right_white_pixels + left_white_pixels) as f32 * 0.08;
        }
    }
    if let (Some(right_pupil), Some(left_pupil)) = (spec.right_pupil, spec.left_pupil) {
        let right_pupil_pixels = region_alpha_pixels(image, right_pupil);
        let left_pupil_pixels = region_alpha_pixels(image, left_pupil);
        if right_pupil_pixels > 0 && left_pupil_pixels > 0 {
            score += region_alpha_coverage(image, right_pupil)
                + region_alpha_coverage(image, left_pupil);
            score += (right_pupil_pixels + left_pupil_pixels) as f32 * 0.06;
        }
    }
    if let Some((right_lid, left_lid)) = eye_lid_rects_if_present(spec) {
        let right_lid_pixels = region_alpha_pixels(image, right_lid);
        let left_lid_pixels = region_alpha_pixels(image, left_lid);
        if right_lid_pixels > 0 && left_lid_pixels > 0 {
            score += (region_alpha_coverage(image, right_lid)
                + region_alpha_coverage(image, left_lid))
                * 0.35;
            score += (right_lid_pixels + left_lid_pixels) as f32 * 0.05;
        }
    }

    (score >= 0.18).then_some(score)
}

fn eye_lid_rects_if_present(spec: EyeExpressionSpec) -> Option<(TextureRectU32, TextureRectU32)> {
    spec.blink.map(|_| eye_lid_rects(spec))
}

fn brow_layout_score(image: &RgbaImage, spec: BrowExpressionSpec) -> Option<f32> {
    let right_pixels = region_alpha_pixels(image, spec.right_brow);
    if right_pixels == 0 {
        return None;
    }
    let mut score = region_alpha_coverage(image, spec.right_brow) + right_pixels as f32 * 0.1;
    if let Some(left_brow) = spec.left_brow {
        let left_pixels = region_alpha_pixels(image, left_brow);
        if left_pixels == 0 {
            return None;
        }
        score += region_alpha_coverage(image, left_brow) + left_pixels as f32 * 0.1;
    }
    Some(score)
}

fn region_alpha_pixels(image: &RgbaImage, rect: TextureRectU32) -> u32 {
    let max_x = (rect.x + rect.w).min(image.width());
    let max_y = (rect.y + rect.h).min(image.height());
    let mut covered = 0u32;
    for py in rect.y..max_y {
        for px in rect.x..max_x {
            if image.get_pixel(px, py).0[3] > 24 {
                covered += 1;
            }
        }
    }
    covered
}

fn region_alpha_coverage(image: &RgbaImage, rect: TextureRectU32) -> f32 {
    let max_x = (rect.x + rect.w).min(image.width());
    let max_y = (rect.y + rect.h).min(image.height());
    let mut covered = 0u32;
    let mut total = 0u32;
    for py in rect.y..max_y {
        for px in rect.x..max_x {
            total += 1;
            if image.get_pixel(px, py).0[3] > 24 {
                covered += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        covered as f32 / total as f32
    }
}

#[derive(Clone, Debug)]
struct LoadedProfile {
    player_name: String,
    active_skin_png: Option<Vec<u8>>,
    skin_variant: MinecraftSkinVariant,
    capes: Vec<CapeChoice>,
    active_cape_id: Option<String>,
}

#[derive(Clone, Debug)]
enum WorkerEvent {
    Refreshed(Result<(String, LoadedProfile), String>),
    Saved(Result<(String, LoadedProfile), String>),
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

fn format_auth_error(operation: &str, err: &auth::AuthError) -> String {
    let message = err.to_string();
    if message.contains("HTTP status 401") {
        return format!(
            "Failed to {operation}: {message}. Minecraft auth token may be expired. Sign out and sign back in, then retry."
        );
    }
    format!("Failed to {operation}: {message}")
}

impl LoadedProfile {
    fn from_profile(profile: MinecraftProfileState) -> Self {
        let active_skin = profile
            .skins
            .iter()
            .find(|skin| skin.state.eq_ignore_ascii_case("active"))
            .or_else(|| profile.skins.first());

        let active_skin_png = active_skin.and_then(|skin| skin.texture_png_bytes());
        let skin_variant = active_skin
            .and_then(|skin| skin.variant.as_deref())
            .map(parse_variant)
            .unwrap_or(MinecraftSkinVariant::Classic);

        let mut active_cape_id = None;
        let mut capes = Vec::with_capacity(profile.capes.len());
        for cape in profile.capes {
            let texture_bytes = cape.texture_png_bytes();
            let texture_size = texture_bytes.as_deref().and_then(decode_image_dimensions);
            if cape.state.eq_ignore_ascii_case("active") {
                active_cape_id = Some(cape.id.clone());
            }
            capes.push(CapeChoice {
                label: cape
                    .alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(cape.id.as_str())
                    .to_owned(),
                id: cape.id,
                texture_bytes,
                texture_size,
            });
        }

        Self {
            player_name: profile.name,
            active_skin_png,
            skin_variant,
            capes,
            active_cape_id,
        }
    }
}

fn parse_variant(raw: &str) -> MinecraftSkinVariant {
    if raw.eq_ignore_ascii_case("slim") {
        MinecraftSkinVariant::Slim
    } else {
        MinecraftSkinVariant::Classic
    }
}

fn decode_skin_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    if w == 64 && (h == 64 || h == 32) {
        Some(image)
    } else {
        None
    }
}

fn decode_generic_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| image.to_rgba8())
}

fn decode_image_dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    let image = image::load_from_memory(bytes).ok()?;
    Some([image.width(), image.height()])
}

fn full_uv_rect() -> Rect {
    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

fn full_face_uvs() -> FaceUvs {
    let full = full_uv_rect();
    FaceUvs {
        top: full,
        bottom: full,
        left: full,
        right: full,
        front: full,
        back: full,
    }
}

fn default_cape_uv_layout() -> FaceUvs {
    cape_uv_layout([64, 32]).unwrap_or_else(full_face_uvs)
}

fn default_elytra_wing_uvs() -> ElytraWingUvs {
    elytra_wing_uvs([64, 32]).unwrap_or(ElytraWingUvs {
        left: full_face_uvs(),
        right: full_face_uvs(),
    })
}

fn elytra_wing_uvs(texture_size: [u32; 2]) -> Option<ElytraWingUvs> {
    if texture_size[0] < 46 || texture_size[1] < 22 {
        return None;
    }
    let inset = 0.0;

    // Vanilla elytra model uses texOffs(22, 0) with a 10x20x2 cuboid.
    // Standard cuboid unwrap coordinates:
    // top(10x2)    at (24, 0)
    // bottom(10x2) at (34, 0)
    // left(2x20)   at (22, 2)
    // front(10x20) at (24, 2)
    // right(2x20)  at (34, 2)
    // back(10x20)  at (36, 2)
    let left = FaceUvs {
        top: flip_uv_rect_x(uv_rect_with_inset(texture_size, 24, 0, 10, 2, inset)),
        bottom: flip_uv_rect_x(uv_rect_with_inset(texture_size, 34, 1, 10, 2, inset)),
        left: flip_uv_rect_x(uv_rect_with_inset(texture_size, 34, 2, 2, 20, inset)),
        right: flip_uv_rect_x(uv_rect_with_inset(texture_size, 22, 2, 2, 20, inset)),
        front: flip_uv_rect_x(uv_rect_with_inset(texture_size, 24, 2, 10, 20, inset)),
        back: flip_uv_rect_x(uv_rect_with_inset(texture_size, 36, 2, 10, 20, inset)),
    };
    // Right wing mirrors the side strip assignment.
    let right = FaceUvs {
        top: uv_rect_with_inset(texture_size, 24, 0, 10, 2, inset),
        bottom: uv_rect_with_inset(texture_size, 34, 1, 10, 2, inset),
        left: uv_rect_with_inset(texture_size, 22, 2, 2, 20, inset),
        right: uv_rect_with_inset(texture_size, 34, 2, 2, 20, inset),
        front: uv_rect_with_inset(texture_size, 24, 2, 10, 20, inset),
        back: uv_rect_with_inset(texture_size, 36, 2, 10, 20, inset),
    };
    Some(ElytraWingUvs { left, right })
}

fn default_elytra_texture_image() -> RgbaImage {
    const DEFAULT_ELYTRA_TEXTURE_PNG: &[u8] = include_bytes!("../assets/default_elytra.png");
    if let Some(image) = decode_generic_rgba(DEFAULT_ELYTRA_TEXTURE_PNG) {
        return image;
    }

    // Fallback only: use a simple neutral texture if embedded bytes ever fail to decode.
    let mut image = RgbaImage::from_pixel(64, 32, image::Rgba([0, 0, 0, 0]));
    let base = image::Rgba([141, 141, 141, 255]);
    let edge = image::Rgba([112, 112, 112, 255]);
    fill_rect_rgba(&mut image, 22, 0, 24, 22, base);
    fill_rect_rgba(&mut image, 22, 0, 24, 1, edge);
    fill_rect_rgba(&mut image, 22, 21, 24, 1, edge);
    fill_rect_rgba(&mut image, 22, 0, 1, 22, edge);
    fill_rect_rgba(&mut image, 45, 0, 1, 22, edge);
    image
}

fn fill_rect_rgba(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: image::Rgba<u8>,
) {
    let max_x = image.width();
    let max_y = image.height();
    for py in y..y.saturating_add(height).min(max_y) {
        for px in x..x.saturating_add(width).min(max_x) {
            image.put_pixel(px, py, color);
        }
    }
}

fn cape_outer_face_uv(texture_size: [u32; 2]) -> Option<Rect> {
    if texture_size[0] < 22 || texture_size[1] < 17 {
        return None;
    }
    Some(uv_rect_with_inset(
        texture_size,
        1,
        1,
        10,
        16,
        UV_EDGE_INSET_BASE_TEXELS,
    ))
}

fn cape_uv_layout(texture_size: [u32; 2]) -> Option<FaceUvs> {
    let outer = cape_outer_face_uv(texture_size)?;
    let inner = uv_rect_with_inset(texture_size, 12, 1, 10, 16, UV_EDGE_INSET_BASE_TEXELS);
    Some(FaceUvs {
        top: uv_rect_with_inset(texture_size, 1, 0, 10, 1, UV_EDGE_INSET_BASE_TEXELS),
        bottom: uv_rect_with_inset(texture_size, 11, 0, 10, 1, UV_EDGE_INSET_BASE_TEXELS),
        left: uv_rect_with_inset(texture_size, 0, 1, 1, 16, UV_EDGE_INSET_BASE_TEXELS),
        right: uv_rect_with_inset(texture_size, 11, 1, 1, 16, UV_EDGE_INSET_BASE_TEXELS),
        // Cape sits behind the torso in our coordinate space, so the cuboid "back" face is the
        // outward-facing panel visible from behind the player.
        front: inner,
        back: outer,
    })
}

fn _absolute_path_string(path: &PathBuf) -> String {
    path.display().to_string()
}
