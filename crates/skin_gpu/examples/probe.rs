//! Draws the skin preview on the GPU for a few frames in a hidden window and writes what it drew to a PNG, so the
//! renderer can be checked without the launcher: `cargo run -p skin_gpu --example probe -- /tmp/skin.png [layers] [walk]`.

use std::path::PathBuf;

use config::SkinPreviewAaMode;
use image::{Rgba, RgbaImage};
use launcher_ui::screens::skins_api::{MotionMode, PreviewSettings, SkinVariant, SkinsSession};
use skin_gpu::{PreviewRenderer, RenderRequest, RenderSettings};
use sturdy::glam::UVec2;
use sturdy::prelude::*;

struct Probe {
    session: SkinsSession,
    renderer: Option<PreviewRenderer>,
    out: PathBuf,
    frames: u32,
    layers: bool,
    walk: bool,
    elytra: bool,
    cape: bool,
    blur: bool,
    started: std::time::Instant,
    cached: Option<launcher_ui::screens::skins_api::GpuFrame>,
    face_quads: Option<Vec<launcher_ui::screens::skins_gpu_scene::SceneQuad>>,
    churned: u32,
}

fn test_skin() -> RgbaImage {
    let mut image = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
    let fill = |image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: [u8; 4]| {
        for yy in y..y + h {
            for xx in x..x + w {
                image.put_pixel(xx, yy, Rgba(c));
            }
        }
    };
    let skin = [214, 160, 120, 255];
    // Head (all faces), then the face details on the front: eyes, mouth.
    fill(&mut image, 0, 0, 32, 16, skin);
    fill(&mut image, 9, 11, 2, 2, [255, 255, 255, 255]);
    fill(&mut image, 13, 11, 2, 2, [255, 255, 255, 255]);
    fill(&mut image, 9, 12, 1, 1, [40, 60, 200, 255]);
    fill(&mut image, 14, 12, 1, 1, [40, 60, 200, 255]);
    fill(&mut image, 10, 14, 4, 1, [150, 60, 60, 255]);
    // Hat layer: a red band.
    fill(&mut image, 32, 0, 32, 16, [0, 0, 0, 0]);
    fill(&mut image, 40, 8, 8, 2, [200, 40, 40, 255]);
    // Torso, arms, legs.
    fill(&mut image, 16, 16, 24, 16, [40, 170, 190, 255]);
    fill(&mut image, 40, 16, 16, 16, skin);
    fill(&mut image, 0, 16, 16, 16, [50, 60, 170, 255]);
    fill(&mut image, 16, 48, 16, 16, [50, 60, 170, 255]);
    fill(&mut image, 32, 48, 16, 16, skin);
    // Jacket layer: a white stripe on the chest and sleeves.
    fill(&mut image, 20, 36, 8, 2, [240, 240, 240, 255]);
    image
}

impl GameLogic for Probe {
    fn init(&mut self, engine: &mut Engine<'_>) -> Result<(), Error> {
        self.renderer = Some(PreviewRenderer::new(engine)?);
        self.session.set_preview_skin(test_skin(), SkinVariant::Classic);
        if self.cape {
            let mut cape = RgbaImage::from_pixel(64, 32, Rgba([0, 0, 0, 0]));
            for y in 0..32 {
                for x in 0..64 {
                    cape.put_pixel(x, y, Rgba([200, (x * 4) as u8, (y * 8) as u8, 255]));
                }
            }
            self.session.set_preview_cape(cape);
        }
        if self.elytra {
            self.session.toggle_elytra();
        }
        if self.walk && self.session.motion_mode() == MotionMode::Idle {
            self.session.toggle_motion_mode();
        }
        Ok(())
    }

