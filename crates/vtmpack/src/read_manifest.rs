use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::{VTMPACK_EXTENSION, VtmpackManifest};

const XZ_MAGIC: &[u8] = &[0xfd, b'7', b'z', b'X', b'Z', 0x00];

#[must_use]
pub fn default_vtmpack_file_name(instance_name: &str) -> String {
    let mut out = String::new();
    for ch in instance_name.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_whitespace() || ch == '.' {
            out.push('-');
        }
    }
    let base = out.trim_matches('-');
    if base.is_empty() {
        format!("instance.{VTMPACK_EXTENSION}")
    } else {
        format!("{base}.{VTMPACK_EXTENSION}")
    }
}

#[must_use]
pub fn enforce_vtmpack_extension(mut path: PathBuf) -> PathBuf {
    let has_extension = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(VTMPACK_EXTENSION));
    if !has_extension {
        path.set_extension(VTMPACK_EXTENSION);
    }
    path
}

pub fn read_vtmpack_manifest(path: &Path) -> Result<VtmpackManifest, String> {
    let archive = open_vtmpack_tar_archive(path)?;
    read_vtmpack_manifest_from_tar_archive(path, archive)
}

pub fn read_vtmpack_manifest_with_progress<F>(
    path: &Path,
    _progress: F,
) -> Result<VtmpackManifest, String>
where
    F: FnMut(u64),
{
    read_vtmpack_manifest(path)
}

pub fn read_vtmpack_manifest_from_tar_archive(
    path: &Path,
    mut archive: tar::Archive<Box<dyn Read>>,
) -> Result<VtmpackManifest, String> {
    for entry in archive
        .entries()
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?
    {
        let mut entry = entry.map_err(|err| format!("failed to read archive entry: {err}"))?;
        let entry_path = entry
            .path()
            .map_err(|err| format!("failed to decode archive path: {err}"))?;
        if entry_path == Path::new("manifest.toml") {
            let mut raw = String::new();
            entry
                .read_to_string(&mut raw)
                .map_err(|err| format!("failed to read manifest.toml: {err}"))?;
            return toml::from_str(&raw)
                .map_err(|err| format!("failed to parse vtmpack manifest: {err}"));
        }
    }

    Err(format!(
        "No manifest.toml found in Vertex pack {}",
        path.display()
    ))
}

pub fn open_vtmpack_tar_archive(path: &Path) -> Result<tar::Archive<Box<dyn Read>>, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    if !bytes.starts_with(XZ_MAGIC) {
        return Err(format!(
            "Unsupported Vertex pack compression in {}. Expected xz.",
            path.display()
        ));
    }
    let decoder = xz2::read::XzDecoder::new(Cursor::new(bytes));
    Ok(tar::Archive::new(Box::new(decoder)))
}

pub fn open_vtmpack_tar_archive_with_progress<F>(
    path: &Path,
    _progress: F,
) -> Result<tar::Archive<Box<dyn Read>>, String>
where
    F: FnMut(u64),
{
    open_vtmpack_tar_archive(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{VTMPACK_MANIFEST_VERSION, VtmpackInstanceMetadata};

    #[test]
    fn reads_xz_vtmpack_manifest() {
        let manifest = VtmpackManifest {
            format: "vtmpack".to_owned(),
            version: VTMPACK_MANIFEST_VERSION,
            instance: VtmpackInstanceMetadata {
                name: "XZ Pack".to_owned(),
                game_version: "1.20.1".to_owned(),
                modloader: "Fabric".to_owned(),
                ..VtmpackInstanceMetadata::default()
            },
            ..VtmpackManifest::default()
        };
        let manifest_bytes = toml::to_string_pretty(&manifest)
            .expect("serialize test manifest")
            .into_bytes();
        let mut tar_bytes = Vec::new();
        {
            let mut archive = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(manifest_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, "manifest.toml", manifest_bytes.as_slice())
                .expect("append manifest");
            archive.finish().expect("finish tar");
        }

        let path = std::env::temp_dir().join(format!(
            "vertexlauncher-xz-vtmpack-test-{}.vtmpack",
            std::process::id()
        ));
        let file = fs::File::create(path.as_path()).expect("create xz test pack");
        let mut encoder = xz2::write::XzEncoder::new(file, 6);
        std::io::copy(&mut Cursor::new(tar_bytes), &mut encoder).expect("compress xz test pack");
        encoder.finish().expect("finish xz test pack");

        let parsed = read_vtmpack_manifest(path.as_path()).expect("read xz manifest");
        let _ = fs::remove_file(path.as_path());

        assert_eq!(parsed.format, "vtmpack");
        assert_eq!(parsed.instance.name, "XZ Pack");
    }
}
