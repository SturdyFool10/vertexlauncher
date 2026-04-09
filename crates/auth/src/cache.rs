use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use crate::constants::{
    ACCOUNT_CACHE_APP_DIR, ACCOUNT_CACHE_FILENAME, LEGACY_ACCOUNT_CACHE_APP_DIR,
    LEGACY_ACCOUNT_CACHE_PATH,
};
use crate::error::AuthError;
use crate::secret_store::{self, RefreshTokenLoadResult};
use crate::types::RefreshTokenState;
use crate::{CachedAccount, CachedAccountsState};

enum AccountsStateLocation {
    Disk,
    SecureStore,
}

#[track_caller]
fn fs_read_to_string(path: impl AsRef<Path>) -> Result<String, AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    Ok(result?)
}

#[track_caller]
fn fs_write_string(path: impl AsRef<Path>, contents: &str) -> Result<(), AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "write_string", path = %path.display());
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let create_result = fs::create_dir_all(parent);
        if let Err(err) = &create_result {
            tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %parent.display(), error = %err);
        }
        create_result?;
    }
    let write_result = fs::write(path, contents);
    if let Err(err) = &write_result {
        tracing::warn!(target: "vertexlauncher/io", op = "write_string", path = %path.display(), error = %err);
    }
    Ok(write_result?)
}

#[track_caller]
fn fs_remove_file(path: impl AsRef<Path>) -> Result<(), AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "remove_file", path = %path.display());
    let result = fs::remove_file(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_file", path = %path.display(), error = %err);
    }
    Ok(result?)
}

pub(crate) fn load_cached_accounts() -> Result<CachedAccountsState, AuthError> {
    let path = account_cache_path();
    migrate_secure_accounts_state_to_disk(&path)?;

    match load_cached_accounts_state_contents(&path)? {
        Some((contents, _)) => parse_cached_accounts_state(contents.as_str()),
        None => {
            tracing::debug!(
                target: "vertexlauncher/auth/cache",
                "cached accounts state missing; using empty cache"
            );
            Ok(CachedAccountsState::default())
        }
    }
}

pub(crate) fn save_cached_accounts(state: &CachedAccountsState) -> Result<(), AuthError> {
    let path = account_cache_path();
    let previous_profile_ids = load_cached_profile_ids_from_persisted_storage(&path)?;
    let mut normalized = state.clone().normalize();
    let current_profile_ids = normalized
        .accounts
        .iter()
        .map(|account| account.minecraft_profile.id.clone())
        .collect::<Vec<_>>();

    for account in &mut normalized.accounts {
        persist_refresh_token(account)?;
        sanitize_cached_profile(account);
    }
    let json = serde_json::to_string(&normalized)?;
    fs_write_string(&path, &json)?;
    for legacy_path in legacy_account_cache_paths(&path) {
        remove_legacy_account_cache_file(&legacy_path);
    }
    let _ = secret_store::delete_accounts_state();

    for profile_id in previous_profile_ids {
        if !current_profile_ids
            .iter()
            .any(|current| current == &profile_id)
        {
            secret_store::delete_refresh_token(&profile_id)?;
        }
    }

    Ok(())
}

pub(crate) fn clear_cached_accounts() -> Result<(), AuthError> {
    let path = account_cache_path();
    let previous_profile_ids = load_cached_profile_ids_from_persisted_storage(&path)?;

    remove_legacy_account_cache_file(&path);
    for legacy_path in legacy_account_cache_paths(&path) {
        remove_legacy_account_cache_file(&legacy_path);
    }
    let _ = secret_store::delete_accounts_state();

    for profile_id in previous_profile_ids {
        secret_store::delete_refresh_token(&profile_id)?;
    }

    Ok(())
}

pub(crate) fn load_cached_account() -> Result<Option<CachedAccount>, AuthError> {
    let state = load_cached_accounts()?;
    Ok(state.active_account().cloned())
}

pub(crate) fn save_cached_account(account: &CachedAccount) -> Result<(), AuthError> {
    let mut state = load_cached_accounts()?;
    state.upsert_and_activate(account.clone());
    save_cached_accounts(&state)
}

pub(crate) fn clear_cached_account() -> Result<(), AuthError> {
    clear_cached_accounts()
}

