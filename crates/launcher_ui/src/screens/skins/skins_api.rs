//! Toolkit-free view API for the Skins screen. A [`SkinsSession`] owns the profile state (skin, variant, capes, save
//! and refresh workers) and the preview camera and animation; the front end draws the controls and shows the
//! image [`SkinsSession::render_preview`] returns.

use std::path::PathBuf;
use std::sync::Arc;

use super::skins_raster::{RasterSettings, render_scenes_async};
use super::*;

pub use auth::MinecraftSkinVariant as SkinVariant;
pub use super::skins_raster::RasterSettings as PreviewRaster;

/// Preview options taken from the launcher settings each frame.
#[derive(Clone, Copy, Debug)]
pub struct PreviewSettings {
    pub aa_mode: SkinPreviewAaMode,
    pub msaa_samples: u32,
    pub texel_aa_mode: SkinPreviewTexelAaMode,
    pub motion_blur_enabled: bool,
    pub motion_blur_amount: f32,
    pub motion_blur_shutter_frames: f32,
    pub motion_blur_sample_count: i32,
    pub layers_3d_enabled: bool,
    pub expressions_enabled: bool,
}

/// A cape tile: the outer face of the cape, enlarged with nearest sampling, as PNG bytes.
#[derive(Clone)]
pub struct CapeView {
    pub id: String,
    pub label: String,
    pub face_png: Option<Arc<[u8]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionMode {
    Idle,
    Walk,
}

pub struct SkinsSession {
    state: SkinManagerState,
    cape_faces: std::collections::HashMap<String, Option<Arc<[u8]>>>,
    gamepad_orbit: f32,
    drag_dx: f32,
    raster: RasterPipe,
}

/// A frame to draw off the UI thread.
struct RasterJob {
    scenes: Vec<(f32, Vec<RenderTriangle>)>,
    skin: Arc<RgbaImage>,
    cape: Option<Arc<RgbaImage>>,
    settings: RasterSettings,
}

/// Draws the preview on the compute backend (the UI engine's task scheduler), one frame at a time, and hands the UI
/// each finished frame once.
///
/// The UI thread only advances the camera and the animation (every frame, by the real time since the last one) and
/// builds the scene; the rasteriser, whose cost varies with the window size, anti-aliasing and motion blur, runs beside
/// it, split into bands across every core. So a slow frame makes the preview update less often but never stalls the
/// rest of the UI, and the poses it draws are still spaced by real time. While a frame is being drawn the UI keeps
/// showing the last finished one and does not queue another, so there is no backlog and the preview never falls behind
/// the animation.
struct RasterPipe {
    last: Option<Arc<RgbaImage>>,
    /// Bumped for every finished frame, so the UI uploads each one once.
    generation: u64,
    pending: Option<launcher_runtime::ComputeHandle<RgbaImage>>,
}

impl RasterPipe {
    fn new() -> Self {
        Self { last: None, generation: 0, pending: None }
    }

    /// Takes the frame being drawn if it has finished. True when nothing is being drawn, so the next can start.
    fn poll(&mut self) -> bool {
        let Some(pending) = self.pending.as_mut() else {
            return true;
        };
        match pending.try_take() {
            None => false,
            Some(Ok(image)) => {
                self.pending = None;
                self.last = Some(Arc::new(image));
                self.generation += 1;
                true
            }
            Some(Err(err)) => {
                tracing::error!(target: "vertexlauncher/skins", "The skin preview frame failed: {err}");
                self.pending = None;
                true
            }
        }
    }

    /// Starts drawing `job`; the result shows up through a later `poll`.
    fn submit(&mut self, job: RasterJob) {
        let RasterJob { scenes, skin, cape, settings } = job;
        self.pending = Some(launcher_runtime::spawn_compute(render_scenes_async(Arc::new(scenes), skin, cape, settings)));
    }
}

impl Default for SkinsSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinsSession {
    pub fn new() -> Self {
        Self { state: SkinManagerState::default(), cape_faces: Default::default(), gamepad_orbit: 0.0, drag_dx: 0.0, raster: RasterPipe::new() }
    }

