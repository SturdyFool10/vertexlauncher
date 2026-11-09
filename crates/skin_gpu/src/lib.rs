//! The skin preview, drawn by the engine on the GPU every frame.
//!
//! The character is a handful of parts (body, head, arms, legs, cape or elytra wings, face). Each part's quads are
//! static for a skin, so each part is one model; the only thing that changes frame to frame is the matrix that places
//! it. `launcher_ui` produces the parts exactly as the CPU preview does (see `skins::gpu_scene`), this crate turns them
//! into engine models and entities, and draws a camera orbiting them into an offscreen target (transparent wherever the character is not) that the UI shows as an
//! image. The skin and cape are engine texture assets whose pixels are replaced in place when they change.
//!
//! Draw with [`PreviewRenderer::render`] from the engine's per-frame hook, then show [`PreviewRenderer::texture`] with
//! `Ui::image`.

mod renderer;

pub use renderer::{HEAD_LAYERS, HeadRenderer, PreviewRenderer, RenderRequest, RenderSettings};

/// The shader that lights the preview: see the file's own comment.
pub const SHADER_SOURCE: &str = include_str!("skin_preview_flat.slang");
