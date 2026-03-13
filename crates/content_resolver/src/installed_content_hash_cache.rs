use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{InstalledContentHashCacheUpdate, ResolvedInstalledContent};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledContentHashCache {
    #[serde(default = "default_hash_cache_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: HashMap<String, Option<ResolvedInstalledContent>>,
}

impl Default for InstalledContentHashCache {
    fn default() -> Self {
        Self {
            version: default_hash_cache_version(),
            entries: HashMap::new(),
        }
    }
}

impl InstalledContentHashCache {
    pub fn into_current(mut self) -> Option<Self> {
        match self.version {
            version if version == default_hash_cache_version() => Some(self),
            1 => {
                // Version 1 cached transient Modrinth failures as permanent misses.
                self.entries.retain(|_, resolution| resolution.is_some());
                self.version = default_hash_cache_version();
                Some(self)
            }
            _ => None,
        }
    }

    pub fn apply_updates(
        &mut self,
        updates: impl IntoIterator<Item = InstalledContentHashCacheUpdate>,
    ) -> bool {
        let mut changed = false;
        for update in updates {
            let previous = self
                .entries
                .insert(update.hash_key, update.resolution.clone());
            if previous != Some(update.resolution) {
                changed = true;
            }
        }
        changed
    }
}

fn default_hash_cache_version() -> u32 {
    2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InstalledContentResolutionKind;
    use modprovider::{ContentSource, UnifiedContentEntry};

    fn resolution(name: &str) -> ResolvedInstalledContent {
        ResolvedInstalledContent {
            entry: UnifiedContentEntry {
                id: format!("modrinth:{name}"),
                name: name.to_owned(),
                summary: String::new(),
                content_type: "mod".to_owned(),
                source: ContentSource::Modrinth,
                project_url: None,
                icon_url: None,
            },
            installed_version_id: None,
            installed_version_label: None,
            resolution_kind: InstalledContentResolutionKind::ExactHash,
            warning_message: None,
            update: None,
        }
    }

    #[test]
    fn v1_cache_migration_discards_negative_entries() {
        let mut entries = HashMap::new();
        entries.insert("sha1:abc".to_owned(), None);
        entries.insert("sha1:def".to_owned(), Some(resolution("Sodium")));

        let migrated = InstalledContentHashCache {
            version: 1,
            entries,
        }
        .into_current()
        .expect("expected cache migration to succeed");

        assert_eq!(migrated.version, default_hash_cache_version());
        assert_eq!(migrated.entries.len(), 1);
        assert!(migrated.entries.contains_key("sha1:def"));
    }
}