    /// Follows the active account and the background workers; call once per frame while the screen is showing.
    /// `opened` is true on the frame the screen was entered. Returns whether work is in flight.
    pub fn tick(
        &mut self,
        auth: Option<&LaunchAuthContext>,
        opened: bool,
        settings: &PreviewSettings,
    ) -> bool {
        let state = &mut self.state;
        state.sync_active_account(auth);
        if opened {
            state.refresh_on_open_pending = true;
        }
        state.preview_msaa_samples = settings.msaa_samples.max(1);
        state.preview_aa_mode = settings.aa_mode;
        state.preview_texel_aa_mode = settings.texel_aa_mode;
        state.preview_motion_blur_enabled = settings.motion_blur_enabled;
        state.preview_motion_blur_amount = settings.motion_blur_amount.clamp(0.0, 1.0);
        state.preview_motion_blur_shutter_frames = settings.motion_blur_shutter_frames.max(0.0);
        state.preview_motion_blur_sample_count = settings.motion_blur_sample_count.max(1) as usize;
        state.preview_3d_layers_enabled = settings.layers_3d_enabled;
        state.expressions_enabled = settings.expressions_enabled;
        state.poll_worker();
        state.poll_pick_skin_result();
        state.try_consume_open_refresh();
        state.ensure_skin_texture();
        state.ensure_default_elytra_texture();
        state.ensure_cape_texture();
        state.refresh_in_progress || state.save_in_progress || state.pick_skin_in_progress
    }

    /// The active account's profile id and name, or `None` when nobody is signed in.
    pub fn account(&self) -> Option<(&str, &str)> {
        Some((self.state.active_profile_id.as_deref().unwrap_or(""), self.state.active_player_name.as_deref()?))
    }

    // -- Skin image -----------------------------------------------------------------------------------------------

    pub fn picking(&self) -> bool {
        self.state.pick_skin_in_progress
    }

    pub fn pending_skin_path(&self) -> Option<&std::path::Path> {
        self.state.pending_skin_path.as_deref()
    }

    pub fn load_skin_path(&mut self, path: PathBuf) {
        self.state.begin_loading_skin_from_path(path);
    }

    pub fn load_skin_bytes(&mut self, name: PathBuf, bytes: Vec<u8>) {
        self.state.begin_loading_skin_from_bytes(name, bytes);
    }

    // -- Model and cape -------------------------------------------------------------------------------------------

    pub fn variant(&self) -> SkinVariant {
        self.state.pending_variant
    }

    pub fn set_variant(&mut self, variant: SkinVariant) {
        self.state.pending_variant = variant;
    }

    pub fn capes(&mut self) -> Vec<CapeView> {
        let faces = &mut self.cape_faces;
        self.state
            .available_capes
            .iter()
            .map(|cape| {
                // Only a finished picture is remembered: a cape whose texture has not arrived yet is tried again.
                let cached = faces.get(&cape.id).cloned().flatten();
                let face_png = cached.or_else(|| {
                    let made = cape.texture_bytes.as_deref().and_then(cape_face_png);
                    if made.is_some() {
                        faces.insert(cape.id.clone(), made.clone());
                    }
                    made
                });
                CapeView { id: cape.id.clone(), label: cape.label.clone(), face_png }
            })
            .collect()
    }

    pub fn selected_cape(&self) -> Option<&str> {
        self.state.pending_cape_id.as_deref()
    }

    pub fn select_cape(&mut self, id: Option<String>) {
        self.state.pending_cape_id = id;
    }

    // -- Saving -----------------------------------------------------------------------------------------------------

    pub fn can_save(&self) -> bool {
        self.state.can_save() && !self.state.save_in_progress
    }

    pub fn saving(&self) -> bool {
        self.state.save_in_progress
    }

    pub fn save(&mut self) {
        self.state.start_save();
    }

    // -- Preview ------------------------------------------------------------------------------------------------------

    pub fn has_skin(&self) -> bool {
        self.state.skin_sample.is_some()
    }

    pub fn show_elytra(&self) -> bool {
        self.state.show_elytra
    }

    pub fn toggle_elytra(&mut self) {
        self.state.show_elytra = !self.state.show_elytra;
    }

    pub fn motion_mode(&self) -> MotionMode {
        match self.state.preview_motion_mode {
            PreviewMotionMode::Idle => MotionMode::Idle,
            PreviewMotionMode::Walk => MotionMode::Walk,
        }
    }

