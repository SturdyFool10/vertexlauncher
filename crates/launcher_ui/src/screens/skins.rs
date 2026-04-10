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

#[path = "skins/auth_error_format.rs"]
mod auth_error_format;
#[path = "skins/cape_choice.rs"]
mod cape_choice;
#[path = "skins/cape_uv_layout.rs"]
mod cape_uv_layout;
#[path = "skins/default_elytra_texture.rs"]
mod default_elytra_texture;
#[path = "skins/loaded_profile.rs"]
mod loaded_profile;
#[path = "skins/preview_motion_mode.rs"]
mod preview_motion_mode;
#[path = "skins/preview_panel.rs"]
mod preview_panel;
#[path = "skins/preview_pose.rs"]
mod preview_pose;
#[path = "skins/profile_cache.rs"]
mod profile_cache;
#[path = "skins/profile_variant.rs"]
mod profile_variant;
#[path = "skins/screen_body.rs"]
mod screen_body;
#[path = "skins/screen_runtime.rs"]
mod screen_runtime;
#[path = "skins/screen_sections.rs"]
mod screen_sections;
#[path = "skins/skin_drop_zone.rs"]
mod skin_drop_zone;
#[path = "skins/skin_image.rs"]
mod skin_image;
#[path = "skins_cape_grid.rs"]
mod skins_cape_grid;
#[path = "skins_expressions.rs"]
mod skins_expressions;
#[path = "skins_preview.rs"]
mod skins_preview;
#[path = "skins_preview_gpu.rs"]
mod skins_preview_gpu;
#[path = "skins_state.rs"]
mod skins_state;
#[path = "skins/worker_event.rs"]
mod worker_event;
use self::auth_error_format::format_auth_error;
use self::cape_choice::CapeChoice;
use self::cape_uv_layout::{
    cape_outer_face_uv, cape_uv_layout, default_cape_uv_layout, default_elytra_wing_uvs,
    elytra_wing_uvs,
};
use self::default_elytra_texture::default_elytra_texture_image;
use self::loaded_profile::LoadedProfile;
use self::profile_cache::{fetch_and_cache_profile, update_cached_profile};
use self::profile_variant::parse_variant;
use self::screen_body::render_skin_screen_contents;
use self::screen_runtime::{
    apply_skin_screen_runtime_settings, prepare_skin_screen_frame, schedule_skin_screen_repaint,
};
use self::skin_image::{decode_generic_rgba, decode_image_dimensions, decode_skin_rgba};
use self::skins_cape_grid::*;
use self::skins_expressions::*;
use self::skins_preview::*;
use self::skins_preview_gpu::*;
use self::skins_state::*;
use self::worker_event::WorkerEvent;
use self::{preview_motion_mode::PreviewMotionMode, preview_pose::PreviewPose};
use self::{preview_panel::render_preview_panel, screen_sections::render_skin_screen_sections};

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

    apply_skin_screen_runtime_settings(
        &mut state,
        active_launch_auth,
        skin_manager_opened,
        skin_manager_account_switched,
        wgpu_target_format,
        skin_preview_msaa_samples,
        preview_aa_mode,
        preview_texel_aa_mode,
        preview_motion_blur_enabled,
        preview_motion_blur_amount,
        preview_motion_blur_shutter_frames,
        preview_motion_blur_sample_count,
        preview_3d_layers_enabled,
        expressions_enabled,
    );
    prepare_skin_screen_frame(&mut state, ui.ctx());
    schedule_skin_screen_repaint(&state, ui.ctx());

    gamepad_scroll(
        egui::ScrollArea::vertical().auto_shrink([false, false]),
        ui,
        |ui| render_skin_screen_contents(ui, text_ui, &mut state, streamer_mode),
    );

    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
}
