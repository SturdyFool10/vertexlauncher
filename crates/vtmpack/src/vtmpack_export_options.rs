use std::collections::BTreeMap;

use crate::{VtmpackCompressionMode, VtmpackProviderMode};

#[derive(Debug, Clone)]
pub struct VtmpackExportOptions {
    pub provider_mode: VtmpackProviderMode,
    pub compression_mode: VtmpackCompressionMode,
    pub included_root_entries: BTreeMap<String, bool>,
}

impl Default for VtmpackExportOptions {
    fn default() -> Self {
        Self {
            provider_mode: VtmpackProviderMode::ExcludeCurseForge,
            compression_mode: VtmpackCompressionMode::Standard,
            included_root_entries: BTreeMap::new(),
        }
    }
}
