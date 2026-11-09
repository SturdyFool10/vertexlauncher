use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use config::SkinPreviewAaMode;
use image::RgbaImage;
use launcher_ui::screens::skins_api::GpuFrame;
use launcher_ui::screens::skins_gpu_scene::{PartTag, SceneQuad, SceneTexture};
use sturdy::assets::{
    Asset, ModelBuilder, ModelPrimitive, ModelVertex, RendererTextureHandle, TextureColorSpace, TextureKind,
    TexturePixelFormat,
};
use sturdy::camera::Camera;
use sturdy::glam::{Mat4, UVec2, Vec3, Vec4};
use sturdy::render::{OffscreenTargetDesc, OffscreenTargetHandle, PostProcessAntiAliasing, RenderGraphSettings};
use sturdy::scene::{ModelRenderer, WorldTransform};
use sturdy::{Engine, Entity, FrameDescription, SceneLighting};

use super::SHADER_SOURCE;

/// The layer the skins screen's preview lives on; only its camera sees it.
const PREVIEW_LAYER: u32 = 1 << 30;
/// The layers small head avatars can live on, one each, apart from the preview's so none draws another.
pub const HEAD_LAYERS: std::ops::RangeInclusive<u32> = 8..=27;
/// Texels with less alpha than this are not drawn.
const ALPHA_CUTOFF: f32 = 0.5;

/// Anti-aliasing for one frame; taken from the launcher's settings.
#[derive(Clone, Copy, Debug)]
pub struct RenderSettings {
    pub aa_mode: SkinPreviewAaMode,
    pub msaa_samples: u32,
    /// Smooth the seams between skin texels (the "texel boundary" anti-aliasing setting).
    pub texel_boundary_aa: bool,
}

/// One frame to draw.
pub struct RenderRequest<'a> {
    pub frame: &'a GpuFrame,
    /// Size of the picture in pixels.
    pub size: UVec2,
    pub settings: RenderSettings,
}

struct Instance {
    entity: Entity,
    model: Asset,
    geometry_hash: u64,
    /// The texel anti-aliasing value last written to the model's material.
    texel_aa: bool,
}

struct TextureSlot {
    asset: Asset,
    hash: u64,
}

/// Owns the preview's engine objects: the target it is drawn into, the shader, the textures and one model/entity per
/// part of the character.
pub struct PreviewRenderer {
    shader: Asset,
    target: Option<(OffscreenTargetHandle, UVec2)>,
    skin: Option<TextureSlot>,
    cape: Option<TextureSlot>,
    parts: HashMap<PartTag, Instance>,
    layer: u32,
}

impl PreviewRenderer {
    /// Creates the shader. Cheap; the target and textures are made on the first [`render`](Self::render).
    pub fn new(engine: &mut Engine<'_>) -> Result<Self, sturdy::Error> {
        Self::on_layer(engine, PREVIEW_LAYER)
    }

    fn on_layer(engine: &mut Engine<'_>, layer: u32) -> Result<Self, sturdy::Error> {
        let path = shader_path();
        write_if_changed(&path, SHADER_SOURCE).map_err(|err| sturdy::Error::new(format!("writing the preview shader: {err}")))?;
        let shader = engine
            .assets()
            .load_shader_with(
                path.to_string_lossy(),
                "skin preview flat",
                "skin_preview_flat",
                "vertexMain",
                "fragmentMain",
                "depthOnlyMain",
                &[],
            )
            .map_err(|err| sturdy::Error::new(format!("loading the preview shader: {err:?}")))?;
        Ok(Self { shader, target: None, skin: None, cape: None, parts: HashMap::new(), layer })
    }

    /// The picture, valid after the first successful [`render`](Self::render), and its size.
    pub fn texture(&self, engine: &mut Engine<'_>) -> Option<(RendererTextureHandle, UVec2)> {
        let (target, size) = self.target?;
        Some((RendererTextureHandle::from_raw(engine.offscreen_target_texture(target)), size))
    }

    /// Draws `request` into the target. Returns `true` when the renderer is missing geometry the frame did not carry
    /// (it was created after the geometry was last sent): ask the session for everything again.
    pub fn render(&mut self, engine: &mut Engine<'_>, request: &RenderRequest<'_>) -> Result<bool, sturdy::Error> {
        let size = UVec2::new(request.size.x.max(16), request.size.y.max(16));
        self.ensure_target(engine, size)?;
        self.sync_textures(engine, request.frame)?;
        let missing = self.sync_parts(engine, request.frame, request.settings.texel_boundary_aa);

        let Some((target, _)) = self.target else {
            return Ok(missing);
        };
        let view = frame_description(request, size, self.layer);
        engine.render_offscreen_view(target, &view)?;
        Ok(missing)
    }

