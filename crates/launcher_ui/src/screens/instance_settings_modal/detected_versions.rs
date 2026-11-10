use vertex_i18n::tr;
use super::*;

pub(super) fn apply_detected_instance_versions(
    state: &mut InstanceScreenState,
    detected: &DetectedInstanceVersions,
) {
    state.game_version_input = detected.game_version.clone();
    if let Some(index) = state
        .available_game_versions
        .iter()
        .position(|entry| entry.id == detected.game_version)
    {
        state.selected_game_version_index = index;
    }

    let (selected_modloader, custom_modloader) = split_modloader(detected.modloader.as_str());
    state.selected_modloader = selected_modloader;
    state.custom_modloader = custom_modloader;
    state.modloader_version_input = detected.modloader_version.clone();
    state.incompatible_modloader_version_warning_key = None;
    state.modloader_versions_status = None;
    state.modloader_versions_status_key = None;

    if selected_modloader != 0
        && selected_modloader != CUSTOM_MODLOADER_INDEX
        && !detected.modloader_version.trim().is_empty()
    {
        let cache_key = modloader_versions_cache_key(
            detected.modloader.as_str(),
            detected.game_version.as_str(),
        );
        let versions = state
            .modloader_versions_cache
            .entry(cache_key)
            .or_insert_with(Vec::new);
        if !versions
            .iter()
            .any(|version| version.eq_ignore_ascii_case(detected.modloader_version.as_str()))
        {
            versions.insert(0, detected.modloader_version.clone());
        }
    }
}

pub(super) fn detected_versions_summary(detected: &DetectedInstanceVersions) -> String {
    if detected.modloader_version.trim().is_empty() {
        tr!("{loader} for Minecraft {game_version}", loader = detected.modloader, game_version = detected.game_version).into_owned()
    } else {
        tr!(
            "{loader} {loader_version} for Minecraft {game_version}",
            loader = detected.modloader,
            loader_version = detected.modloader_version,
            game_version = detected.game_version
        )
        .into_owned()
    }
}
