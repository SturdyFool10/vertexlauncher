use vertex_i18n::tr;
use super::*;

pub(super) fn modified_millis(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    Some(
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_default(),
    )
}

pub(super) fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(super) fn format_time_ago(timestamp_ms: Option<u64>, now_ms: u64) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "never".to_owned();
    };
    let seconds = now_ms.saturating_sub(timestamp_ms) / 1000;
    if seconds < 60 {
        return tr!("{seconds}s ago", seconds = seconds).into_owned();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return tr!("{minutes}m ago", minutes = minutes).into_owned();
    }
    let hours = minutes / 60;
    if hours < 24 {
        return tr!("{hours}h ago", hours = hours).into_owned();
    }
    let days = hours / 24;
    tr!("{days}d ago", days = days).into_owned()
}




