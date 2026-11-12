//! Draws the "Random" cape tile's picture (a cape face with the dice as a negative filter) in a hidden window, twice with
//! different capes, and writes both to PNGs: `cargo run -p skin_gpu --example dice_probe -- /tmp/dice.png`.
//! It also reports whether both draws used the very same target.

use std::path::PathBuf;
use std::sync::Arc;

use image::{Rgba, RgbaImage};
use skin_gpu::CapeDice;
use sturdy::glam::UVec2;
use sturdy::prelude::*;

struct Probe {
    dice: CapeDice,
    out: PathBuf,
    frames: u32,
    first_target: Option<String>,
}

/// A 70 x 112 cape face: a horizontal gradient with a vertical stripe, so an inverted dice stands out against it.
fn cape_face(base: [u8; 3]) -> RgbaImage {
    let mut image = RgbaImage::new(70, 112);
    for y in 0..112u32 {
        for x in 0..70u32 {
            let stripe = (x / 7) % 2 == 0;
            let shade = (y * 255 / 111) as u8;
            let pixel = if stripe { [base[0], base[1], base[2]] } else { [shade, 255 - shade, base[2] / 2] };
            image.put_pixel(x, y, Rgba([pixel[0], pixel[1], pixel[2], 255]));
        }
    }
    image
}

impl GameLogic for Probe {
    fn init(&mut self, _engine: &mut Engine<'_>) -> Result<(), Error> {
        Ok(())
    }

    fn frame(&mut self, engine: &mut Engine<'_>, _frame: &Frame) -> Option<FrameDescription> {
        let (key, face) = if self.frames < 60 { (1u64, cape_face([200, 60, 60])) } else { (2u64, cape_face([40, 120, 220])) };
        match self.dice.render(engine, key, &Arc::new(face)) {
            Ok(Some(handle)) => {
                let name = format!("{handle:?}");
                match &self.first_target {
                    None => self.first_target = Some(name.clone()),
                    Some(first) if *first != name => eprintln!("[dice] THE TARGET CHANGED: {first} -> {name}"),
                    _ => {}
                }
                for (frame, suffix) in [(55, "a"), (115, "b")] {
                    if self.frames == frame {
                        if let Some(image) = read_back_texture(engine, handle, UVec2::new(140, 224)) {
                            let path = self.out.with_extension(format!("{suffix}.png"));
                            image.save(&path).expect("write png");
                            eprintln!("[dice] wrote {} (target {name})", path.display());
                        } else {
                            eprintln!("[dice] could not read the target back");
                        }
                    }
                }
            }
            Ok(None) => eprintln!("[dice] frame {}: no picture", self.frames),
            Err(err) => eprintln!("[dice] frame {}: render failed: {err}", self.frames),
        }
        self.frames += 1;
        if self.frames == 120 {
            self.dice.release(engine);
            if let Some(mut window) = engine.primary_window_control() {
                window.request_close();
            }
        }
        Some(FrameDescription::ui_only())
    }
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

fn main() -> std::process::ExitCode {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "cape_dice_probe.png".to_owned()));
    sturdy::run(
        RuntimeConfig::new("cape dice probe").size(200, 260).visible(false),
        Probe { dice: CapeDice::new(), out, frames: 0, first_target: None },
    )
}
