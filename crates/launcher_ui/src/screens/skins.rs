use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use auth::{CachedAccount, MinecraftProfileState, MinecraftSkinVariant};
use bytemuck::{Pod, Zeroable};
use config::SkinPreviewAaMode;
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui};
use image::{RgbaImage, imageops::FilterType};
use textui::{ButtonOptions, TextUi};

use super::LaunchAuthContext;
use crate::{notification, privacy, ui::style};

const PREVIEW_ORBIT_SECONDS: f64 = 45.0;
const PREVIEW_TARGET_FPS: f32 = 60.0;
const PREVIEW_HEIGHT: f32 = 460.0;
const CAMERA_DRAG_SENSITIVITY_RAD_PER_POINT: f32 = 0.0046;
const CAMERA_INERTIA_FRICTION_PER_SEC: f32 = 2.0;
const CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC: f32 = 0.015;
const UV_EDGE_INSET_BASE_TEXELS: f32 = 0.08;
const UV_EDGE_INSET_OVERLAY_TEXELS: f32 = 0.38;
const CAPE_TILE_WIDTH_MIN: f32 = 132.0;
const CAPE_TILE_HEIGHT: f32 = 186.0;
const SKIN_PREVIEW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SKIN_PREVIEW_NEAR: f32 = 1.5;
const SKIN_PREVIEW_ANISOTROPY_CLAMP: u16 = 16;
const MOTION_BLUR_MIN_ANGULAR_SPAN: f32 = 0.015;

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
    preview_motion_blur_enabled: bool,
    preview_motion_blur_amount: f32,
    preview_motion_blur_shutter_frames: f32,
    preview_motion_blur_sample_count: i32,
    preview_3d_layers_enabled: bool,
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
    state.preview_motion_blur_enabled = preview_motion_blur_enabled;
    state.preview_motion_blur_amount = preview_motion_blur_amount.clamp(0.0, 1.0);
    state.preview_motion_blur_shutter_frames = preview_motion_blur_shutter_frames.max(0.0);
    state.preview_motion_blur_sample_count = preview_motion_blur_sample_count.max(1) as usize;
    state.preview_3d_layers_enabled = preview_3d_layers_enabled;
    if state.last_preview_aa_mode != state.preview_aa_mode
        || state.last_preview_motion_blur_enabled != state.preview_motion_blur_enabled
        || (state.last_preview_motion_blur_amount - state.preview_motion_blur_amount).abs()
            > f32::EPSILON
        || (state.last_preview_motion_blur_shutter_frames
            - state.preview_motion_blur_shutter_frames)
            .abs()
            > f32::EPSILON
        || state.last_preview_motion_blur_sample_count != state.preview_motion_blur_sample_count
        || state.last_preview_3d_layers_enabled != state.preview_3d_layers_enabled
    {
        state.preview_texture = None;
        state.preview_history = None;
        state.last_preview_aa_mode = state.preview_aa_mode;
        state.last_preview_motion_blur_enabled = state.preview_motion_blur_enabled;
        state.last_preview_motion_blur_amount = state.preview_motion_blur_amount;
        state.last_preview_motion_blur_shutter_frames = state.preview_motion_blur_shutter_frames;
        state.last_preview_motion_blur_sample_count = state.preview_motion_blur_sample_count;
        state.last_preview_3d_layers_enabled = state.preview_3d_layers_enabled;
    }
    state.poll_worker(ui.ctx());
    state.try_consume_open_refresh();
    state.ensure_skin_texture(ui.ctx());
    state.ensure_default_elytra_texture(ui.ctx());
    state.ensure_cape_texture(ui.ctx());
    ui.ctx()
        .request_repaint_after(Duration::from_secs_f32(1.0 / PREVIEW_TARGET_FPS));

    egui::ScrollArea::vertical()
        .id_salt("skins_screen_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_contents(ui, text_ui, &mut state, streamer_mode);
        });

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

    let button_style = neutral_button_style(ui);

    ui.horizontal(|ui| {
        if text_ui
            .button(
                ui,
                "skins_refresh_profile",
                "Refresh profile",
                &button_style,
            )
            .clicked()
        {
            state.start_refresh();
        }
    });

    ui.add_space(style::SPACE_LG);
    let _ = text_ui.label(
        ui,
        "skins_picker_heading",
        "Skin Image",
        &style::section_heading(ui),
    );

    if text_ui
        .button(ui, "skins_pick_file", "Choose skin image", &button_style)
        .clicked()
    {
        state.pick_skin_file();
    }

    if let Some(path) = state.pending_skin_path.as_deref() {
        ui.add_space(style::SPACE_XS);
        let _ = text_ui.label(ui, "skins_selected_path", path, &muted);
    }

    ui.add_space(style::SPACE_SM);
    let mut model_button_style = button_style.clone();
    model_button_style.min_size = egui::vec2(120.0, style::CONTROL_HEIGHT);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_XS, style::SPACE_XS);
        let _ = text_ui.label(ui, "skins_model_label", "Model:", &body);
        if text_ui
            .selectable_button(
                ui,
                "skins_model_classic",
                "Classic",
                state.pending_variant == MinecraftSkinVariant::Classic,
                &model_button_style,
            )
            .clicked()
        {
            state.pending_variant = MinecraftSkinVariant::Classic;
        }
        if text_ui
            .selectable_button(
                ui,
                "skins_model_slim",
                "Slim (Alex)",
                state.pending_variant == MinecraftSkinVariant::Slim,
                &model_button_style,
            )
            .clicked()
        {
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
    let viewport_width = ui.clip_rect().width().max(1.0);
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

    paint_preview_background(&painter, rect);

    let now = ui.input(|i| i.time);
    let dt = state.consume_frame_dt(now);

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
    let walk = (now as f32 * 3.3).sin();

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
            now as f32,
            walk,
            variant,
            show_elytra,
            wgpu_target_format,
            preview_msaa_samples,
            preview_aa_mode,
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

    let toggle_rect = Rect::from_min_size(
        egui::pos2(rect.left() + 14.0, rect.bottom() - 46.0),
        egui::vec2(154.0, 32.0),
    );
    let toggle_text = if state.show_elytra {
        "Elytra: On"
    } else {
        "Elytra: Off"
    };
    let mut button_clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(toggle_rect), |ui| {
        let mut toggle_style = neutral_button_style(ui);
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

fn paint_preview_background(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(
        rect,
        CornerRadius::same(8),
        Color32::from_rgba_premultiplied(23, 26, 32, 186),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0, Color32::from_rgba_premultiplied(165, 173, 184, 42)),
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
    time_seconds: f32,
    walk_phase: f32,
    variant: MinecraftSkinVariant,
    show_elytra: bool,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
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
        time_seconds,
        walk_phase,
        variant,
        preview_3d_layers_enabled,
        show_elytra,
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
                time_seconds,
                walk_phase,
                preview_motion_blur_shutter_frames,
                preview_motion_blur_sample_count,
                variant,
                preview_3d_layers_enabled,
                show_elytra,
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
        preview_texture,
        preview_history,
    );
}

struct BuiltCharacterScene {
    triangles: Vec<RenderTriangle>,
    cape_render_sample: Option<Arc<RgbaImage>>,
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
    time_seconds: f32,
    walk_phase: f32,
    variant: MinecraftSkinVariant,
    preview_3d_layers_enabled: bool,
    show_elytra: bool,
    skin_sample: Option<Arc<RgbaImage>>,
    cape_sample: Option<Arc<RgbaImage>>,
    default_elytra_sample: Option<Arc<RgbaImage>>,
) -> BuiltCharacterScene {
    let arm_width = if variant == MinecraftSkinVariant::Slim {
        3.0
    } else {
        4.0
    };
    let bob = walk_phase.abs() * 0.55;
    let leg_swing = walk_phase * 0.62;
    let arm_swing = -walk_phase * 0.74;

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
            rotate_x: 0.0,
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
                    rotate_x: 0.0,
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
                rotate_x: 0.0,
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
            rotate_x: 0.0,
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
                    rotate_x: 0.0,
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
                rotate_x: 0.0,
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
            walk_phase,
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
            time_seconds,
            walk_phase,
            uv_layout,
            light_dir,
        );
    }

    BuiltCharacterScene {
        triangles: scene_tris,
        cape_render_sample,
    }
}

