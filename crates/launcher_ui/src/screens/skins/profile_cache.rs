use super::*;

/// Fetches the latest Minecraft profile and persists it back into the cached account
/// snapshot for the active profile.
///
/// `profile_id` should match the active Minecraft profile identifier. `access_token`
/// must be a valid token accepted by the auth backend.
///
/// Returns a user-facing error string on network or cache failures.
pub(super) fn fetch_and_cache_profile(
    profile_id: String,
    access_token: &str,
    display_name: &str,
) -> Result<LoadedProfile, String> {
    tracing::info!(
        target: "vertexlauncher/skins",
        display_name,
        "Fetching latest skin profile."
    );
    let profile = auth::fetch_minecraft_profile(access_token)
        .map_err(|err| format!("Failed to fetch latest profile: {err}"))?;
    tracing::info!(
        target: "vertexlauncher/skins",
        display_name,
        skins = profile.skins.len(),
        capes = profile.capes.len(),
        "Fetched latest skin profile."
    );
    update_cached_profile(profile_id.as_str(), &profile, display_name)?;
    Ok(LoadedProfile::from_profile(profile))
}

/// Rewrites the cached Minecraft profile snapshot for one cached account if present.
///
/// `profile_id` is matched case-insensitively against cached profile identifiers.
/// Returns `Ok(())` when no matching cache entry exists.
pub(super) fn update_cached_profile(
    profile_id: &str,
    profile: &MinecraftProfileState,
    display_name: &str,
) -> Result<(), String> {
    tracing::info!(
        target: "vertexlauncher/skins",
        display_name,
        "Updating cached account profile snapshot."
    );
    let mut cache =
        auth::load_cached_accounts().map_err(|err| format!("Cache read failed: {err}"))?;
    let mut changed = false;

    for account in &mut cache.accounts {
        if account
            .minecraft_profile
            .id
            .eq_ignore_ascii_case(profile_id)
        {
            account.minecraft_profile = profile.clone();
            account.cached_at_unix_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            changed = true;
            break;
        }
    }

    if changed {
        auth::save_cached_accounts(&cache).map_err(|err| format!("Cache write failed: {err}"))?;
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name,
            "Cached account profile snapshot updated."
        );
    } else {
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name,
            "No matching cached account found to update."
        );
    }

    Ok(())
}
