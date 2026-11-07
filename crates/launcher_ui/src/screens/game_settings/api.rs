//! Toolkit-free view API for the game settings dialog.
//!
//! A [`Session`] owns the live view of an instance's `options.txt`. The UI asks it for [`Row`]s to draw and sends
//! edits back; schema knowledge (value formats, version translation, key codes, languages, devices) stays here.

use super::*;

pub use instances::PackOverride;
pub use mc_options::schema::Group;

/// The kind of control a setting needs, with its current value.
pub enum Control {
    Toggle(bool),
    Int {
        value: i32,
        min: i32,
        max: i32,
    },
    /// `value`, `min`, `max` are in display units (degrees for FOV). `percent` marks a 0..1 value shown as a percentage.
    Float {
        value: f32,
        min: f32,
        max: f32,
        step: f32,
        percent: bool,
    },
    Choice {
        labels: Vec<String>,
        stored: Vec<String>,
        selected: usize,
    },
    Text(String),
    /// A list value, read-only: a summary of the items.
    List(String),
    Keybind {
        shown: String,
        /// "Also used by: …" when another binding has the same key.
        conflict: Option<String>,
    },
    /// A key this launcher has no definition for, edited as raw text.
    Raw(String),
}

pub enum Row {
    /// A group heading in search results.
    Heading(String),
    /// A sub-heading inside a group ("Mod & other key binds").
    Subheading(String),
    Setting {
        key: String,
        label: String,
        tip: Option<String>,
        control: Control,
    },
}

/// What the dialog header shows.
pub struct Header {
    pub title: String,
    pub subtitle: String,
    pub running: bool,
    pub error: Option<String>,
    pub saving: bool,
}

/// A resource pack as listed on the packs tab.
pub struct PackView {
    pub name: String,
    pub detail: String,
    pub verdict: Option<PackOverride>,
}

pub struct Session {
    instance_id: String,
    instance_name: String,
    game_version: String,
    version: Option<Ver>,
    live: LiveOptions,
    audio_devices: Arc<Mutex<Option<Vec<String>>>>,
    pack_rows: Option<(Instant, Arc<[PackRow]>)>,
}