struct WeightedPreviewScene {
    weight: f32,
    triangles: Vec<RenderTriangle>,
}

fn build_motion_blur_scene_samples(
    rect: Rect,
    cape_uv: FaceUvs,
    yaw: f32,
    yaw_velocity: f32,
    time_seconds: f32,
    walk_phase: f32,
    shutter_frames: f32,
    sample_count: usize,
    variant: MinecraftSkinVariant,
    preview_3d_layers_enabled: bool,
    show_elytra: bool,
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
        let scene = build_character_scene(
            rect,
            cape_uv,
            sample_yaw,
            time_seconds,
            walk_phase,
            variant,
            preview_3d_layers_enabled,
            show_elytra,
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
) {
    let callback = SkinPreviewPostProcessWgpuCallback::from_weighted_scenes(
        scenes,
        skin_sample,
        cape_sample,
        target_format,
        scene_msaa_samples,
        present_msaa_samples,
        preview_aa_mode,
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
        let use_fxaa = self.aa_mode == SkinPreviewAaMode::Fxaa;
        let use_taa = self.aa_mode == SkinPreviewAaMode::Taa;
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
        } else if use_fxaa {
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
            if resources.taa_history_valid {
                queue.write_buffer(
                    &resources.scalar_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&GpuPreviewScalarUniform {
                        value: [0.22, 0.0, 0.0, 0.0],
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
                resources.present_source = PresentSource::PostProcess;
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
                resources.present_source = PresentSource::Accumulation;
            }
            resources.taa_history_valid = true;
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
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct VertexIn {
    @location(0) pos_points: vec2<f32>,
    @location(1) camera_z: f32,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

struct Globals {
    screen_size_points: vec2<f32>,
    _pad: vec2<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@group(0) @binding(0)
var preview_tex: texture_2d<f32>;
@group(0) @binding(1)
var preview_sampler: sampler;
@group(1) @binding(0)
var<uniform> globals: Globals;

fn sample_preview_pixel_art(uv: vec2<f32>) -> vec4<f32> {
    let dims_i = textureDimensions(preview_tex);
    let dims = vec2<f32>(dims_i);
    let texel = 0.5 / dims;
    let clamped_uv = clamp(uv, texel, vec2<f32>(1.0) - texel);
    let uv_grad_x = dpdx(clamped_uv);
    let uv_grad_y = dpdy(clamped_uv);
    let texel_grad_x = uv_grad_x * dims;
    let texel_grad_y = uv_grad_y * dims;
    let footprint = max(
        max(abs(texel_grad_x.x), abs(texel_grad_x.y)),
        max(abs(texel_grad_y.x), abs(texel_grad_y.y)),
    );
    let pixel = clamp(
        vec2<i32>(clamped_uv * dims),
        vec2<i32>(0),
        vec2<i32>(dims_i) - vec2<i32>(1),
    );
    let nearest = textureLoad(preview_tex, pixel, 0);
    let filtered = textureSampleGrad(
        preview_tex,
        preview_sampler,
        clamped_uv,
        uv_grad_x,
        uv_grad_y,
    );
    let filtered_mix = smoothstep(0.85, 1.35, footprint);
    return mix(nearest, filtered, filtered_mix);
}

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    let x_ndc = (input.pos_points.x / globals.screen_size_points.x) * 2.0 - 1.0;
    let y_ndc = 1.0 - (input.pos_points.y / globals.screen_size_points.y) * 2.0;
    let z_cam = max(input.camera_z, 1.5 + 0.0001);
    let clip_w = z_cam;
    let clip_z = z_cam - 1.5;
    out.pos = vec4<f32>(x_ndc * clip_w, y_ndc * clip_w, clip_z, clip_w);
    out.uv = input.uv;
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let sampled = sample_preview_pixel_art(input.uv) * input.color;
    if sampled.a <= 0.001 {
        discard;
    }
    return sampled;
}
"#,
            )),
        });

        let accumulate_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-accumulate-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct Scalar {
    value: vec4<f32>,
};

struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0)
var source_tex: texture_2d<f32>;
@group(1) @binding(0)
var<uniform> scalar: Scalar;

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenOut {
    var out: FullscreenOut;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = textureDimensions(source_tex);
    let pixel = clamp(vec2<i32>(pos.xy), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(source_tex, pixel, 0) * scalar.value.x;
}
"#,
            )),
        });

        let fxaa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-fxaa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0)
