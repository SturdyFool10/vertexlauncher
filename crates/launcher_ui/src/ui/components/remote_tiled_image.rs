use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use egui::Ui;
use image::ImageFormat;

use crate::app::tokio_runtime;

const TILE_MAX_DIM: u32 = 4096;

enum RemoteImageState {
    Loading,
    Ready(TiledImage),
    Failed,
}

struct TiledImage {
    width: u32,
    height: u32,
    tiles: Vec<TileImage>,
}

struct TileImage {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    uri: String,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct RemoteImageCache {
    states: HashMap<String, RemoteImageState>,
    tx: Option<mpsc::Sender<(String, Result<TiledImage, String>)>>,
    rx: Option<Arc<Mutex<mpsc::Receiver<(String, Result<TiledImage, String>)>>>>,
}

fn cache() -> &'static Mutex<RemoteImageCache> {
    static CACHE: OnceLock<Mutex<RemoteImageCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RemoteImageCache::default()))
}

fn ensure_channel(cache: &mut RemoteImageCache) {
    if cache.tx.is_some() && cache.rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(String, Result<TiledImage, String>)>();
    cache.tx = Some(tx);
    cache.rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_updates(cache: &mut RemoteImageCache) -> bool {
    let mut updates = Vec::new();
    let mut should_reset = false;
    if let Some(rx) = cache.rx.as_ref() {
        match rx.lock() {
            Ok(receiver) => loop {
                match receiver.try_recv() {
                    Ok(update) => updates.push(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        should_reset = true;
                        break;
                    }
                }
            },
            Err(_) => should_reset = true,
        }
    }

    if should_reset {
        cache.tx = None;
        cache.rx = None;
    }

    let mut did_update = false;
    for (url, result) in updates {
        match result {
            Ok(image) => {
                cache.states.insert(url, RemoteImageState::Ready(image));
            }
            Err(_) => {
                cache.states.insert(url, RemoteImageState::Failed);
            }
        }
        did_update = true;
    }
    did_update
}

pub fn show(
    ui: &mut Ui,
    url: &str,
    desired_size: egui::Vec2,
    id_source: impl Hash,
    placeholder_svg: &[u8],
) {
    let normalized_url = url.trim();
    if normalized_url.is_empty() {
        show_placeholder(ui, desired_size, id_source, placeholder_svg);
        return;
    }

    let mut render_ready: Option<TiledImage> = None;
    {
        let Ok(mut cache) = cache().lock() else {
            show_placeholder(ui, desired_size, id_source, placeholder_svg);
            return;
        };
        let mut request_follow_up_repaint = poll_updates(&mut cache);

        match cache.states.get(normalized_url) {
            Some(RemoteImageState::Ready(image)) => {
                render_ready = Some(image.clone_for_render());
            }
            Some(RemoteImageState::Loading) => {
                request_follow_up_repaint = true;
            }
            Some(RemoteImageState::Failed) => {
                show_placeholder(ui, desired_size, id_source, placeholder_svg);
                return;
            }
            None => {
                ensure_channel(&mut cache);
                let Some(tx) = cache.tx.as_ref().cloned() else {
                    show_placeholder(ui, desired_size, id_source, placeholder_svg);
                    return;
                };
                cache
                    .states
                    .insert(normalized_url.to_owned(), RemoteImageState::Loading);
                request_follow_up_repaint = true;
                let url_owned = normalized_url.to_owned();
                let _ = tokio_runtime::spawn(async move {
                    let url_for_worker = url_owned.clone();
                    let result = tokio_runtime::spawn_blocking(move || {
                        fetch_and_tile_remote_image(url_for_worker.as_str())
                    })
                    .await
                    .map_err(|err| format!("remote icon worker join failed: {err}"))
                    .and_then(|inner| inner);
                    let _ = tx.send((url_owned, result));
                });
            }
        }

        if request_follow_up_repaint {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    if let Some(image) = render_ready {
        show_tiled(ui, &image, desired_size);
    } else {
        show_placeholder(ui, desired_size, id_source, placeholder_svg);
    }
}

impl TiledImage {
    fn clone_for_render(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            tiles: self
                .tiles
                .iter()
                .map(|tile| TileImage {
                    x: tile.x,
                    y: tile.y,
                    width: tile.width,
                    height: tile.height,
                    uri: tile.uri.clone(),
                    bytes: tile.bytes.clone(),
                })
                .collect(),
        }
    }
}

fn show_tiled(ui: &mut Ui, image: &TiledImage, desired_size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let width = image.width.max(1) as f32;
    let height = image.height.max(1) as f32;

    for tile in &image.tiles {
        let min_x = rect.left() + rect.width() * (tile.x as f32 / width);
        let min_y = rect.top() + rect.height() * (tile.y as f32 / height);
        let max_x = rect.left() + rect.width() * ((tile.x + tile.width) as f32 / width);
        let max_y = rect.top() + rect.height() * ((tile.y + tile.height) as f32 / height);
        let tile_rect =
            egui::Rect::from_min_max(egui::pos2(min_x, min_y), egui::pos2(max_x, max_y));
        let image = egui::Image::from_bytes(tile.uri.clone(), tile.bytes.clone())
            .fit_to_exact_size(tile_rect.size());
        let _ = ui.put(tile_rect, image);
    }
}

fn show_placeholder(
    ui: &mut Ui,
    desired_size: egui::Vec2,
    id_source: impl Hash,
    placeholder_svg: &[u8],
) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id_source.hash(&mut hasher);
    ui.add(
        egui::Image::from_bytes(
            format!("bytes://remote-placeholder/{}", hasher.finish()),
            placeholder_svg.to_vec(),
        )
        .fit_to_exact_size(desired_size),
    );
}

fn fetch_and_tile_remote_image(url: &str) -> Result<TiledImage, String> {
    let response = ureq::get(url)
        .call()
        .map_err(|err| format!("failed to fetch remote icon {url}: {err}"))?;
    let mut reader = response.into_body().into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read remote icon bytes {url}: {err}"))?;
    let image = image::load_from_memory(bytes.as_slice())
        .map_err(|err| format!("failed to decode remote icon {url}: {err}"))?;

    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err("remote icon has invalid dimensions".to_owned());
    }

    let mut tiles = Vec::new();
    for y in (0..height).step_by(TILE_MAX_DIM as usize) {
        for x in (0..width).step_by(TILE_MAX_DIM as usize) {
            let tile_width = (width - x).min(TILE_MAX_DIM);
            let tile_height = (height - y).min(TILE_MAX_DIM);
            let tile_image = image.crop_imm(x, y, tile_width, tile_height);
            let mut cursor = std::io::Cursor::new(Vec::new());
            tile_image
                .write_to(&mut cursor, ImageFormat::Png)
                .map_err(|err| format!("failed to encode tiled icon {url}: {err}"))?;
            let tile_bytes = cursor.into_inner();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            url.hash(&mut hasher);
            x.hash(&mut hasher);
            y.hash(&mut hasher);
            tile_width.hash(&mut hasher);
            tile_height.hash(&mut hasher);
            let uri = format!("bytes://remote-tile/{:x}.png", hasher.finish());
            tiles.push(TileImage {
                x,
                y,
                width: tile_width,
                height: tile_height,
                uri,
                bytes: tile_bytes,
            });
        }
    }

    Ok(TiledImage {
        width,
        height,
        tiles,
    })
}
