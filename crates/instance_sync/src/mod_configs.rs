//! Mod config sync (Iris, Distant Horizons, Voxy): settings are copied key by key into the
//! same file of other instances, leaving the rest of each file (and its formatting) alone.

use vertex_i18n::tr;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use instances::InstanceStore;

use crate::fs_util::{modified, write_atomic_bytes};
use crate::{SyncInstance, SyncReport};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    /// `key=value` lines.
    Properties,
    /// `[section]` headers with `key = value` lines.
    Toml,
    /// A flat JSON object of scalars.
    FlatJson,
}

pub(crate) struct ModConfig {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) file: &'static str,
    pub(crate) format: Format,
    /// Keys never copied (hardware-, server- or version-specific).
    pub(crate) excluded_keys: &'static [&'static str],
    /// TOML sections never copied.
    pub(crate) excluded_sections: &'static [&'static str],
    /// If both files have this key with different values, their formats differ: skip.
    pub(crate) must_match_key: Option<&'static str>,
    /// Extra per-key veto given the target instance root, the key and the new value.
    pub(crate) allow: fn(&Path, &str, &str) -> bool,
}

fn always(_: &Path, _: &str, _: &str) -> bool {
    true
}

/// A shader pack can only be selected where its file exists.
fn iris_allow(target_root: &Path, key: &str, value: &str) -> bool {
    key != "shaderPack" || value.is_empty() || target_root.join("shaderpacks").join(value).exists()
}

pub(crate) static MOD_CONFIGS: &[ModConfig] = &[
    ModConfig {
        id: "iris",
        name: "Iris Shaders",
        file: "config/iris.properties",
        format: Format::Properties,
        excluded_keys: &["enableDebugOptions"],
        excluded_sections: &[],
        must_match_key: None,
        allow: iris_allow,
    },
    ModConfig {
        id: "distant_horizons",
        name: "Distant Horizons",
        file: "config/DistantHorizons.toml",
        format: Format::Toml,
        excluded_keys: &[
            "_version",
            "numberOfThreads",
            "runTimeRatioForXThreads",
            "threadPreset",
            "serverId",
            "serverKey",
            "levelKeyPrefix",
        ],
        excluded_sections: &["server"],
        must_match_key: Some("_version"),
        allow: always,
    },
    ModConfig {
        id: "voxy",
        name: "Voxy",
        file: "config/voxy-config.json",
        format: Format::FlatJson,
        excluded_keys: &["service_threads"],
        excluded_sections: &[],
        must_match_key: None,
        allow: always,
    },
];

/// `(section, key)` → raw value, for line-based formats.
fn read_lines(text: &str, format: Format) -> BTreeMap<(String, String), String> {
    let mut section = String::new();
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if format == Format::Toml && trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.trim_matches(['[', ']']).to_owned();
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            map.insert(
                (section.clone(), key.trim().to_owned()),
                value.trim().to_owned(),
            );
        }
    }
    map
}

/// A value that spans lines can't be replaced line-by-line.
fn is_partial_value(value: &str) -> bool {
    (value.starts_with('[') && !value.ends_with(']'))
        || value == "\"\"\""
        || value.starts_with("\"\"\"")
}

fn merge_lines(
    source: &str,
    target: &str,
    config: &ModConfig,
    target_root: &Path,
) -> Option<String> {
    let wanted = read_lines(source, config.format);
    let current = read_lines(target, config.format);
    if let Some(guard) = config.must_match_key {
        let key_of = |map: &BTreeMap<(String, String), String>| {
            map.iter()
                .find(|((_, k), _)| k == guard)
                .map(|(_, v)| v.clone())
        };
        if let (Some(a), Some(b)) = (key_of(&wanted), key_of(&current))
            && a != b
        {
            return None;
        }
    }
    let mut section = String::new();
    let mut changed = false;
    let mut out = Vec::new();
    for line in target.lines() {
        let trimmed = line.trim();
        if config.format == Format::Toml && trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.trim_matches(['[', ']']).to_owned();
        }
        let replacement = (!trimmed.is_empty() && !trimmed.starts_with('#'))
            .then(|| line.split_once('='))
            .flatten()
            .and_then(|(lhs, old_value)| {
                let key = lhs.trim().to_owned();
                let new_value = wanted.get(&(section.clone(), key.clone()))?;
                let top_section = section.split('.').next().unwrap_or("");
                if config.excluded_keys.contains(&key.as_str())
                    || config.excluded_sections.contains(&top_section)
                    || is_partial_value(new_value)
                    || is_partial_value(old_value.trim())
                    || new_value == old_value.trim()
                    || !(config.allow)(target_root, &key, new_value.trim_matches('"'))
                {
                    return None;
                }
                let spacer = if config.format == Format::Toml {
                    " "
                } else {
                    ""
                };
                Some(format!("{lhs}={spacer}{new_value}"))
            });
        match replacement {
            Some(new_line) => {
                changed = true;
                out.push(new_line);
            }
            None => out.push(line.to_owned()),
        }
    }
    let _ = current;
    changed.then(|| {
        let mut text = out.join("\n");
        if target.ends_with('\n') {
            text.push('\n');
        }
        text
    })
}

