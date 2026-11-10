use vertex_i18n::tr;
use super::*;



pub(super) fn metadata_validation_error(state: &InstanceScreenState) -> Option<String> {
    let trimmed_name = state.name_input.trim();
    let requested_modloader = selected_modloader_value(state);
    let requested_game_version = state.game_version_input.trim().to_owned();
    let validation_error = if trimmed_name.is_empty() {
        Some(tr!("Name cannot be empty.").into_owned())
    } else if requested_game_version.is_empty() {
        Some(tr!("Minecraft game version cannot be empty.").into_owned())
    } else if requested_modloader.trim().is_empty() {
        Some(tr!("Modloader cannot be empty.").into_owned())
    } else if support_catalog_ready(state)
        && !state.loader_support.supports_loader(
            requested_modloader.as_str(),
            requested_game_version.as_str(),
        )
        && state.selected_modloader != CUSTOM_MODLOADER_INDEX
    {
        Some(tr!("{requested_modloader} is not available for Minecraft {requested_game_version}.", requested_modloader = requested_modloader, requested_game_version = requested_game_version).into_owned())
    } else {
        resolve_modloader_version_for_settings(
            state,
            requested_modloader.as_str(),
            requested_game_version.as_str(),
        )
        .err()
    };
    validation_error
}

pub(super) fn handle_save_versions_click(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
    instances_changed: &mut bool,
) {
    match save_instance_metadata_and_versions(state, instance_id, instances) {
        Ok(()) => {
            *instances_changed = true;
            if let Some(saved) = instances.find(instance_id) {
                tracing::info!(
                    target: "vertexlauncher/ui/instance",
                    instance_id = %instance_id,
                    saved_modloader = %saved.modloader,
                    saved_game_version = %saved.game_version,
                    saved_modloader_version = %saved.modloader_version,
                    "Saved instance metadata and versions."
                );
            }
            state.status_message = Some(tr!("Saved metadata and version settings.").into_owned());
        }
        Err(err) => {
            tracing::warn!(
                target: "vertexlauncher/ui/instance",
                instance_id = %instance_id,
                error = %err,
                "Failed to save instance metadata and versions."
            );
            state.status_message = Some(err);
        }
    }
}

pub(super) fn handle_reinstall_click(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
    config: &Config,
    instances_changed: &mut bool,
) {
    match save_instance_metadata_and_versions(state, instance_id, instances) {
        Ok(()) => {
            *instances_changed = true;
            let game_version = state.game_version_input.trim().to_owned();
            let modloader = selected_modloader_value(state);
            if let Some(saved_instance) = instances.find(instance_id).cloned() {
                let modloader_version =
                    normalize_optional(saved_instance.modloader_version.as_str());
                let installations_root = config.minecraft_installations_root_path().to_path_buf();
                let instance_root =
                    instances::instance_root_path(&installations_root, &saved_instance);
                let (linux_set_opengl_driver, linux_use_zink_driver) =
                    effective_linux_graphics_settings_for_state(state, config);
                request_runtime_prepare(
                    state,
                    RuntimePrepareOperation::ReinstallProfile,
                    instance_root,
                    game_version.clone(),
                    modloader.clone(),
                    modloader_version,
                    config.effective_required_java_major(game_version.as_str()),
                    config.choose_java_executable(
                        state.java_override_enabled,
                        state.java_override_runtime_major,
                        config.effective_required_java_major(game_version.as_str()),
                    ),
                    config.download_max_concurrent(),
                    config.parsed_download_speed_limit_bps(),
                    linux_set_opengl_driver,
                    linux_use_zink_driver,
                    config.default_instance_max_memory_mib(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                );
            } else {
                state.status_message = Some(tr!("Instance was removed before reinstall.").into_owned());
            }
        }
        Err(err) => {
            state.status_message = Some(err);
        }
    }
}