    fn frame(&mut self, engine: &mut Engine<'_>, _frame: &Frame) -> Option<FrameDescription> {
        let settings = PreviewSettings {
            aa_mode: match std::env::var("PROBE_AA").as_deref() { Ok("off") => SkinPreviewAaMode::Off, Ok("fxaa") => SkinPreviewAaMode::Fxaa, Ok("smaa") => SkinPreviewAaMode::Smaa, Ok("taa") => SkinPreviewAaMode::Taa, Ok("fxaataa") => SkinPreviewAaMode::FxaaTaa, _ => SkinPreviewAaMode::Msaa },
            msaa_samples: 4,
            texel_aa_mode: config::SkinPreviewTexelAaMode::Off,
            motion_blur_enabled: self.blur,
            motion_blur_amount: 0.15,
            motion_blur_shutter_frames: 0.75,
            motion_blur_sample_count: 5,
            layers_3d_enabled: self.layers,
            expressions_enabled: std::env::var("PROBE_EXPR").is_ok(),
        };
        self.session.set_preview_settings(&settings);
        let now = if std::env::var("PROBE_FIXED").is_ok() { 4.0 + f64::from(self.frames) * 0.0 } else { self.started.elapsed().as_secs_f64() * std::env::var("PROBE_TIME_SCALE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0) + 4.0 };
        // From frame 30 on the same frame is drawn again, so comparing the texel anti-aliasing is a like-for-like picture.
        let stress: Option<u32> = std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok());
        let frame = if stress.is_none() && self.frames >= 30 && self.cached.is_some() { self.cached.clone() } else { self.session.gpu_frame(now, &settings) };
        // PROBE_CHURN=n: the face part is rebuilt every n frames (a new model each time, the old one unloaded), the way the launcher's
        // blinking face does.
        let mut frame = frame;
        if let (Some(frame), Some(every)) = (frame.as_mut(), std::env::var("PROBE_CHURN").ok().and_then(|v| v.parse::<u32>().ok())) {
            for part in frame.parts.iter_mut().filter(|part| part.tag == launcher_ui::screens::skins_gpu_scene::PartTag::Face) {
                if part.quads.is_some() {
                    self.face_quads = part.quads.clone();
                }
                if self.frames % every == 0 {
                    self.churned += 1;
                    part.quads = self.face_quads.clone();
                }
                part.geometry_hash = part.geometry_hash.wrapping_add(u64::from(self.churned));
            }
        }
        self.cached = frame.clone();
        if let (Some(frame), Some(renderer)) = (frame, self.renderer.as_mut()) {
            let request = RenderRequest {
                frame: &frame,
                size: UVec2::new(480, 460),
                settings: RenderSettings { aa_mode: settings.aa_mode, msaa_samples: std::env::var("PROBE_MSAA").ok().and_then(|v| v.parse().ok()).unwrap_or(4), texel_boundary_aa: self.frames >= 30 && std::env::var("PROBE_NO_TEXEL").is_err() },
            };
            match renderer.render(engine, &request) {
                Ok(resend) => {
                    if resend {
                        self.session.gpu_resend_all();
                    }
                }
                Err(err) => eprintln!("[probe] render failed: {err}"),
            }
        }
        if self.frames == 5 && std::env::var("PROBE_HEAD").is_ok() {
            // The sign-in button's avatar: the head alone, small.
            let skin = std::sync::Arc::new(test_skin());
            let mut heads = skin_gpu::HeadRenderer::new(8);
            let size = UVec2::new(88, 88);
            match heads.render(engine, &skin, size, 0) {
                Ok(Some(handle)) => {
                    if let Some(image) = read_back_texture(engine, handle, size) {
                        image.save(self.out.with_extension("head.png")).expect("write png");
                    }
                }
                other => eprintln!("[probe] head not drawn: {:?}", other.map(|handle| handle.is_some())),
            }
            heads.destroy(engine);
        }
        if self.frames == 5 {
            if let (Some(count), Some(renderer)) = (std::env::var("PROBE_BULK").ok().and_then(|v| v.parse::<usize>().ok()), self.renderer.as_mut()) {
                eprintln!("[probe] bulk: {}", renderer.debug_bulk(engine, count));
            }
        }
        if let (Some(mode), Some(renderer)) = (std::env::var("PROBE_RAW").ok(), self.renderer.as_mut()) {
            renderer.debug_churn(engine, mode == "entity" || mode == "both", mode == "model" || mode == "both");
        }
        self.frames += 1;
        if let Some(limit) = stress {
            if self.frames % 100 == 0 {
                if let Some(renderer) = self.renderer.as_ref() {
                    eprintln!("[probe] frame {}: parts {:?}, rebuilds {}, coverage {:?}", self.frames, renderer.part_count(), self.churned, renderer.coverage(engine));
                }
            }
            if self.frames >= limit {
                if let Some(mut window) = engine.primary_window_control() {
                    window.request_close();
                }
            }
            return Some(FrameDescription::ui_only());
        }
        if self.frames == 31 {
            // One more frame, same pose, with the texel anti-aliasing on.
            if let Some(renderer) = self.renderer.as_ref() {
                if let Some(image) = read_back(engine, renderer) {
                    image.save(self.out.with_extension("texel.png")).expect("write png");
                }
            }
        }
        if self.frames == 32 {
            if let Some(mut window) = engine.primary_window_control() {
                window.request_close();
            }
        }
        if self.frames == 30 {
            if let Some(renderer) = self.renderer.as_ref() {
                match read_back(engine, renderer) {
                    Some(image) => {
                        image.save(&self.out).expect("write png");
                        eprintln!("[probe] wrote {}", self.out.display());
                    }
                    None => eprintln!("[probe] could not read the target back"),
                }
            }
        }
        Some(FrameDescription::ui_only())
    }
}

fn read_back(engine: &mut Engine<'_>, renderer: &PreviewRenderer) -> Option<RgbaImage> {
    let (handle, size) = renderer.texture(engine)?;
    read_back_texture(engine, handle, size)
}

fn read_back_texture(engine: &mut Engine<'_>, handle: sturdy::assets::RendererTextureHandle, size: UVec2) -> Option<RgbaImage> {
    use sturdy::rhi::{
        AccessFlags, BufferDesc, BufferUsage, CommandEncoderDesc, MemoryLocation, PipelineStage, QueueClass, TextureBarrier,
        TextureLayout, TextureSubresourceRange,
    };
    let resources = engine.assets().rhi_texture(handle)?;
    let (w, h) = (size.x, size.y);
    let row_pitch = (w * 4).div_ceil(256) * 256;
    let mut rhi = engine.rhi();
    let readback = rhi
        .create_buffer(&BufferDesc { size: (row_pitch * h) as u64, usage: BufferUsage::TRANSFER_DST.bits(), memory: MemoryLocation::HostReadback, label: "probe".to_string() })
        .ok()?;
    let mut encoder = rhi.create_command_encoder(&CommandEncoderDesc { queue: QueueClass::Graphics, label: "probe".to_string() }).ok()?;
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
    let commands = encoder.finish().ok()?;
    rhi.submit(&[commands]).ok()?;
    rhi.wait_idle();
    let mapped = rhi.map_buffer(readback).ok()?;
    let bytes: Vec<u8> = mapped[..(row_pitch * h) as usize].to_vec();
    drop(mapped);
    rhi.destroy_buffer(readback);
    let mut image = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let at = (y * row_pitch + x * 4) as usize;
            // The target is BGRA or RGBA depending on the device default; the probe asks for the format it was given.
            let (b0, b1, b2, b3) = (bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]);
            image.put_pixel(x, y, Rgba([b2, b1, b0, b3]));
        }
    }
    Some(image)
}