/// Scalar leaves of a flat JSON object as their raw JSON text.
fn json_scalars(text: &str) -> BTreeMap<String, String> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter(|(_, v)| !v.is_object() && !v.is_array() && !v.is_null())
        .map(|(k, v)| (k, v.to_string()))
        .collect()
}

fn merge_flat_json(
    source: &str,
    target: &str,
    config: &ModConfig,
    target_root: &Path,
) -> Option<String> {
    let wanted = json_scalars(source);
    let current = json_scalars(target);
    let mut text = target.to_owned();
    let mut changed = false;
    for (key, new_value) in wanted {
        let Some(old_value) = current.get(&key) else {
            continue;
        };
        if *old_value == new_value
            || config.excluded_keys.contains(&key.as_str())
            || !(config.allow)(target_root, &key, new_value.trim_matches('"'))
        {
            continue;
        }
        // Replace just the value token so layout and key order are untouched.
        let needle = format!("\"{key}\"");
        let Some(key_at) = text.find(&needle) else {
            continue;
        };
        let after_key = key_at + needle.len();
        let Some(colon) = text[after_key..].find(':') else {
            continue;
        };
        let value_start = after_key + colon + 1;
        let leading = text[value_start..].len() - text[value_start..].trim_start().len();
        let value_begin = value_start + leading;
        let value_end = text[value_begin..]
            .find([',', '}', '\n'])
            .map_or(text.len(), |offset| value_begin + offset);
        let trailing =
            text[value_begin..value_end].len() - text[value_begin..value_end].trim_end().len();
        text.replace_range(value_begin..value_end - trailing, &new_value);
        changed = true;
    }
    changed.then_some(text)
}

pub(crate) fn merge(
    source: &str,
    target: &str,
    config: &ModConfig,
    target_root: &Path,
) -> Option<String> {
    match config.format {
        Format::Properties | Format::Toml => merge_lines(source, target, config, target_root),
        Format::FlatJson => merge_flat_json(source, target, config, target_root),
    }
}

/// Keeps enabled mod configs in step across opted-in instances that have them.
pub(crate) fn sync(
    instances: &[SyncInstance],
    store: &InstanceStore,
    skip: &HashSet<String>,
    report: &mut SyncReport,
) {
    let sync = &store.game_settings_sync;
    let participants: Vec<&SyncInstance> = instances
        .iter()
        .filter(|instance| sync.instances.contains(&instance.id))
        .collect();
    if participants.len() < 2 {
        return;
    }
    for config in MOD_CONFIGS
        .iter()
        .filter(|c| sync.mod_configs.contains(c.id))
    {
        sync_one(&participants, config, skip, report);
        if config.id == "iris" {
            sync_shaderpack_options(&participants, skip, report);
        }
    }
}

fn sync_one(
    participants: &[&SyncInstance],
    config: &ModConfig,
    skip: &HashSet<String>,
    report: &mut SyncReport,
) {
    let files: Vec<(&SyncInstance, SystemTime)> = participants
        .iter()
        .filter_map(|instance| Some((*instance, modified(&instance.root.join(config.file))?)))
        .collect();
    let Some((master, master_modified)) = files.iter().max_by_key(|(_, m)| *m).copied() else {
        return;
    };
    let Ok(source) = fs::read_to_string(master.root.join(config.file)) else {
        return;
    };
    for (instance, when) in files {
        if instance.id == master.id || when >= master_modified || skip.contains(&instance.id) {
            continue;
        }
        let path = instance.root.join(config.file);
        let Ok(target) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(merged) = merge(&source, &target, config, &instance.root) {
            match write_atomic_bytes(&path, merged.as_bytes(), Some(master_modified)) {
                Ok(()) => report.mod_config_files += 1,
                Err(err) => report.errors.push(tr!("{id}: failed to write {file}: {err}", id = instance.id, file = config.file, err = err).into_owned()),
            }
        }
    }
}