    fn ensure_target(&mut self, engine: &mut Engine<'_>, size: UVec2) -> Result<(), sturdy::Error> {
        match self.target {
            Some((_, current)) if current == size => {}
            Some((target, _)) => {
                engine.resize_offscreen_target(target, size)?;
                self.target = Some((target, size));
            }
            None => {
                let target = engine.create_offscreen_target(&OffscreenTargetDesc::new(size).label("skin preview"))?;
                self.target = Some((target, size));
            }
        }
        Ok(())
    }

    /// Makes the skin and cape textures hold the frame's images, replacing their pixels in place when they changed.
    fn sync_textures(&mut self, engine: &mut Engine<'_>, frame: &GpuFrame) -> Result<(), sturdy::Error> {
        sync_texture(engine, &mut self.skin, Some((&frame.skin, frame.skin_hash)), "skin preview skin")?;
        let cape = frame.cape.as_ref().zip(frame.cape_hash);
        sync_texture(engine, &mut self.cape, cape, "skin preview cape")
    }

    /// Creates, moves and removes the part entities. True when a part had to be skipped for want of its geometry.
    fn sync_parts(&mut self, engine: &mut Engine<'_>, frame: &GpuFrame, texel_aa: bool) -> bool {
        let mut missing = false;
        let present: Vec<PartTag> = frame.parts.iter().map(|part| part.tag).collect();
        let gone: Vec<PartTag> = self.parts.keys().copied().filter(|tag| !present.contains(tag)).collect();
        for tag in gone {
            self.remove_part(engine, tag);
        }
        for part in &frame.parts {
            let wanted = self.parts.get(&part.tag).map(|instance| instance.geometry_hash);
            if wanted != Some(part.geometry_hash) {
                let Some(quads) = &part.quads else {
                    // Same hash as the session believes it sent, but this renderer has no (or other) geometry.
                    missing = true;
                    continue;
                };
                self.remove_part(engine, part.tag);
                match self.build_part(engine, part.tag, part.geometry_hash, quads, part.transform, texel_aa) {
                    Ok(instance) => {
                        self.parts.insert(part.tag, instance);
                    }
                    Err(err) => {
                        tracing::warn!(target: "vertexlauncher/skin_gpu", "Could not create the {:?} of the preview: {err}", part.tag)
                    }
                }
                continue;
            }
            if let Some(instance) = self.parts.get_mut(&part.tag) {
                let _ = engine.ecs().set(instance.entity, WorldTransform::from_matrix(Mat4::from_cols_array(&part.transform)));
                if instance.texel_aa != texel_aa {
                    let _ = engine.assets().set_model_float(instance.model, 0, "texel_aa", if texel_aa { 1.0 } else { 0.0 });
                    instance.texel_aa = texel_aa;
                }
            }
        }
        missing
    }

    fn remove_part(&mut self, engine: &mut Engine<'_>, tag: PartTag) {
        if let Some(instance) = self.parts.remove(&tag) {
            engine.ecs().despawn(instance.entity);
            let _ = engine.assets().unload(instance.model);
        }
    }

    fn build_part(
        &self,
        engine: &mut Engine<'_>,
        tag: PartTag,
        geometry_hash: u64,
        quads: &[SceneQuad],
        transform: [f32; 16],
        texel_aa: bool,
    ) -> Result<Instance, String> {
        let texture = match quads.first().map(|quad| quad.texture) {
            Some(SceneTexture::Cape) => self.cape.as_ref(),
            _ => self.skin.as_ref(),
        }
        .ok_or_else(|| "its texture is not loaded".to_owned())?;
        let (vertices, indices) = mesh_of(quads);
        let primitive = ModelPrimitive::new(vertices, indices, self.shader)
            .with_texture("base_color_texture", texture.asset)
            // Faces are drawn from both sides: a quad is a thin box side seen from outside, but the voxel layer's
            // inner faces and the cape's back face are looked at from either side as the camera orbits.
            .with_double_sided(true);
        let model = engine
            .assets()
            .create_model(ModelBuilder::new(format!("skin preview {tag:?}")).with_primitive(primitive))
            .map_err(|err| format!("{err:?}"))?;
        let _ = engine.assets().set_model_float(model, 0, "alpha_cutoff", ALPHA_CUTOFF);
        let _ = engine.assets().set_model_float(model, 0, "texel_aa", if texel_aa { 1.0 } else { 0.0 });
        let entity = engine
            .ecs()
            .spawn_builder()
            .with(WorldTransform::from_matrix(Mat4::from_cols_array(&transform)))
            .with(ModelRenderer::new(model).with_visibility_mask(self.layer))
            .build()
            .map_err(|err| format!("{err:?}"))?;
        Ok(Instance { entity, model, geometry_hash, texel_aa })
    }

