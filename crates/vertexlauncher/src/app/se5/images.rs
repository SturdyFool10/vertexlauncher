//! Images for the SE5 UI: instance thumbnails, account avatars, screenshots, icons from the web. Decoding runs on a
//! worker with the engine's own decoder (`sturdy::images`), the pixels are uploaded as engine textures on the UI
//! thread, and the least recently used textures are unloaded once the cache is full.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use vertex_shell::sturdy::Engine;
use vertex_shell::sturdy::assets::{
    Asset, RendererTextureHandle, TextureColorSpace, TextureKind, TexturePixelFormat,
};
use vertex_shell::sturdy::images::{DecodeOptions, DecodedImage, PixelFormat, decode};

/// Textures kept alive at once; beyond this the least recently drawn ones are unloaded.
const MAX_TEXTURES: usize = 192;

/// Longest edge of a texture made from a picture; bigger ones are shrunk on the worker first.
const MAX_TEXTURE_EDGE: u32 = 2560;

/// Images at least this wide and tall would be uploaded block-compressed; 0 disables it (compression is synchronous).
const COMPRESS_MIN_DIMENSION: u32 = 0;

/// Longest the UI thread spends uploading decoded images in one frame; the rest wait for the next.
const UPLOAD_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

#[derive(Clone)]
pub enum ImageSource {
    File(PathBuf),
    Bytes(Arc<[u8]>),
    /// Downloaded on the worker (project icons from Modrinth/CurseForge).
    Url(String),
}

/// A texture ready to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub texture: RendererTextureHandle,
    pub width: u32,
    pub height: u32,
}

impl Picture {
    pub fn aspect(&self) -> f32 {
        if self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

enum Slot {
    Loading,
    Failed,
    Ready { asset: Asset, picture: Picture },
}

struct Entry {
    slot: Slot,
    last_used: u64,
    /// Loads that have failed so far, and when the next try is allowed. A failure is often temporary (a screenshot the
    /// game is still writing, a download that timed out), so it is retried with a growing pause instead of for good.
    failures: u8,
    retry_at: Option<std::time::Instant>,
}

/// Tries before a picture is given up on, and the pause after the first failure (doubled after each).
const MAX_LOAD_ATTEMPTS: u8 = 5;
const FIRST_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

type Decoded = (String, Result<DecodedImage, String>);

pub struct ImageCache {
    entries: HashMap<String, Entry>,
    /// Textures that could not be released yet because the engine still referenced them; tried again each frame
    /// rather than forgotten (forgetting them leaks the texture).
    unload_later: Vec<Asset>,
    tx: mpsc::Sender<Decoded>,
    rx: mpsc::Receiver<Decoded>,
    tick: u64,
}

impl Default for ImageCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            unload_later: Vec::new(),
            tx,
            rx,
            tick: 0,
        }
    }
}