impl Session {
    /// Opens the live view of the instance's options and starts the audio device lookup.
    pub fn open(instance: &InstanceRecord, installations_root: &Path) -> Self {
        let instance_root = instance_root_path(installations_root, instance);
        let path = instance_root.join("options.txt");
        let audio_devices = Arc::new(Mutex::new(None));
        {
            let audio_devices = Arc::clone(&audio_devices);
            launcher_runtime::spawn_blocking_detached(move || {
                let devices = audio_devices::list_output_devices();
                if let Ok(mut slot) = audio_devices.lock() {
                    *slot = Some(devices);
                }
            });
        }
        Self {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            game_version: instance.game_version.clone(),
            version: parse_game_version(&instance.game_version),
            live: open_live_options(path, instance_root.as_path(), &instance.game_version),
            audio_devices,
            pack_rows: None,
        }
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Follows the file and flushes edits that have been idle long enough. Returns when to look again.
    pub fn poll(&mut self, instances: &InstanceStore, config: &Config) {
        let polled = self.live.poll(SAVE_DELAY);
        if polled == Poll::Saved && instances.game_settings_sync.instances.contains(&self.instance_id) {
            // Propagate right away instead of waiting for the next launch or exit.
            sync_runner::spawn(instances.clone(), SyncRunConfig::from_config(config));
        }
    }

    /// Saves anything still pending; call before dropping the session.
    pub fn flush(&mut self) {
        let _ = self.live.flush();
    }

    pub fn header(&self, instances: &InstanceStore, config: &Config) -> Header {
        let running = instances
            .find(&self.instance_id)
            .map(|instance| instance_root_path(config.minecraft_installations_root_path(), instance))
            .is_some_and(|root| installation::is_instance_running(root.as_path()));
        Header {
            title: "Game settings".to_owned(),
            subtitle: format!("{} · Minecraft {} · {}", self.instance_name, self.game_version, self.live.path().display()),
            running,
            error: self.live.last_error().map(str::to_owned),
            saving: self.live.has_pending(),
        }
    }

    /// The rows for a tab, or for every tab (with headings) when `search` is not empty.
    pub fn rows(&self, group: Group, search: &str) -> Vec<Row> {
        if search.trim().is_empty() {
            return self.group_rows(group, search, None);
        }
        let mut rows = Vec::new();
        for group in Group::ALL {
            rows.extend(self.group_rows(group, search, Some(group.label())));
        }
        rows
    }

    fn group_rows(&self, group: Group, search: &str, heading: Option<&str>) -> Vec<Row> {
        let version = self.version;
        let live = &self.live;
        let defs: Vec<&'static OptionDef> = schema::defs_for(version)
            .filter(|def| !def.hidden && def.group == group && def_matches_search(search, def))
            .collect();
        let mut body: Vec<Row> = Vec::new();

        if group == Group::Keybinds {
            let usage = keybind_usage(live, version);
            let mut shown = std::collections::HashSet::new();
            for def in &defs {
                shown.insert(def.key);
                body.push(self.keybind_row(def.key, def.label, def.default, &usage));
            }
            // Binds from mods (and other versions) that the file already contains.
            let extra: Vec<String> = live
                .file()
                .keys()
                .into_iter()
                .filter(|key| schema::is_keybind_key(key) && !shown.contains(key))
                .filter(|key| matches_search(search, &pretty_key(key), key))
                .map(str::to_owned)
                .collect();
            if !extra.is_empty() {
                body.push(Row::Subheading("Mod & other key binds".to_owned()));
                for key in extra {
                    let label = pretty_key(&key);
                    body.push(self.keybind_row(&key, &label, "key.keyboard.unknown", &usage));
                }
            }
        } else {
            for def in defs {
                if let Some(row) = self.def_row(def) {
                    body.push(row);
                }
            }
            if group == Group::Internal {
                // Keys this launcher has no definition for: shown raw so nothing is hidden.
                let unknown: Vec<String> = live
                    .file()
                    .keys()
                    .into_iter()
                    .filter(|key| !schema::is_keybind_key(key) && schema::find_def(key, version).is_none())
                    .filter(|key| !key.starts_with("soundCategory_") || schema::find_def_any(key).is_none())
                    .filter(|key| matches_search(search, key, key))
                    .map(str::to_owned)
                    .collect();
                if !unknown.is_empty() {
                    body.push(Row::Subheading("Other keys (mods and other versions)".to_owned()));
                    for key in unknown {
                        let current = live.get(&key).unwrap_or_default().to_owned();
                        body.push(Row::Setting {
                            label: key.clone(),
                            tip: Some("Not a setting this launcher knows; edited as raw text.".to_owned()),
                            control: Control::Raw(current),
                            key,
                        });
                    }
                }
            }
        }

        match (heading, body.is_empty()) {
            (Some(text), false) => {
                let mut rows = vec![Row::Heading(text.to_owned())];
                rows.extend(body);
                rows
            }
            _ => body,
        }
    }

    fn def_row(&self, def: &OptionDef) -> Option<Row> {
        let live = &self.live;
        let shown = shown_value(live, def).to_owned();
        let tip = Some(tooltip_for(def));
        let choice = |labels: Vec<(String, String)>| {
            let current = shown.clone();
            let mut labels = labels;
            let mut selected = labels.iter().position(|(stored, _)| *stored == current);
            if selected.is_none() {
                // Show what the file really holds instead of pretending it's a known choice.
                labels.push((current.clone(), format!("{current} (unrecognized)")));
                selected = Some(labels.len() - 1);
            }
            Control::Choice {
                selected: selected.unwrap_or(0),
                stored: labels.iter().map(|(stored, _)| stored.clone()).collect(),
                labels: labels.into_iter().map(|(_, label)| label).collect(),
            }
        };
        let control = match def.kind {
            Kind::Bool => Control::Toggle(value::parse_bool(&shown).unwrap_or(def.default == "true")),
            Kind::Int { min, max } => Control::Int {
                value: value::parse_i64(&shown).or_else(|| def.default.parse().ok()).unwrap_or(min).clamp(min, max) as i32,
                min: min as i32,
                max: max as i32,
            },
            Kind::Float { min, max } => {
                let stored = value::parse_f64(&shown).unwrap_or_else(|| def.default.parse().unwrap_or(min));
                match def.display {
                    Display::Degrees { offset, scale } => Control::Float {
                        value: (offset + stored * scale) as f32,
                        min: (offset + min * scale) as f32,
                        max: (offset + max * scale) as f32,
                        step: 1.0,
                        percent: false,
                    },
                    Display::Percent if min == 0.0 && max == 1.0 => {
                        Control::Float { value: stored as f32, min: 0.0, max: 1.0, step: 0.01, percent: true }
                    }
                    _ => Control::Float {
                        value: stored as f32,
                        min: min as f32,
                        max: max as f32,
                        step: ((max - min) / 100.0).max(0.01) as f32,
                        percent: false,
                    },
                }
            }
            Kind::Choice(choices) => {
                choice(choices.iter().map(|(stored, label)| ((*stored).to_owned(), (*label).to_owned())).collect())
            }
            Kind::IntChoice(choices) => {
                choice(choices.iter().map(|(stored, label)| (stored.to_string(), (*label).to_owned())).collect())
            }
            Kind::Text if def.key == "lang" => choice(language_choices(&shown)),
            Kind::Text if def.key == "soundDevice" => {
                let devices = self.audio_devices.lock().ok().and_then(|slot| slot.clone());
                choice(sound_device_choices(&shown, devices.as_deref()))
            }
            Kind::Text => Control::Text(shown.clone()),
            Kind::List => {
                let items = value::parse_list(live.get(def.key).unwrap_or(def.default));
                Control::List(if items.is_empty() { "none".to_owned() } else { items.join(", ") })
            }
            Kind::Keybind => return None,
        };
        Some(Row::Setting { key: def.key.to_owned(), label: def.label.to_owned(), tip, control })
    }

    fn keybind_row(&self, key: &str, label: &str, default: &str, usage: &BTreeMap<String, Vec<String>>) -> Row {
        let modern = self.modern_binding(key, default);
        let conflict = usage.get(&modern).filter(|others| others.len() > 1 && modern != "key.keyboard.unknown").map(|others| {
            format!(
                "Also used by: {}",
                others.iter().filter(|other| other.as_str() != label).cloned().collect::<Vec<_>>().join(", ")
            )
        });
        Row::Setting {
            key: key.to_owned(),
            label: label.to_owned(),
            tip: None,
            control: Control::Keybind { shown: keys::display_name(&modern), conflict },
        }
    }

    /// The binding as a modern key name, whatever the file's version stores.
    fn modern_binding(&self, key: &str, default: &str) -> String {
        let raw = self.live.get(key).unwrap_or(default).to_owned();
        if self.is_legacy_keys() {
            raw.parse::<i32>().ok().and_then(keys::legacy_to_modern).unwrap_or(raw)
        } else {
            raw
        }
    }

    fn is_legacy_keys(&self) -> bool {
        self.version.is_some_and(|v| v < Ver(1, 13, 0))
    }

    // -- Edits ------------------------------------------------------------------------------------------------------

    /// Sets a setting from its UI text (`true`/`false`, a number, a choice's stored value, or free text).
    pub fn set_text(&mut self, key: &str, text: &str) {
        if let Some(def) = schema::find_def(key, self.version) {
            commit(&mut self.live, def, self.version, text);
        }
    }

    /// Sets a float setting from the number its control shows (degrees for FOV).
    pub fn set_float(&mut self, key: &str, edited: f32) {
        let Some(def) = schema::find_def(key, self.version) else { return };
        let text = match def.display {
            Display::Degrees { offset, scale } => format!("{}", (f64::from(edited) - offset) / scale),
            _ => format!("{edited}"),
        };
        commit(&mut self.live, def, self.version, &text);
    }

    /// Sets an unknown key as raw text.
    pub fn set_raw(&mut self, key: &str, text: &str) {
        if self.live.get(key) != Some(text) {
            self.live.edit(key, Some(text));
        }
    }

    /// Binds `key` to a modern key name such as `key.keyboard.w`; `key.keyboard.unknown` unbinds.
    pub fn set_keybind(&mut self, key: &str, modern_name: &str) {
        let stored = if self.is_legacy_keys() {
            match keys::modern_to_legacy(modern_name) {
                Some(code) => code.to_string(),
                None => return,
            }
        } else {
            modern_name.to_owned()
        };
        if self.live.get(key) != Some(stored.as_str()) {
            self.live.edit(key, Some(&stored));
        }
    }

    // -- Resource packs -----------------------------------------------------------------------------------------------

    pub fn packs(&mut self, instances: &InstanceStore, config: &Config) -> Vec<PackView> {
        let stale = self.pack_rows.as_ref().is_none_or(|(when, _)| when.elapsed() > Duration::from_secs(1));
        if stale {
            self.pack_rows = Some((Instant::now(), Arc::from(self.collect_pack_rows(instances, config))));
        }
        let rows = self.pack_rows.as_ref().map(|(_, rows)| Arc::clone(rows)).unwrap_or_else(|| Arc::from(Vec::new()));
        rows.iter()
            .map(|row| {
                let status = pack_status(instances, &self.instance_id, &self.game_version, &row.name, row.support);
                let declared = row.support.map_or_else(|| "none".to_owned(), |s| s.serialize());
                let location = if row.present_here {
                    "In this instance".to_owned()
                } else {
                    format!("Only in {}", row.elsewhere.join(", "))
                };
                PackView {
                    name: row.name.clone(),
                    detail: format!("{location} · declared formats: {declared} · {}", status_text(status, row.support, self.version)),
                    verdict: instances.pack_override(&row.name, &self.instance_id),
                }
            })
            .collect()
    }

    pub fn set_pack_override(
        &mut self,
        instances: &mut InstanceStore,
        config: &Config,
        pack: &str,
        target: Option<PackOverride>,
    ) {
        instances.set_pack_override(pack, &self.instance_id, target);
        self.pack_rows = None;
        if instances.game_settings_sync.resource_packs {
            sync_runner::spawn(instances.clone(), SyncRunConfig::from_config(config));
        }
    }

    fn collect_pack_rows(&self, instances: &InstanceStore, config: &Config) -> Vec<PackRow> {
        let root_of = |instance: &InstanceRecord| instance_root_path(config.minecraft_installations_root_path(), instance);
        let mut rows: BTreeMap<String, PackRow> = BTreeMap::new();
        let Some(this) = instances.find(&self.instance_id) else { return Vec::new() };
        for pack in scan_packs(&root_of(this)) {
            rows.insert(
                pack.name.clone(),
                PackRow { name: pack.name, support: pack.support, present_here: true, elsewhere: Vec::new() },
            );
        }
        // Packs other syncing instances have, so they can be vouched for before they arrive.
        for other in &instances.instances {
            if other.id == self.instance_id || !instances.game_settings_sync.instances.contains(&other.id) {
                continue;
            }
            for pack in scan_packs(&root_of(other)) {
                let row = rows.entry(pack.name.clone()).or_insert_with(|| PackRow {
                    name: pack.name.clone(),
                    support: pack.support,
                    present_here: false,
                    elsewhere: Vec::new(),
                });
                row.elsewhere.push(other.name.clone());
            }
        }
        let mut rows: Vec<PackRow> = rows.into_values().collect();
        rows.sort_by_key(|row| row.name.to_lowercase());
        rows
    }
}

/// Runs a sync pass for every instance in the sync group now.
pub fn sync_now(instances: &InstanceStore, config: &Config) {
    sync_runner::spawn(instances.clone(), SyncRunConfig::from_config(config));
}

/// Groups the sync tab lets you turn on or off (everything but the bookkeeping groups).
pub fn syncable_groups() -> Vec<Group> {
    Group::ALL.into_iter().filter(|g| !matches!(g, Group::Online | Group::Internal)).collect()
}

/// Mods whose config files can be synced, as (id, name).
pub fn available_mod_configs() -> Vec<(&'static str, &'static str)> {
    instance_sync::available_mod_configs().into_iter().map(|(id, name)| (id, name)).collect()
}
