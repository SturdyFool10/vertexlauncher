use super::*;

pub(crate) fn download_file_simple(url: &str, destination: &Path) -> Result<(), InstallationError> {
    if destination.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs_create_dir_all(parent)?;
    }
    let response = ureq::get(url)
        .header("User-Agent", OPENJDK_USER_AGENT)
        .call()
        .map_err(map_ureq_error)?;
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
    let temp = temporary_download_path(destination);
    let mut file = fs_file_create(&temp)?;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url,
                temp_path = %temp.display(),
                destination = %destination.display(),
                error = %err,
                "Failed while reading OpenJDK download response body."
            );
            InstallationError::Io(err)
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|err| {
            tracing::error!(
                target: "vertexlauncher/installation/downloads",
                url,
                temp_path = %temp.display(),
                destination = %destination.display(),
                error = %err,
                "Failed while writing OpenJDK download chunk to temporary file."
            );
            InstallationError::Io(err)
        })?;
    }
    file.flush().map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url,
            temp_path = %temp.display(),
            destination = %destination.display(),
            error = %err,
            "Failed while flushing OpenJDK temporary download file."
        );
        InstallationError::Io(err)
    })?;
    fs_rename(temp.as_path(), destination).map_err(|err| {
        tracing::error!(
            target: "vertexlauncher/installation/downloads",
            url,
            temp_path = %temp.display(),
            destination = %destination.display(),
            error = %err,
            "Failed while promoting OpenJDK temporary download file into place."
        );
        err
    })?;
    Ok(())
}

pub(crate) fn temporary_download_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.downloading"))
        .unwrap_or_else(|| "download.downloading".to_owned());
    destination.with_file_name(file_name)
}

pub(crate) fn extract_archive(
    archive_path: &Path,
    destination: &Path,
) -> Result<(), InstallationError> {
    if destination.exists() {
        fs_remove_dir_all(destination)?;
    }
    fs_create_dir_all(destination)?;
    let file_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if file_name.ends_with(".zip") {
        let file = fs_file_open(archive_path)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|err| InstallationError::Io(std::io::Error::other(err.to_string())))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|err| InstallationError::Io(std::io::Error::other(err.to_string())))?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            let out_path = destination.join(enclosed);
            if entry.is_dir() {
                fs_create_dir_all(&out_path)?;
                continue;
            }
            if let Some(parent) = out_path.parent() {
                fs_create_dir_all(parent)?;
            }
            let mut out = fs_file_create(&out_path)?;
            std::io::copy(&mut entry, &mut out)?;
        }
        return Ok(());
    }

    if file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz") {
        let tar_gz = fs_file_open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(tar_gz);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(destination)?;
        return Ok(());
    }

    Err(InstallationError::Io(std::io::Error::new(
        ErrorKind::InvalidInput,
        format!("unsupported archive format: {}", archive_path.display()),
    )))
}

pub(crate) fn find_java_executable_under(
    root: &Path,
) -> Result<Option<PathBuf>, InstallationError> {
    if !root.exists() {
        return Ok(None);
    }
    let binary = if cfg!(target_os = "windows") {
        "java.exe"
    } else {
        "java"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs_read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.eq_ignore_ascii_case(binary)
                && path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .is_some_and(|part| part.eq_ignore_ascii_case("bin"))
            {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}