var source_tex: texture_2d<f32>;

fn rgb_luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
}

fn load_rgba(pixel: vec2<i32>) -> vec4<f32> {
    let dims = textureDimensions(source_tex);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(source_tex, clamped, 0);
}

fn sample_linear(pixel: vec2<f32>) -> vec4<f32> {
    let dims = textureDimensions(source_tex);
    let max_pixel = vec2<f32>(vec2<i32>(dims) - vec2<i32>(1));
    let p = clamp(pixel, vec2<f32>(0.0), max_pixel);
    let p0 = vec2<i32>(floor(p));
    let p1 = min(p0 + vec2<i32>(1), vec2<i32>(dims) - vec2<i32>(1));
    let f = fract(p);
    let c00 = load_rgba(p0);
    let c10 = load_rgba(vec2<i32>(p1.x, p0.y));
    let c01 = load_rgba(vec2<i32>(p0.x, p1.y));
    let c11 = load_rgba(p1);
    let top = mix(c00, c10, f.x);
    let bottom = mix(c01, c11, f.x);
    return mix(top, bottom, f.y);
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenOut {
    var out: FullscreenOut;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let nw = load_rgba(p + vec2<i32>(-1, -1));
    let ne = load_rgba(p + vec2<i32>(1, -1));
    let sw = load_rgba(p + vec2<i32>(-1, 1));
    let se = load_rgba(p + vec2<i32>(1, 1));
    let m = load_rgba(p);

    let luma_nw = rgb_luma(nw.rgb);
    let luma_ne = rgb_luma(ne.rgb);
    let luma_sw = rgb_luma(sw.rgb);
    let luma_se = rgb_luma(se.rgb);
    let luma_m = rgb_luma(m.rgb);

    let luma_min = min(luma_m, min(min(luma_nw, luma_ne), min(luma_sw, luma_se)));
    let luma_max = max(luma_m, max(max(luma_nw, luma_ne), max(luma_sw, luma_se)));
    let luma_range = luma_max - luma_min;
    let threshold = max(1.0 / 16.0, luma_max * (1.0 / 8.0));
    if luma_range < threshold {
        return m;
    }

    var dir = vec2<f32>(
        -((luma_nw + luma_ne) - (luma_sw + luma_se)),
        (luma_nw + luma_sw) - (luma_ne + luma_se),
    );
    let dir_reduce = max(
        ((luma_nw + luma_ne + luma_sw + luma_se) * 0.25) * (1.0 / 8.0),
        1.0 / 128.0,
    );
    let rcp_dir_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + dir_reduce);
    dir = clamp(dir * rcp_dir_min, vec2<f32>(-8.0), vec2<f32>(8.0));

    let fp = pos.xy;
    let rgb_a = 0.5 * (
        sample_linear(fp + dir * (1.0 / 3.0 - 0.5)).rgb +
        sample_linear(fp + dir * (2.0 / 3.0 - 0.5)).rgb
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        sample_linear(fp + dir * -0.5).rgb +
        sample_linear(fp + dir * 0.5).rgb
    );

    let luma_b = rgb_luma(rgb_b);
    var final_rgb = rgb_b;
    if luma_b < luma_min || luma_b > luma_max {
        final_rgb = rgb_a;
    }
    return vec4<f32>(final_rgb, m.a);
}
"#,
            )),
        });

        let smaa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-smaa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0)