fn account_cache_path() -> PathBuf {
    std::env::var("VERTEX_ACCOUNT_CACHE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_account_cache_path())
}

fn default_account_cache_path() -> PathBuf {
    default_account_cache_dir(ACCOUNT_CACHE_APP_DIR)
        .map(|dir| dir.join(ACCOUNT_CACHE_FILENAME))
        .unwrap_or_else(|| PathBuf::from(ACCOUNT_CACHE_APP_DIR).join(ACCOUNT_CACHE_FILENAME))
}

fn legacy_default_account_cache_path() -> Option<PathBuf> {
    default_account_cache_dir(LEGACY_ACCOUNT_CACHE_APP_DIR)
        .map(|dir| dir.join(ACCOUNT_CACHE_FILENAME))
}

fn default_account_cache_dir(app_dir: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            if !local_app_data.trim().is_empty() {
                return Some(PathBuf::from(local_app_data).join(app_dir));
            }
        }

        if let Ok(app_data) = std::env::var("APPDATA") {
            if !app_data.trim().is_empty() {
                return Some(PathBuf::from(app_data).join(app_dir));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                return Some(
                    PathBuf::from(home)
                        .join("Library")
                        .join("Application Support")
                        .join(app_dir),
                );
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
            if !state_home.trim().is_empty() {
                return Some(PathBuf::from(state_home).join(app_dir));
            }
        }

        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                return Some(
                    PathBuf::from(home)
                        .join(".local")
                        .join("state")
                        .join(app_dir),
                );
            }
        }
    }

    None
}

fn legacy_account_cache_paths(active_path: &Path) -> Vec<PathBuf> {
    if std::env::var("VERTEX_ACCOUNT_CACHE_PATH").is_ok() {
        return Vec::new();
    }

    let mut candidates = Vec::new();

    if let Some(legacy_path) = legacy_default_account_cache_path()
        && legacy_path != active_path
    {
        candidates.push(legacy_path);
    }

    let legacy_cwd_path = PathBuf::from(LEGACY_ACCOUNT_CACHE_PATH);
    if legacy_cwd_path != active_path && !candidates.iter().any(|path| path == &legacy_cwd_path) {
        candidates.push(legacy_cwd_path);
    }

    candidates
}

fn migrate_secure_accounts_state_to_disk(target_path: &Path) -> Result<(), AuthError> {
    let Some(contents) = secret_store::load_accounts_state()? else {
        return Ok(());
    };

    if target_path.exists() {
        let _ = secret_store::delete_accounts_state();
        return Ok(());
    }

    let state = parse_cached_accounts_state(contents.as_str())?;
    save_cached_accounts(&state)?;
    secret_store::delete_accounts_state()?;
    tracing::info!(
        target: "vertexlauncher/auth/cache",
        path = %target_path.display(),
        "migrated cached account metadata from secure storage to disk"
    );
    Ok(())
}

fn load_cached_accounts_state_contents(
    path: &Path,
) -> Result<Option<(String, AccountsStateLocation)>, AuthError> {
    let mut candidate_paths = vec![path.to_path_buf()];
    for legacy_path in legacy_account_cache_paths(path) {
        if !candidate_paths
            .iter()
            .any(|candidate| candidate == &legacy_path)
        {
            candidate_paths.push(legacy_path);
        }
    }

    for candidate in candidate_paths {
        if !candidate.exists() {
            continue;
        }
        return Ok(Some((
            fs_read_to_string(&candidate)?,
            AccountsStateLocation::Disk,
        )));
    }

    if let Some(contents) = secret_store::load_accounts_state()? {
        return Ok(Some((contents, AccountsStateLocation::SecureStore)));
    }

    Ok(None)
}

fn remove_legacy_account_cache_file(path: &Path) {
    if path.exists() {
        let _ = fs_remove_file(path);
    }
}

