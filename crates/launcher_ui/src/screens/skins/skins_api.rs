//! Toolkit-free view API for the Skins screen. A [`SkinsSession`] owns the profile state (skin, variant, capes, save
//! and refresh workers) and the preview camera and animation; the front end draws the controls and shows the
//! frame [`SkinsSession::gpu_frame`] describes (drawn by the engine on the GPU).

use std::path::PathBuf;
use std::sync::Arc;

use super::*;

pub use auth::MinecraftSkinVariant as SkinVariant;

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
    /// Geometry hash of each part as last handed out by `gpu_frame`.
    gpu_sent: std::collections::HashMap<super::gpu_scene::PartTag, u64>,
    /// What the geometry in `gpu_sent` was built from; while it is unchanged the parts are not generated again.
    gpu_key: Option<GpuGeometryKey>,
    /// The skin image last hashed, so a frame does not hash it again.
    gpu_skin_hash: Option<(Arc<RgbaImage>, u64)>,
}

#[derive(Clone, PartialEq, Eq)]
struct GpuGeometryKey {
    skin: u64,
    cape: Option<u64>,
    variant: u8,
    layers_3d: bool,
    elytra: bool,
}

/// One part of the character for a GPU renderer; see [`SkinsSession::gpu_frame`].
#[derive(Clone, Debug)]
pub struct GpuPart {
    pub tag: super::gpu_scene::PartTag,
    /// Column-major matrix placing the part in the world this frame.
    pub transform: [f32; 16],
    pub geometry_hash: u64,
    /// The part's quads in its own space; `None` when unchanged since the previous frame.
    pub quads: Option<Vec<super::gpu_scene::SceneQuad>>,
}

#[derive(Clone, Copy, Debug)]
pub struct GpuMotionBlur {
    pub amount: f32,
    pub shutter_frames: f32,
    pub samples: usize,
}

/// The character and camera for one frame of the GPU preview.
#[derive(Clone)]
pub struct GpuFrame {
    pub camera: super::gpu_scene::SceneCamera,
    /// How fast the camera is orbiting (radians a second), for motion blur.
    pub yaw_velocity: f32,
    pub parts: Vec<GpuPart>,
    pub skin: Arc<RgbaImage>,
    pub skin_hash: u64,
    /// The image cape and elytra quads sample (the chosen cape, or the stock elytra texture).
    pub cape: Option<Arc<RgbaImage>>,
    pub cape_hash: Option<u64>,
    pub motion_blur: Option<GpuMotionBlur>,
}

impl Default for SkinsSession {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinsSession {
    pub fn new() -> Self {
        Self { state: SkinManagerState::default(), cape_faces: Default::default(), gamepad_orbit: 0.0, drag_dx: 0.0, gpu_sent: Default::default(), gpu_key: None, gpu_skin_hash: None }
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
        state.apply_preview_settings(settings);
        state.poll_worker();
        state.poll_pick_skin_result();
        state.try_consume_open_refresh();
        state.ensure_skin_texture();
        state.ensure_default_elytra_texture();
        state.ensure_cape_texture();
        state.refresh_in_progress || state.save_in_progress || state.pick_skin_in_progress
    }

