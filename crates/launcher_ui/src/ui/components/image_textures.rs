use std::{
    collections::{HashSet, VecDeque},
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use shared_lru::ThreadSafeLru;

use crate::app::tokio_runtime;

const IMAGE_TEXTURE_MAX_BYTES: usize = 128 * 1024 * 1024;
const IMAGE_TEXTURE_STALE_FRAMES: u64 = 600;
const IMAGE_TEXTURE_FETCH_MAX_PER_FRAME: usize = 128;
const IMAGE_TEXTURE_UPLOAD_MAX_PER_FRAME: usize = 32;
const IMAGE_TEXTURE_UPLOAD_MAX_BYTES_PER_FRAME: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ManagedTextureKey {
    source_key: String,
    options: TextureOptions,
}

#[derive(Clone)]
enum ManagedTextureState {
    Loading,
    Ready(TextureHandle),
    Failed,
}

#[derive(Clone)]
struct ManagedTextureEntry {
    state: ManagedTextureState,
    last_touched_frame: u64,
}

struct ReadyTextureUpload {
    key: ManagedTextureKey,
    image: ColorImage,
    approx_bytes: usize,
}

struct ManagedTextureCache {
    entries: ThreadSafeLru<ManagedTextureKey, ManagedTextureEntry>,
    frame_index: u64,
    pending: HashSet<ManagedTextureKey>,
    ready: VecDeque<(ManagedTextureKey, Result<ReadyTextureUpload, String>)>,
    tx: Option<mpsc::Sender<(ManagedTextureKey, Result<ReadyTextureUpload, String>)>>,
    rx: Option<Arc<Mutex<mpsc::Receiver<(ManagedTextureKey, Result<ReadyTextureUpload, String>)>>>>,
}

#[derive(Clone)]
pub enum ManagedTextureStatus {
    Loading,
    Ready(TextureHandle),
    Failed,
}

impl Default for ManagedTextureCache {
    fn default() -> Self {
        Self {
            entries: ThreadSafeLru::new(IMAGE_TEXTURE_MAX_BYTES),
            frame_index: 0,
            pending: HashSet::new(),
            ready: VecDeque::new(),
            tx: None,
            rx: None,
        }
    }
}

fn cache() -> &'static Mutex<ManagedTextureCache> {
    static CACHE: OnceLock<Mutex<ManagedTextureCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ManagedTextureCache::default()))
}