    /// Releases everything the renderer made.
    pub fn destroy(&mut self, engine: &mut Engine<'_>) {
        let tags: Vec<PartTag> = self.parts.keys().copied().collect();
        for tag in tags {
            self.remove_part(engine, tag);
        }
        for slot in [self.skin.take(), self.cape.take()].into_iter().flatten() {
            let _ = engine.assets().unload(slot.asset);
        }
        if let Some((target, _)) = self.target.take() {
            engine.destroy_offscreen_target(target);
        }
        let _ = engine.assets().unload(self.shader);
    }
}

fn sync_texture(
    engine: &mut Engine<'_>,
    slot: &mut Option<TextureSlot>,
    image: Option<(&Arc<RgbaImage>, u64)>,
    label: &str,
) -> Result<(), sturdy::Error> {
    let Some((image, hash)) = image else {
        return Ok(());
    };
    let (width, height) = (image.width(), image.height());
    match slot {
        Some(current) if current.hash == hash => Ok(()),
        Some(current) => {
            // The same asset and renderer texture, new pixels (and maybe a new size): the models using it keep working.
            engine
                .assets()
                .replace_texture(
                    current.asset,
                    image.as_raw(),
                    width,
                    height,
                    TexturePixelFormat::Rgba8,
                    TextureColorSpace::Linear,
                    TextureKind::ColorAlpha,
                    false,
                    false,
                )
                .map_err(sturdy::Error::new)?;
            current.hash = hash;
            Ok(())
        }
        None => {
            // Linear on purpose: the shader lights the raw bytes and linearises the result itself. Nearest sampling keeps
            // texels square; no mip chain, no compression (the images are tiny and the edges must stay exact).
            let asset = engine
                .assets()
                .create_texture_sampled(
                    image.as_raw(),
                    width,
                    height,
                    TexturePixelFormat::Rgba8,
                    TextureColorSpace::Linear,
                    TextureKind::ColorAlpha,
                    label,
                    false,
                    false,
                    true,
                )
                .map_err(|err| sturdy::Error::new(format!("{err:?}")))?;
            *slot = Some(TextureSlot { asset, hash });
            Ok(())
        }
    }
}

