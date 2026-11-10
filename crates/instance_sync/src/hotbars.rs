use vertex_i18n::tr;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::time::SystemTime;

use crate::fs_util::{modified, write_atomic};
use crate::nbt::{self, Tag};
use crate::versions::compare_game_versions;
use crate::{SyncInstance, SyncReport};

const HOTBAR_FILE: &str = "hotbar.nbt";

struct HotbarFile {
    bytes: Vec<u8>,
    modified: SystemTime,
    data_version: Option<i32>,
}

fn load(instance: &SyncInstance) -> Option<HotbarFile> {
    let path = instance.root.join(HOTBAR_FILE);
    let bytes = fs::read(&path).ok()?;
    let data_version = nbt::parse(&bytes)
        .ok()
        .and_then(|file| nbt::get(&file.root, "DataVersion").and_then(Tag::as_int));
    Some(HotbarFile {
        bytes,
        modified: modified(&path)?,
        data_version,
    })
}

/// Minecraft upgrades older item data when it loads a file, but can't read newer data.
/// So hotbars only flow from older-or-equal versions to newer-or-equal ones.
fn may_flow(
    source: (&SyncInstance, &HotbarFile),
    target: (&SyncInstance, Option<&HotbarFile>),
) -> bool {
    if let (Some(from), Some(to)) = (
        source.1.data_version,
        target.1.and_then(|file| file.data_version),
    ) {
        return from <= to;
    }
    matches!(
        compare_game_versions(&source.0.game_version, &target.0.game_version),
        Some(Ordering::Less | Ordering::Equal)
    )
}

/// Copies the most recently saved compatible hotbar file into each instance.
pub(crate) fn sync(instances: &[SyncInstance], skip: &HashSet<String>, report: &mut SyncReport) {
    let files: Vec<Option<HotbarFile>> = instances.iter().map(load).collect();

    for (target_index, target) in instances.iter().enumerate() {
        if skip.contains(&target.id) {
            continue;
        }
        let target_file = files[target_index].as_ref();
        let best = instances
            .iter()
            .zip(&files)
            .enumerate()
            .filter(|(index, _)| *index != target_index)
            .filter_map(|(_, (instance, file))| Some((instance, file.as_ref()?)))
            .filter(|source| may_flow(*source, (target, target_file)))
            .max_by_key(|(_, file)| file.modified);
        let Some((source, source_file)) = best else {
            continue;
        };
        let needs_copy = match target_file {
            None => true,
            Some(existing) => {
                source_file.modified > existing.modified && source_file.bytes != existing.bytes
            }
        };
        if !needs_copy {
            continue;
        }
        let path = target.root.join(HOTBAR_FILE);
        match write_atomic(&path, &source_file.bytes, Some(source_file.modified)) {
            Ok(()) => report.hotbars_copied += 1,
            Err(err) => report.errors.push(tr!("{id}: failed to copy hotbars from {id2}: {err}", id = target.id, id2 = source.id, err = err).into_owned()),
        }
    }
}
