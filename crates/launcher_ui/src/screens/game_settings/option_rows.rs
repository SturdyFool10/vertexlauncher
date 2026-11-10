use vertex_i18n::tr;
use super::*;

/// Writes a new value for `def`, formatted the way this version's file expects.
pub(super) fn commit(
    live: &mut LiveOptions,
    def: &OptionDef,
    version: Option<Ver>,
    new_text: &str,
) {
    if let Some(raw) = format_for_def(def, new_text, live.file(), version)
        && live.get(def.key) != Some(raw.as_str())
    {
        live.edit(def.key, Some(&raw));
    }
}

pub(super) fn tooltip_for(def: &OptionDef) -> String {
    mc_options::descriptions::tooltip(def)
}

/// Raw value shown for a key: what's in the file, else the default.
pub(super) fn shown_value<'a>(live: &'a LiveOptions, def: &'a OptionDef) -> &'a str {
    value::unquote(live.get(def.key).unwrap_or(def.default)).0
}



/// Languages for the dropdown. The stored code keeps whatever case the file uses (older versions
/// write `en_US`), so the current language is selected regardless of case.
pub(super) fn language_choices(current: &str) -> Vec<(String, String)> {
    mc_options::languages::LANGUAGES
        .iter()
        .map(|(code, name)| {
            let stored = if code.eq_ignore_ascii_case(current) {
                current.to_owned()
            } else {
                (*code).to_owned()
            };
            (stored, format!("{} ({code})", vertex_i18n::t(name)))
        })
        .collect()
}

/// Output devices for the dropdown: the system default, every device found, and whatever the file
/// currently holds. Minecraft stores `OpenAL Soft on <name>`; a bare name is kept bare.
pub(super) fn sound_device_choices(
    current: &str,
    devices: Option<&[String]>,
) -> Vec<(String, String)> {
    let bare_current = current
        .strip_prefix(audio_devices::OPENAL_PREFIX)
        .unwrap_or(current);
    let prefixed = current.is_empty() || current.starts_with(audio_devices::OPENAL_PREFIX);
    let mut choices = vec![(String::new(), tr!("System Default").into_owned())];
    let mut found_current = current.is_empty();
    for name in devices.unwrap_or_default() {
        let stored = if name == bare_current {
            found_current = true;
            current.to_owned()
        } else if prefixed {
            format!("{}{name}", audio_devices::OPENAL_PREFIX)
        } else {
            name.clone()
        };
        choices.push((stored, name.clone()));
    }
    if !found_current {
        // Not currently plugged in (or not detectable): keep it selectable.
        choices.push((current.to_owned(), bare_current.to_owned()));
    }
    choices
}