    pub fn toggle_motion_mode(&mut self) {
        self.state.preview_motion_mode = match self.state.preview_motion_mode {
            PreviewMotionMode::Idle => PreviewMotionMode::Walk,
            PreviewMotionMode::Walk => PreviewMotionMode::Idle,
        };
    }

    /// The pointer went down on the preview.
    pub fn drag_started(&mut self, now: f64) {
        self.state.begin_manual_camera_control(now);
        self.state.camera_drag_active = true;
        self.state.camera_drag_velocity = 0.0;
        self.state.camera_inertial_velocity = 0.0;
        self.drag_dx = 0.0;
    }

    /// The pointer moved by `dx` points while dragging.
    pub fn dragged(&mut self, dx: f32) {
        if self.state.camera_drag_active {
            self.drag_dx += dx;
        }
    }

    /// The pointer was released; the model keeps turning with the speed of the drag, then eases to a stop.
    pub fn drag_stopped(&mut self, now: f64) {
        let state = &mut self.state;
        if !state.camera_drag_active {
            return;
        }
        state.camera_drag_active = false;
        state.camera_inertial_velocity = state.camera_drag_velocity;
        if state.camera_inertial_velocity.abs() < CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC {
            state.camera_inertial_velocity = 0.0;
            state.finish_manual_camera_control(now);
        }
    }

    /// Right-stick orbit input in -1..1.
    pub fn set_gamepad_orbit(&mut self, input: f32) {
        self.gamepad_orbit = input.clamp(-1.0, 1.0);
    }

    /// Advances the camera and the walk animation to `now` (seconds) and draws a `width` x `height` frame, or
    /// `None` while there is no skin to show. The preview animates continuously, so keep drawing while it shows.
    pub fn render_preview(&mut self, width: u32, height: u32, now: f64, settings: &PreviewSettings) -> Option<Arc<RgbaImage>> {
        let dt = self.state.consume_frame_dt(now);
        self.advance_camera(now, dt);
        let worker_free = self.raster.poll();
        let state = &mut self.state;

        let blend_target = match state.preview_motion_mode {
            PreviewMotionMode::Idle => 0.0,
            PreviewMotionMode::Walk => 1.0,
        };
        let blend_speed = if blend_target > state.preview_motion_blend { 5.4 } else { 4.2 };
        let blend_alpha = 1.0 - (-blend_speed * dt.max(0.0)).exp();
        state.preview_motion_blend += (blend_target - state.preview_motion_blend) * blend_alpha;
        state.preview_motion_blend = state.preview_motion_blend.clamp(0.0, 1.0);
        // Like Minecraft's walk animation, the swing position only advances while moving, so the idle stance keeps
        // whatever stride phase the walk ended on.
        state.preview_limb_swing += dt.clamp(0.0, 0.1) * 20.0 * PREVIEW_WALK_LIMB_SPEED * state.preview_motion_blend;
        let pose = PreviewPose {
            time_seconds: now as f32,
            locomotion_blend: state.preview_motion_blend,
            limb_swing: state.preview_limb_swing,
        };
        state.refresh_expression_layout_cache();

        let skin = state.skin_sample.clone()?;
        if !worker_free {
            // The previous frame is still being drawn: the animation above has advanced, and the next frame
            // is built from it as soon as the worker is free.
            return self.raster.last.clone();
        }
        let orbit_time = state.effective_orbit_time(now);
        let yaw = ((orbit_time / PREVIEW_ORBIT_SECONDS) as f32) * std::f32::consts::TAU + state.camera_yaw_offset;
        let yaw_velocity = if state.camera_drag_active {
            state.camera_drag_velocity
        } else if state.orbit_pause_started_at.is_some() {
            state.camera_inertial_velocity
        } else {
            std::f32::consts::TAU / PREVIEW_ORBIT_SECONDS as f32
        };

        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width as f32, height as f32));
        let cape_sample = state.cape_sample.clone();
        let elytra = state.default_elytra_sample.clone();
        let variant = state.pending_variant;
        let expressions = state.expressions_enabled;
        let layout = state.cached_expression_layout;
        let layers = settings.layers_3d_enabled;