fn parse_cached_accounts_state(contents: &str) -> Result<CachedAccountsState, AuthError> {
    match serde_json::from_str::<CachedAccountsState>(contents) {
        Ok(state) => finalize_loaded_accounts(state.normalize()),
        Err(state_error) => {
            if let Ok(single_account) = serde_json::from_str::<CachedAccount>(contents) {
                tracing::info!(
                    target: "vertexlauncher/auth/cache",
                    "migrated single-account secure cache format into multi-account state"
                );
                let mut state = CachedAccountsState::default();
                state.upsert_and_activate(single_account);
                return finalize_loaded_accounts(state);
            }

            tracing::warn!(
                target: "vertexlauncher/auth/cache",
                error = %state_error,
                "failed to parse cached accounts state"
            );
            Err(AuthError::Json(state_error))
        }
    }
}

fn load_cached_profile_ids_from_persisted_storage(path: &Path) -> Result<Vec<String>, AuthError> {
    let Some((contents, _)) = load_cached_accounts_state_contents(path)? else {
        return Ok(Vec::new());
    };
    if let Ok(state) = serde_json::from_str::<CachedAccountsState>(&contents) {
        return Ok(state
            .accounts
            .into_iter()
            .map(|account| account.minecraft_profile.id)
            .filter(|profile_id| !profile_id.trim().is_empty())
            .collect());
    }

    if let Ok(account) = serde_json::from_str::<CachedAccount>(&contents) {
        if account.minecraft_profile.id.trim().is_empty() {
            return Ok(Vec::new());
        }
        return Ok(vec![account.minecraft_profile.id]);
    }

    Ok(Vec::new())
}

fn sanitize_cached_profile(account: &mut CachedAccount) {
    // Secrets and account-scoped auth identifiers do not belong in metadata
    // cache files. Access tokens stay in memory only, and refresh tokens are
    // persisted separately in OS-backed secure storage.
    account.minecraft_access_token = None;
    account.microsoft_refresh_token = None;
    account.xuid = None;

    // Keep lightweight identity/profile metadata only; avoid stale or heavy
    // texture payloads in the on-disk cache.
    account.minecraft_profile.skins.clear();
    for cape in &mut account.minecraft_profile.capes {
        cape.state.clear();
        cape.texture_png_base64 = None;
    }
}

fn finalize_loaded_accounts(
    mut state: CachedAccountsState,
) -> Result<CachedAccountsState, AuthError> {
    let mut migrated_plaintext_token = false;

    for account in &mut state.accounts {
        let profile_id = account.minecraft_profile.id.trim();
        if profile_id.is_empty() {
            account.microsoft_refresh_token = None;
            account.refresh_token_state = RefreshTokenState::Missing;
            continue;
        }

        if let Some(token) = account
            .microsoft_refresh_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            secret_store::store_refresh_token(profile_id, token)?;
            migrated_plaintext_token = true;
        }

        match secret_store::load_refresh_token(profile_id)? {
            RefreshTokenLoadResult::Present(token) => {
                account.microsoft_refresh_token = Some(Zeroizing::new(token).to_string());
                account.refresh_token_state = RefreshTokenState::Present;
            }
            RefreshTokenLoadResult::Missing => {
                account.microsoft_refresh_token = None;
                account.refresh_token_state = RefreshTokenState::Missing;
            }
            RefreshTokenLoadResult::Unavailable => {
                account.microsoft_refresh_token = None;
                account.refresh_token_state = RefreshTokenState::Unavailable;
            }
        }
    }

    if migrated_plaintext_token {
        save_cached_accounts(&state)?;
    }

    Ok(state)
}

fn persist_refresh_token(account: &mut CachedAccount) -> Result<(), AuthError> {
    let profile_id = account.minecraft_profile.id.trim();
    if profile_id.is_empty() {
        account.microsoft_refresh_token = None;
        account.refresh_token_state = RefreshTokenState::Missing;
        return Ok(());
    }

    match refresh_token_persist_action(account) {
        RefreshTokenPersistAction::Store(token) => {
            secret_store::store_refresh_token(profile_id, token)?;
            account.refresh_token_state = RefreshTokenState::Present;
        }
        RefreshTokenPersistAction::Delete => {
            secret_store::delete_refresh_token(profile_id)?;
            account.refresh_token_state = RefreshTokenState::Missing;
        }
        RefreshTokenPersistAction::Preserve => {
            tracing::warn!(
                target: "vertexlauncher/auth/cache",
                profile_id,
                "skipping refresh-token write-back because its secure-storage state is unavailable"
            );
        }
    }

    Ok(())
}

