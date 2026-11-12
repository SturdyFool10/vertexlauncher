//! The "Random" cape tile's picture: a cape's face seen through a negative filter wherever the dice icon is drawn.
//!
//! Everything here is made once and then reused, so showing a different cape every second costs no GPU memory:
//!
//! * one offscreen target, drawn into again each time the cape changes;
//! * one texture asset for the cape, whose pixels are replaced in place (see `sync_texture`);
//! * one SVG asset for the dice, rasterised by the engine, whose alpha is the mask;
//! * one model and entity: a quad that exactly fills the target.
//!
//! The shader (`cape_dice_invert.slang`) shows `lerp(cape, 1 - cape, mask)`.

use std::path::PathBuf;
use std::sync::Arc;

use image::RgbaImage;
use sturdy::assets::{Asset, ModelBuilder, ModelPrimitive, ModelVertex, RendererTextureHandle};
use sturdy::camera::Camera;
use sturdy::glam::{Mat4, UVec2, Vec2, Vec3, Vec4};
use sturdy::render::{OffscreenTargetDesc, OffscreenTargetHandle, RenderGraphSettings, ToneMappingOperator};
use sturdy::scene::{ModelRenderer, WorldTransform};
use sturdy::{Engine, Entity, FrameDescription, SceneLighting};

use crate::renderer::{TextureSlot, sync_texture, write_if_changed};

const SHADER_SOURCE: &str = include_str!("cape_dice_invert.slang");
/// The engine layer the picture's quad lives on; only its camera sees it.
const DICE_LAYER: u32 = 1 << 28;
/// A cape face is 10 by 16 texels; the target is 14 pixels per texel so the dice's strokes stay clean at the 64 x 100 the
/// tile shows it at (the UI scales it down).
const TARGET_SIZE: UVec2 = UVec2::new(140, 224);
/// How large the dice is rasterised, in pixels along its longest side.
const MASK_PX: f32 = 160.0;
/// The mask's raster is made by the engine; draw every frame for this many frames after it was asked for, so the picture
/// shows the dice as soon as the raster is there even when the cape has not changed since.
const SETTLE_FRAMES: u32 = 45;

struct Quad {
    entity: Entity,
    model: Asset,
}

/// The picture. Draw it with [`CapeDice::render`] from the UI thread and show the texture it returns with `Ui::image`.
#[derive(Default)]
pub struct CapeDice {
    shader: Option<Asset>,
    mask: Option<Asset>,
    cape: Option<TextureSlot>,
    quad: Option<Quad>,
    target: Option<OffscreenTargetHandle>,
    /// The engine's device generation everything above was made for.
    generation: Option<u64>,
    /// What the target currently shows.
    drawn: Option<u64>,
    settle: u32,
}

impl CapeDice {
    pub fn new() -> Self {
        Self::default()
    }

    /// The picture of `cape` (a cape face as RGBA) through the dice. `key` names the picture: it is drawn again only when
    /// the key changes (or for a moment after everything is first made). `None` while it cannot be made.
    pub fn render(
        &mut self,
        engine: &mut Engine<'_>,
        key: u64,
        cape: &Arc<RgbaImage>,
    ) -> Result<Option<RendererTextureHandle>, sturdy::Error> {
        // After the graphics device changed nothing made before is trusted: it is all made again on the new device.
        let generation = engine.device_generation();
        if self.generation.is_some_and(|previous| previous != generation) {
            self.release(engine);
        }
        self.generation = Some(generation);

        let shader = match self.shader {
            Some(shader) => shader,
            None => {
                let shader = load_shader(engine)?;
                self.shader = Some(shader);
                shader
            }
        };
        if self.mask.is_none() {
            self.mask = Some(load_mask(engine)?);
            self.settle = SETTLE_FRAMES;
        }
        let mask = self.mask.expect("set above");
        let target = match self.target {
            Some(target) => target,
            None => {
                let target = engine.create_offscreen_target(&OffscreenTargetDesc::new(TARGET_SIZE).label("cape dice"))?;
                self.target = Some(target);
                target
            }
        };
        // Same asset, new pixels: the model keeps working and no texture is made.
        sync_texture(engine, &mut self.cape, Some((cape, key)), "cape dice cape")?;
        if self.quad.is_none() {
            let Some(cape_slot) = self.cape.as_ref() else {
                return Ok(None);
            };
            self.quad = Some(build_quad(engine, shader, cape_slot.asset, mask)?);
        }

        if self.drawn != Some(key) || self.settle > 0 {
            engine.render_offscreen_view(target, &frame_description())?;
            self.drawn = Some(key);
            self.settle = self.settle.saturating_sub(1);
        }
        let raw = engine.offscreen_target_texture(target);
        Ok((raw != 0).then(|| RendererTextureHandle::from_raw(raw)))
    }

    /// Releases everything the picture made.
    pub fn release(&mut self, engine: &mut Engine<'_>) {
        if let Some(quad) = self.quad.take() {
            let _ = engine.ecs().despawn(quad.entity);
            let _ = engine.assets().unload(quad.model);
        }
        if let Some(slot) = self.cape.take() {
            let _ = engine.assets().unload(slot.asset);
        }
        if let Some(mask) = self.mask.take() {
            let _ = engine.assets().unload(mask);
        }
        if let Some(shader) = self.shader.take() {
            let _ = engine.assets().unload(shader);
        }
        if let Some(target) = self.target.take() {
            engine.destroy_offscreen_target(target);
        }
        self.drawn = None;
        self.settle = 0;
    }
}

/// A quad in the XY plane that exactly fills the target when seen from `CAMERA_DISTANCE` along Z.
const CAMERA_DISTANCE: f32 = 4.0;
const CAMERA_FOV_DEGREES: f32 = 30.0;

