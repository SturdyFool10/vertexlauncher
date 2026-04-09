use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use crate::app::tokio_runtime;
use shared_lru::ThreadSafeLru;

use super::{image_memory::load_image_path_for_memory, image_textures};

const LAZY_IMAGE_MAX_BYTES: usize = 64 * 1024 * 1024;
const LAZY_IMAGE_STALE_FRAMES: u64 = 900;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LazyImageBytesStatus {
    Unrequested,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Debug)]
enum LazyImageBytesState {
    Loading,
    Ready(Arc<[u8]>),
    Failed,
}

#[derive(Clone, Debug)]
struct LazyImageEntry {
    state: LazyImageBytesState,
    last_touched_frame: u64,
}

#[derive(Clone, Debug)]
pub struct LazyImageBytes {
    states: ThreadSafeLru<String, LazyImageEntry>,
    frame_index: u64,
    generation: u64,
    results_tx: Option<mpsc::Sender<(u64, String, Result<Arc<[u8]>, String>)>>,
    results_rx: Option<Arc<Mutex<mpsc::Receiver<(u64, String, Result<Arc<[u8]>, String>)>>>>,
}

impl Default for LazyImageBytes {
    fn default() -> Self {
        Self {
            states: ThreadSafeLru::new(LAZY_IMAGE_MAX_BYTES),
            frame_index: 0,
            generation: 0,
            results_tx: None,
            results_rx: None,
        }
    }
}

