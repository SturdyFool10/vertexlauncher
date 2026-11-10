//! Dropdown labels for the Minecraft version catalog, shared by the content browser and
//! discover screens (mounted by both via `#[path]`).

use vertex_i18n::tr;
use super::*;

/// Labels for a game-version dropdown: "Any version" followed by one label per catalog entry.
///
/// Built once when the catalog arrives and stored on the screen state, so the per-frame
/// dropdown code does not allocate a `String` per version.
pub(crate) fn game_version_option_labels(versions: &[MinecraftVersionEntry]) -> Vec<String> {
    let mut labels = Vec::with_capacity(versions.len() + 1);
    labels.push(tr!("Any version").into_owned());
    labels.extend(versions.iter().map(MinecraftVersionEntry::display_label));
    labels
}
