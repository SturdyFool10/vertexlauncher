//! Images for the SE5 UI: instance thumbnails, account avatars, screenshots, icons from the web.
//!
//! Every picture is a managed texture asset of the engine (`Assets::load_texture_managed`). The engine decodes it off the
//! render thread (shrunk to the size it is drawn at), reloads it in place when its file changes or appears, releases
//! pictures nothing draws and brings them back when drawn again, and looks after the graphics device changing. This
//! file only maps the launcher's keys to assets; there is no decoding, uploading, retrying or eviction here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use vertex_shell::sturdy::Engine;
use vertex_shell::sturdy::assets::{Asset, RendererTextureHandle, TextureDetail, TextureLoadOptions};

/// Textures the engine keeps resident at once; beyond this the ones drawn least recently are released (and come back by
/// themselves when drawn).
const RESIDENT_TEXTURES: u64 = 192;

/// Longest edge of a texture made from a picture, whatever size it is drawn at.
const MAX_TEXTURE_EDGE: u32 = 2560;

#[derive(Clone)]
pub enum ImageSource {
    File(PathBuf),
    Bytes(Arc<[u8]>),
    /// Downloaded on a worker (project icons from Modrinth/CurseForge), then handed to the engine as bytes.
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

/// Downloads that failed so far, and when the next try is allowed (a network error is usually temporary).
struct DownloadFailure {
    attempts: u8,
    retry_at: std::time::Instant,
}

const MAX_DOWNLOAD_ATTEMPTS: u8 = 5;
const FIRST_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

type Downloaded = (String, u32, Result<Vec<u8>, String>);

pub struct ImageCache {
    assets: HashMap<String, Asset>,
    /// Downloads in flight, and the ones that failed.
    downloading: HashMap<String, ()>,
    failed_downloads: HashMap<String, DownloadFailure>,
    /// Keys whose picture the engine could not decode (as of the last `get`).
    undecodable: std::collections::HashSet<String>,
    tx: mpsc::Sender<Downloaded>,
    rx: mpsc::Receiver<Downloaded>,
    limit_set: bool,
}

impl Default for ImageCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            assets: HashMap::new(),
            downloading: HashMap::new(),
            failed_downloads: HashMap::new(),
            undecodable: std::collections::HashSet::new(),
            tx,
            rx,
            limit_set: false,
        }
    }
}

fn options(max_dimension: u32) -> TextureLoadOptions {
    let limit = if max_dimension == 0 {
        MAX_TEXTURE_EDGE
    } else {
        max_dimension.clamp(64, MAX_TEXTURE_EDGE)
    };
    TextureLoadOptions {
        max_dimension: limit,
        ..TextureLoadOptions::default()
    }
}

