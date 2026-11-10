use vertex_i18n::tr;
use super::*;
use content_resolver::ResolvedInstalledContent;

/// Owned display strings for one installed-content row, derived from the disk entry and its
/// resolved metadata (when the lookup has finished).
pub(super) struct InstalledEntryDisplay {
    pub(super) display_name: String,
    pub(super) description: String,
    pub(super) platform_label: String,
    pub(super) version_label: Option<String>,
    pub(super) warning_message: Option<String>,
    pub(super) update_label: Option<String>,
    pub(super) update_version_id: Option<String>,
    pub(super) icon_url: Option<String>,
}

impl InstalledEntryDisplay {
    pub(super) fn new(
        entry: &InstalledContentFile,
        metadata: Option<&ResolvedInstalledContent>,
    ) -> Self {
        // For disabled mods the disk name ends with .DISABLED; strip it for display.
        let effective_file_name = if entry.disabled {
            entry
                .file_path
                .with_extension("")
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.file_name.clone())
        } else {
            entry.file_name.clone()
        };
        let display_name = metadata
            .map(|value| value.entry.name.clone())
            .unwrap_or_else(|| effective_file_name.clone());
        let description = metadata
            .map(|value| {
                if value.entry.summary.trim().is_empty() {
                    effective_file_name.clone()
                } else {
                    value.entry.summary.clone()
                }
            })
            .unwrap_or_else(|| effective_file_name.clone());
        Self {
            display_name,
            description,
            platform_label: metadata
                .map(|value| value.entry.source.label().to_owned())
                .unwrap_or_else(|| tr!("Unknown").into_owned()),
            version_label: metadata.and_then(|value| value.installed_version_label.clone()),
            warning_message: metadata.and_then(|value| value.warning_message.clone()),
            update_label: metadata
                .and_then(|value| value.update.as_ref())
                .map(|update| tr!("Update: {version}", version = update.latest_version_label).into_owned()),
            update_version_id: metadata
                .and_then(|value| value.update.as_ref())
                .map(|update| update.latest_version_id.clone()),
            icon_url: metadata
                .and_then(|value| value.entry.icon_url.as_deref())
                .map(str::to_owned),
        }
    }
}
