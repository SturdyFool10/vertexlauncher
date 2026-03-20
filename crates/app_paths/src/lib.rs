use directories::ProjectDirs;
use std::path::PathBuf;

const APP_DIR_NAME: &str = "vertexlauncher";
const LEGACY_APP_DIR_NAME: &str = "vertex-launcher";

pub fn portable_root() -> Option<PathBuf> {
    explicit_portable_root().or_else(appimage_portable_root)
}

fn explicit_portable_root() -> Option<PathBuf> {
    std::env::var("VERTEX_CONFIG_LOCATION")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn appimage_portable_root() -> Option<PathBuf> {
    std::env::var("APPIMAGE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .and_then(|path| {
            let file_name = path.file_name()?.to_string_lossy().into_owned();
            Some(path.with_file_name(format!("{file_name}.data")))
        })
}

fn project_dirs_for(app_dir_name: &str) -> Option<ProjectDirs> {
    ProjectDirs::from("", "", app_dir_name)
}

fn project_dirs() -> Option<ProjectDirs> {
    project_dirs_for(APP_DIR_NAME)
}

fn legacy_project_dirs() -> Option<ProjectDirs> {
    project_dirs_for(LEGACY_APP_DIR_NAME)
}

fn fallback_root() -> PathBuf {
    PathBuf::from(APP_DIR_NAME)
}

pub fn config_base_path() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("config");
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.config_dir().join("config");
    }
    fallback_root().join("config")
}

pub fn config_root() -> PathBuf {
    if let Some(root) = portable_root() {
        return root;
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.config_dir().to_path_buf();
    }
    fallback_root()
}

pub fn instances_store_path() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("instances.json");
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.config_dir().join("instances.json");
    }
    fallback_root().join("instances.json")
}

pub fn installations_root() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("instances");
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.data_local_dir().join("instances");
    }
    fallback_root().join("instances")
}

pub fn cache_root() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("cache");
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.cache_dir().to_path_buf();
    }
    fallback_root().join("cache")
}

pub fn logs_dir() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("logs");
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.data_local_dir().join("logs");
    }
    fallback_root().join("logs")
}

pub fn themes_dir() -> PathBuf {
    if let Some(root) = portable_root() {
        return root.join("themes");
    }
    if let Some(project_dirs) = project_dirs() {
        return project_dirs.config_dir().join("themes");
    }
    fallback_root().join("themes")
}

pub fn legacy_config_base_path() -> Option<PathBuf> {
    legacy_project_dirs().map(|project_dirs| project_dirs.config_dir().join("config"))
}

pub fn legacy_instances_store_path() -> Option<PathBuf> {
    legacy_project_dirs().map(|project_dirs| project_dirs.config_dir().join("instances.json"))
}

pub fn legacy_themes_dir() -> Option<PathBuf> {
    legacy_project_dirs().map(|project_dirs| project_dirs.config_dir().join("themes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        sync::{Mutex, OnceLock},
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvReset {
        vertex_config_location: Option<OsString>,
        appimage: Option<OsString>,
    }

    impl EnvReset {
        fn capture() -> Self {
            Self {
                vertex_config_location: std::env::var_os("VERTEX_CONFIG_LOCATION"),
                appimage: std::env::var_os("APPIMAGE"),
            }
        }
    }

    impl Drop for EnvReset {
        fn drop(&mut self) {
            restore_env(
                "VERTEX_CONFIG_LOCATION",
                self.vertex_config_location.as_ref(),
            );
            restore_env("APPIMAGE", self.appimage.as_ref());
        }
    }

    fn restore_env(key: &str, value: Option<&OsString>) {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn portable_root_prefers_explicit_vertex_config_location() {
        let _lock = env_lock().lock().unwrap();
        let _reset = EnvReset::capture();

        unsafe {
            std::env::set_var("VERTEX_CONFIG_LOCATION", "/tmp/vertex-portable");
            std::env::set_var("APPIMAGE", "/opt/Vertex/VertexLauncher.AppImage");
        }

        assert_eq!(portable_root(), Some(PathBuf::from("/tmp/vertex-portable")));
    }

    #[test]
    fn portable_root_falls_back_to_appimage_sibling_data_dir() {
        let _lock = env_lock().lock().unwrap();
        let _reset = EnvReset::capture();

        unsafe {
            std::env::remove_var("VERTEX_CONFIG_LOCATION");
            std::env::set_var("APPIMAGE", "/opt/Vertex/VertexLauncher-x86_64.AppImage");
        }

        assert_eq!(
            portable_root(),
            Some(PathBuf::from(
                "/opt/Vertex/VertexLauncher-x86_64.AppImage.data"
            ))
        );
    }

    #[test]
    fn empty_env_values_do_not_enable_portable_mode() {
        let _lock = env_lock().lock().unwrap();
        let _reset = EnvReset::capture();

        unsafe {
            std::env::set_var("VERTEX_CONFIG_LOCATION", "   ");
            std::env::set_var("APPIMAGE", "   ");
        }

        assert_eq!(portable_root(), None);
    }
}