impl ImageCache {
    /// The picture for `key`, starting its load on first use. `max_dimension` is how big it will be drawn at most, so a
    /// large screenshot does not become a large texture for a thumbnail. `None` until the first pixels are there.
    pub fn get(
        &mut self,
        engine: &mut Engine<'_>,
        key: &str,
        source: impl FnOnce() -> ImageSource,
        max_dimension: u32,
    ) -> Option<Picture> {
        if !self.assets.contains_key(key) && !self.downloading.contains_key(key) {
            let retry_blocked = self
                .failed_downloads
                .get(key)
                .is_some_and(|failure| {
                    failure.attempts >= MAX_DOWNLOAD_ATTEMPTS
                        || std::time::Instant::now() < failure.retry_at
                });
            if !retry_blocked {
                match source() {
                    ImageSource::File(path) => {
                        let loaded = engine.assets().load_texture_managed(
                            path.to_string_lossy(),
                            options(max_dimension),
                            key,
                        );
                        match loaded {
                            Ok(asset) => {
                                self.assets.insert(key.to_owned(), asset);
                            }
                            Err(err) => {
                                tracing::debug!(target: "vertexlauncher/se5/images", "Could not load image {key}: {err:?}");
                            }
                        }
                    }
                    ImageSource::Bytes(bytes) => {
                        let loaded = engine.assets().create_texture_managed_from_bytes(
                            key,
                            &bytes,
                            options(max_dimension),
                            key,
                        );
                        match loaded {
                            Ok(asset) => {
                                self.assets.insert(key.to_owned(), asset);
                            }
                            Err(err) => {
                                tracing::debug!(target: "vertexlauncher/se5/images", "Could not load image {key}: {err:?}");
                            }
                        }
                    }
                    ImageSource::Url(url) => {
                        self.downloading.insert(key.to_owned(), ());
                        let tx = self.tx.clone();
                        let key = key.to_owned();
                        // Waiting on the network would tie up one of the engine's workers, so it stays on the blocking pool.
                        launcher_runtime::spawn_blocking_detached(move || {
                            let result = download(&url);
                            let _ = tx.send((key, max_dimension, result));
                        });
                    }
                }
            }
        }
        let asset = *self.assets.get(key)?;
        let mut assets = engine.assets();
        let progress = assets.texture_progress(asset).ok()?;
        if progress.detail == TextureDetail::Failed {
            self.undecodable.insert(key.to_owned());
        } else {
            self.undecodable.remove(key);
        }
        // A texture the engine released keeps the size of its picture (so layouts do not jump) and shows nothing until it
        // is drawn again, which the engine notices; only the 1x1 stand-in of a load that has not produced anything yet is
        // "not there".
        let showing = matches!(progress.detail, TextureDetail::Partial | TextureDetail::Complete)
            || progress.size.x > 1
            || progress.size.y > 1;
        if !showing {
            return None;
        }
        let texture = assets.texture_handle(asset).ok()?;
        Some(Picture {
            texture,
            width: progress.size.x,
            height: progress.size.y,
        })
    }

    /// Whether `key` failed to load (so the caller can draw a placeholder instead of waiting): the engine could not decode
    /// it as of the last `get`, or a download was given up on. A file that failed because it was missing or half written
    /// loads by itself when it appears or finishes, and this turns false again then.
    pub fn failed(&self, key: &str) -> bool {
        self.undecodable.contains(key)
            || self
                .failed_downloads
                .get(key)
                .is_some_and(|failure| failure.attempts >= MAX_DOWNLOAD_ATTEMPTS)
    }

    /// Loads `key` again from its source now. The engine already reloads files that change on their own; this is for when
    /// the caller knows better.
    pub fn invalidate(&mut self, engine: &mut Engine<'_>, key: &str) {
        self.failed_downloads.remove(key);
        if let Some(asset) = self.assets.get(key) {
            let _ = engine.assets().reload(*asset);
        }
    }

    /// Hands finished downloads to the engine. Call once per frame.
    pub fn pump(&mut self, engine: &mut Engine<'_>) {
        if !self.limit_set {
            self.limit_set = true;
            engine
                .assets()
                .set_texture_residency_limit(RESIDENT_TEXTURES);
        }
        while let Ok((key, max_dimension, result)) = self.rx.try_recv() {
            self.downloading.remove(&key);
            match result {
                Ok(bytes) => {
                    self.failed_downloads.remove(&key);
                    match engine.assets().create_texture_managed_from_bytes(
                        &key,
                        &bytes,
                        options(max_dimension),
                        &key,
                    ) {
                        Ok(asset) => {
                            self.assets.insert(key, asset);
                        }
                        Err(err) => {
                            tracing::debug!(target: "vertexlauncher/se5/images", "Could not load image {key}: {err:?}");
                            self.failed_downloads.insert(
                                key,
                                DownloadFailure {
                                    attempts: MAX_DOWNLOAD_ATTEMPTS,
                                    retry_at: std::time::Instant::now(),
                                },
                            );
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(target: "vertexlauncher/se5/images", "Could not download image {key}: {err}");
                    let attempts = self
                        .failed_downloads
                        .get(&key)
                        .map_or(0, |failure| failure.attempts)
                        .saturating_add(1);
                    // Doubling pause: 1 s, 2 s, 4 s, 8 s, then the picture is given up on.
                    let pause = FIRST_RETRY_DELAY * (1_u32 << (attempts - 1).min(8));
                    self.failed_downloads.insert(
                        key,
                        DownloadFailure {
                            attempts,
                            retry_at: std::time::Instant::now() + pause,
                        },
                    );
                }
            }
        }
    }
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
