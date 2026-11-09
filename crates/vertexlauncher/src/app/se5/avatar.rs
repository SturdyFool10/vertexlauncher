//! The head shown on the sign-in button: the active account's skin, its head drawn by the engine on the GPU
//! (`skin_gpu::HeadRenderer`) and shown as an image. The skin itself is downloaded here (network work), decoded, and
//! kept in memory; drawing, and drawing again after a graphics-device change, is the renderer's business.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};

use image::RgbaImage;
use vertex_shell::sturdy::Engine;
use vertex_shell::sturdy::assets::RendererTextureHandle;
use vertex_shell::sturdy::glam::UVec2;

/// Largest skin file accepted; real skins are a few kilobytes.
const MAX_SKIN_BYTES: u64 = 2 * 1024 * 1024;

enum Skin {
    Loading,
    Ready(Arc<RgbaImage>),
    /// Failed; tried again at `retry_at` until `attempts` runs out.
    Failed { attempts: u8, retry_at: std::time::Instant },
}

const MAX_ATTEMPTS: u8 = 4;

pub struct AvatarHeads {
    skins: HashMap<String, Skin>,
    tx: mpsc::Sender<(String, Result<RgbaImage, String>)>,
    rx: mpsc::Receiver<(String, Result<RgbaImage, String>)>,
    /// One renderer (and target) per skin; each lives on a layer of its own so heads do not draw into each other.
    renderers: HashMap<String, skin_gpu::HeadRenderer>,
    /// The renderer could not start; the flat avatar picture is used instead.
    unavailable: bool,
}

impl Default for AvatarHeads {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            skins: HashMap::new(),
            tx,
            rx,
            renderers: HashMap::new(),
            unavailable: false,
        }
    }
}

impl AvatarHeads {
    /// Takes in skins that finished downloading. Call once per frame.
    pub fn pump(&mut self) {
        while let Ok((url, result)) = self.rx.try_recv() {
            let previous_attempts = match self.skins.get(&url) {
                Some(Skin::Failed { attempts, .. }) => *attempts,
                _ => 0,
            };
            let skin = match result {
                Ok(image) => Skin::Ready(Arc::new(image)),
                Err(err) => {
                    tracing::debug!(target: "vertexlauncher/se5/avatar", "Could not load the skin {url}: {err}");
                    let attempts = previous_attempts.saturating_add(1);
                    Skin::Failed {
                        attempts,
                        retry_at: std::time::Instant::now()
                            + std::time::Duration::from_secs(2_u64 << attempts.min(6)),
                    }
                }
            };
            self.skins.insert(url, skin);
        }
    }

    /// The head of the skin at `skin_url`, `size` pixels, or `None` while the skin is loading or if it cannot be drawn
    /// (the caller shows the flat avatar picture then).
    pub fn head(
        &mut self,
        engine: &mut Engine<'_>,
        skin_url: &str,
        size: UVec2,
    ) -> Option<RendererTextureHandle> {
        if self.unavailable {
            return None;
        }
        let start = match self.skins.get(skin_url) {
            None => true,
            Some(Skin::Failed { attempts, retry_at }) => {
                *attempts < MAX_ATTEMPTS && std::time::Instant::now() >= *retry_at
            }
            _ => false,
        };
        if start {
            let attempts = match self.skins.get(skin_url) {
                Some(Skin::Failed { attempts, .. }) => *attempts,
                _ => 0,
            };
            // Keep the failure count while it loads again, so the pause keeps growing.
            self.skins.insert(
                skin_url.to_owned(),
                if attempts == 0 {
                    Skin::Loading
                } else {
                    Skin::Failed { attempts, retry_at: std::time::Instant::now() + std::time::Duration::from_secs(3600) }
                },
            );
            let tx = self.tx.clone();
            let url = skin_url.to_owned();
            launcher_runtime::spawn_blocking_detached(move || {
                let result = download(&url);
                let _ = tx.send((url, result));
            });
        }
        let Some(Skin::Ready(skin)) = self.skins.get(skin_url) else {
            return None;
        };
        let skin = Arc::clone(skin);
        let generation = engine.device_generation();
        if !self.renderers.contains_key(skin_url) {
            // Layers are few; heads beyond them fall back to the flat picture.
            let used = self.renderers.len() as u32;
            let layer = skin_gpu::HEAD_LAYERS.start() + used;
            if layer > *skin_gpu::HEAD_LAYERS.end() {
                return None;
            }
            self.renderers.insert(skin_url.to_owned(), skin_gpu::HeadRenderer::new(layer));
        }
        let renderer = self.renderers.get_mut(skin_url)?;
        match renderer.render(engine, &skin, size, generation) {
            Ok(handle) => handle,
            Err(err) => {
                tracing::warn!(target: "vertexlauncher/se5/avatar", "The head avatar could not be drawn ({err}); using the flat picture.");
                self.unavailable = true;
                for (_, mut renderer) in self.renderers.drain() {
                    renderer.destroy(engine);
                }
                None
            }
        }
    }
}

fn download(url: &str) -> Result<RgbaImage, String> {
    use std::io::Read;
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to download {url}: {err}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_SKIN_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read {url}: {err}"))?;
    let skin = image::load_from_memory(&bytes)
        .map_err(|err| format!("not a skin: {err}"))?
        .to_rgba8();
    if skin.width() < 64 || skin.height() < 16 {
        return Err("the skin texture is smaller than expected".to_owned());
    }
    Ok(skin)
}
