use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use crate::app::tokio_runtime;

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

#[derive(Clone, Debug, Default)]
pub struct LazyImageBytes {
    states: HashMap<String, LazyImageBytesState>,
    results_tx: Option<mpsc::Sender<(String, Result<Arc<[u8]>, String>)>>,
    results_rx: Option<Arc<Mutex<mpsc::Receiver<(String, Result<Arc<[u8]>, String>)>>>>,
}

impl LazyImageBytes {
    pub fn poll(&mut self) -> bool {
        let mut updates = Vec::new();
        let mut should_reset = false;
        if let Some(rx) = self.results_rx.as_ref() {
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
            self.results_tx = None;
            self.results_rx = None;
        }

        let mut did_update = false;
        for (key, result) in updates {
            match result {
                Ok(bytes) => {
                    self.states.insert(key, LazyImageBytesState::Ready(bytes));
                }
                Err(_) => {
                    self.states.insert(key, LazyImageBytesState::Failed);
                }
            }
            did_update = true;
        }
        did_update
    }

    pub fn has_in_flight(&self) -> bool {
        self.states
            .values()
            .any(|state| matches!(state, LazyImageBytesState::Loading))
    }

    pub fn status(&self, key: &str) -> LazyImageBytesStatus {
        match self.states.get(key) {
            Some(LazyImageBytesState::Loading) => LazyImageBytesStatus::Loading,
            Some(LazyImageBytesState::Ready(_)) => LazyImageBytesStatus::Ready,
            Some(LazyImageBytesState::Failed) => LazyImageBytesStatus::Failed,
            None => LazyImageBytesStatus::Unrequested,
        }
    }

    pub fn bytes(&self, key: &str) -> Option<Arc<[u8]>> {
        match self.states.get(key) {
            Some(LazyImageBytesState::Ready(bytes)) => Some(Arc::clone(bytes)),
            _ => None,
        }
    }

    pub fn request(&mut self, key: impl Into<String>, path: PathBuf) -> LazyImageBytesStatus {
        let key = key.into();
        match self.status(key.as_str()) {
            LazyImageBytesStatus::Loading
            | LazyImageBytesStatus::Ready
            | LazyImageBytesStatus::Failed => {
                return self.status(key.as_str());
            }
            LazyImageBytesStatus::Unrequested => {}
        }

        self.ensure_channel();
        let Some(tx) = self.results_tx.as_ref().cloned() else {
            self.states.insert(key, LazyImageBytesState::Failed);
            return LazyImageBytesStatus::Failed;
        };

        self.states
            .insert(key.clone(), LazyImageBytesState::Loading);
        let key_for_task = key.clone();
        let path_label = path.display().to_string();
        let _ = tokio_runtime::spawn_detached(async move {
            let result = fs::read(path.as_path())
                .map(Arc::<[u8]>::from)
                .map_err(|err| format!("failed to read '{path_label}': {err}"));
            let _ = tx.send((key_for_task, result));
        });

        LazyImageBytesStatus::Loading
    }

    pub fn retain_loaded(&mut self, keep: &HashSet<String>) {
        self.states.retain(|key, state| {
            keep.contains(key.as_str()) || matches!(state, LazyImageBytesState::Loading)
        });
    }

    fn ensure_channel(&mut self) {
        if self.results_tx.is_some() && self.results_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<(String, Result<Arc<[u8]>, String>)>();
        self.results_tx = Some(tx);
        self.results_rx = Some(Arc::new(Mutex::new(rx)));
    }
}