fn build_quad(engine: &mut Engine<'_>, shader: Asset, cape: Asset, mask: Asset) -> Result<Quad, sturdy::Error> {
    let aspect = TARGET_SIZE.x as f32 / TARGET_SIZE.y as f32;
    let half_height = CAMERA_DISTANCE * (CAMERA_FOV_DEGREES.to_radians() * 0.5).tan();
    let half_width = half_height * aspect;
    let corner = |x: f32, y: f32, u: f32, v: f32| ModelVertex {
        position: Vec3::new(x, y, 0.0),
        normal: Vec3::Z,
        uv: Vec2::new(u, v),
        color: Vec4::ONE,
        tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
    };
    let vertices = vec![
        corner(-half_width, half_height, 0.0, 0.0),
        corner(half_width, half_height, 1.0, 0.0),
        corner(half_width, -half_height, 1.0, 1.0),
        corner(-half_width, -half_height, 0.0, 1.0),
    ];
    let primitive = ModelPrimitive::new(vertices, vec![0, 1, 2, 0, 2, 3], shader)
        .with_texture("cape_texture", cape)
        .with_texture("mask_texture", mask)
        .with_double_sided(true);
    let model = engine
        .assets()
        .create_model(ModelBuilder::new("cape dice").with_primitive(primitive))
        .map_err(|err| sturdy::Error::new(format!("{err:?}")))?;
    let _ = engine.assets().set_model_float(model, 0, "mask_scale_y", TARGET_SIZE.y as f32 / TARGET_SIZE.x as f32);
    let _ = engine.assets().set_model_float(model, 0, "alpha_cutoff", 0.5);
    let entity = engine
        .ecs()
        .spawn_builder()
        .with(WorldTransform::from_matrix(Mat4::IDENTITY))
        .with(ModelRenderer::new(model).with_visibility_mask(DICE_LAYER))
        .build()
        .map_err(|err| sturdy::Error::new(format!("{err:?}")))?;
    Ok(Quad { entity, model })
}

fn frame_description() -> FrameDescription {
    let aspect = TARGET_SIZE.x as f32 / TARGET_SIZE.y as f32;
    let mut camera = Camera::perspective().fov_degrees(CAMERA_FOV_DEGREES).at(Vec3::new(0.0, 0.0, CAMERA_DISTANCE));
    camera.near_clip = 0.1;
    camera.far_clip = 50.0;
    camera.aspect_ratio = Some(aspect);
    camera.culling_mask = DICE_LAYER;
    camera.clear_color = Vec4::ZERO;
    let mut rig = camera.rig(aspect);
    rig.look_at(Vec3::ZERO, Vec3::Y);
    let camera = rig.camera();

    // The same plain graph as the skin preview: transparent wherever the cape is not, nothing but the surface itself.
    let mut graph = RenderGraphSettings::default();
    graph.atmosphere.enabled = false;
    graph.scene.background_color = Some(Vec4::ZERO);
    graph.scene.preserve_alpha = true;
    graph.shadows.enabled = false;
    graph.ambient_occlusion.enabled = false;
    graph.bloom.enabled = false;
    graph.tone_mapping.enabled = true;
    graph.tone_mapping.operation = ToneMappingOperator::PsychoV;
    graph.restir_gi.enabled = false;
    FrameDescription::new(camera)
        .label("cape dice")
        .lighting(SceneLighting::default().ambient_radiance(Vec3::ZERO).exposure(1.0))
        .render_graph(graph)
}

/// Where a file may live, in the order they are tried: the launcher's cache, then the system's temporary folder.
fn cache_paths(folder: &str, file: &str) -> Vec<PathBuf> {
    vec![
        app_paths::cache_root().join(folder).join(file),
        std::env::temp_dir().join("vertexlauncher-cache").join(folder).join(file),
    ]
}

fn load_shader(engine: &mut Engine<'_>) -> Result<Asset, sturdy::Error> {
    let mut problems = Vec::new();
    for path in cache_paths("shaders", "cape_dice_invert.slang") {
        if let Err(err) = write_if_changed(&path, SHADER_SOURCE) {
            problems.push(format!("writing '{}': {err}", path.display()));
            continue;
        }
        match engine.assets().load_shader_with(
            path.to_string_lossy(),
            "cape dice invert",
            "cape_dice_invert",
            "vertexMain",
            "fragmentMain",
            "depthOnlyMain",
            &[],
        ) {
            Ok(shader) => return Ok(shader),
            Err(err) => problems.push(format!("loading '{}': {err:?}", path.display())),
        }
    }
    Err(sturdy::Error::new(format!("the cape dice shader could not be prepared: {}", problems.join("; "))))
}

/// The dice icon, white on transparent, written next to the launcher's caches and rasterised by the engine.
fn load_mask(engine: &mut Engine<'_>) -> Result<Asset, sturdy::Error> {
    let white = String::from_utf8_lossy(launcher_ui::assets::DICE_SVG).replace("currentColor", "#ffffff");
    let mut problems = Vec::new();
    for path in cache_paths("icons", "cape_dice_mask.svg") {
        if let Err(err) = write_if_changed(&path, &white) {
            problems.push(format!("writing '{}': {err}", path.display()));
            continue;
        }
        match engine.assets().load_svg(path.to_string_lossy(), MASK_PX, "cape dice mask") {
            Ok(asset) => return Ok(asset),
            Err(err) => problems.push(format!("loading '{}': {err:?}", path.display())),
        }
    }
    Err(sturdy::Error::new(format!("the dice mask could not be prepared: {}", problems.join("; "))))
}
