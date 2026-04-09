use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ScreenshotEntry {
    pub(crate) instance_name: String,
    pub(crate) path: PathBuf,
    pub(crate) file_name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) modified_at_ms: Option<u64>,
}

impl ScreenshotEntry {
    pub(crate) fn key(&self) -> String {
        self.path.to_string_lossy().to_string()
    }

    pub(crate) fn uri(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.path.hash(&mut hasher);
        self.modified_at_ms.hash(&mut hasher);
        format!("bytes://home/screenshot/{}.png", hasher.finish())
    }

    pub(crate) fn aspect_ratio(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }
}