var source_tex: texture_2d<f32>;

fn rgb_luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
}

fn load_rgba(pixel: vec2<i32>) -> vec4<f32> {
    let dims = textureDimensions(source_tex);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(source_tex, clamped, 0);
}

fn sample_linear(pixel: vec2<f32>) -> vec4<f32> {
    let dims = textureDimensions(source_tex);
    let max_pixel = vec2<f32>(vec2<i32>(dims) - vec2<i32>(1));
    let p = clamp(pixel, vec2<f32>(0.0), max_pixel);
    let p0 = vec2<i32>(floor(p));
    let p1 = min(p0 + vec2<i32>(1), vec2<i32>(dims) - vec2<i32>(1));
    let f = fract(p);
    let c00 = load_rgba(p0);
    let c10 = load_rgba(vec2<i32>(p1.x, p0.y));
    let c01 = load_rgba(vec2<i32>(p0.x, p1.y));
    let c11 = load_rgba(p1);
    let top = mix(c00, c10, f.x);
    let bottom = mix(c01, c11, f.x);
    return mix(top, bottom, f.y);
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenOut {
    var out: FullscreenOut;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let c = load_rgba(p);
    let l = load_rgba(p + vec2<i32>(-1, 0));
    let r = load_rgba(p + vec2<i32>(1, 0));
    let t = load_rgba(p + vec2<i32>(0, -1));
    let b = load_rgba(p + vec2<i32>(0, 1));

    let luma_c = rgb_luma(c.rgb);
    let luma_l = rgb_luma(l.rgb);
    let luma_r = rgb_luma(r.rgb);
    let luma_t = rgb_luma(t.rgb);
    let luma_b = rgb_luma(b.rgb);

    let edge_h = max(abs(luma_l - luma_c), abs(luma_r - luma_c));
    let edge_v = max(abs(luma_t - luma_c), abs(luma_b - luma_c));
    let threshold = max(0.04, luma_c * 0.12);

    if max(edge_h, edge_v) < threshold {
        return c;
    }

    let center = pos.xy;
    if edge_h >= edge_v {
        let a = sample_linear(center + vec2<f32>(-0.75, 0.0));
        let b = sample_linear(center + vec2<f32>(0.75, 0.0));
        let long_a = sample_linear(center + vec2<f32>(-1.5, 0.0));
        let long_b = sample_linear(center + vec2<f32>(1.5, 0.0));
        let blend = clamp((edge_h - threshold) * 6.0, 0.0, 1.0);
        let neighbor = mix(0.5 * (a + b), 0.5 * (long_a + long_b), 0.35);
        return mix(c, vec4<f32>(neighbor.rgb, c.a), blend * 0.75);
    }

    let sample_a = sample_linear(center + vec2<f32>(0.0, -0.75));
    let sample_b = sample_linear(center + vec2<f32>(0.0, 0.75));
    let long_sample_a = sample_linear(center + vec2<f32>(0.0, -1.5));
    let long_sample_b = sample_linear(center + vec2<f32>(0.0, 1.5));
    let blend = clamp((edge_v - threshold) * 6.0, 0.0, 1.0);
    let neighbor = mix(
        0.5 * (sample_a + sample_b),
        0.5 * (long_sample_a + long_sample_b),
        0.35,
    );
    return mix(c, vec4<f32>(neighbor.rgb, c.a), blend * 0.75);
}
"#,
            )),
        });

        let taa_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-taa-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct Scalar {
    value: vec4<f32>,
};

struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0)
var current_tex: texture_2d<f32>;
@group(1) @binding(0)
var history_tex: texture_2d<f32>;
@group(2) @binding(0)
var<uniform> scalar: Scalar;

fn load_current(pixel: vec2<i32>) -> vec4<f32> {
    let dims = textureDimensions(current_tex);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(current_tex, clamped, 0);
}

fn load_history(pixel: vec2<i32>) -> vec4<f32> {
    let dims = textureDimensions(history_tex);
    let clamped = clamp(pixel, vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(history_tex, clamped, 0);
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenOut {
    var out: FullscreenOut;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(pos.xy);
    let current = load_current(pixel);
    var lo = current;
    var hi = current;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let sample = load_current(pixel + vec2<i32>(x, y));
            lo = min(lo, sample);
            hi = max(hi, sample);
        }
    }
    let history = clamp(load_history(pixel), lo, hi);
    let current_weight = clamp(scalar.value.x, 0.05, 1.0);
    return mix(history, current, current_weight);
}
"#,
            )),
        });

        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skins-preview-present-shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
};

@group(0) @binding(0)
var source_tex: texture_2d<f32>;

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenOut {
    var out: FullscreenOut;
    let x = f32((vertex_index << 1u) & 2u);
    let y = f32(vertex_index & 2u);
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = textureDimensions(source_tex);
    let pixel = clamp(vec2<i32>(pos.xy), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    return textureLoad(source_tex, pixel, 0);
}
"#,
            )),
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
                bind_group_layouts: &[&texture_bind_group_layout, &scene_uniform_layout],
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
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r#"
struct VertexIn {
    @location(0) pos_points: vec2<f32>,
    @location(1) camera_z: f32,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

struct Globals {
    screen_size_points: vec2<f32>,
    _pad: vec2<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    let x_ndc = (input.pos_points.x / globals.screen_size_points.x) * 2.0 - 1.0;
    let y_ndc = 1.0 - (input.pos_points.y / globals.screen_size_points.y) * 2.0;
    let z_cam = max(input.camera_z, 1.5 + 0.0001);
    let clip_w = z_cam;
    let clip_z = z_cam - 1.5;
    out.pos = vec4<f32>(x_ndc * clip_w, y_ndc * clip_w, clip_z, clip_w);
    out.uv = input.uv;
    out.color = input.color;
    return out;
}

@group(0) @binding(0)
var preview_tex: texture_2d<f32>;
@group(0) @binding(1)
var preview_sampler: sampler;
@group(1) @binding(0)
var<uniform> globals: Globals;

fn sample_preview_pixel_art(uv: vec2<f32>) -> vec4<f32> {
    let dims_i = textureDimensions(preview_tex);
    let dims = vec2<f32>(dims_i);
    let texel = 0.5 / dims;
    let clamped_uv = clamp(uv, texel, vec2<f32>(1.0) - texel);
    let uv_grad_x = dpdx(clamped_uv);
    let uv_grad_y = dpdy(clamped_uv);
    let texel_grad_x = uv_grad_x * dims;
    let texel_grad_y = uv_grad_y * dims;
    let footprint = max(
        max(abs(texel_grad_x.x), abs(texel_grad_x.y)),
        max(abs(texel_grad_y.x), abs(texel_grad_y.y)),
    );
    let pixel = clamp(
        vec2<i32>(clamped_uv * dims),
        vec2<i32>(0),
        vec2<i32>(dims_i) - vec2<i32>(1),
    );
    let nearest = textureLoad(preview_tex, pixel, 0);
    let filtered = textureSampleGrad(
        preview_tex,
        preview_sampler,
        clamped_uv,
        uv_grad_x,
        uv_grad_y,
    );
    let filtered_mix = smoothstep(0.85, 1.35, footprint);
    return mix(nearest, filtered, filtered_mix);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let sampled = sample_preview_pixel_art(input.uv) * input.color;
    if sampled.a <= 0.001 {
        discard;
    }
    return sampled;
}
"#,
            )),
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

    let available_width = ui.available_width().max(1.0);
    let tile_gap = egui::vec2(style::SPACE_MD, style::SPACE_MD);
    let tile_width = (max_label_width + 24.0)
        .max(CAPE_TILE_WIDTH_MIN)
        .min(available_width);
    let columns =
        (((available_width + tile_gap.x) / (tile_width + tile_gap.x)).floor() as usize).max(1);
    let total_items = state.available_capes.len() + 1;
    let grid_columns = total_items.min(columns).max(1);
    let grid_width =
        (grid_columns as f32 * tile_width) + (grid_columns.saturating_sub(1) as f32 * tile_gap.x);
    let grid_leading_space = ((available_width - grid_width) * 0.5).max(0.0);

    let mut pending_selection = None;
    for row_start in (0..total_items).step_by(columns) {
        let row_end = (row_start + columns).min(total_items);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = tile_gap.x;
            if grid_leading_space > 0.0 {
                ui.add_space(grid_leading_space);
            }

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
    let tile_rect = rect.shrink2(egui::vec2(style::SPACE_XS * 0.5, style::SPACE_XS * 0.5));

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

    let fill = if selected {
        ui.visuals()
            .selection
            .bg_fill
            .gamma_multiply(0.24 + hover_t * 0.06 + press_t * 0.04)
    } else {
        ui.visuals()
            .widgets
            .inactive
            .bg_fill
            .gamma_multiply(1.0 + hover_t * 0.08 + press_t * 0.04)
    };
    let stroke = if selected {
        let mut stroke = ui.visuals().selection.stroke;
        stroke.width += hover_t * 0.5;
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
        selected_t,
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

            egui::Image::from_bytes(uri, bytes.to_vec())
                .uv(back_uv)
                .fit_to_exact_size(back_rect.size())
                .texture_options(TextureOptions::NEAREST)
                .paint_at(ui, back_rect);
        } else {
            let image = egui::Image::from_bytes(uri, bytes.to_vec())
                .fit_to_exact_size(preview_rect.size())
                .texture_options(TextureOptions::NEAREST);
            image.paint_at(ui, preview_rect);
        }
    } else {
        ui.painter().rect_filled(
            preview_rect,
            CornerRadius::same(6),
            ui.visuals().widgets.noninteractive.bg_fill,
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
    pending_skin_path: Option<String>,
    pending_variant: MinecraftSkinVariant,
    available_capes: Vec<CapeChoice>,
    initial_cape_id: Option<String>,
    pending_cape_id: Option<String>,
    show_elytra: bool,
    status_message: Option<String>,
    save_in_progress: bool,
    refresh_in_progress: bool,
    worker_rx: Option<Arc<Mutex<Receiver<WorkerEvent>>>>,
    wgpu_target_format: Option<wgpu::TextureFormat>,
    preview_msaa_samples: u32,
    preview_aa_mode: SkinPreviewAaMode,
    last_preview_aa_mode: SkinPreviewAaMode,
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
    skin_texture_hash: Option<u64>,
    skin_texture: Option<TextureHandle>,
    skin_sample: Option<Arc<RgbaImage>>,
    animated_skin_texture_hash: Option<u64>,
    animated_skin_texture: Option<TextureHandle>,
    animated_skin_sample: Option<Arc<RgbaImage>>,
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
            pending_variant: MinecraftSkinVariant::Classic,
            available_capes: Vec::new(),
            initial_cape_id: None,
            pending_cape_id: None,
            show_elytra: false,
            status_message: None,
            save_in_progress: false,
            refresh_in_progress: false,
            worker_rx: None,
            wgpu_target_format: None,
            preview_msaa_samples: 1,
            preview_aa_mode: SkinPreviewAaMode::Msaa,
            last_preview_aa_mode: SkinPreviewAaMode::Msaa,
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
            skin_texture_hash: None,
            skin_texture: None,
            skin_sample: None,
            animated_skin_texture_hash: None,
            animated_skin_texture: None,
            animated_skin_sample: None,
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
        self.status_message = None;
        self.show_elytra = false;
        self.active_profile_id = Some(normalized_profile_id.clone());
        self.active_player_name = Some(auth.player_name.clone());
        self.access_token = auth.access_token.clone();
        self.base_skin_png = None;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.pending_variant = MinecraftSkinVariant::Classic;
        self.available_capes.clear();
        self.initial_cape_id = None;
        self.pending_cape_id = None;
        self.skin_texture_hash = None;
        self.skin_texture = None;
        self.skin_sample = None;
        self.animated_skin_texture_hash = None;
        self.animated_skin_texture = None;
        self.animated_skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_texture = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
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
                notification::error!("skin_manager", "Failed to load account cache: {err}");
            }
        }
    }

    fn apply_account_snapshot(&mut self, account: &CachedAccount) {
        self.active_profile_id = Some(account.minecraft_profile.id.to_ascii_lowercase());
        self.active_player_name = Some(account.minecraft_profile.name.clone());

        // Do not trust cached equipped skin/cape state; always load current equip from Mojang.
        self.base_skin_png = None;
        self.pending_variant = MinecraftSkinVariant::Classic;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.skin_texture_hash = None;
        self.skin_texture = None;
        self.skin_sample = None;
        self.animated_skin_texture_hash = None;
        self.animated_skin_texture = None;
        self.animated_skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_texture = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
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
                        tracing::info!(
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

    fn ensure_skin_texture(&mut self, ctx: &egui::Context) {
        let active_png = self.preview_skin_png();
        let Some(bytes) = active_png else {
            self.skin_texture = None;
            self.skin_texture_hash = None;
            self.skin_sample = None;
            self.animated_skin_texture = None;
            self.animated_skin_texture_hash = None;
            self.animated_skin_sample = None;
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
            self.animated_skin_texture = None;
            self.animated_skin_texture_hash = None;
            self.animated_skin_sample = None;
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
        let picked = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_title("Select Minecraft Skin")
            .pick_file();

        let Some(path) = picked else {
            return;
        };

        match std::fs::read(path.as_path()) {
            Ok(bytes) => {
                if decode_skin_rgba(&bytes).is_none() {
                    notification::error!(
                        "skin_manager",
                        "Selected image must be a valid PNG skin (expected 64x64 or 64x32)."
                    );
                    return;
                }
                self.pending_skin_png = Some(bytes);
                self.pending_skin_path = Some(path.display().to_string());
                self.skin_texture_hash = None;
                self.skin_sample = None;
                self.animated_skin_texture = None;
                self.animated_skin_texture_hash = None;
                self.animated_skin_sample = None;
                self.preview_texture = None;
                self.preview_history = None;
            }
            Err(err) => {
                notification::error!("skin_manager", "Failed to read image: {err}");
            }
        }
    }

    fn can_save(&self) -> bool {
        self.access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|token| !token.is_empty())
            && (self.pending_skin_png.is_some() || self.pending_cape_id != self.initial_cape_id)
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
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| {
                fetch_and_cache_profile(profile_id, &token, display_name_for_log.as_str())
            })
            .unwrap_or_else(|_| Err("Skin profile refresh task panicked.".to_owned()));
            let _ = tx.send(WorkerEvent::Refreshed(
                result.map(|loaded| (profile_id_for_result, loaded)),
            ));
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
        let pending_variant = self.pending_variant;
        let pending_cape = self.pending_cape_id.clone();
        let initial_cape = self.initial_cape_id.clone();
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name = self.active_player_name.as_deref().unwrap_or("unknown"),
            has_skin_change = pending_skin.is_some(),
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

        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(|| -> Result<LoadedProfile, String> {
                let mut latest_profile: Option<MinecraftProfileState> = None;
                if let Some(bytes) = pending_skin.as_deref() {
                    tracing::info!(
                        target: "vertexlauncher/skins",
                        display_name = display_name_for_log.as_str(),
                        png_bytes = bytes.len(),
                        variant = pending_variant.as_api_str(),
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
            })
            .unwrap_or_else(|_| Err("Skin save task panicked.".to_owned()));

            let _ = tx.send(WorkerEvent::Saved(
                result.map(|loaded| (profile_id_for_result, loaded)),
            ));
        });
    }

    fn apply_loaded_profile(&mut self, profile: LoadedProfile) {
        self.active_player_name = Some(profile.player_name);
        self.base_skin_png = profile.active_skin_png;
        self.pending_skin_png = None;
        self.pending_skin_path = None;
        self.pending_variant = profile.skin_variant;
        self.available_capes = profile.capes;
        self.initial_cape_id = profile.active_cape_id.clone();
        self.pending_cape_id = profile.active_cape_id;
        self.skin_texture_hash = None;
        self.skin_sample = None;
        self.animated_skin_texture_hash = None;
        self.animated_skin_texture = None;
        self.animated_skin_sample = None;
        self.cape_texture_hash = None;
        self.cape_sample = None;
        self.preview_texture = None;
        self.preview_history = None;
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

fn neutral_button_style(ui: &Ui) -> ButtonOptions {
    ButtonOptions {
        min_size: egui::vec2(160.0, style::CONTROL_HEIGHT),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..ButtonOptions::default()
    }
}

fn _absolute_path_string(path: &PathBuf) -> String {
    path.display().to_string()
}
