use vertex_i18n::tr;
use super::*;

pub(super) fn validate_move_dest(path_str: &str) -> Result<PathBuf, String> {
    let path_str = path_str.trim();
    if path_str.is_empty() {
        return Err(tr!("Please enter or browse to a destination folder.").into_owned());
    }
    let path = PathBuf::from(path_str);
    if path.is_dir() {
        match std::fs::read_dir(&path) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(tr!("The destination folder must be empty.").into_owned());
                }
            }
            Err(err) => {
                return Err(tr!("Cannot read destination folder: {err}", err = err).into_owned());
            }
        }
        return Ok(path);
    }
    if path.exists() {
        return Err(tr!("The destination path already exists and is not a folder.").into_owned());
    }
    match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() || parent.is_dir() => {}
        Some(parent) => {
            return Err(tr!("Parent folder does not exist: {parent}", parent = parent.display()).into_owned());
        }
        None => {
            return Err(tr!("The destination path is invalid.").into_owned());
        }
    }
    Ok(path)
}

pub(super) fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        tr!("{size:.1} GB", size = bytes as f64 / 1_073_741_824.0).into_owned()
    } else if bytes >= 1_048_576 {
        tr!("{size:.1} MB", size = bytes as f64 / 1_048_576.0).into_owned()
    } else if bytes >= 1024 {
        tr!("{size:.1} KB", size = bytes as f64 / 1024.0).into_owned()
    } else {
        tr!("{bytes} B", bytes = bytes).into_owned()
    }
}






