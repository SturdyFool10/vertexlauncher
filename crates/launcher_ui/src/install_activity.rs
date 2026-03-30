use installation::{InstallProgress, InstallStage};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct InstallActivitySnapshot {
    pub instance_id: String,
    pub stage: InstallStage,
    pub message: String,
    pub downloaded_files: u32,
    pub total_files: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<u64>,
    pub updated_at: Instant,
}

#[derive(Default)]
struct InstallActivityStore {
    active: Option<InstallActivitySnapshot>,
}

static INSTALL_ACTIVITY: OnceLock<Mutex<InstallActivityStore>> = OnceLock::new();

fn store() -> &'static Mutex<InstallActivityStore> {
    INSTALL_ACTIVITY.get_or_init(|| Mutex::new(InstallActivityStore::default()))
}

pub fn set_progress(instance_id: &str, progress: &InstallProgress) {
    let mut guard = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    guard.active = Some(InstallActivitySnapshot {
        instance_id: instance_id.to_owned(),
        stage: progress.stage,
        message: progress.message.clone(),
        downloaded_files: progress.downloaded_files,
        total_files: progress.total_files,
        downloaded_bytes: progress.downloaded_bytes,
        total_bytes: progress.total_bytes,
        bytes_per_second: progress.bytes_per_second,
        eta_seconds: progress.eta_seconds,
        updated_at: Instant::now(),
    });
}

pub fn set_status(instance_id: &str, stage: InstallStage, message: impl Into<String>) {
    let mut guard = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    guard.active = Some(InstallActivitySnapshot {
        instance_id: instance_id.to_owned(),
        stage,
        message: message.into(),
        downloaded_files: 0,
        total_files: 1,
        downloaded_bytes: 0,
        total_bytes: None,
        bytes_per_second: 0.0,
        eta_seconds: None,
        updated_at: Instant::now(),
    });
}

pub fn clear_instance(instance_id: &str) {
    let mut guard = match store().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if guard
        .active
        .as_ref()
        .is_some_and(|snapshot| snapshot.instance_id == instance_id)
    {
        guard.active = None;
    }
}

pub fn is_instance_installing(instance_id: &str) -> bool {
    let Ok(mut guard) = store().lock() else {
        return false;
    };
    if let Some(active) = guard.active.as_ref() {
        if active.updated_at.elapsed() > Duration::from_secs(15) {
            guard.active = None;
            return false;
        }
        return active.instance_id == instance_id;
    }
    false
}

pub fn snapshot() -> Option<InstallActivitySnapshot> {
    let mut guard = store().lock().ok()?;
    if let Some(active) = guard.active.as_ref()
        && active.updated_at.elapsed() > Duration::from_secs(15)
    {
        guard.active = None;
        return None;
    }
    guard.active.clone()
}
