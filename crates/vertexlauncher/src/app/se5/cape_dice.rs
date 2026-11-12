//! The picture on the skins screen's "Random" tile: one of the player's capes, a different one every second, with the dice
//! icon drawn over it as a negative filter. The GPU work is `skin_gpu::CapeDice` (one target and one texture, overwritten
//! each time); this decides which cape is shown and when, and keeps the decoded cape faces.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::RgbaImage;
use launcher_ui::screens::skins_api::CapeView;
use vertex_shell::sturdy::Engine;
use vertex_shell::sturdy::assets::RendererTextureHandle;

/// How long each cape is shown.
const SHOW_FOR: Duration = Duration::from_secs(1);
/// After the GPU picture failed, wait this long before trying again (the tile shows the plain dice meanwhile).
const RETRY_AFTER: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct CapeDiceFx {
    renderer: skin_gpu::CapeDice,
    /// Decoded cape faces by cape id, with the size of the PNG they came from (to notice a changed one).
    faces: HashMap<String, (usize, Arc<RgbaImage>)>,
    current: Option<String>,
    picked_at: Option<Instant>,
    failed_at: Option<Instant>,
}

impl CapeDiceFx {
    /// The picture for this frame, or `None` when there is none to show (no cape has a picture, or the GPU could not make it).
    pub fn texture(&mut self, engine: &mut Engine<'_>, capes: &[CapeView]) -> Option<RendererTextureHandle> {
        if self.failed_at.is_some_and(|at| at.elapsed() < RETRY_AFTER) {
            return None;
        }
        let candidates: Vec<&CapeView> = capes.iter().filter(|cape| cape.face_png.is_some()).collect();
        if candidates.is_empty() {
            return None;
        }
        let still_there = self.current.as_deref().is_some_and(|id| candidates.iter().any(|cape| cape.id == id));
        if !still_there || self.picked_at.is_none_or(|at| at.elapsed() >= SHOW_FOR) {
            let others: Vec<&&CapeView> = candidates.iter().filter(|cape| Some(cape.id.as_str()) != self.current.as_deref()).collect();
            let pool = if others.is_empty() { candidates.iter().collect() } else { others };
            // No random-number crate for one pick a second: the clock's nanoseconds are random enough.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.subsec_nanos() as usize);
            self.current = Some(pool[nanos % pool.len()].id.clone());
            self.picked_at = Some(Instant::now());
        }
        let cape = candidates.iter().find(|cape| Some(cape.id.as_str()) == self.current.as_deref())?;
        let png = cape.face_png.as_ref()?;
        let face = match self.faces.get(&cape.id) {
            Some((len, face)) if *len == png.len() => Arc::clone(face),
            _ => {
                let face = Arc::new(image::load_from_memory(png).ok()?.to_rgba8());
                self.faces.insert(cape.id.clone(), (png.len(), Arc::clone(&face)));
                face
            }
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&cape.id, png.len()).hash(&mut hasher);
        match self.renderer.render(engine, hasher.finish(), &face) {
            Ok(texture) => texture,
            Err(err) => {
                tracing::warn!(target: "vertexlauncher/se5/cape_dice", "The random cape picture could not be drawn: {err}");
                self.failed_at = Some(Instant::now());
                None
            }
        }
    }

    /// Releases the GPU objects (the engine is shutting down or the screen is gone).
    pub fn release(&mut self, engine: &mut Engine<'_>) {
        self.renderer.release(engine);
    }
}
