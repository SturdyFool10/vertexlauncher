use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use config::SkinPreviewAaMode;
use image::RgbaImage;
use launcher_ui::screens::skins_api::{GpuFrame, GpuPart};
use launcher_ui::screens::skins_gpu_scene::{PartTag, SceneQuad, SceneTexture};
use sturdy::assets::{
    Asset, ModelBuilder, ModelPrimitive, ModelVertex, RendererTextureHandle, TextureColorSpace, TextureKind,
    TexturePixelFormat,
};
use sturdy::camera::Camera;
use sturdy::glam::{Mat4, UVec2, Vec3, Vec4};
use sturdy::render::{OffscreenTargetDesc, OffscreenTargetHandle, PostProcessAntiAliasing, RenderGraphSettings, ToneMappingOperator};
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

/// One quad of the animated face. Its geometry is a fixed unit square; where the quad is, how big and how it is turned is the
/// entity's transform, which changes every frame without touching the model. The model is only made again when the quad's texture
/// rectangle changes (a blink, another expression), which is rare.
struct FaceQuad {
    instance: Instance,
    /// What the model was made from (texture rectangle and texture), to tell when it has to be made again.
    key: u64,
    /// The unit square placed in the head's space.
    local: Mat4,
}

/// A quad worked out for the face: what its model is made from, and how it is placed.
struct CanonicalQuad {
    key: u64,
    local: Mat4,
    quad: SceneQuad,
}

/// Splits a face quad into a fixed unit square and the matrix that places it. A parallelogram (every panel of the face is one) is
/// the unit square turned, scaled and moved; anything else keeps its own corners and is made again whenever they change.
fn canonical_quad(quad: &SceneQuad) -> CanonicalQuad {
    use std::hash::{Hash, Hasher};
    let [v0, v1, v2, v3] = quad.vertices.map(Vec3::from_array);
    let (e1, e2) = (v1 - v0, v3 - v0);
    let scale = e1.length().max(e2.length());
    let parallelogram = (v2 - (v1 + v3 - v0)).length() <= 1e-3 * (1.0 + scale);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for uv in quad.uv {
        uv.map(f32::to_bits).hash(&mut hasher);
    }
    (quad.texture as u8).hash(&mut hasher);
    parallelogram.hash(&mut hasher);
    if parallelogram {
        let normal = Vec3::from_array(quad.normal);
        let mut canonical = *quad;
        canonical.vertices = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        canonical.normal = [0.0, 0.0, 1.0];
        CanonicalQuad {
            key: hasher.finish(),
            local: Mat4::from_cols(e1.extend(0.0), e2.extend(0.0), normal.extend(0.0), v0.extend(1.0)),
            quad: canonical,
        }
    } else {
        for vertex in quad.vertices {
            vertex.map(f32::to_bits).hash(&mut hasher);
        }
        CanonicalQuad { key: hasher.finish(), local: Mat4::IDENTITY, quad: *quad }
    }
}

struct TextureSlot {
    asset: Asset,
    hash: u64,
}

/// A part that could not be built, and when to try again: retrying every frame would repeat the same failure (and its log line)
/// sixty times a second.
struct PartFailure {
    attempts: u32,
    retry_at: Instant,
}

/// A model that could not be unloaded when its part went away; unloading is tried again until it works, so the GPU memory is not
/// lost for the rest of the session.
struct PendingUnload {
    asset: Asset,
    attempts: u32,
    /// Not before this: frames already submitted may still be drawing the model, and freeing it under them has been seen to leave the
    /// engine drawing nothing at all.
    not_before: Instant,
}

/// How long a model stays loaded after its part was replaced.
const UNLOAD_DELAY: Duration = Duration::from_secs(2);

/// Gives up on an unload after this many attempts (one per frame), and says so.
const MAX_UNLOAD_ATTEMPTS: u32 = 600;