impl LazyImageBytes {
    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame_index = self.frame_index.saturating_add(1);
        self.trim_stale(ctx);
        self.trim_to_budget(ctx);
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let mut updates = Vec::new();
        let mut should_reset = false;
        if let Some(rx) = self.results_rx.as_ref() {
            match rx.lock() {
                Ok(receiver) => loop {
                    match receiver.try_recv() {
                        Ok(update) => updates.push(update),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            tracing::error!(
                                target: "vertexlauncher/lazy_image",
                                "Lazy image worker disconnected unexpectedly."
                            );
                            should_reset = true;
                            break;
                        }
                    }
                },
                Err(_) => {
                    tracing::error!(
                        target: "vertexlauncher/lazy_image",
                        "Lazy image receiver mutex was poisoned."
                    );
                    should_reset = true;
                }
            }
        }

        if should_reset {
            self.results_tx = None;
            self.results_rx = None;
            // Remove entries stuck in Loading state — their in-flight tasks held the
            // old sender and can no longer deliver results. Dropping them lets the
            // next request() call re-dispatch on the new channel.
            let _ = self.states.write(|state| {
                state.retain(|_, entry| !matches!(entry.value.state, LazyImageBytesState::Loading))
            });
        }

        let mut did_update = false;
        for (generation, key, result) in updates {
            if generation != self.generation {
                continue;
            }
            match result {
                Ok(bytes) => {
                    let approx_bytes = bytes.len();
                    let evicted = self.states.write(|state| {
                        state.insert_without_eviction(
                            key.clone(),
                            LazyImageEntry {
                                last_touched_frame: self.frame_index,
                                state: LazyImageBytesState::Ready(bytes),
                            },
                            approx_bytes,
                        );
                        state.evict_to_budget_where(|_, entry| {
                            !matches!(entry.value.state, LazyImageBytesState::Loading)
                        })
                    });
                    for (evicted_key, _) in evicted {
                        image_textures::evict_source_key(evicted_key.as_str());
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        target: "vertexlauncher/lazy_image",
                        image_key = %key,
                        error = %err,
                        "Lazy image load failed."
                    );
                    let evicted = self.states.write(|state| {
                        state.insert_without_eviction(
                            key.clone(),
                            LazyImageEntry {
                                last_touched_frame: self.frame_index,
                                state: LazyImageBytesState::Failed,
                            },
                            0,
                        );
                        state.evict_to_budget_where(|_, entry| {
                            !matches!(entry.value.state, LazyImageBytesState::Loading)
                        })
                    });
                    for (evicted_key, _) in evicted {
                        image_textures::evict_source_key(evicted_key.as_str());
                    }
                }
            }
            did_update = true;
        }
        if did_update {
            self.trim_to_budget(ctx);
        }
        did_update
    }

    pub fn has_in_flight(&self) -> bool {
        self.states.read(|state| {
            state.values_any(|entry| matches!(entry.value.state, LazyImageBytesState::Loading))
        })
    }

    pub fn status(&self, key: &str) -> LazyImageBytesStatus {
        self.states.read(
            |state| match state.get_borrowed(key).map(|e| &e.value.state) {
                Some(LazyImageBytesState::Loading) => LazyImageBytesStatus::Loading,
                Some(LazyImageBytesState::Ready(_)) => LazyImageBytesStatus::Ready,
                Some(LazyImageBytesState::Failed) => LazyImageBytesStatus::Failed,
                None => LazyImageBytesStatus::Unrequested,
            },
        )
    }

    pub fn bytes(&self, key: &str) -> Option<Arc<[u8]>> {
        self.states.read(
            |state| match state.get_borrowed(key).map(|e| &e.value.state) {
                Some(LazyImageBytesState::Ready(bytes)) => Some(Arc::clone(bytes)),
                _ => None,
            },
        )
    }

    pub fn request(&mut self, key: impl Into<String>, path: PathBuf) -> LazyImageBytesStatus {
        let key = key.into();
        if let Some(entry) = self.states.write(|state| {
            let entry = state.touch(&key)?;
            entry.value.last_touched_frame = self.frame_index;
            Some(entry.value.clone())
        }) {
            return match entry.state {
                LazyImageBytesState::Loading => LazyImageBytesStatus::Loading,
                LazyImageBytesState::Ready(_) => LazyImageBytesStatus::Ready,
                LazyImageBytesState::Failed => LazyImageBytesStatus::Failed,
            };
        }

        self.ensure_channel();
        let Some(tx) = self.results_tx.as_ref().cloned() else {
            let _ = self.states.write(|state| {
                state.insert_without_eviction(
                    key,
                    LazyImageEntry {
                        state: LazyImageBytesState::Failed,
                        last_touched_frame: self.frame_index,
                    },
                    0,
                );
            });
            return LazyImageBytesStatus::Failed;
        };

        let _ = self.states.write(|state| {
            state.insert_without_eviction(
                key.clone(),
                LazyImageEntry {
                    state: LazyImageBytesState::Loading,
                    last_touched_frame: self.frame_index,
                },
                0,
            );
        });
        let key_for_task = key.clone();
        let generation = self.generation;
        tokio_runtime::spawn_detached(async move {
            let result = load_image_path_for_memory(path.clone()).await;
            if let Err(err) = tx.send((generation, key_for_task.clone(), result)) {
                tracing::error!(
                    target: "vertexlauncher/lazy_image",
                    key = %key_for_task,
                    path = %path.display(),
                    error = %err,
                    "Failed to deliver lazy-image bytes result."
                );
            }
        });

        LazyImageBytesStatus::Loading
    }

    pub fn retain_loaded(&mut self, _ctx: &egui::Context, keep: &HashSet<String>) {
        let frame_index = self.frame_index;
        let _ = self.states.write(|state| {
            for key in keep {
                if let Some(entry) = state.touch(key) {
                    entry.value.last_touched_frame = frame_index;
                }
            }
        });
    }

    pub fn clear(&mut self, _ctx: &egui::Context) {
        for key in self.states.write(|state| state.keys_cloned()) {
            image_textures::evict_source_key(key.as_str());
        }
        let _ = self.states.write(|state| state.clear());
        self.generation = self.generation.saturating_add(1);
        self.results_tx = None;
        self.results_rx = None;
    }

    fn ensure_channel(&mut self) {
        if self.results_tx.is_some() && self.results_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<(u64, String, Result<Arc<[u8]>, String>)>();
        self.results_tx = Some(tx);
        self.results_rx = Some(Arc::new(Mutex::new(rx)));
    }

    fn trim_stale(&mut self, _ctx: &egui::Context) {
        let stale_before = self.frame_index.saturating_sub(LAZY_IMAGE_STALE_FRAMES);
        let evicted = self.states.write(|state| {
            state.retain(|_, entry| {
                matches!(entry.value.state, LazyImageBytesState::Loading)
                    || entry.value.last_touched_frame >= stale_before
            })
        });
        for (key, _) in evicted {
            image_textures::evict_source_key(key.as_str());
        }
    }

    fn trim_to_budget(&mut self, _ctx: &egui::Context) {
        let evicted = self.states.write(|state| {
            state.evict_to_budget_where(|_, entry| {
                !matches!(entry.value.state, LazyImageBytesState::Loading)
            })
        });
        for (key, entry) in evicted {
            if !matches!(entry.state, LazyImageBytesState::Loading) {
                image_textures::evict_source_key(key.as_str());
            }
        }
    }
}