/// The same drawing, done from the UI draw hook (what the launcher does) instead of the frame callback.
fn install_hook(out: PathBuf, layers: bool) {
    use sturdy::ui::{self, ElementDesc, Sizing};
    let mut session = SkinsSession::new();
    session.set_preview_skin(test_skin(), SkinVariant::Classic);
    let mut renderer: Option<PreviewRenderer> = None;
    let started = std::time::Instant::now();
    let mut frames = 0u32;
    let mut face_quads: Option<Vec<launcher_ui::screens::skins_gpu_scene::SceneQuad>> = None;
    let mut churned = 0u32;
    let mut spent = std::time::Duration::ZERO;
    ui::install_draw_hook(move |ui| {
        let settings = PreviewSettings {
            aa_mode: match std::env::var("PROBE_AA").as_deref() { Ok("off") => SkinPreviewAaMode::Off, Ok("fxaa") => SkinPreviewAaMode::Fxaa, Ok("smaa") => SkinPreviewAaMode::Smaa, Ok("taa") => SkinPreviewAaMode::Taa, Ok("fxaataa") => SkinPreviewAaMode::FxaaTaa, _ => SkinPreviewAaMode::Msaa },
            msaa_samples: 4,
            texel_aa_mode: config::SkinPreviewTexelAaMode::Off,
            motion_blur_enabled: false,
            motion_blur_amount: 0.15,
            motion_blur_shutter_frames: 0.75,
            motion_blur_sample_count: 5,
            layers_3d_enabled: layers,
            expressions_enabled: std::env::var("PROBE_EXPR").is_ok(),
        };
        session.set_preview_settings(&settings);
        if renderer.is_none() {
            renderer = Some(PreviewRenderer::new(&mut ui.engine()).expect("renderer"));
            session.gpu_resend_all();
        }
        let now = started.elapsed().as_secs_f64() + 4.0;
        let mut hook_frame = session.gpu_frame(now, &settings);
        if let (Some(frame), Some(every)) = (hook_frame.as_mut(), std::env::var("PROBE_CHURN").ok().and_then(|v| v.parse::<u32>().ok())) {
            for part in frame.parts.iter_mut().filter(|part| part.tag == launcher_ui::screens::skins_gpu_scene::PartTag::Face) {
                if part.quads.is_some() {
                    face_quads = part.quads.clone();
                }
                if frames % every == 0 {
                    churned += 1;
                    part.quads = face_quads.clone();
                }
                part.geometry_hash = part.geometry_hash.wrapping_add(u64::from(churned));
            }
        }
        if let (Some(frame), Some(renderer)) = (hook_frame, renderer.as_mut()) {
            let request = RenderRequest {
                frame: &frame,
                size: if std::env::var("PROBE_RESIZE").is_ok() && (frames / 5) % 2 == 1 { UVec2::new(480, 464) } else { UVec2::new(480, 460) },
                settings: RenderSettings { aa_mode: settings.aa_mode, msaa_samples: std::env::var("PROBE_MSAA").ok().and_then(|v| v.parse().ok()).unwrap_or(4), texel_boundary_aa: false },
            };
            let began = std::time::Instant::now();
            if renderer.render(&mut ui.engine(), &request).expect("render") {
                session.gpu_resend_all();
            }
            if frames >= 10 {
                spent += began.elapsed();
            }
            if let Some((handle, size)) = renderer.texture(&mut ui.engine()) {
                ui.image(&ElementDesc::new().width(Sizing::Fixed(size.x as f32)).height(Sizing::Fixed(size.y as f32)), handle);
            }
            frames += 1;
            if let Some(limit) = std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse::<u32>().ok()) {
                if frames % 100 == 0 {
                    eprintln!("[probe] hook frame {frames}: parts {:?}, rebuilds {churned}, coverage {:?}", renderer.part_count(), renderer.coverage(&mut ui.engine()));
                }
                if frames >= limit {
                    if let Some(mut window) = ui.engine().primary_window_control() {
                        window.request_close();
                    }
                }
            } else if frames == 30 {
                eprintln!("[probe] render() on the engine thread: {:?} per frame (frames 10..30)", spent / 20);
                if let Some(image) = read_back(&mut ui.engine(), renderer) {
                    image.save(&out).expect("write png");
                    eprintln!("[probe] wrote {} (from the UI hook)", out.display());
                }
                if let Some(mut window) = ui.engine().primary_window_control() {
                    window.request_close();
                }
            }
        }
    });
}

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "skin_preview_probe.png".to_owned()));
    let rest: Vec<String> = args.collect();
    if rest.iter().any(|a| a == "hook") {
        struct Idle;
        impl GameLogic for Idle {
            fn init(&mut self, _engine: &mut Engine<'_>) -> Result<(), Error> {
                Ok(())
            }
            fn frame(&mut self, _engine: &mut Engine<'_>, _frame: &Frame) -> Option<FrameDescription> {
                Some(FrameDescription::ui_only())
            }
        }
        install_hook(out, rest.iter().any(|a| a == "layers"));
        return sturdy::run(RuntimeConfig::new("skin preview probe").size(480, 460).visible(false), Idle);
    }
    sturdy::run(
        RuntimeConfig::new("skin preview probe").size(480, 460).visible(false),
        Probe {
            session: SkinsSession::new(),
            renderer: None,
            out,
            frames: 0,
            layers: rest.iter().any(|a| a == "layers"),
            walk: rest.iter().any(|a| a == "walk"),
            elytra: rest.iter().any(|a| a == "elytra"),
            cape: rest.iter().any(|a| a == "cape"),
            blur: rest.iter().any(|a| a == "blur"),
            started: std::time::Instant::now(),
            cached: None,
            face_quads: None,
            churned: 0,
        },
    )
}