impl ImageCache {
    /// The picture for `key`, starting its load on first use. `max_dimension` asks the decoder for a cheap
    /// reduced-size decode when the source is much bigger (thumbnails of large screenshots).
    pub fn get(
        &mut self,
        key: &str,
        source: impl FnOnce() -> ImageSource,
        max_dimension: u32,
    ) -> Option<Picture> {
        let tick = self.tick;
        let mut retrying = None;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = tick;
            match &entry.slot {
                Slot::Ready { picture, .. } => return Some(*picture),
                Slot::Loading => return None,
                Slot::Failed => {
                    let due = entry.failures < MAX_LOAD_ATTEMPTS
                        && entry.retry_at.is_some_and(|at| std::time::Instant::now() >= at);
                    if !due {
                        return None;
                    }
                    entry.slot = Slot::Loading;
                    retrying = Some(());
                }
            }
        }
        if retrying.is_none() {
            self.entries.insert(
                key.to_owned(),
                Entry {
                    slot: Slot::Loading,
                    last_used: tick,
                    failures: 0,
                    retry_at: None,
                },
            );
        }
        let source = source();
        let tx = self.tx.clone();
        let key = key.to_owned();
        // Decoding is CPU work, so it runs on the compute backend (the engine's scheduler). Fetching a URL waits on the
        // network, which would tie up one of the engine's workers, so that stays on the blocking pool.
        if matches!(source, ImageSource::Url(_)) {
            launcher_runtime::spawn_blocking_detached(move || {
                let result = load(&source, &key, max_dimension);
                let _ = tx.send((key, result));
            });
        } else {
            drop(launcher_runtime::spawn_compute(async move {
                let result = load(&source, &key, max_dimension);
                let _ = tx.send((key, result));
            }));
        }
        None
    }

    /// Whether `key` failed to load (so the caller can draw a placeholder instead of waiting).
    pub fn failed(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|entry| {
            matches!(entry.slot, Slot::Failed) && entry.failures >= MAX_LOAD_ATTEMPTS
        })
    }

    /// Drops `key`, so the next `get` loads it again (the file changed).
    pub fn invalidate(&mut self, engine: &mut Engine<'_>, key: &str) {
        if let Some(Entry {
            slot: Slot::Ready { asset, .. },
            ..
        }) = self.entries.remove(key)
        {
            self.release(engine, asset);
        }
    }

    /// Releases a texture, keeping hold of it to try again if the engine still has it in use.
    fn release(&mut self, engine: &mut Engine<'_>, asset: Asset) {
        if engine.assets().unload(asset).is_err() && engine.assets().contains(asset) {
            self.unload_later.push(asset);
        }
    }

    /// Uploads freshly decoded images and unloads the least recently used ones. Call once per frame.
    pub fn pump(&mut self, engine: &mut Engine<'_>) {
        self.tick += 1;
        for asset in std::mem::take(&mut self.unload_later) {
            self.release(engine, asset);
        }
        self.refresh_handles(engine);
        let started = std::time::Instant::now();
        loop {
            if started.elapsed() > UPLOAD_BUDGET {
                // Out of time: the rest stay queued; look again next frame.
                break;
            }
            let Ok((key, result)) = self.rx.try_recv() else {
                break;
            };
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            entry.slot = match result.and_then(|image| upload(engine, &key, &image)) {
                Ok((asset, picture)) => {
                    entry.failures = 0;
                    entry.retry_at = None;
                    Slot::Ready { asset, picture }
                }
                Err(err) => {
                    tracing::debug!(target: "vertexlauncher/se5/images", "Could not load image {key}: {err}");
                    entry.failures = entry.failures.saturating_add(1);
                    // Doubling pause: 1 s, 2 s, 4 s, 8 s, then the picture is given up on.
                    let pause = FIRST_RETRY_DELAY * (1_u32 << (entry.failures - 1).min(8));
                    entry.retry_at = Some(std::time::Instant::now() + pause);
                    Slot::Failed
                }
            };
        }
        self.trim(engine);
    }

    /// Something is still loading; the UI should look again soon.
    pub fn loading(&self) -> bool {
        self.entries
            .values()
            .any(|entry| matches!(entry.slot, Slot::Loading))
    }

    /// Makes sure every picture drawn recently still points at a live texture. The engine replaces its texture handles
    /// when the graphics device is switched (a setting change, a lost device), and a stale handle draws as a blank
    /// white box, so the handle is read again from the asset each frame, and a picture whose asset is gone is dropped
    /// so it loads again.
    fn refresh_handles(&mut self, engine: &mut Engine<'_>) {
        let in_use_since = self.tick.saturating_sub(2);
        let mut gone = Vec::new();
        for (key, entry) in &mut self.entries {
            let Slot::Ready { asset, picture } = &mut entry.slot else {
                continue;
            };
            if entry.last_used < in_use_since {
                continue;
            }
            match engine.assets().texture_handle(*asset) {
                Ok(handle) => picture.texture = handle,
                Err(_) => gone.push(key.clone()),
            }
        }
        for key in gone {
            self.entries.remove(&key);
        }
    }

    fn trim(&mut self, engine: &mut Engine<'_>) {
        let ready = self
            .entries
            .values()
            .filter(|entry| matches!(entry.slot, Slot::Ready { .. }))
            .count();
        if ready <= MAX_TEXTURES {
            return;
        }
        // Only pictures nothing drew last frame can go. A screen showing more than the limit at once would otherwise
        // have its own pictures unloaded out from under it, drawn blank, and reloaded the next frame, over and over:
        // the limit is soft, and the cache stays above it while that many are in view.
        let in_use_since = self.tick.saturating_sub(1);
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(entry.slot, Slot::Ready { .. }) && entry.last_used < in_use_since
            })
            .map(|(key, entry)| (entry.last_used, key.clone()))
            .collect();
        by_age.sort();
        for (_, key) in by_age.into_iter().take(ready - MAX_TEXTURES) {
            if let Some(Entry {
                slot: Slot::Ready { asset, .. },
                ..
            }) = self.entries.remove(&key)
            {
                self.release(engine, asset);
            }
        }
    }
}

