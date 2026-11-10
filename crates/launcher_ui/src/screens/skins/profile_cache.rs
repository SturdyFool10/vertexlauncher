use vertex_i18n::tr;
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
        .map_err(|err| tr!("Failed to fetch latest profile: {err}", err = err).into_owned())?;
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
        auth::load_cached_accounts().map_err(|err| tr!("Cache read failed: {err}", err = err).into_owned())?;
    let mut changed = false;
    let mut new_capes: Vec<String> = Vec::new();

    for account in &mut cache.accounts {
        if account
            .minecraft_profile
            .id
            .eq_ignore_ascii_case(profile_id)
        {
            for cape in &profile.capes {
                let already_known = account
                    .minecraft_profile
                    .capes
                    .iter()
                    .any(|known| known.id == cape.id);
                if !already_known {
                    new_capes.push(
                        cape.alias
                            .clone()
                            .unwrap_or_else(|| tr!("Unknown cape").into_owned()),
                    );
                }
            }
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
        auth::save_cached_accounts(&cache).map_err(|err| tr!("Cache write failed: {err}", err = err).into_owned())?;
        // Only announce after the new list is persisted so a cape is congratulated exactly once.
        // Accounts with no cache entry never reach here: with nothing to compare against, every
        // cape would look new.
        if !new_capes.is_empty() {
            // Capes found by separate checks while a congratulation is still showing merge into
            // it ("...got three new capes: A, B, and C, Congrats!").
            let account_name = display_name.to_owned();
            crate::notification::NotificationBuilder::new(
                crate::notification::Severity::Info,
                "skins",
                String::new(),
            )
            .icon(crate::assets::CONFETTI_SVG)
            .stack(
                format!("new_capes:{account_name}"),
                new_capes,
                move |capes| new_capes_message(&account_name, capes).unwrap_or_default(),
            )
            .emit();
        }
    } else {
        tracing::info!(
            target: "vertexlauncher/skins",
            display_name,
            "No matching cached account found to update."
        );
    }

    Ok(())
}

/// Congratulation text for the capes an account gained, worded for one, two, or many capes.
/// `None` when there is nothing new.
fn new_capes_message(account_name: &str, cape_names: &[String]) -> Option<String> {
    use crate::notification::natural_language::join_natural;
    if cape_names.is_empty() {
        return None;
    }
    Some(vertex_i18n::trn!(
        cape_names.len(),
        "Your account {account_name} got a new cape: {capes}, Congrats!",
        "Your account {account_name} got {amount} new capes: {capes}, Congrats!",
        account_name = account_name,
        amount = crate::notification::natural_language::count_word(cape_names.len()),
        capes = join_natural(cape_names)
    ))
}

#[cfg(test)]
mod tests {
    use super::new_capes_message;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn message_matches_the_number_of_new_capes() {
        assert_eq!(new_capes_message("Steve", &[]), None);
        assert_eq!(
            new_capes_message("Steve", &names(&["Migrator"])).as_deref(),
            Some("Your account Steve got a new cape: Migrator, Congrats!")
        );
        assert_eq!(
            new_capes_message("Steve", &names(&["Migrator", "Vanilla"])).as_deref(),
            Some("Your account Steve got two new capes: Migrator and Vanilla, Congrats!")
        );
        assert_eq!(
            new_capes_message("Steve", &names(&["Migrator", "Vanilla", "Mojang"])).as_deref(),
            Some(
                "Your account Steve got three new capes: Migrator, Vanilla, and Mojang, Congrats!"
            )
        );
    }
}