/// Owns the preview's engine objects: the target it is drawn into, the shader, the textures and one model/entity per
/// part of the character.
pub struct PreviewRenderer {
    shader: Asset,
    target: Option<(OffscreenTargetHandle, UVec2)>,
    skin: Option<TextureSlot>,
    cape: Option<TextureSlot>,
    parts: HashMap<PartTag, Instance>,
    /// The face, one model per quad (see [`FaceQuad`]).
    face: Vec<Option<FaceQuad>>,
    layer: u32,
    /// The engine's device generation everything above was made for.
    generation: Option<u64>,
    part_failures: HashMap<PartTag, PartFailure>,
    pending_unloads: Vec<PendingUnload>,
    churn: Option<(Entity, Asset)>,
    churn_model: Option<Asset>,
}

impl PreviewRenderer {
    /// Creates the shader. Cheap; the target and textures are made on the first [`render`](Self::render).
    pub fn new(engine: &mut Engine<'_>) -> Result<Self, sturdy::Error> {
        Self::on_layer(engine, PREVIEW_LAYER)
    }

    fn on_layer(engine: &mut Engine<'_>, layer: u32) -> Result<Self, sturdy::Error> {
        let shader = load_shader(engine)?;
        Ok(Self {
            shader,
            target: None,
            skin: None,
            cape: None,
            parts: HashMap::new(),
            face: Vec::new(),
            layer,
            generation: None,
            part_failures: HashMap::new(),
            pending_unloads: Vec::new(),
            churn: None,
            churn_model: None,
        })
    }

    /// The picture, valid after the first successful [`render`](Self::render), and its size.
    ///
    /// `None` when there is no picture to show: not drawn yet, or the engine no longer has the target (it answers 0 for one it does not
    /// know, and showing texture 0 would be a blank canvas with nothing to say why).
    pub fn texture(&self, engine: &mut Engine<'_>) -> Option<(RendererTextureHandle, UVec2)> {
        let (target, size) = self.target?;
        let raw = engine.offscreen_target_texture(target);
        (raw != 0).then(|| (RendererTextureHandle::from_raw(raw), size))
    }

    /// Reads the picture back and returns the share of its pixels that are not fully transparent, or why that could not be done.
    /// Stalls the GPU for a moment: a diagnostic, for an occasional status line, not for every frame.
    pub fn coverage(&self, engine: &mut Engine<'_>) -> Result<f32, String> {
        use sturdy::rhi::{
            AccessFlags, BufferDesc, BufferUsage, CommandEncoderDesc, MemoryLocation, PipelineStage, QueueClass, TextureBarrier,
            TextureLayout, TextureSubresourceRange,
        };
        let (handle, size) = self.texture(engine).ok_or("the engine has no picture for the target")?;
        let resources = engine.assets().rhi_texture(handle).ok_or("the picture has no GPU texture")?;
        let (w, h) = (size.x, size.y);
        let row_pitch = (w * 4).div_ceil(256) * 256;
        let mut rhi = engine.rhi();
        let readback = rhi
            .create_buffer(&BufferDesc { size: (row_pitch * h) as u64, usage: BufferUsage::TRANSFER_DST.bits(), memory: MemoryLocation::HostReadback, label: "preview coverage".to_string() })
            .map_err(|err| format!("readback buffer: {err:?}"))?;
        let mut encoder = rhi
            .create_command_encoder(&CommandEncoderDesc { queue: QueueClass::Graphics, label: "preview coverage".to_string() })
            .map_err(|err| format!("readback encoder: {err:?}"))?;
        let range = TextureSubresourceRange { base_mip_level: 0, mip_level_count: 1, base_array_layer: 0, array_layer_count: 1 };
        let barrier = |old_layout, new_layout, src_access: AccessFlags, dst_access: AccessFlags| TextureBarrier {
            texture: resources.texture,
            src_stage: PipelineStage::ALL_COMMANDS.bits(),
            src_access: src_access.bits(),
            dst_stage: PipelineStage::ALL_COMMANDS.bits(),
            dst_access: dst_access.bits(),
            ownership: Default::default(),
            old_layout,
            new_layout,
            range,
        };
        encoder.barrier(&[], &[], &[barrier(TextureLayout::ShaderReadOnly, TextureLayout::TransferSrc, AccessFlags::SHADER_READ, AccessFlags::TRANSFER_READ)]);
        encoder.copy_texture_to_buffer(
            resources.texture,
            readback,
            &sturdy::rhi::BufferTextureCopy {
                buffer_offset: 0,
                buffer_row_length: row_pitch / 4,
                buffer_image_height: h,
                mip_level: 0,
                base_array_layer: 0,
                array_layer_count: 1,
                offset_x: 0,
                offset_y: 0,
                offset_z: 0,
                extent_width: w,
                extent_height: h,
                extent_depth_or_layers: 1,
            },
        );
        encoder.barrier(&[], &[], &[barrier(TextureLayout::TransferSrc, TextureLayout::ShaderReadOnly, AccessFlags::TRANSFER_READ, AccessFlags::SHADER_READ)]);
        let result = (|| {
            let commands = encoder.finish().map_err(|err| format!("readback commands: {err:?}"))?;
            rhi.submit(&[commands]).map_err(|err| format!("readback submit: {err:?}"))?;
            rhi.wait_idle();
            let mapped = rhi.map_buffer(readback).map_err(|err| format!("readback map: {err:?}"))?;
            let mut covered = 0usize;
            for y in 0..h as usize {
                let row = &mapped[y * row_pitch as usize..y * row_pitch as usize + w as usize * 4];
                covered += row.chunks_exact(4).filter(|pixel| pixel[3] != 0).count();
            }
            Ok(covered as f32 / (w as usize * h as usize) as f32)
        })();
        rhi.destroy_buffer(readback);
        result
    }

