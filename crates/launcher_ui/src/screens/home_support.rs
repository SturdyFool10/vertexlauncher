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
        return format!("{seconds}s ago");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

pub(super) fn open_home_instance_folder(
    instance_id: &str,
    instances: &InstanceStore,
    config: &Config,
) -> Result<(), String> {
    let Some(instance) = instances
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
    else {
        return Err(format!("unknown instance id: {instance_id}"));
    };
    let root = instance_root_path(config.minecraft_installations_root_path(), instance);
    desktop::open_in_file_manager(root.as_path())
}

pub(super) fn open_home_instance(output: &mut HomeOutput, instance_id: &str) {
    output.selected_instance_id = Some(instance_id.to_owned());
    output.requested_screen = Some(AppScreen::Instance);
}
