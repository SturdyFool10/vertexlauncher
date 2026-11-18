use super::*;

pub(super) fn map_join_error(err: launcher_runtime::TaskError) -> InstallationError {
    InstallationError::Io(std::io::Error::other(format!(
        "background task failed: {err}"
    )))
}