    /// Takes the preview options without following an account; [`tick`](Self::tick) does this every frame.
    pub fn set_preview_settings(&mut self, settings: &PreviewSettings) {
        self.state.apply_preview_settings(settings);
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

    /// Advances the camera and the walk animation to `now` and returns the real time since the previous call and the
    /// pose to draw.
    fn animate(&mut self, now: f64) -> (f32, PreviewPose) {
        let dt = self.state.consume_frame_dt(now);
        self.advance_camera(now, dt);
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
        (dt, pose)
    }

    /// The orbit angle and its speed at `now`, for the camera.
    fn orbit(&self, now: f64) -> (f32, f32) {
        let state = &self.state;
        let orbit_time = state.effective_orbit_time(now);
        let yaw = ((orbit_time / PREVIEW_ORBIT_SECONDS) as f32) * std::f32::consts::TAU + state.camera_yaw_offset;
        let yaw_velocity = if state.camera_drag_active {
            state.camera_drag_velocity
        } else if state.orbit_pause_started_at.is_some() {
            state.camera_inertial_velocity
        } else {
            std::f32::consts::TAU / PREVIEW_ORBIT_SECONDS as f32
        };
        (yaw, yaw_velocity)
    }

    /// Advances the preview to `now` and returns the character for a GPU renderer to draw this very frame. `None` while there is no skin.
    ///
    /// Parts come with their matrix every call; their quads only when they differ from what the previous call handed
    /// out (`quads` is `None` otherwise), so a renderer keeps a mesh per part and only moves it.
    pub fn gpu_frame(&mut self, now: f64, settings: &PreviewSettings) -> Option<GpuFrame> {
        let (_dt, pose) = self.animate(now);
        let (yaw, yaw_velocity) = self.orbit(now);
        let state = &self.state;
        let skin = state.skin_sample.clone()?;
        let input = super::gpu_scene::SceneInput {
            pose,
            variant: state.pending_variant,
            layers_3d: settings.layers_3d_enabled,
            show_elytra: state.show_elytra,
            expressions: state.expressions_enabled,
            expression_layout: state.cached_expression_layout,
            skin: Arc::clone(&skin),
            cape: state.cape_sample.clone(),
            default_elytra: state.default_elytra_sample.clone(),
            cape_uv: state.cape_uv,
        };
        let skin_hash = match &self.gpu_skin_hash {
            Some((image, hash)) if Arc::ptr_eq(image, &skin) => *hash,
            _ => {
                let hash = hash_preview_image(&skin);
                self.gpu_skin_hash = Some((Arc::clone(&skin), hash));
                hash
            }
        };
        let cape_hash_of = |image: &Option<Arc<RgbaImage>>| image.as_deref().map(hash_preview_image);
        let key = GpuGeometryKey {
            skin: skin_hash,
            cape: cape_hash_of(&input.cape).or_else(|| cape_hash_of(&input.default_elytra)),
            variant: match input.variant {
                MinecraftSkinVariant::Slim => 1,
                _ => 0,
            },
            layers_3d: input.layers_3d,
            elytra: input.show_elytra,
        };
        // While nothing the geometry depends on changed, the parts the renderer holds are not generated again (the
        // voxel layers cost milliseconds); the face animates, so it is always generated.
        let skip: Vec<super::gpu_scene::PartTag> = if self.gpu_key.as_ref() == Some(&key) {
            self.gpu_sent.keys().copied().filter(|tag| *tag != super::gpu_scene::PartTag::Face).collect()
        } else {
            Vec::new()
        };
        let (scene_parts, cape_sample) = super::gpu_scene::capture_scene_with_cape(&input, &skip);
        let mut parts = Vec::with_capacity(scene_parts.len());
        let mut sent = std::mem::take(&mut self.gpu_sent);
        let mut next = std::collections::HashMap::new();
        for part in scene_parts {
            if part.skipped {
                if let Some(hash) = sent.remove(&part.tag) {
                    next.insert(part.tag, hash);
                    parts.push(GpuPart { tag: part.tag, transform: part.transform, geometry_hash: hash, quads: None });
                    continue;
                }
            }
            let hash = super::gpu_scene::local_geometry_hash(&part);
            let changed = sent.remove(&part.tag) != Some(hash);
            next.insert(part.tag, hash);
            parts.push(GpuPart {
                tag: part.tag,
                transform: part.transform,
                geometry_hash: hash,
                quads: changed.then_some(part.quads),
            });
        }
        self.gpu_sent = next;
        self.gpu_key = Some(key);
        let cape_hash = cape_sample.as_deref().map(hash_preview_image);
        Some(GpuFrame {
            camera: super::gpu_scene::scene_camera(yaw),
            yaw_velocity,
            parts,
            skin,
            skin_hash,
            cape: cape_sample,
            cape_hash,
            motion_blur: state.preview_motion_blur_enabled.then_some(GpuMotionBlur {
                amount: state.preview_motion_blur_amount,
                shutter_frames: state.preview_motion_blur_shutter_frames,
                samples: state.preview_motion_blur_sample_count,
            }),
        })
    }

    /// Shows `skin` as the preview's character without any account or profile behind it. For tools and tests.
    pub fn set_preview_skin(&mut self, skin: RgbaImage, variant: SkinVariant) {
        self.state.skin_sample = Some(Arc::new(skin));
        self.state.pending_variant = variant;
        self.state.ensure_default_elytra_texture();
    }

    /// Puts `cape` on the preview character. For tools and tests.
    pub fn set_preview_cape(&mut self, cape: RgbaImage) {
        self.state.cape_uv = cape_uv_layout([cape.width(), cape.height()]).unwrap_or_else(default_cape_uv_layout);
        self.state.cape_sample = Some(Arc::new(cape));
    }

    /// Forgets which geometry was handed to the renderer, so the next [`gpu_frame`](Self::gpu_frame) sends all of it
    /// (the renderer was recreated, or lost its meshes).
    pub fn gpu_resend_all(&mut self) {
        self.gpu_sent.clear();
        self.gpu_key = None;
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

/// `matrix` (column-major, affine) turned by `radians` about the vertical axis through `pivot`, anticlockwise seen from
/// above (a turn to the left for a character facing away from you).
fn turn_about_vertical_axis(matrix: [f32; 16], pivot: [f32; 3], radians: f32) -> [f32; 16] {
    let (sin, cos) = radians.sin_cos();
    let rotate = |x: f32, y: f32, z: f32| [cos * x + sin * z, y, -sin * x + cos * z];
    let mut out = matrix;
    for column in 0..3 {
        let turned = rotate(matrix[column * 4], matrix[column * 4 + 1], matrix[column * 4 + 2]);
        out[column * 4..column * 4 + 3].copy_from_slice(&turned);
    }
    let offset = rotate(matrix[12] - pivot[0], matrix[13] - pivot[1], matrix[14] - pivot[2]);
    out[12] = pivot[0] + offset[0];
    out[13] = pivot[1] + offset[1];
    out[14] = pivot[2] + offset[2];
    out
}

/// A fingerprint of a skin image, to tell when it changed.
pub fn hash_skin(skin: &RgbaImage) -> u64 {
    hash_preview_image(skin)
}

/// The head of `skin` for a GPU renderer: just the head part (with its second layer), and a camera framing it from the
/// front, a little turned. For small avatars, such as the sign-in button's.
pub fn head_frame(skin: Arc<RgbaImage>) -> Option<GpuFrame> {
    use super::gpu_scene::PartTag;
    let input = super::gpu_scene::SceneInput {
        pose: PreviewPose { time_seconds: 0.0, locomotion_blend: 0.0, limb_swing: 0.0 },
        variant: MinecraftSkinVariant::Classic,
        layers_3d: true,
        show_elytra: false,
        expressions: false,
        expression_layout: None,
        skin: Arc::clone(&skin),
        cape: None,
        default_elytra: None,
        cape_uv: default_cape_uv_layout(),
    };
    // Everything but the head is skipped, so only its geometry is generated.
    let skip = [
        PartTag::Body,
        PartTag::Face,
        PartTag::RightArm,
        PartTag::LeftArm,
        PartTag::RightLeg,
        PartTag::LeftLeg,
        PartTag::Cape,
        PartTag::ElytraLeft,
        PartTag::ElytraRight,
    ];
    let (scene_parts, _) = super::gpu_scene::capture_scene_with_cape(&input, &skip);
    let head = scene_parts.into_iter().find(|part| part.tag == PartTag::Head && !part.skipped)?;

    // The head's centre in the world: the middle of its local bounding box, placed by the part's matrix.
    let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
    for quad in &head.quads {
        for vertex in quad.vertices {
            for axis in 0..3 {
                low[axis] = low[axis].min(vertex[axis]);
                high[axis] = high[axis].max(vertex[axis]);
            }
        }
    }
    let local_centre = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, (low[2] + high[2]) / 2.0];
    let m = head.transform;
    let centre = [
        m[0] * local_centre[0] + m[4] * local_centre[1] + m[8] * local_centre[2] + m[12],
        m[1] * local_centre[0] + m[5] * local_centre[1] + m[9] * local_centre[2] + m[13],
        m[2] * local_centre[0] + m[6] * local_centre[1] + m[10] * local_centre[2] + m[14],
    ];

    // The preview camera at a slight angle gives the direction the character is seen from; keep that direction, but
    // aim at the head and come in until it fills the picture.
    let mut camera = super::gpu_scene::scene_camera(0.5);
    let toward = [camera.eye[0] - camera.target[0], 0.0, camera.eye[2] - camera.target[2]];
    let length = (toward[0] * toward[0] + toward[2] * toward[2]).sqrt().max(1e-6);
    // The head with its second layer is about 10 units across (14 on the diagonal of a three-quarter view).
    let distance = 7.4 / (camera.vertical_fov_radians / 2.0).tan();
    camera.target = centre;
    camera.eye = [
        centre[0] + toward[0] / length * distance,
        centre[1] + distance * 0.12,
        centre[2] + toward[2] / length * distance,
    ];

    // The head is turned a quarter turn to its left (anticlockwise seen from above), about the vertical axis through its
    // centre, so its face looks at the camera.
    let transform = turn_about_vertical_axis(head.transform, centre, std::f32::consts::FRAC_PI_2);

    let skin_hash = hash_preview_image(&skin);
    let geometry_hash = super::gpu_scene::local_geometry_hash(&head);
    Some(GpuFrame {
        camera,
        yaw_velocity: 0.0,
        parts: vec![GpuPart { tag: head.tag, transform, geometry_hash, quads: Some(head.quads) }],
        skin,
        skin_hash,
        cape: None,
        cape_hash: None,
        motion_blur: None,
    })
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
    #[ignore = "timing probe: cargo test -p launcher_ui --release gpu_frame_timing -- --ignored --nocapture"]
    fn gpu_frame_timing() {
        for layers in [false, true] {
            let mut session = SkinsSession::new();
            session.set_preview_skin(RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 80, 255])), SkinVariant::Classic);
            let preview = PreviewSettings { layers_3d_enabled: layers, ..settings() };
            session.set_preview_settings(&preview);
            let _ = session.gpu_frame(0.0, &preview);
            let started = std::time::Instant::now();
            let frames = 200;
            for step in 0..frames {
                let _ = session.gpu_frame(1.0 + f64::from(step) * 0.007, &preview).expect("a skin is set");
            }
            println!("layers {layers}: {:?} per gpu_frame", started.elapsed() / frames);
        }
    }