        let blur = if state.preview_motion_blur_enabled {
            build_motion_blur_scene_samples(
                rect,
                state.cape_uv,
                yaw,
                yaw_velocity,
                pose,
                state.preview_motion_blur_shutter_frames,
                state.preview_motion_blur_sample_count,
                variant,
                layers,
                state.show_elytra,
                expressions,
                layout,
                Some(Arc::clone(&skin)),
                cape_sample.clone(),
                elytra.clone(),
                state.preview_motion_blur_amount,
            )
        } else {
            Vec::new()
        };
        let raster = RasterSettings::for_modes(width, height, settings.aa_mode, settings.msaa_samples, settings.texel_aa_mode);
        if !blur.is_empty() {
            // The cape texture is whatever the first sample resolved; samples differ only in pose.
            let cape = build_character_scene(
                rect,
                state.cape_uv,
                yaw,
                pose,
                variant,
                layers,
                state.show_elytra,
                expressions,
                layout,
                Some(Arc::clone(&skin)),
                cape_sample.clone(),
                elytra.clone(),
            )
            .cape_render_sample;
            let scenes = blur.into_iter().map(|s| (s.weight, s.triangles)).collect();
            self.raster.submit(RasterJob { scenes, skin, cape, settings: raster });
            return self.raster.last.clone();
        }
        let scene = build_character_scene(
            rect,
            state.cape_uv,
            yaw,
            pose,
            variant,
            layers,
            state.show_elytra,
            expressions,
            layout,
            Some(Arc::clone(&skin)),
            cape_sample,
            elytra,
        );
        let scenes = vec![(1.0, scene.triangles)];
        let cape = scene.cape_render_sample;
        self.raster.submit(RasterJob { scenes, skin, cape, settings: raster });
        self.raster.last.clone()
    }

    /// Changes whenever `render_preview` has a new frame to show.
    pub fn preview_generation(&self) -> u64 {
        self.raster.generation
    }

    fn advance_camera(&mut self, now: f64, dt: f32) {
        let state = &mut self.state;
        if state.camera_drag_active {
            let yaw_step = std::mem::take(&mut self.drag_dx) * CAMERA_DRAG_SENSITIVITY_RAD_PER_POINT;
            state.camera_yaw_offset += yaw_step;
            // The speed the model is being turned at, which is what it keeps turning at once let go. While the pointer
            // moves it follows the speed of each step, lightly smoothed (a single step is noisy: the pointer moves in
            // whole pixels and frames are uneven) but quick enough that a flick is thrown at the strength it was made
            // with. Frames where the pointer did not move do not pull the speed down, since a hand often stops
            // reporting for a frame as it lets go; only a pointer held still for a moment lets the speed die away,
            // so letting go after a pause does not fling the model with a stale one.
            if dt > 0.000_1 {
                if yaw_step.abs() > 0.0 {
                    state.camera_drag_idle_secs = 0.0;
                    let blend = 1.0 - (-dt / 0.02).exp();
                    state.camera_drag_velocity += (yaw_step / dt - state.camera_drag_velocity) * blend;
                } else {
                    state.camera_drag_idle_secs += dt;
                    if state.camera_drag_idle_secs > 0.12 {
                        state.camera_drag_velocity *= (-dt / 0.05).exp();
                    }
                }
            }
        }
        if !state.camera_drag_active && state.orbit_pause_started_at.is_some() {
            state.camera_yaw_offset += state.camera_inertial_velocity * dt;
            if dt > 0.0 {
                state.camera_inertial_velocity *= (-CAMERA_INERTIA_FRICTION_PER_SEC * dt).exp();
            }
            if state.camera_inertial_velocity.abs() < CAMERA_INERTIA_STOP_THRESHOLD_RAD_PER_SEC {
                state.camera_inertial_velocity = 0.0;
                state.finish_manual_camera_control(now);
            }
        }
        if !state.camera_drag_active {
            let input = self.gamepad_orbit;
            let strength = if input.abs() > GAMEPAD_ORBIT_DEADZONE {
                ((input.abs() - GAMEPAD_ORBIT_DEADZONE) / (1.0 - GAMEPAD_ORBIT_DEADZONE)).clamp(0.0, 1.0) * input.signum()
            } else {
                0.0
            };
            if strength.abs() > 0.0 {
                state.begin_manual_camera_control(now);
                state.camera_inertial_velocity = strength * GAMEPAD_ORBIT_MAX_RAD_PER_SEC;
                state.camera_yaw_offset += state.camera_inertial_velocity * dt;
            }
        }
    }
}

