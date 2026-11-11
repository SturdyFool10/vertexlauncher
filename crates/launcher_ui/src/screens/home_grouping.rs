//! Collapses worlds and servers that exist in several instances into one Home entry, with a
//! row of per-instance launch buttons.

use std::collections::BTreeMap;

use super::*;


/// One instance an entry can be launched in, and what to launch there (the world folder
/// name or server address as that instance knows it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryInstance {
    pub(crate) instance_id: String,
    pub(crate) instance_name: String,
    pub(crate) target: String,
    pub(crate) favorite: bool,
}

/// Merges entries sharing a key into one, keeping the most recently used as the primary.
fn collapse<T>(
    entries: Vec<T>,
    key_of: impl Fn(&T) -> String,
    instances_of: impl Fn(&mut T) -> &mut Vec<EntryInstance>,
    last_used_of: impl Fn(&T) -> Option<u64>,
    absorb: impl Fn(&mut T, T),
) -> Vec<T> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, T> = BTreeMap::new();
    for mut entry in entries {
        let key = key_of(&entry);
        match groups.remove(&key) {
            None => {
                order.push(key.clone());
                groups.insert(key, entry);
            }
            Some(mut existing) => {
                let entry_is_newer = last_used_of(&entry) > last_used_of(&existing);
                let mut merged = std::mem::take(instances_of(&mut existing));
                merged.append(instances_of(&mut entry));
                let mut winner = if entry_is_newer {
                    absorb(&mut entry, existing);
                    entry
                } else {
                    absorb(&mut existing, entry);
                    existing
                };
                merged.sort_by(|a, b| {
                    a.instance_name
                        .to_lowercase()
                        .cmp(&b.instance_name.to_lowercase())
                        .then_with(|| a.instance_id.cmp(&b.instance_id))
                });
                // The same server or world can be listed twice by one instance.
                merged.dedup_by(|a, b| a.instance_id == b.instance_id);
                *instances_of(&mut winner) = merged;
                groups.insert(key, winner);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|key| groups.remove(&key))
        .collect()
}

pub(super) fn collapse_worlds(worlds: Vec<WorldEntry>) -> Vec<WorldEntry> {
    collapse(
        worlds,
        |world| world.group_key.clone(),
        |world| &mut world.instances,
        |world| world.last_used_at_ms,
        |winner, loser| {
            winner.favorite |= loser.favorite;
            winner.last_used_at_ms = winner.last_used_at_ms.max(loser.last_used_at_ms);
            if winner.thumbnail_png.is_none() {
                winner.thumbnail_png = loser.thumbnail_png;
            }
        },
    )
}

pub(super) fn collapse_servers(servers: Vec<ServerEntry>) -> Vec<ServerEntry> {
    collapse(
        servers,
        |server| server.favorite_id.clone(),
        |server| &mut server.instances,
        |server| server.last_used_at_ms,
        |winner, loser| {
            winner.favorite |= loser.favorite;
            winner.protocol_version = winner.protocol_version.or(loser.protocol_version);
            winner.last_used_at_ms = winner.last_used_at_ms.max(loser.last_used_at_ms);
            if winner.icon_png.is_none() {
                winner.icon_png = loser.icon_png;
            }
        },
    )
}









#[cfg(test)]
mod tests {
    use super::*;

    fn server(instance: &str, name: &str, last_used: u64, favorite: bool) -> ServerEntry {
        ServerEntry {
            instance_id: instance.to_owned(),
            server_name: "Server".to_owned(),
            address: "play.example.net".to_owned(),
            favorite_id: "play.example.net".to_owned(),
            icon_png: None,
            last_used_at_ms: Some(last_used),
            favorite,
            protocol_version: None,
            instances: vec![EntryInstance {
                instance_id: instance.to_owned(),
                instance_name: name.to_owned(),
                target: "play.example.net".to_owned(),
                favorite,
            }],
        }
    }

    #[test]
    fn servers_in_several_instances_collapse_to_the_most_recent() {
        let merged = collapse_servers(vec![
            server("b", "Bravo", 10, false),
            server("a", "alpha", 30, true),
            server("c", "Charlie", 20, false),
        ]);
        assert_eq!(merged.len(), 1);
        let entry = &merged[0];
        assert_eq!(entry.instance_id, "a");
        assert!(entry.favorite);
        assert_eq!(entry.last_used_at_ms, Some(30));
        let names: Vec<&str> = entry
            .instances
            .iter()
            .map(|i| i.instance_name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "Bravo", "Charlie"]);
    }

    #[test]
    fn distinct_servers_stay_separate() {
        let mut other = server("a", "Alpha", 5, false);
        other.favorite_id = "other.example.net".to_owned();
        assert_eq!(
            collapse_servers(vec![server("a", "Alpha", 1, false), other]).len(),
            2
        );
    }
}