/// Two triangles per quad, in the quad's own space.
fn mesh_of(quads: &[SceneQuad]) -> (Vec<ModelVertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for quad in quads {
        let base = vertices.len() as u32;
        let normal = Vec3::from_array(quad.normal);
        // The face's texel rectangle (the quad's UV may run backwards when mirrored), for the shader's texel clamping.
        let (u_values, v_values) = (quad.uv.map(|uv| uv[0]), quad.uv.map(|uv| uv[1]));
        let bounds = Vec4::new(
            u_values.iter().cloned().fold(f32::MAX, f32::min),
            v_values.iter().cloned().fold(f32::MAX, f32::min),
            u_values.iter().cloned().fold(f32::MIN, f32::max),
            v_values.iter().cloned().fold(f32::MIN, f32::max),
        );
        for corner in 0..4 {
            vertices.push(ModelVertex {
                position: Vec3::from_array(quad.vertices[corner]),
                normal,
                uv: sturdy::glam::Vec2::from_array(quad.uv[corner]),
                // The vertex colour is unused by the shader's lighting, so it carries the face's texel rectangle.
                color: bounds,
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

fn frame_description(request: &RenderRequest<'_>, size: UVec2, layer: u32) -> FrameDescription {
    let scene = request.frame.camera;
    let eye = Vec3::from_array(scene.eye);
    let aspect = size.x as f32 / size.y as f32;
    let mut camera = Camera::perspective().fov_degrees(scene.vertical_fov_radians.to_degrees()).at(eye);
    camera.near_clip = scene.near;
    camera.far_clip = 400.0;
    camera.aspect_ratio = Some(aspect);
    camera.culling_mask = layer;
    camera.clear_color = Vec4::ZERO;
    let mut rig = camera.rig(aspect);
    rig.look_at(Vec3::from_array(scene.target), Vec3::from_array(scene.up));
    let camera = rig.camera();

    let mut graph = RenderGraphSettings::default();
    // The picture is transparent wherever the character is not: no atmosphere (no sky, no haze), a clear background with
    // alpha 0, and the frame keeps its alpha so the UI blends it over the canvas it sits on.
    graph.atmosphere.enabled = false;
    graph.scene.background_color = Some(Vec4::ZERO);
    graph.scene.preserve_alpha = true;
    graph.shadows.enabled = false;
    graph.ambient_occlusion.enabled = false;
    graph.bloom.enabled = false;
    graph.tone_mapping.enabled = false;
    graph.restir_gi.enabled = false;
    apply_anti_aliasing(&mut graph, request.settings);
    if let Some(blur) = request.frame.motion_blur {
        graph.motion_blur.enabled = true;
        graph.motion_blur.intensity = (blur.amount / 0.15).clamp(0.0, 4.0);
        graph.motion_blur.shutter_angle_degrees = (blur.shutter_frames * 360.0).clamp(0.0, 720.0);
        graph.motion_blur.sample_count = blur.samples.clamp(2, 32) as u32;
    }

    FrameDescription::new(camera)
        .label("skin preview")
        .lighting(SceneLighting::default().ambient_radiance(Vec3::ZERO).exposure(1.0))
        .render_graph(graph)
}

fn apply_anti_aliasing(graph: &mut RenderGraphSettings, settings: RenderSettings) {
    let aa = &mut graph.anti_aliasing;
    // The engine's MSAA only smooths the silhouette of geometry: it cannot see edges that come from the skin's texels
    // (a cut-out jacket or hat layer, the borders of the face), because those are not geometry edges. So every mode
    // that is meant to smooth the picture also runs the post edge filter, which looks at the finished colours; "Off" is
    // the only mode that leaves edges alone.
    aa.msaa_samples = 1;
    aa.post_process = PostProcessAntiAliasing::None;
    match settings.aa_mode {
        SkinPreviewAaMode::Off => {}
        SkinPreviewAaMode::Msaa => {
            aa.msaa_samples = settings.msaa_samples.clamp(1, 8);
            aa.post_process = PostProcessAntiAliasing::Fxaa;
        }
        SkinPreviewAaMode::Taa => {
            aa.msaa_samples = 4;
            aa.post_process = PostProcessAntiAliasing::Fxaa;
        }
        SkinPreviewAaMode::Fxaa | SkinPreviewAaMode::Smaa | SkinPreviewAaMode::FxaaTaa => {
            aa.post_process = PostProcessAntiAliasing::Fxaa;
        }
    }
}

fn shader_path() -> PathBuf {
    app_paths::cache_root().join("shaders").join("skin_preview_flat.slang")
}

fn write_if_changed(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// A small picture of a skin's head, drawn by the engine: the sign-in button's avatar. The head is drawn into an
/// offscreen target that the UI shows as an image, and drawn again only when something it depends on changes (the skin,
/// the size, the graphics device), not every frame.
pub struct HeadRenderer {
    inner: Option<PreviewRenderer>,
    drawn: Option<(u64, UVec2, u64)>,
    layer: u32,
}

impl HeadRenderer {
    /// A head that lives on engine layer bit `layer_bit` (one of [`HEAD_LAYERS`]); every head needs its own.
    pub fn new(layer_bit: u32) -> Self {
        Self { inner: None, drawn: None, layer: 1 << layer_bit.clamp(*HEAD_LAYERS.start(), *HEAD_LAYERS.end()) }
    }

    /// The head of `skin`, `size` pixels square-ish, ready to show as an image. `generation` is the engine's device
    /// generation, so a rebuilt device draws it again. `None` while the renderer cannot make a picture.
    pub fn render(
        &mut self,
        engine: &mut Engine<'_>,
        skin: &Arc<RgbaImage>,
        size: UVec2,
        generation: u64,
    ) -> Result<Option<RendererTextureHandle>, sturdy::Error> {
        if self.inner.is_none() {
            self.inner = Some(PreviewRenderer::on_layer(engine, self.layer)?);
        }
        let Some(renderer) = self.inner.as_mut() else {
            return Ok(None);
        };
        let size = UVec2::new(size.x.max(16), size.y.max(16));
        let skin_hash = launcher_ui::screens::skins_api::hash_skin(skin);
        let key = (skin_hash, size, generation);
        if self.drawn != Some(key) {
            let Some(frame) = launcher_ui::screens::skins_api::head_frame(Arc::clone(skin)) else {
                return Ok(None);
            };
            let request = RenderRequest {
                frame: &frame,
                size,
                settings: RenderSettings { aa_mode: SkinPreviewAaMode::Msaa, msaa_samples: 4, texel_boundary_aa: true },
            };
            renderer.render(engine, &request)?;
            self.drawn = Some(key);
        }
        Ok(renderer.texture(engine).map(|(handle, _)| handle))
    }

    /// Releases the engine objects.
    pub fn destroy(&mut self, engine: &mut Engine<'_>) {
        if let Some(mut renderer) = self.inner.take() {
            renderer.destroy(engine);
        }
        self.drawn = None;
    }
}
