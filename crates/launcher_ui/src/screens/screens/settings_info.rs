#[derive(Debug, Clone, Default)]
pub struct SettingsGraphicsAdapterInfo {
    pub label: String,
    pub hash: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsInfo {
    pub cpu: String,
    pub gpu: String,
    pub memory: String,
    pub graphics_api: String,
    pub graphics_driver: String,
    pub app_version: String,
    pub available_graphics_adapters: Vec<SettingsGraphicsAdapterInfo>,
}