    #[test]
    fn gpu_frames_hand_out_geometry_once_and_only_move_it_afterwards() {
        let mut session = SkinsSession::new();
        session.set_preview_skin(RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 80, 255])), SkinVariant::Classic);
        let preview = PreviewSettings { layers_3d_enabled: true, ..settings() };
        session.set_preview_settings(&preview);

        let first = session.gpu_frame(1.0, &preview).expect("a skin is set");
        assert!(first.parts.iter().all(|part| part.quads.is_some()), "the first frame carries every part's quads");
        let voxels = first.parts.iter().map(|part| part.quads.as_ref().map_or(0, Vec::len)).sum::<usize>();
        assert!(voxels > 1000, "an opaque skin's voxel layers are thousands of quads, got {voxels}");

        let second = session.gpu_frame(1.3, &preview).expect("a skin is set");
        assert_eq!(first.parts.len(), second.parts.len());
        for (a, b) in first.parts.iter().zip(&second.parts) {
            assert_eq!(a.tag, b.tag);
            assert_eq!(a.geometry_hash, b.geometry_hash, "{:?}", a.tag);
            assert!(b.quads.is_none(), "{:?}'s geometry is not sent again", a.tag);
        }
        assert_eq!(first.skin_hash, second.skin_hash);

        // Something the geometry depends on changed: everything is sent again.
        let flat = PreviewSettings { layers_3d_enabled: false, ..preview };
        let third = session.gpu_frame(1.6, &flat).expect("a skin is set");
        assert!(third.parts.iter().all(|part| part.quads.is_some()));

        // A renderer that lost its meshes asks for everything.
        session.gpu_resend_all();
        let fourth = session.gpu_frame(1.9, &flat).expect("a skin is set");
        assert!(fourth.parts.iter().all(|part| part.quads.is_some()));
    }

    #[test]
    fn a_head_frame_is_only_the_head_looked_at_from_the_front() {
        let skin = Arc::new(RgbaImage::from_pixel(64, 64, image::Rgba([200, 120, 80, 255])));
        let frame = head_frame(skin).expect("a skin gives a head");
        assert_eq!(frame.parts.len(), 1);
        assert_eq!(frame.parts[0].tag, super::super::gpu_scene::PartTag::Head);
        assert!(frame.parts[0].quads.as_ref().is_some_and(|quads| quads.len() >= 6));
        // The camera looks at the head, which is high up, and from a short distance.
        let (eye, target) = (frame.camera.eye, frame.camera.target);
        assert!(target[1] > 20.0, "the head is above the middle of the body, got {target:?}");
        let distance = ((eye[0] - target[0]).powi(2) + (eye[1] - target[1]).powi(2) + (eye[2] - target[2]).powi(2)).sqrt();
        assert!((10.0..40.0).contains(&distance), "close to the head, got {distance}");
    }

    #[test]
    fn no_frame_without_a_skin() {
        let mut session = SkinsSession::new();
        assert!(session.gpu_frame(0.0, &settings()).is_none());
    }
}

