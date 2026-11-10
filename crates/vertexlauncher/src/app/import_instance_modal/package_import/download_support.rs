use vertex_i18n::tr;
use super::*;

pub(crate) fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    throttle_download_url(url);
    let mut response = ureq::get(url)
        .call()
        .map_err(|err| tr!("download request failed for {url}: {err}", url = url, err = err).into_owned())?;
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|err| tr!("failed to read download body from {url}: {err}", url = url, err = err).into_owned())?;
    fs_write_logged(destination, bytes)
        .map_err(|err| tr!("failed to write {destination}: {err}", destination = destination.display(), err = err).into_owned())
}

pub(crate) fn throttle_download_url(url: &str) {
    let Some(spacing) = download_spacing_for_url(url) else {
        return;
    };
    let lock = download_throttle_store(url);
    let Ok(mut next_allowed) = lock.lock() else {
        tracing::error!(
            target: "vertexlauncher/import_instance",
            url,
            throttle_spacing_ms = spacing.as_millis() as u64,
            "Import-instance download throttle mutex was poisoned."
        );
        return;
    };
    let now = Instant::now();
    if *next_allowed > now {
        std::thread::sleep(next_allowed.saturating_duration_since(now));
    }
    *next_allowed = Instant::now() + spacing;
}

pub(crate) fn download_spacing_for_url(url: &str) -> Option<Duration> {
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        Some(MODRINTH_DOWNLOAD_MIN_SPACING)
    } else if host.contains("curseforge.com") || host.contains("forgecdn.net") {
        Some(CURSEFORGE_DOWNLOAD_MIN_SPACING)
    } else {
        None
    }
}

pub(crate) fn download_throttle_store(url: &str) -> &'static Mutex<Instant> {
    static MODRINTH: OnceLock<Mutex<Instant>> = OnceLock::new();
    static CURSEFORGE: OnceLock<Mutex<Instant>> = OnceLock::new();
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host.contains("modrinth.com") {
        MODRINTH.get_or_init(|| Mutex::new(Instant::now()))
    } else {
        CURSEFORGE.get_or_init(|| Mutex::new(Instant::now()))
    }
}