fn load(source: &ImageSource, key: &str, max_dimension: u32) -> Result<DecodedImage, String> {
    let bytes: std::borrow::Cow<'_, [u8]> = match source {
        ImageSource::File(path) => std::borrow::Cow::Owned(
            match vertex_shell::sturdy::images::read_file_accelerated(path) {
                Some(bytes) => bytes,
                None => std::fs::read(path).map_err(|err| err.to_string())?,
            },
        ),
        ImageSource::Bytes(bytes) => std::borrow::Cow::Borrowed(bytes),
        ImageSource::Url(url) => std::borrow::Cow::Owned(download(url)?),
    };
    let options = DecodeOptions {
        preview_min_dimension: max_dimension,
        ..DecodeOptions::default()
    };
    let image = decode(&bytes, options, key).map_err(|err| err.to_string())?;
    if image.format != PixelFormat::Rgba8 {
        return Err("unsupported pixel format".to_owned());
    }
    // A texture built from fewer pixels than its size says would show garbage or white.
    let wanted = image.width as usize * image.height as usize * 4;
    if image.width == 0 || image.height == 0 || image.pixels().len() != wanted {
        return Err("the decoded picture is empty or incomplete".to_owned());
    }
    Ok(fit_texture(image, max_dimension))
}

/// Shrinks pictures bigger than they will be shown on the worker, so the texture the UI thread has to create and mip is
/// a sane size: to `wanted` on the longest edge when the caller said how big it will be drawn (a thumbnail; the decoder
/// only skips work for some formats, a PNG screenshot still arrives full size), and in any case to `MAX_TEXTURE_EDGE`.
/// Creating a full-size texture took over 100 ms of a UI frame per screenshot tile.
fn fit_texture(mut image: DecodedImage, wanted: u32) -> DecodedImage {
    let limit = if wanted == 0 { MAX_TEXTURE_EDGE } else { wanted.clamp(64, MAX_TEXTURE_EDGE) };
    let longest = image.width.max(image.height);
    if longest <= limit {
        return image;
    }
    let scale = limit as f32 / longest as f32;
    let (width, height) = (
        ((image.width as f32 * scale).round() as u32).max(1),
        ((image.height as f32 * scale).round() as u32).max(1),
    );
    let Some(source) =
        image::RgbaImage::from_raw(image.width, image.height, image.frames[0].pixels.clone())
    else {
        return image;
    };
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    image.frames.truncate(1);
    image.frames[0].pixels = resized.into_raw();
    image.width = width;
    image.height = height;
    image
}

fn upload(
    engine: &mut Engine<'_>,
    key: &str,
    image: &DecodedImage,
) -> Result<(Asset, Picture), String> {
    let mut assets = engine.assets();
    let asset = assets
        .create_texture(
            image.pixels(),
            image.width,
            image.height,
            TexturePixelFormat::Rgba8,
            TextureColorSpace::Srgb,
            TextureKind::ColorAlpha,
            key,
            // The engine compresses on the calling thread, which stalled the UI with a screenshot grid; opt-in only.
            COMPRESS_MIN_DIMENSION != 0 && image.width.min(image.height) >= COMPRESS_MIN_DIMENSION,
            true,
        )
        .map_err(|err| format!("{err:?}"))?;
    let texture = assets
        .texture_handle(asset)
        .map_err(|err| format!("{err:?}"))?;
    Ok((
        asset,
        Picture {
            texture,
            width: image.width,
            height: image.height,
        },
    ))
}

/// Largest remote image accepted; project icons and banners are far below this.
const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024;

fn download(url: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to download {url}: {err}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_DOWNLOAD_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read {url}: {err}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_screenshot_decodes_at_full_size() {
        let dir = std::env::temp_dir().join("vertex-image-probe");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.png");
        image::RgbImage::from_fn(3840, 2160, |x, y| {
            image::Rgb([(x % 255) as u8, (y % 255) as u8, 90])
        })
        .save(&path)
        .unwrap();
        let started = std::time::Instant::now();
        let decoded = load(&ImageSource::File(path.clone()), "probe", 0).expect("decodes");
        println!("full decode took {:?}", started.elapsed());
        assert_eq!((decoded.width, decoded.height), (2560, 1440));
        let _ = std::fs::remove_file(path);
    }
}
