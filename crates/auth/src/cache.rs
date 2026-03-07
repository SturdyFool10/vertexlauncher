use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::constants::{ACCOUNT_CACHE_APP_DIR, ACCOUNT_CACHE_FILENAME, LEGACY_ACCOUNT_CACHE_PATH};
use crate::error::AuthError;
use crate::types::{CachedAccount, CachedAccountsState};

#[track_caller]
fn fs_read_to_string(path: impl AsRef<Path>) -> Result<String, AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    Ok(fs::read_to_string(path)?)
}

#[track_caller]
fn fs_read(path: impl AsRef<Path>) -> Result<Vec<u8>, AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read", path = %path.display());
    Ok(fs::read(path)?)
}

#[track_caller]
fn fs_remove_file(path: impl AsRef<Path>) -> Result<(), AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "remove_file", path = %path.display());
    Ok(fs::remove_file(path)?)
}

#[track_caller]
fn fs_create_dir_all(path: impl AsRef<Path>) -> Result<(), AuthError> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    Ok(fs::create_dir_all(path)?)
}

#[track_caller]
fn fs_open_options_create_truncate_write(path: impl AsRef<Path>) -> Result<fs::File, AuthError> {
    let path = path.as_ref();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "open_options(create,truncate,write)",
        path = %path.display()
    );
    Ok(fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?)
}

#[track_caller]
fn fs_rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), AuthError> {
    let from = from.as_ref();
    let to = to.as_ref();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "rename",
        from = %from.display(),
        to = %to.display()
    );
    Ok(fs::rename(from, to)?)
}

pub(crate) fn load_cached_accounts() -> Result<CachedAccountsState, AuthError> {
    let path = account_cache_path();
    maybe_migrate_legacy_account_cache(&path)?;

    if !path.exists() {
        tracing::debug!(
            target: "vertexlauncher/auth/cache",
            path = %path.display(),
            "account cache file missing; using empty cache"
        );
        return Ok(CachedAccountsState::default());
    }

    let contents = fs_read_to_string(&path)?;

    match serde_json::from_str::<CachedAccountsState>(&contents) {
        Ok(state) => Ok(state.normalize()),
        Err(state_error) => {
            // Backward compatibility with the old single-account cache format.
            if let Ok(single_account) = serde_json::from_str::<CachedAccount>(&contents) {
                tracing::info!(
                    target: "vertexlauncher/auth/cache",
                    "migrated single-account cache format into multi-account state"
                );
                let mut state = CachedAccountsState::default();
                state.upsert_and_activate(single_account);
                return Ok(state);
            }

            tracing::warn!(
                target: "vertexlauncher/auth/cache",
                path = %path.display(),
                error = %state_error,
                "failed to parse account cache"
            );
            Err(AuthError::Json(state_error))
        }
    }
}

pub(crate) fn save_cached_accounts(state: &CachedAccountsState) -> Result<(), AuthError> {
    let path = account_cache_path();
    maybe_migrate_legacy_account_cache(&path)?;
    let json = serde_json::to_string_pretty(&state.clone().normalize())?;
    write_secure_file_atomic(&path, json.as_bytes())
}

pub(crate) fn clear_cached_accounts() -> Result<(), AuthError> {
    let path = account_cache_path();
    maybe_migrate_legacy_account_cache(&path)?;

    if path.exists() {
        fs_remove_file(path)?;
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
    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            if !local_app_data.trim().is_empty() {
                return PathBuf::from(local_app_data)
                    .join(ACCOUNT_CACHE_APP_DIR)
                    .join(ACCOUNT_CACHE_FILENAME);
            }
        }

        if let Ok(app_data) = std::env::var("APPDATA") {
            if !app_data.trim().is_empty() {
                return PathBuf::from(app_data)
                    .join(ACCOUNT_CACHE_APP_DIR)
                    .join(ACCOUNT_CACHE_FILENAME);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                return PathBuf::from(home)
                    .join("Library")
                    .join("Application Support")
                    .join(ACCOUNT_CACHE_APP_DIR)
                    .join(ACCOUNT_CACHE_FILENAME);
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
            if !state_home.trim().is_empty() {
                return PathBuf::from(state_home)
                    .join(ACCOUNT_CACHE_APP_DIR)
                    .join(ACCOUNT_CACHE_FILENAME);
            }
        }

        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                return PathBuf::from(home)
                    .join(".local")
                    .join("state")
                    .join(ACCOUNT_CACHE_APP_DIR)
                    .join(ACCOUNT_CACHE_FILENAME);
            }
        }
    }

    PathBuf::from(ACCOUNT_CACHE_FILENAME)
}

fn maybe_migrate_legacy_account_cache(target_path: &Path) -> Result<(), AuthError> {
    let legacy_path = PathBuf::from(LEGACY_ACCOUNT_CACHE_PATH);

    if target_path == legacy_path || target_path.exists() || !legacy_path.exists() {
        return Ok(());
    }

    let bytes = fs_read(&legacy_path)?;
    write_secure_file_atomic(target_path, &bytes)?;
    fs_remove_file(&legacy_path)?;
    tracing::info!(
        target: "vertexlauncher/auth/cache",
        legacy_path = %legacy_path.display(),
        target_path = %target_path.display(),
        "migrated legacy account cache path"
    );
    Ok(())
}

fn write_secure_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs_create_dir_all(parent)?;
        }
    }

    let temp_path = path.with_extension("tmp");

    {
        let mut file = fs_open_options_create_truncate_write(&temp_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
    }

    #[cfg(windows)]
    {
        // Atomic rename over an existing file is not always available on Windows,
        // so best-effort remove first before rename.
        if path.exists() {
            let _ = fs_remove_file(path);
        }
    }

    fs_rename(temp_path, path)?;
    Ok(())
}
