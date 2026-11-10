//! The head shown on the sign-in button: the active account's skin, its head drawn by the engine on the GPU
//! (`skin_gpu::HeadRenderer`) and shown as an image. The skin itself is downloaded here (network work), decoded, and
//! kept in memory; drawing, and drawing again after a graphics-device change, is the renderer's business.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

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

/// A head renderer, the engine layer it lives on, and when it was last asked for a picture.
struct Head {
    renderer: skin_gpu::HeadRenderer,
    layer: u32,
    last_frame: u64,
    last_used: Instant,
}

/// Renderers nobody has asked for in this long are released, so skins that were on screen once do not hold GPU memory and one of the
/// few layers for the rest of the session.
const IDLE_RELEASE: Duration = Duration::from_secs(120);

pub struct AvatarHeads {
    skins: HashMap<String, Skin>,
    tx: mpsc::Sender<(String, Result<RgbaImage, String>)>,
    rx: mpsc::Receiver<(String, Result<RgbaImage, String>)>,
    /// One renderer (and target) per skin; each lives on a layer of its own so heads do not draw into each other.
    renderers: HashMap<String, Head>,
    /// Counts `pump` calls, i.e. frames: which renderers were used this frame.
    frame: u64,
    last_trim: Instant,
    /// The renderer failed: the flat avatar picture is used until the pause is over, then drawing is tried again. A graphics hiccup
    /// must not cost the heads for the rest of the session.
    failure: Option<HeadFailure>,
    failures_in_a_row: u32,
}

struct HeadFailure {
    retry_at: Instant,
}

impl Default for AvatarHeads {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            skins: HashMap::new(),
            tx,
            rx,
            renderers: HashMap::new(),
            frame: 0,
            last_trim: Instant::now(),
            failure: None,
            failures_in_a_row: 0,
        }
    }
}

impl AvatarHeads {
    /// Takes in skins that finished downloading. Call once per frame.
    pub fn pump(&mut self) {
        self.frame = self.frame.wrapping_add(1);
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
        if let Some(failure) = &self.failure {
            if Instant::now() < failure.retry_at {
                return None;
            }
            self.failure = None;
        }
        self.release_idle(engine);
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
            let layer = self.free_layer(engine)?;
            self.renderers.insert(
                skin_url.to_owned(),
                Head { renderer: skin_gpu::HeadRenderer::new(layer), layer, last_frame: self.frame, last_used: Instant::now() },
            );
        }
        let head = self.renderers.get_mut(skin_url)?;
        head.last_frame = self.frame;
        head.last_used = Instant::now();
        match head.renderer.render(engine, &skin, size, generation) {
            Ok(Some(handle)) => {
                self.failures_in_a_row = 0;
                Some(handle)
            }
            Ok(None) => None,
            Err(err) => {
                self.failures_in_a_row = self.failures_in_a_row.saturating_add(1);
                // 2 s, 4 s ... up to a minute.
                let pause = Duration::from_secs((2_u64 << self.failures_in_a_row.saturating_sub(1).min(5)).min(60));
                tracing::warn!(
                    target: "vertexlauncher/se5/avatar",
                    "The head avatar could not be drawn ({err}); using the flat picture and trying again in {}s (failure {} in a row).",
                    pause.as_secs(),
                    self.failures_in_a_row
                );
                for (_, mut head) in self.renderers.drain() {
                    head.renderer.destroy(engine);
                }
                self.failure = Some(HeadFailure { retry_at: Instant::now() + pause });
                None
            }
        }
    }

    /// An engine layer no renderer is using. When all are taken, the one asked for least recently (and not this frame) is released
    /// and its layer reused; `None` only when every layer is in use by a head drawn this very frame.
    fn free_layer(&mut self, engine: &mut Engine<'_>) -> Option<u32> {
        let taken: Vec<u32> = self.renderers.values().map(|head| head.layer).collect();
        if let Some(layer) = skin_gpu::HEAD_LAYERS.clone().find(|layer| !taken.contains(layer)) {
            return Some(layer);
        }
        let oldest = self
            .renderers
            .iter()
            .filter(|(_, head)| head.last_frame != self.frame)
            .min_by_key(|(_, head)| head.last_used)
            .map(|(url, _)| url.clone())?;
        let mut head = self.renderers.remove(&oldest)?;
        head.renderer.destroy(engine);
        Some(head.layer)
    }

    /// Releases renderers that have not been asked for in a while. Checked at most every ten seconds.
    fn release_idle(&mut self, engine: &mut Engine<'_>) {
        if self.last_trim.elapsed() < Duration::from_secs(10) {
            return;
        }
        self.last_trim = Instant::now();
        let idle: Vec<String> = self
            .renderers
            .iter()
            .filter(|(_, head)| head.last_used.elapsed() > IDLE_RELEASE)
            .map(|(url, _)| url.clone())
            .collect();
        for url in idle {
            if let Some(mut head) = self.renderers.remove(&url) {
                head.renderer.destroy(engine);
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