enum RefreshTokenPersistAction<'a> {
    Store(&'a str),
    Delete,
    Preserve,
}

fn refresh_token_persist_action(account: &CachedAccount) -> RefreshTokenPersistAction<'_> {
    if let Some(token) = account
        .microsoft_refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        return RefreshTokenPersistAction::Store(token);
    }

    match account.refresh_token_state {
        RefreshTokenState::Present | RefreshTokenState::Missing => {
            RefreshTokenPersistAction::Delete
        }
        RefreshTokenState::Unavailable | RefreshTokenState::Unknown => {
            RefreshTokenPersistAction::Preserve
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RefreshTokenPersistAction, refresh_token_persist_action, sanitize_cached_profile};
    use crate::types::{
        CachedAccount, MinecraftCapeState, MinecraftProfileState, MinecraftSkinState,
        RefreshTokenState,
    };

    #[test]
    fn sanitize_cached_profile_removes_sensitive_fields() {
        let mut account = CachedAccount {
            minecraft_profile: MinecraftProfileState {
                id: "profile-id".to_owned(),
                name: "Player".to_owned(),
                skins: vec![MinecraftSkinState {
                    id: "skin-id".to_owned(),
                    state: "active".to_owned(),
                    url: "https://example.invalid/skin".to_owned(),
                    variant: Some("classic".to_owned()),
                    alias: Some("Default".to_owned()),
                    texture_png_base64: Some("skin-bytes".to_owned()),
                }],
                capes: vec![MinecraftCapeState {
                    id: "cape-id".to_owned(),
                    state: "active".to_owned(),
                    url: "https://example.invalid/cape".to_owned(),
                    alias: Some("Cape".to_owned()),
                    texture_png_base64: Some("cape-bytes".to_owned()),
                }],
            },
            minecraft_access_token: Some("minecraft-access-token".to_owned()),
            microsoft_refresh_token: Some("microsoft-refresh-token".to_owned()),
            microsoft_client_id: Some("client-id".to_owned()),
            microsoft_token_uri: Some("https://example.invalid/token".to_owned()),
            microsoft_scope: Some("offline_access".to_owned()),
            refresh_token_state: RefreshTokenState::Present,
            xuid: Some("xuid-value".to_owned()),
            user_type: Some("msa".to_owned()),
            avatar_png_base64: Some("avatar-bytes".to_owned()),
            avatar_source_skin_url: Some("https://example.invalid/avatar".to_owned()),
            cached_at_unix_secs: 123,
        };

        sanitize_cached_profile(&mut account);

        assert!(account.minecraft_access_token.is_none());
        assert!(account.microsoft_refresh_token.is_none());
        assert!(account.xuid.is_none());
        assert_eq!(account.user_type.as_deref(), Some("msa"));
        assert_eq!(
            account.avatar_source_skin_url.as_deref(),
            Some("https://example.invalid/avatar")
        );
        assert!(account.minecraft_profile.skins.is_empty());
        assert_eq!(account.minecraft_profile.capes.len(), 1);
        assert!(account.minecraft_profile.capes[0].state.is_empty());
        assert!(
            account.minecraft_profile.capes[0]
                .texture_png_base64
                .is_none()
        );
    }

    #[test]
    fn preserves_refresh_token_when_secure_store_state_is_unavailable() {
        let account = CachedAccount {
            minecraft_profile: MinecraftProfileState {
                id: "profile-id".to_owned(),
                name: "Player".to_owned(),
                skins: Vec::new(),
                capes: Vec::new(),
            },
            minecraft_access_token: None,
            microsoft_refresh_token: None,
            microsoft_client_id: None,
            microsoft_token_uri: None,
            microsoft_scope: None,
            refresh_token_state: RefreshTokenState::Unavailable,
            xuid: None,
            user_type: Some("msa".to_owned()),
            avatar_png_base64: None,
            avatar_source_skin_url: None,
            cached_at_unix_secs: 123,
        };

        assert!(matches!(
            refresh_token_persist_action(&account),
            RefreshTokenPersistAction::Preserve
        ));
    }
}
