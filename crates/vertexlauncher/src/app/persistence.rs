use super::*;

pub(super) fn ensure_config_save_channel(app: &mut VertexApp) {
    if app.config_save_results_tx.is_some() && app.config_save_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    app.config_save_results_tx = Some(tx);
    app.config_save_results_rx = Some(rx);
}

pub(super) fn start_pending_config_save(app: &mut VertexApp) {
    if app.config_save_in_flight {
        return;
    }
    let Some(config) = app.pending_config_save.take() else {
        return;
    };

    ensure_config_save_channel(app);
    let Some(tx) = app.config_save_results_tx.as_ref().cloned() else {
        app.pending_config_save = Some(config);
        return;
    };

    app.config_save_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = save_config(&config).map_err(|err| err.to_string());
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/config",
                error = %err,
                "Failed to deliver config-save result."
            );
        }
    });
}

pub(super) fn queue_config_save(app: &mut VertexApp) {
    app.pending_config_save = Some(app.config.clone());
    start_pending_config_save(app);
}

pub(super) fn poll_config_save_results(app: &mut VertexApp) {
    let mut should_reset_channel = false;
    let mut saw_result = false;
    loop {
        let Some(result) = app.config_save_results_rx.as_ref().map(|rx| rx.try_recv()) else {
            return;
        };
        match result {
            Ok(result) => {
                saw_result = true;
                app.config_save_in_flight = false;
                if let Err(err) = result {
                    tracing::error!(
                        target: "vertexlauncher/app/config",
                        "Failed to save config: {err}"
                    );
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/app/config",
                    "Config-save worker disconnected unexpectedly."
                );
                should_reset_channel = true;
                app.config_save_in_flight = false;
                break;
            }
        }
    }

    if should_reset_channel {
        app.config_save_results_tx = None;
        app.config_save_results_rx = None;
    }
    if saw_result || !app.config_save_in_flight {
        start_pending_config_save(app);
    }
}

pub(super) fn ensure_instance_store_save_channel(app: &mut VertexApp) {
    if app.instance_store_save_results_tx.is_some() && app.instance_store_save_results_rx.is_some()
    {
        return;
    }
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    app.instance_store_save_results_tx = Some(tx);
    app.instance_store_save_results_rx = Some(rx);
}

pub(super) fn start_pending_instance_store_save(app: &mut VertexApp) {
    if app.instance_store_save_in_flight {
        return;
    }
    let Some(store) = app.pending_instance_store_save.take() else {
        return;
    };

    ensure_instance_store_save_channel(app);
    let Some(tx) = app.instance_store_save_results_tx.as_ref().cloned() else {
        app.pending_instance_store_save = Some(store);
        return;
    };

    app.instance_store_save_in_flight = true;
    let _ = tokio_runtime::spawn_detached(async move {
        let result = save_instance_store(&store).map_err(|err| err.to_string());
        if let Err(err) = tx.send(result) {
            tracing::error!(
                target: "vertexlauncher/app/instances",
                error = %err,
                "Failed to deliver instance-store save result."
            );
        }
    });
}

pub(super) fn queue_instance_store_save(app: &mut VertexApp) {
    app.pending_instance_store_save = Some(app.instance_store.clone());
    start_pending_instance_store_save(app);
}

pub(super) fn ensure_initial_instance_install_channel(app: &mut VertexApp) {
    if app.initial_install_results_tx.is_some() && app.initial_install_results_rx.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<InitialInstanceInstallResult>();
    app.initial_install_results_tx = Some(tx);
    app.initial_install_results_rx = Some(rx);
}

pub(super) fn poll_instance_store_save_results(app: &mut VertexApp) {
    let mut should_reset_channel = false;
    let mut saw_result = false;
    loop {
        let Some(result) = app
            .instance_store_save_results_rx
            .as_ref()
            .map(|rx| rx.try_recv())
        else {
            return;
        };
        match result {
            Ok(result) => {
                saw_result = true;
                app.instance_store_save_in_flight = false;
                if let Err(err) = result {
                    tracing::error!(
                        target: "vertexlauncher/app/instances",
                        "Failed to save instances: {err}"
                    );
                }
            }
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/app/instances",
                    "Instance-store save worker disconnected unexpectedly."
                );
                should_reset_channel = true;
                app.instance_store_save_in_flight = false;
                break;
            }
        }
    }

    if should_reset_channel {
        app.instance_store_save_results_tx = None;
        app.instance_store_save_results_rx = None;
    }
    if saw_result || !app.instance_store_save_in_flight {
        start_pending_instance_store_save(app);
    }
}

pub(super) fn poll_initial_instance_install_results(app: &mut VertexApp) {
    let mut updates = Vec::new();
    let mut should_reset_channel = false;
    loop {
        let Some(result) = app
            .initial_install_results_rx
            .as_ref()
            .map(|rx| rx.try_recv())
        else {
            return;
        };
        match result {
            Ok(update) => updates.push(update),
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                tracing::error!(
                    target: "vertexlauncher/app/initial_install",
                    "Initial-install result worker disconnected unexpectedly."
                );
                should_reset_channel = true;
                break;
            }
        }
    }

    if should_reset_channel {
        app.initial_install_results_tx = None;
        app.initial_install_results_rx = None;
    }

    for update in updates {
        match update {
            InitialInstanceInstallResult::Failed {
                instance_id,
                instance_name,
                error,
            } => {
                tracing::warn!(
                    target: "vertexlauncher/app/initial_install",
                    instance_id,
                    instance_name = %instance_name,
                    "Rolling back failed initial install."
                );
                match delete_instance(
                    &mut app.instance_store,
                    instance_id.as_str(),
                    app.config.minecraft_installations_root_path(),
                ) {
                    Ok(_) => {
                        queue_instance_store_save(app);
                        app.refresh_instance_shortcuts();
                        if app.selected_instance_id.as_deref() == Some(instance_id.as_str()) {
                            app.selected_instance_id =
                                app.instance_shortcuts.first().map(|s| s.id.clone());
                            if app.active_screen == screens::AppScreen::Instance {
                                app.active_screen = screens::AppScreen::Home;
                            }
                        }
                        notification::error!(
                            format!("installation/{instance_name}"),
                            "{}: initial install failed and the incomplete instance was removed: {}",
                            instance_name,
                            error
                        );
                    }
                    Err(delete_err) => {
                        notification::error!(
                            format!("installation/{instance_name}"),
                            "{}: initial install failed: {}. Cleanup also failed: {}",
                            instance_name,
                            error,
                            delete_err
                        );
                    }
                }
            }
        }
    }
}

pub(super) fn poll_finished_instance_process_notifications(app: &mut VertexApp) {
    for process in take_finished_instance_processes() {
        if process.exit_code != Some(1) {
            continue;
        }

        let instance_name = app.instance_store.instances.iter().find_map(|instance| {
            let instance_root =
                instance_root_path(app.config.minecraft_installations_root_path(), instance);
            (normalize_path_key(instance_root.as_path()) == process.instance_root)
                .then(|| instance.name.as_str())
        });

        if let Some(instance_name) = instance_name {
            notification::error!(
                format!("instance/crash/{}", process.pid),
                "{} crashed with exit code 1.",
                instance_name
            );
        } else {
            notification::error!(
                format!("instance/crash/{}", process.pid),
                "An instance crashed with exit code 1: {}",
                process.instance_root
            );
        }
    }
}