/// The outer face of a cape texture, cut out and enlarged with nearest sampling so a tile shows crisp pixels.
fn cape_face_png(bytes: &[u8]) -> Option<Arc<[u8]>> {
    let image = decode_generic_rgba(bytes)?;
    if image.width() < 22 || image.height() < 17 {
        return None;
    }
    // The layout is written for a 64x32 cape; higher-resolution capes are the same picture scaled up.
    let unit = (image.width() / 64).max(1);
    let (x0, y0, w, h) = (unit, unit, 10 * unit, 16 * unit);
    if x0 + w > image.width() || y0 + h > image.height() {
        return None;
    }
    let face = image::imageops::crop_imm(&image, x0, y0, w, h).to_image();
    let scale = (112 / face.height().max(1)).clamp(1, 16);
    let big = image::imageops::resize(&face, face.width() * scale, face.height() * scale, FilterType::Nearest);
    let mut out = std::io::Cursor::new(Vec::new());
    big.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(Arc::from(out.into_inner().into_boxed_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> PreviewSettings {
        PreviewSettings {
            aa_mode: SkinPreviewAaMode::Msaa,
            msaa_samples: 4,
            texel_aa_mode: SkinPreviewTexelAaMode::Off,
            motion_blur_enabled: false,
            motion_blur_amount: 0.15,
            motion_blur_shutter_frames: 0.75,
            motion_blur_sample_count: 5,
            layers_3d_enabled: false,
            expressions_enabled: false,
        }
    }

    #[test]
    fn a_skin_renders_a_character_in_the_middle_of_the_frame() {
        let mut session = SkinsSession::new();
        session.state.skin_sample = Some(Arc::new(RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 80, 255]))));
        session.state.ensure_default_elytra_texture();
        // The frame is drawn on a worker thread, so it arrives on a later call.
        let mut frame = None;
        for step in 0..500 {
            frame = session.render_preview(240, 280, 1.0 + f64::from(step) * 0.016, &settings());
            if frame.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let frame = frame.expect("a skin is loaded");
        assert_eq!((frame.width(), frame.height()), (240, 280));
        assert!(frame.get_pixel(120, 140).0[3] > 0, "the body is opaque in the middle");
        assert_eq!(frame.get_pixel(2, 2).0[3], 0, "the corner is empty");
    }

    #[test]
    #[ignore = "timing probe: cargo test -p launcher_ui --release timing -- --ignored --nocapture"]
    fn timing_probe() {
        for samples in [1u32, 4, 8] {
            let mut session = SkinsSession::new();
            session.state.skin_sample = Some(Arc::new(RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 80, 255]))));
            session.state.ensure_default_elytra_texture();
            let mut preview = PreviewSettings { msaa_samples: samples, layers_3d_enabled: true, expressions_enabled: true, ..settings() };
            preview.aa_mode = SkinPreviewAaMode::Msaa;
            session.state.pending_variant = session.state.pending_variant;
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 460.0));
            let started = std::time::Instant::now();
            let scene = build_character_scene(rect, session.state.cape_uv, 0.5, PreviewPose { time_seconds: 1.0, locomotion_blend: 0.0, limb_swing: 0.0 }, session.state.pending_variant, true, false, true, session.state.cached_expression_layout, session.state.skin_sample.clone(), None, None);
            let built = started.elapsed();
            let raster = RasterSettings::for_modes(1000, 460, preview.aa_mode, preview.msaa_samples, preview.texel_aa_mode);
            let started = std::time::Instant::now();
            let skin = session.state.skin_sample.clone().unwrap();
            let _ = super::super::skins_raster::render_scenes(&[(1.0, scene.triangles.as_slice())], &skin, None, raster);
            println!("msaa {samples}: {} tris, scene {built:?}, raster {:?}", scene.triangles.len(), started.elapsed());
        }
    }

    #[test]
    fn nothing_renders_without_a_skin() {
        let mut session = SkinsSession::new();
        assert!(session.render_preview(64, 64, 0.0, &settings()).is_none());
    }
}