pub fn begin_frame(ctx: &Context) {
    let Ok(mut cache) = cache().lock() else {
        tracing::error!(
            target: "vertexlauncher/image_textures",
            "Managed image texture cache mutex was poisoned."
        );
        return;
    };

    cache.frame_index = cache.frame_index.saturating_add(1);
    poll_updates(ctx, &mut cache);
    trim_stale(&mut cache);
    trim_to_budget(&mut cache);

    if !cache.pending.is_empty() || !cache.ready.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

pub fn request_texture(
    ctx: &Context,
    source_key: impl Into<String>,
    bytes: Arc<[u8]>,
    options: TextureOptions,
) -> ManagedTextureStatus {
    let key = ManagedTextureKey {
        source_key: source_key.into(),
        options,
    };
    let Ok(mut cache) = cache().lock() else {
        tracing::error!(
            target: "vertexlauncher/image_textures",
            texture_key = %key.source_key,
            "Managed image texture cache mutex was poisoned."
        );
        return ManagedTextureStatus::Failed;
    };

    if let Some(entry) = cache.entries.write(|state| {
        let entry = state.touch(&key)?;
        entry.value.last_touched_frame = cache.frame_index;
        Some(entry.value.clone())
    }) {
        return match entry.state {
            ManagedTextureState::Loading => ManagedTextureStatus::Loading,
            ManagedTextureState::Ready(texture) => ManagedTextureStatus::Ready(texture),
            ManagedTextureState::Failed => ManagedTextureStatus::Failed,
        };
    }

    ensure_channel(&mut cache);
    let Some(tx) = cache.tx.as_ref().cloned() else {
        return ManagedTextureStatus::Failed;
    };

    cache.entries.write(|state| {
        state.insert_without_eviction(
            key.clone(),
            ManagedTextureEntry {
                state: ManagedTextureState::Loading,
                last_touched_frame: cache.frame_index,
            },
            0,
        );
    });
    cache.pending.insert(key.clone());

    tokio_runtime::spawn_blocking_detached(move || {
        let result = decode_ready_texture(key.clone(), bytes.as_ref());
        if let Err(err) = tx.send((key.clone(), result)) {
            tracing::error!(
                target: "vertexlauncher/image_textures",
                texture_key = %key.source_key,
                error = %err,
                "Failed to deliver managed image texture result."
            );
        }
    });
    ctx.request_repaint_after(Duration::from_millis(16));
    ManagedTextureStatus::Loading
}

pub fn evict_source_key(source_key: &str) {
    let Ok(mut cache) = cache().lock() else {
        tracing::error!(
            target: "vertexlauncher/image_textures",
            texture_key = %source_key,
            "Managed image texture cache mutex was poisoned."
        );
        return;
    };

    cache.pending.retain(|key| key.source_key != source_key);
    cache.ready.retain(|(key, _)| key.source_key != source_key);
    let _ = cache
        .entries
        .write(|state| state.retain(|key, _| key.source_key != source_key));
}

fn ensure_channel(cache: &mut ManagedTextureCache) {
    if cache.tx.is_some() && cache.rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<(ManagedTextureKey, Result<ReadyTextureUpload, String>)>();
    cache.tx = Some(tx);
    cache.rx = Some(Arc::new(Mutex::new(rx)));
}

fn poll_updates(ctx: &Context, cache: &mut ManagedTextureCache) {
    let Some(rx) = cache.rx.as_ref() else {
        return;
    };
    let mut should_reset_channel = false;
    match rx.lock() {
        Ok(receiver) => {
            for _ in 0..IMAGE_TEXTURE_FETCH_MAX_PER_FRAME {
                match receiver.try_recv() {
                    Ok(update) => cache.ready.push_back(update),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        should_reset_channel = true;
                        break;
                    }
                }
            }
        }
        Err(_) => should_reset_channel = true,
    }

    let mut uploaded_count = 0usize;
    let mut uploaded_bytes = 0usize;
    while uploaded_count < IMAGE_TEXTURE_UPLOAD_MAX_PER_FRAME
        && uploaded_bytes < IMAGE_TEXTURE_UPLOAD_MAX_BYTES_PER_FRAME
    {
        let Some((key, result)) = cache.ready.pop_front() else {
            break;
        };
        cache.pending.remove(&key);
        let is_loading = cache.entries.read(|state| {
            matches!(
                state.get(&key).map(|entry| &entry.value.state),
                Some(ManagedTextureState::Loading)
            )
        });
        if !is_loading {
            continue;
        }

        match result {
            Ok(upload) => {
                uploaded_count = uploaded_count.saturating_add(1);
                uploaded_bytes = uploaded_bytes.saturating_add(upload.approx_bytes);
                let texture = ctx.load_texture(
                    managed_texture_name(&upload.key),
                    upload.image,
                    upload.key.options,
                );
                let approx_bytes = texture.byte_size().max(upload.approx_bytes);
                let evicted = cache.entries.write(|state| {
                    state.insert_without_eviction(
                        upload.key.clone(),
                        ManagedTextureEntry {
                            state: ManagedTextureState::Ready(texture),
                            last_touched_frame: cache.frame_index,
                        },
                        approx_bytes,
                    );
                    state.evict_to_budget_where(|_, entry| {
                        !matches!(entry.value.state, ManagedTextureState::Loading)
                    })
                });
                drop(evicted);
            }
            Err(err) => {
                tracing::warn!(
                    target: "vertexlauncher/image_textures",
                    texture_key = %key.source_key,
                    error = %err,
                    "Failed to decode managed image texture."
                );
                let evicted = cache.entries.write(|state| {
                    state.insert_without_eviction(
                        key,
                        ManagedTextureEntry {
                            state: ManagedTextureState::Failed,
                            last_touched_frame: cache.frame_index,
                        },
                        0,
                    );
                    state.evict_to_budget_where(|_, entry| {
                        !matches!(entry.value.state, ManagedTextureState::Loading)
                    })
                });
                drop(evicted);
            }
        }
    }

    if should_reset_channel {
        cache.tx = None;
        cache.rx = None;
        cache.pending.clear();
        cache.ready.clear();
        let _ = cache.entries.write(|state| {
            state.retain(|_, entry| !matches!(entry.value.state, ManagedTextureState::Loading))
        });
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

fn trim_stale(cache: &mut ManagedTextureCache) {
    let stale_before = cache.frame_index.saturating_sub(IMAGE_TEXTURE_STALE_FRAMES);
    let evicted = cache.entries.write(|state| {
        state.retain(|_, entry| {
            matches!(entry.value.state, ManagedTextureState::Loading)
                || entry.value.last_touched_frame >= stale_before
        })
    });
    drop(evicted);
}

fn trim_to_budget(cache: &mut ManagedTextureCache) {
    let evicted = cache.entries.write(|state| {
        state.evict_to_budget_where(|_, entry| {
            !matches!(entry.value.state, ManagedTextureState::Loading)
        })
    });
    drop(evicted);
}

fn decode_ready_texture(
    key: ManagedTextureKey,
    bytes: &[u8],
) -> Result<ReadyTextureUpload, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|err| format!("failed to decode '{}': {err}", key.source_key))?
        .to_rgba8();
    let normalized_image = if image.width() == 0 || image.height() == 0 {
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0]))
    } else {
        image
    };
    let size = [
        normalized_image.width() as usize,
        normalized_image.height() as usize,
    ];
    let color_image = ColorImage::from_rgba_unmultiplied(size, normalized_image.as_raw());
    let approx_bytes = size[0]
        .saturating_mul(size[1])
        .saturating_mul(std::mem::size_of::<egui::Color32>());
    Ok(ReadyTextureUpload {
        key,
        image: color_image,
        approx_bytes,
    })
}

fn managed_texture_name(key: &ManagedTextureKey) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!("managed_image_texture_{:016x}", hasher.finish())
}
