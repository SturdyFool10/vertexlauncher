use vertex_i18n::tr;
use super::*;

/// How long settings must stay unchanged before they are written to disk.
pub(super) const RUNTIME_OVERRIDES_AUTOSAVE_DELAY: Duration = Duration::from_millis(250);

pub(super) fn runtime_overrides_fingerprint(state: &InstanceScreenState) -> String {
    format!(
        "{:?}",
        (
            state.memory_override_enabled,
            state.memory_override_mib,
            &state.cli_args_input,
            &state.env_vars_input,
            state.java_override_enabled,
            state.java_override_runtime_major,
            state.linux_set_opengl_driver,
            state.linux_use_zink_driver,
            state.discord_rich_presence_mod_installed,
        )
    )
}

/// Validates and persists the runtime override fields of the settings modal.
pub(super) fn save_runtime_overrides(
    state: &InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
    java_options: &[(u8, String)],
    memory_slider_max: u128,
) -> Result<(), String> {
    let java_override_runtime_major = if state.java_override_enabled {
        if java_options.is_empty() {
            return Err(
                tr!("Cannot save Java override: configure at least one global Java path in Settings.")
                    .into_owned(),
            );
        }
        let selected = state.java_override_runtime_major.and_then(|major| {
            java_options
                .iter()
                .find_map(|(candidate, _)| (*candidate == major).then_some(major))
        });
        selected.or_else(|| java_options.first().map(|(major, _)| *major))
    } else {
        None
    };
    let memory_override = state.memory_override_enabled.then(|| {
        state
            .memory_override_mib
            .clamp(INSTANCE_DEFAULT_MAX_MEMORY_MIB_MIN, memory_slider_max)
    });
    let (linux_set_opengl_driver, linux_use_zink_driver) =
        linux_instance_driver_settings_for_save(state, instances.find(instance_id));
    set_instance_settings(
        instances,
        instance_id,
        memory_override,
        normalize_optional(state.cli_args_input.as_str()),
        normalize_optional(state.env_vars_input.as_str()),
        state.java_override_enabled,
        java_override_runtime_major,
        linux_set_opengl_driver,
        linux_use_zink_driver,
        state.discord_rich_presence_mod_installed,
    )
    .map_err(|err| err.to_string())
}

/// Saves runtime overrides once they have been idle for [`RUNTIME_OVERRIDES_AUTOSAVE_DELAY`]. Returns whether the
/// store changed and, while edits are waiting out the delay, how long until the UI should look again.
pub(super) fn autosave_runtime_overrides_step(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
    java_options: &[(u8, String)],
    memory_slider_max: u128,
) -> (bool, Option<Duration>) {
    let fingerprint = runtime_overrides_fingerprint(state);
    let Some(saved) = state.runtime_overrides_saved_fingerprint.as_ref() else {
        state.runtime_overrides_saved_fingerprint = Some(fingerprint);
        return (false, None);
    };
    if *saved == fingerprint {
        state.runtime_overrides_dirty_since = None;
        return (false, None);
    }
    let dirty_since = match state.runtime_overrides_dirty_since.as_ref() {
        Some((pending, since)) if *pending == fingerprint => *since,
        _ => {
            let now = Instant::now();
            state.runtime_overrides_dirty_since = Some((fingerprint.clone(), now));
            now
        }
    };
    let elapsed = dirty_since.elapsed();
    if elapsed < RUNTIME_OVERRIDES_AUTOSAVE_DELAY {
        return (false, Some(RUNTIME_OVERRIDES_AUTOSAVE_DELAY - elapsed));
    }
    state.runtime_overrides_dirty_since = None;
    state.runtime_overrides_saved_fingerprint = Some(fingerprint);
    match save_runtime_overrides(state, instance_id, instances, java_options, memory_slider_max) {
        Ok(()) => (true, None),
        Err(err) => {
            state.status_message = Some(err);
            (false, None)
        }
    }
}



/// Writes pending edits immediately, used when the modal closes mid-delay.
pub(super) fn flush_runtime_overrides_autosave(
    state: &mut InstanceScreenState,
    instance_id: &str,
    instances: &mut InstanceStore,
    config: &Config,
) -> bool {
    if state.runtime_overrides_dirty_since.take().is_none() {
        return false;
    }
    state.runtime_overrides_saved_fingerprint = Some(runtime_overrides_fingerprint(state));
    let java_options = configured_java_path_options(config);
    let (memory_slider_max, _) = memory_slider_max_mib();
    save_runtime_overrides(
        state,
        instance_id,
        instances,
        &java_options,
        memory_slider_max,
    )
    .is_ok()
}
