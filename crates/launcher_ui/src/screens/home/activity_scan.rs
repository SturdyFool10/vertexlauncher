use super::*;

static HOME_ACTIVITY_RESULTS: OnceLock<Mutex<HomeActivityResultChannel>> = OnceLock::new();

pub(super) fn home_activity_results() -> &'static Mutex<HomeActivityResultChannel> {
    HOME_ACTIVITY_RESULTS.get_or_init(|| {
        let (result_tx, result_rx) = mpsc::channel::<HomeActivityScanResult>();
        Mutex::new(HomeActivityResultChannel {
            rx: result_rx,
            tx: result_tx,
        })
    })
}

pub(super) fn build_home_activity_scan_request(
    instances: &InstanceStore,
    config: &Config,
) -> HomeActivityScanRequest {
    let installations_root = config.minecraft_installations_root_path().to_path_buf();
    let activity_instances = instances
        .instances
        .iter()
        .map(|instance| HomeActivityScanInstance {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            instance_root: instance_root_path(installations_root.as_path(), instance),
            game_version: instance.game_version.clone(),
            favorite_world_ids: instance.favorite_world_ids.clone(),
            favorite_server_ids: instance.favorite_server_ids.clone(),
        })
        .collect();

    HomeActivityScanRequest {
        scanned_instance_count: instances.instances.len(),
        instances: activity_instances,
        synced_worlds: instances.synced_worlds.clone(),
    }
}

pub(super) fn poll_home_activity_results(state: &mut HomeState) {
    let Ok(channel) = home_activity_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id = state.latest_requested_activity_scan_id,
            "Home activity results receiver mutex was poisoned while polling scan results."
        );
        return;
    };

    while let Ok(result) = channel.rx.try_recv() {
        if result.request_id != state.latest_requested_activity_scan_id {
            continue;
        }
        state.worlds = result.worlds;
        state.servers = result.servers;
        state.scanned_instance_count = result.scanned_instance_count;
        state.last_scan_at = Some(Instant::now());
        state.activity_scan_pending = false;
        retain_known_server_pings(state);
    }
}

pub(super) fn refresh_home_state(
    state: &mut HomeState,
    instances: &InstanceStore,
    config: &Config,
    force: bool,
) {
    if state.activity_scan_pending && !force {
        queue_server_pings(state);
        return;
    }

    let request_id = state.latest_requested_activity_scan_id.saturating_add(1);
    let request = build_home_activity_scan_request(instances, config);
    let Ok(channel) = home_activity_results().lock() else {
        tracing::error!(
            target: "vertexlauncher/home",
            request_id,
            scanned_instance_count = request.scanned_instance_count,
            "Home activity results channel mutex was poisoned while scheduling a home activity scan."
        );
        return;
    };
    let result_tx = channel.tx.clone();
    drop(channel);

    state.latest_requested_activity_scan_id = request_id;
    state.activity_scan_pending = true;
    tokio_runtime::spawn_detached(async move {
        let scanned_instance_count = request.scanned_instance_count;
        let result = tokio_runtime::spawn_blocking(move || HomeActivityScanResult {
            request_id,
            scanned_instance_count,
            worlds: collect_worlds_from_request(&request),
            servers: collect_servers_from_request(&request),
        })
        .await;
        let Ok(result) = result else {
            tracing::error!(
                target: "vertexlauncher/home",
                request_id,
                "Failed to complete home activity scan task."
            );
            return;
        };
        if let Err(err) = result_tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/home",
                request_id,
                error = %err,
                "Failed to deliver home activity scan result."
            );
        }
    });
}

pub(super) fn collect_worlds_from_request(request: &HomeActivityScanRequest) -> Vec<WorldEntry> {
    let mut worlds = Vec::new();
    for instance in &request.instances {
        let saves_dir = instance.instance_root.join("saves");
        let Ok(entries) = fs::read_dir(saves_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let world_id = entry.file_name().to_string_lossy().to_string();
            if world_id.trim().is_empty() {
                continue;
            }
            let level_dat_path = path.join("level.dat");
            let metadata = parse_world_metadata(level_dat_path.as_path()).unwrap_or_default();
            let world_name = metadata
                .level_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(world_id.as_str())
                .to_owned();
            let last_used_at_ms = metadata
                .last_played_ms
                .or_else(|| modified_millis(level_dat_path.as_path()))
                .or_else(|| modified_millis(path.as_path()));
            worlds.push(WorldEntry {
                instance_id: instance.instance_id.clone(),
                instance_name: instance.instance_name.clone(),
                world_id: world_id.clone(),
                world_name,
                game_mode: metadata.game_mode,
                hardcore: metadata.hardcore,
                cheats_enabled: metadata.cheats_enabled,
                difficulty: metadata.difficulty,
                version_name: metadata.version_name,
                thumbnail_png: read_world_thumbnail(path.join("icon.png").as_path()),
                last_used_at_ms,
                favorite: instance.favorite_world_ids.iter().any(|id| id == &world_id),
                group_key: world_group_key(
                    request,
                    instance.instance_id.as_str(),
                    world_id.as_str(),
                    path.as_path(),
                ),
                instances: vec![EntryInstance {
                    instance_id: instance.instance_id.clone(),
                    instance_name: instance.instance_name.clone(),
                    target: world_id.clone(),
                    favorite: instance.favorite_world_ids.iter().any(|id| id == &world_id),
                }],
            });
        }
    }
    worlds.sort_by(|a, b| {
        b.last_used_at_ms
            .unwrap_or(0)
            .cmp(&a.last_used_at_ms.unwrap_or(0))
            .then_with(|| a.world_name.cmp(&b.world_name))
    });
    home_grouping::collapse_worlds(worlds)
}

/// Worlds are the same when they share a synced-world id, or resolve to the same folder
/// (for example through a symlink).
pub(super) fn world_group_key(
    request: &HomeActivityScanRequest,
    instance_id: &str,
    folder: &str,
    path: &Path,
) -> String {
    if let Some(synced) = request
        .synced_worlds
        .iter()
        .find(|world| world.folder_name == folder && world.members.contains_key(instance_id))
    {
        return format!("synced:{}", synced.id);
    }
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub(super) fn read_world_thumbnail(path: &Path) -> Option<Arc<[u8]>> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    // Guard against unexpectedly large files in world folders.
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    Some(crate::image_memory::prepare_owned_image_bytes_for_memory(bytes))
}