    /// Diagnostic only: one create/destroy cycle of a throwaway quad, to find what the engine runs out of. `entity` spawns (and
    /// despawns the previous) entity, `model` creates (and unloads the previous) model; with `entity` and not `model` one shared model is used.
    pub fn debug_churn(&mut self, engine: &mut Engine<'_>, entity: bool, model: bool) {
        let skin = self.skin.as_ref().map(|slot| slot.asset);
        let Some(skin) = skin else { return };
        let quad = SceneQuad {
            transform: Mat4::IDENTITY.to_cols_array(),
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            normal: [0.0, 0.0, 1.0],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            texture: SceneTexture::Skin,
        };
        let make = |engine: &mut Engine<'_>, shader: Asset| -> Option<Asset> {
            let (vertices, indices) = mesh_of(&[quad]);
            let primitive = ModelPrimitive::new(vertices, indices, shader).with_texture("base_color_texture", skin).with_double_sided(true);
            engine.assets().create_model(ModelBuilder::new("churn").with_primitive(primitive)).ok()
        };
        if let Some((old_entity, old_model)) = self.churn.take() {
            if entity {
                engine.ecs().despawn(old_entity);
            }
            if model {
                let _ = engine.assets().unload(old_model);
            }
        }
        let current = if model || self.churn_model.is_none() {
            let Some(created) = make(engine, self.shader) else { return };
            if !model {
                self.churn_model = Some(created);
            }
            created
        } else {
            self.churn_model.unwrap()
        };
        let spawned = if entity {
            engine
                .ecs()
                .spawn_builder()
                .with(WorldTransform::from_matrix(Mat4::from_translation(Vec3::new(500.0, 0.0, 0.0))))
                .with(ModelRenderer::new(current).with_visibility_mask(self.layer))
                .build()
                .ok()
        } else {
            None
        };
        if let Some(spawned) = spawned {
            self.churn = Some((spawned, current));
        } else if model {
            let _ = engine.assets().unload(current);
        }
    }

    /// Diagnostic only: makes `count` throwaway quad models, drops them all, then makes them again, and reports the times. For finding
    /// what the engine does with very many meshes.
    pub fn debug_bulk(&mut self, engine: &mut Engine<'_>, count: usize) -> String {
        let Some(skin) = self.skin.as_ref().map(|slot| slot.asset) else { return "no skin texture yet".to_owned() };
        let quad = SceneQuad {
            transform: Mat4::IDENTITY.to_cols_array(),
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            normal: [0.0, 0.0, 1.0],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            texture: SceneTexture::Skin,
        };
        let mut report = String::new();
        let mut models: Vec<Asset> = Vec::with_capacity(count);
        for round in 0..2 {
            let started = Instant::now();
            let mut failed = 0usize;
            let mut first_error: Option<String> = None;
            for index in 0..count {
                let (vertices, indices) = mesh_of(&[quad]);
                let primitive = ModelPrimitive::new(vertices, indices, self.shader).with_texture("base_color_texture", skin);
                match engine.assets().create_model(ModelBuilder::new("bulk").with_primitive(primitive)) {
                    Ok(model) => models.push(model),
                    Err(err) => {
                        failed += 1;
                        if first_error.is_none() {
                            first_error = Some(format!("first failure at model {index}: {err:?}"));
                        }
                    }
                }
            }
            if let Some(error) = first_error {
                report += &format!("[{error}] ");
            }
            let created = started.elapsed();
            let started = Instant::now();
            let mut unload_failed = 0usize;
            for model in models.drain(..) {
                if engine.assets().unload(model).is_err() {
                    unload_failed += 1;
                }
            }
            report += &format!(
                "round {round}: {count} models made in {created:?} ({failed} failed), dropped in {:?} ({unload_failed} failed); ",
                started.elapsed()
            );
        }
        report
    }

    /// How many parts are built on the GPU, and the pending (not yet unloaded) models; for diagnostics.
    pub fn part_count(&self) -> (usize, usize) {
        let face_built = usize::from(!self.face.is_empty() && self.face.iter().all(Option::is_some));
        (self.parts.len() + face_built, self.pending_unloads.len())
    }

    /// Draws `request` into the target. Returns `true` when the renderer is missing geometry the frame did not carry
    /// (it was created after the geometry was last sent): ask the session for everything again.
    pub fn render(&mut self, engine: &mut Engine<'_>, request: &RenderRequest<'_>) -> Result<bool, sturdy::Error> {
        let size = UVec2::new(request.size.x.max(16), request.size.y.max(16));
        // After the graphics device changed (a switch, a recovery from a lost device) nothing made before is trusted: everything is made
        // again on the new device, and the caller is told its geometry has to be sent again.
        let generation = engine.device_generation();
        let rebuilt = match self.generation {
            Some(previous) if previous != generation => {
                tracing::info!(target: "vertexlauncher/skin_gpu", "The graphics device changed (generation {previous} -> {generation}); rebuilding the preview.");
                self.rebuild(engine)?;
                true
            }
            _ => false,
        };
        self.generation = Some(generation);
        self.retry_unloads(engine);
        self.ensure_target(engine, size)?;
        self.sync_textures(engine, request.frame)?;
        let missing = self.sync_parts(engine, request.frame, request.settings.texel_boundary_aa) || rebuilt;

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
        if !present.contains(&PartTag::Face) {
            self.remove_face(engine);
        }
        for part in &frame.parts {
            if part.tag == PartTag::Face {
                missing |= self.sync_face(engine, part, texel_aa);
                continue;
            }
            let wanted = self.parts.get(&part.tag).map(|instance| instance.geometry_hash);
            if wanted != Some(part.geometry_hash) {
                let Some(quads) = &part.quads else {
                    // Same hash as the session believes it sent, but this renderer has no (or other) geometry.
                    missing = true;
                    continue;
                };
                if self.part_failures.get(&part.tag).is_some_and(|failure| Instant::now() < failure.retry_at) {
                    continue;
                }
                self.remove_part(engine, part.tag);
                match self.build_part(engine, part.tag, part.geometry_hash, quads, part.transform, texel_aa) {
                    Ok(instance) => {
                        self.part_failures.remove(&part.tag);
                        self.parts.insert(part.tag, instance);
                    }
                    Err(err) => self.note_part_failure(part.tag, &err),
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

    /// Brings the face's quads up to date: each is moved by its transform every frame and made again only when its texture rectangle
    /// changed. True when the geometry was needed but not sent.
    fn sync_face(&mut self, engine: &mut Engine<'_>, part: &GpuPart, texel_aa: bool) -> bool {
        let head = Mat4::from_cols_array(&part.transform);
        if let Some(quads) = &part.quads {
            let canonical: Vec<CanonicalQuad> = quads.iter().map(canonical_quad).collect();
            if self.face.len() != canonical.len() {
                self.remove_face(engine);
                self.face.resize_with(canonical.len(), || None);
            }
            let paused = self.part_failures.get(&PartTag::Face).is_some_and(|failure| Instant::now() < failure.retry_at);
            for (index, wanted) in canonical.into_iter().enumerate() {
                match &mut self.face[index] {
                    Some(current) if current.key == wanted.key => {
                        current.local = wanted.local;
                        continue;
                    }
                    _ => {}
                }
                if paused {
                    continue;
                }
                if let Some(old) = self.face[index].take() {
                    engine.ecs().despawn(old.instance.entity);
                    self.unload_model(engine, old.instance.model);
                }
                match self.build_part(engine, PartTag::Face, 0, std::slice::from_ref(&wanted.quad), (head * wanted.local).to_cols_array(), texel_aa) {
                    Ok(instance) => {
                        self.part_failures.remove(&PartTag::Face);
                        self.face[index] = Some(FaceQuad { instance, key: wanted.key, local: wanted.local });
                    }
                    Err(err) => self.note_part_failure(PartTag::Face, &err),
                }
            }
        } else if self.face.is_empty() {
            // Nothing built, and no geometry sent: ask for it.
            return true;
        }
        for quad in self.face.iter_mut().flatten() {
            let _ = engine.ecs().set(quad.instance.entity, WorldTransform::from_matrix(head * quad.local));
            if quad.instance.texel_aa != texel_aa {
                let _ = engine.assets().set_model_float(quad.instance.model, 0, "texel_aa", if texel_aa { 1.0 } else { 0.0 });
                quad.instance.texel_aa = texel_aa;
            }
        }
        false
    }

    fn remove_face(&mut self, engine: &mut Engine<'_>) {
        for quad in std::mem::take(&mut self.face).into_iter().flatten() {
            engine.ecs().despawn(quad.instance.entity);
            self.unload_model(engine, quad.instance.model);
        }
    }

    /// Notes that a part could not be built: it is tried again after a doubling pause, and said on the first failure and each time the
    /// attempts double.
    fn note_part_failure(&mut self, tag: PartTag, err: &str) {
        let failure = self.part_failures.entry(tag).or_insert(PartFailure { attempts: 0, retry_at: Instant::now() });
        failure.attempts += 1;
        // Doubling pause, 0.5 s to 30 s.
        let pause = (Duration::from_millis(500) * 2_u32.saturating_pow((failure.attempts - 1).min(6))).min(Duration::from_secs(30));
        failure.retry_at = Instant::now() + pause;
        if failure.attempts.is_power_of_two() {
            tracing::warn!(
                target: "vertexlauncher/skin_gpu",
                "Could not create the {tag:?} of the preview (attempt {}, trying again in {:.1}s): {err}",
                failure.attempts,
                pause.as_secs_f32()
            );
        }
    }

    fn remove_part(&mut self, engine: &mut Engine<'_>, tag: PartTag) {
        if let Some(instance) = self.parts.remove(&tag) {
            engine.ecs().despawn(instance.entity);
            self.unload_model(engine, instance.model);
        }
    }

    /// Queues a model for unloading a little later (see [`PendingUnload`]); an unload that fails is tried again each frame.
    fn unload_model(&mut self, _engine: &mut Engine<'_>, model: Asset) {
        self.pending_unloads.push(PendingUnload { asset: model, attempts: 0, not_before: Instant::now() + UNLOAD_DELAY });
    }

    fn retry_unloads(&mut self, engine: &mut Engine<'_>) {
        if self.pending_unloads.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_unloads);
        let now = Instant::now();
        for mut item in pending {
            if now < item.not_before {
                self.pending_unloads.push(item);
                continue;
            }
            match engine.assets().unload(item.asset) {
                Ok(()) => {}
                Err(err) if item.attempts >= MAX_UNLOAD_ATTEMPTS => {
                    tracing::warn!(
                        target: "vertexlauncher/skin_gpu",
                        "Giving up unloading a preview model after {} attempts ({err:?}); its GPU memory stays in use until exit.",
                        item.attempts
                    );
                }
                Err(_) => {
                    item.attempts += 1;
                    self.pending_unloads.push(item);
                }
            }
        }
    }

    /// Makes everything again on the current device after the device changed: what was made for the old one is released as far as the
    /// engine allows (errors do not matter, the old device may be gone) and the shader, target, textures and parts start over.
    fn rebuild(&mut self, engine: &mut Engine<'_>) -> Result<(), sturdy::Error> {
        let tags: Vec<PartTag> = self.parts.keys().copied().collect();
        for tag in tags {
            if let Some(instance) = self.parts.remove(&tag) {
                engine.ecs().despawn(instance.entity);
                let _ = engine.assets().unload(instance.model);
            }
        }
        for quad in std::mem::take(&mut self.face).into_iter().flatten() {
            engine.ecs().despawn(quad.instance.entity);
            let _ = engine.assets().unload(quad.instance.model);
        }
        self.pending_unloads.clear();
        self.part_failures.clear();
        for slot in [self.skin.take(), self.cape.take()].into_iter().flatten() {
            let _ = engine.assets().unload(slot.asset);
        }
        if let Some((target, _)) = self.target.take() {
            engine.destroy_offscreen_target(target);
        }
        let _ = engine.assets().unload(self.shader);
        self.shader = load_shader(engine)?;
        Ok(())
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
        self.remove_face(engine);
        for item in self.pending_unloads.drain(..) {
            if engine.assets().unload(item.asset).is_err() {
                tracing::warn!(target: "vertexlauncher/skin_gpu", "A preview model could not be unloaded at shutdown of its renderer; its GPU memory stays in use.");
            }
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

/// Writes the shader next to the launcher's other caches and loads it. When that folder cannot be written (a full disk, a read-only
/// profile) the system's temporary folder is tried before giving up, so a cache problem does not cost the preview.
fn load_shader(engine: &mut Engine<'_>) -> Result<Asset, sturdy::Error> {
    let mut problems = Vec::new();
    for path in shader_paths() {
        if let Err(err) = write_if_changed(&path, SHADER_SOURCE) {
            problems.push(format!("writing '{}': {err}", path.display()));
            continue;
        }
        match engine.assets().load_shader_with(
            path.to_string_lossy(),
            "skin preview flat",
            "skin_preview_flat",
            "vertexMain",
            "fragmentMain",
            "depthOnlyMain",
            &[],
        ) {
            Ok(shader) => {
                if !problems.is_empty() {
                    tracing::warn!(target: "vertexlauncher/skin_gpu", "The preview shader was loaded from '{}' after: {}", path.display(), problems.join("; "));
                }
                return Ok(shader);
            }
            Err(err) => problems.push(format!("loading '{}': {err:?}", path.display())),
        }
    }
    Err(sturdy::Error::new(format!("the preview shader could not be prepared: {}", problems.join("; "))))
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
    // PsychoV keeps bright, saturated parts of the skin from clipping to white.
    graph.tone_mapping.enabled = true;
    graph.tone_mapping.operation = ToneMappingOperator::PsychoV;
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

/// Where the shader file may live, in the order they are tried.
fn shader_paths() -> Vec<PathBuf> {
    vec![
        app_paths::cache_root().join("shaders").join("skin_preview_flat.slang"),
        std::env::temp_dir().join("vertexlauncher-shaders").join("skin_preview_flat.slang"),
    ]
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
