use super::*;

pub fn display_user_path(path: &Path) -> String {
    #[cfg(target_os = "windows")]
    {
        return normalize_windows_cli_path(path.as_os_str().to_string_lossy().as_ref());
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.as_os_str().to_string_lossy().into_owned()
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn normalize_windows_cli_path(raw: &str) -> String {
    if let Some(stripped) = raw.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{stripped}");
    }
    if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        return stripped.to_owned();
    }
    raw.to_owned()
}

pub(crate) fn normalize_child_process_path(path: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        return PathBuf::from(normalize_windows_cli_path(
            path.as_os_str().to_string_lossy().as_ref(),
        ));
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.to_path_buf()
    }
}

pub fn normalize_path_key(path: &Path) -> String {
    let normalized = fs_canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    display_user_path(normalized.as_path())
}

#[track_caller]
pub(crate) fn fs_create_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display());
    let result = fs::create_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "create_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_remove_dir_all(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display());
    let result = fs::remove_dir_all(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "remove_dir_all", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_read_to_string(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display());
    let result = fs::read_to_string(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_to_string", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_read_dir(path: impl AsRef<Path>) -> std::io::Result<fs::ReadDir> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "read_dir", path = %path.display());
    let result = fs::read_dir(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "read_dir", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    let from = from.as_ref();
    let to = to.as_ref();
    tracing::debug!(
        target: "vertexlauncher/io",
        op = "rename",
        from = %from.display(),
        to = %to.display()
    );
    let result = fs::rename(from, to);
    if let Err(err) = &result {
        tracing::warn!(
            target: "vertexlauncher/io",
            op = "rename",
            from = %from.display(),
            to = %to.display(),
            error = %err
        );
    }
    result
}

#[track_caller]
pub(crate) fn fs_canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "canonicalize", path = %path.display());
    let result = fs::canonicalize(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "canonicalize", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "write", path = %path.display());
    let result = fs::write(path, contents);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "write", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_file_create(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_create", path = %path.display());
    let result = fs::File::create(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "file_create", path = %path.display(), error = %err);
    }
    result
}

#[track_caller]
pub(crate) fn fs_file_open(path: impl AsRef<Path>) -> std::io::Result<fs::File> {
    let path = path.as_ref();
    tracing::debug!(target: "vertexlauncher/io", op = "file_open", path = %path.display());
    let result = fs::File::open(path);
    if let Err(err) = &result {
        tracing::warn!(target: "vertexlauncher/io", op = "file_open", path = %path.display(), error = %err);
    }
    result
}
