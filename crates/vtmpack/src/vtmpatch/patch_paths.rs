use vertex_i18n::tr;
use super::*;

// ── Archive helpers ───────────────────────────────────────────────────────────

pub(super) fn open_patch_archive(path: &Path) -> Result<tar::Archive<Box<dyn Read>>, String> {
    let bytes =
        fs::read(path).map_err(|err| tr!("failed to open {path}: {err}", path = path.display(), err = err).into_owned())?;
    let decoder = xz2::read::XzDecoder::new(Cursor::new(bytes));
    Ok(tar::Archive::new(Box::new(decoder)))
}

pub(super) fn archive_path_to_instance_path(
    instance_root: &Path,
    archive_path: &Path,
) -> Result<PathBuf, String> {
    let archive_name = path_to_string(archive_path);
    if archive_name == PATCH_MANIFEST_PATH || archive_name == "manifest.toml" {
        return Err(tr!("patch entry {archive_name} is not an instance file", archive_name = archive_name).into_owned());
    }
    if archive_name == CONTENT_MANIFEST_ARCHIVE_PATH {
        return Ok(instance_root.join(CONTENT_MANIFEST_FILE_NAME));
    }
    if let Some(relative) = archive_name.strip_prefix("bundled_mods/") {
        return join_safe(&instance_root.join("mods"), Path::new(relative));
    }
    if let Some(relative) = archive_name.strip_prefix("configs/") {
        return join_safe(&instance_root.join("config"), Path::new(relative));
    }
    if let Some(relative) = archive_name.strip_prefix("root_entries/") {
        return join_safe(instance_root, Path::new(relative));
    }
    Err(tr!("unsupported patch entry path: {archive_name}", archive_name = archive_name).into_owned())
}

pub(super) fn join_safe(base: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut out = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(tr!("unsafe patch path component in {relative}", relative = relative.display()).into_owned());
            }
        }
    }
    Ok(out)
}

pub(super) fn instance_relative_to_archive_path(path: &Path) -> PathBuf {
    let normalized = normalize_pack_path(path);
    let text = path_to_string(&normalized);
    if let Some(relative) = text.strip_prefix("mods/") {
        Path::new("bundled_mods").join(relative)
    } else if let Some(relative) = text.strip_prefix("config/") {
        Path::new("configs").join(relative)
    } else {
        Path::new("root_entries").join(normalized)
    }
}

pub(super) fn collect_regular_files_recursive(
    root: &Path,
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_regular_files_recursive(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

pub(super) fn path_to_archive_name(path: &Path) -> Result<String, String> {
    let value = path_to_string(path);
    if value.trim().is_empty() {
        Err(tr!("empty archive path").into_owned())
    } else {
        Ok(value)
    }
}

pub(super) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn hash_bytes_sha512_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha512::new();
    Sha2Digest::update(&mut hasher, bytes);
    bytes_to_lower_hex(Sha2Digest::finalize(hasher).as_slice())
}

pub(super) fn bytes_to_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(super) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