/// Per-pack shader option files (`shaderpacks/<pack>.txt`), for packs the target has.
fn sync_shaderpack_options(
    participants: &[&SyncInstance],
    skip: &HashSet<String>,
    report: &mut SyncReport,
) {
    static PACK_OPTIONS: ModConfig = ModConfig {
        id: "iris_pack",
        name: vertex_i18n::src!("Shader pack options"),
        file: "",
        format: Format::Properties,
        excluded_keys: &[],
        excluded_sections: &[],
        must_match_key: None,
        allow: always,
    };
    let list = |instance: &SyncInstance| -> Vec<String> {
        fs::read_dir(instance.root.join("shaderpacks"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.ends_with(".txt").then_some(name)
            })
            .collect()
    };
    let mut names: Vec<String> = participants.iter().flat_map(|i| list(i)).collect();
    names.sort();
    names.dedup();
    for name in names {
        let pack = name.trim_end_matches(".txt");
        let holders: Vec<(&SyncInstance, SystemTime)> = participants
            .iter()
            .filter_map(|i| Some((*i, modified(&i.root.join("shaderpacks").join(&name))?)))
            .collect();
        let Some((master, master_modified)) = holders.iter().max_by_key(|(_, m)| *m).copied()
        else {
            continue;
        };
        let Ok(source) = fs::read_to_string(master.root.join("shaderpacks").join(&name)) else {
            continue;
        };
        for instance in participants {
            if instance.id == master.id
                || skip.contains(&instance.id)
                || !instance.root.join("shaderpacks").join(pack).exists()
            {
                continue;
            }
            let path = instance.root.join("shaderpacks").join(&name);
            let existing = fs::read_to_string(&path).ok();
            if modified(&path).is_some_and(|m| m >= master_modified) {
                continue;
            }
            let new_text = match &existing {
                Some(target) => merge_lines(&source, target, &PACK_OPTIONS, &instance.root),
                None => Some(source.clone()),
            };
            if let Some(text) = new_text {
                match write_atomic_bytes(&path, text.as_bytes(), Some(master_modified)) {
                    Ok(()) => report.mod_config_files += 1,
                    Err(err) => report.errors.push(tr!("{id}: failed to write shaderpacks/{name}: {err}", id = instance.id, name = name, err = err).into_owned()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(id: &str) -> &'static ModConfig {
        MOD_CONFIGS.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn properties_merge_changes_only_differing_keys_and_checks_the_shader_pack() {
        let source =
            "#comment\nenableShaders=true\nmaxShadowRenderDistance=12\nshaderPack=Missing.zip\n";
        let target = "#other comment\nenableShaders=false\nmaxShadowRenderDistance=12\nshaderPack=Mine.zip\nextra=1\n";
        let root = std::env::temp_dir().join("vertex-modcfg-test-root");
        let merged = merge(source, target, config("iris"), &root).unwrap();
        assert_eq!(
            merged,
            "#other comment\nenableShaders=true\nmaxShadowRenderDistance=12\nshaderPack=Mine.zip\nextra=1\n"
        );
    }

    #[test]
    fn toml_merge_respects_sections_exclusions_and_format_version() {
        let source = "_version = 4\n[server]\n\tport = 1\n[client]\n\tquality = \"HIGH\"\n\tnumberOfThreads = 16\n\tfancy = true\n";
        let target = "_version = 4\n[server]\n\tport = 2\n[client]\n\t# note\n\tquality = \"LOW\"\n\tnumberOfThreads = 4\n\tfancy = true\n";
        let root = Path::new(".");
        let merged = merge(source, target, config("distant_horizons"), root).unwrap();
        assert_eq!(
            merged,
            "_version = 4\n[server]\n\tport = 2\n[client]\n\t# note\n\tquality = \"HIGH\"\n\tnumberOfThreads = 4\n\tfancy = true\n"
        );
        let other_format = target.replace("_version = 4", "_version = 5");
        assert!(merge(source, &other_format, config("distant_horizons"), root).is_none());
    }

    #[test]
    fn flat_json_merge_replaces_only_value_tokens() {
        let source = "{\n  \"enabled\": false,\n  \"section_render_distance\": 8,\n  \"service_threads\": 16\n}";
        let target = "{\n  \"enabled\": true,\n  \"section_render_distance\": 4,\n  \"service_threads\": 8\n}\n";
        let merged = merge(source, target, config("voxy"), Path::new(".")).unwrap();
        assert_eq!(
            merged,
            "{\n  \"enabled\": false,\n  \"section_render_distance\": 8,\n  \"service_threads\": 8\n}\n"
        );
    }
}
