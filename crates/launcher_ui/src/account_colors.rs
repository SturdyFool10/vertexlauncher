//! Per-account colors, so accounts that share a skin can still be told apart at a glance.
//!
//! Colors are kept on disk as a plain JSON map from player UUID to `#rrggbb`. Player UUIDs
//! are public, so this is not stored with the sign-in tokens.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::geom::Color32;


const COLORS_FILE_NAME: &str = "account_colors.json";

/// A named color offered in the account color picker.
#[derive(Clone, Copy, Debug)]
pub struct PaletteColor {
    pub name: &'static str,
    pub color: Color32,
}

const fn swatch(name: &'static str, r: u8, g: u8, b: u8) -> PaletteColor {
    PaletteColor {
        name,
        color: Color32::from_rgb(r, g, b),
    }
}

/// Red, yellow and blue.
pub const PRIMARY_COLORS: [PaletteColor; 3] = [
    swatch("Red", 0xE5, 0x39, 0x35),
    swatch("Yellow", 0xFD, 0xD8, 0x35),
    swatch("Blue", 0x1E, 0x88, 0xE5),
];

/// Orange, green and purple: each is the mix of two primaries.
pub const SECONDARY_COLORS: [PaletteColor; 3] = [
    swatch("Orange", 0xFB, 0x8C, 0x00),
    swatch("Green", 0x43, 0xA0, 0x47),
    swatch("Purple", 0x8E, 0x24, 0xAA),
];

/// The six colors between each primary and secondary neighbor on the color wheel.
pub const TERTIARY_COLORS: [PaletteColor; 6] = [
    swatch("Red-orange", 0xF4, 0x51, 0x1E),
    swatch("Yellow-orange", 0xFF, 0xB3, 0x00),
    swatch("Yellow-green", 0x9C, 0xCC, 0x65),
    swatch("Blue-green", 0x00, 0x89, 0x7B),
    swatch("Blue-purple", 0x5E, 0x35, 0xB1),
    swatch("Red-purple", 0xD8, 0x1B, 0x60),
];

/// Parses a color typed by the user: `#rrggbb`, `#rgb` (the `#` is optional), or
/// `rgb(r, g, b)` with each channel `0`-`255`. Surrounding whitespace and case are ignored.
pub fn parse_color(text: &str) -> Option<Color32> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if let Some(args) = lower
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_rgb_arguments(args);
    }
    parse_hex(lower.strip_prefix('#').unwrap_or(&lower))
}

fn parse_hex(digits: &str) -> Option<Color32> {
    if !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |index: usize| u8::from_str_radix(&digits[index..=index], 16).ok();
    let byte = |index: usize| u8::from_str_radix(&digits[index..index + 2], 16).ok();
    match digits.len() {
        3 => Some(Color32::from_rgb(
            nibble(0)? * 17,
            nibble(1)? * 17,
            nibble(2)? * 17,
        )),
        6 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        _ => None,
    }
}

fn parse_rgb_arguments(args: &str) -> Option<Color32> {
    let mut channels = args
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u8>().ok());
    let (r, g, b) = (channels.next()??, channels.next()??, channels.next()??);
    channels
        .next()
        .is_none()
        .then(|| Color32::from_rgb(r, g, b))
}

/// `#rrggbb` for `color`, ignoring alpha.
pub fn format_hex(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

/// Black or white, whichever reads better on top of `background`.
pub fn contrasting_color(background: Color32) -> Color32 {
    // Rec. 709 luma on the gamma-encoded channels is plenty for a marker dot.
    let luma = 0.2126 * f32::from(background.r())
        + 0.7152 * f32::from(background.g())
        + 0.0722 * f32::from(background.b());
    if luma > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Account ids arrive as dashed or dashless UUIDs in any case; colors are keyed on the
/// dashless lowercase form.
fn normalize_account_id(account_id: &str) -> String {
    account_id
        .trim()
        .chars()
        .filter(|ch| *ch != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// UUID -> color map backed by a JSON file.
#[derive(Debug)]
pub struct AccountColorStore {
    path: PathBuf,
    colors: BTreeMap<String, Color32>,
}

impl AccountColorStore {
    /// Loads the colors at `path`. A missing file is an empty store; an unreadable one is set
    /// aside (as `<name>.corrupt`) so the next save does not silently destroy it.
    pub fn load(path: PathBuf) -> Self {
        let mut colors = BTreeMap::new();
        match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<BTreeMap<String, String>>(&text) {
                Ok(raw) => {
                    for (account_id, value) in raw {
                        if let Some(color) = parse_color(&value) {
                            colors.insert(normalize_account_id(&account_id), color);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "vertexlauncher/account_colors",
                        path = %path.display(),
                        error = %error,
                        "Account colors file is not valid JSON; setting it aside."
                    );
                    let _ = fs::rename(&path, path.with_extension("json.corrupt"));
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    target: "vertexlauncher/account_colors",
                    path = %path.display(),
                    error = %error,
                    "Could not read the account colors file."
                );
            }
        }
        Self { path, colors }
    }

    pub fn get(&self, account_id: &str) -> Option<Color32> {
        self.colors.get(&normalize_account_id(account_id)).copied()
    }

    /// Sets (or, with `None`, clears) an account's color and writes the file.
    pub fn set(&mut self, account_id: &str, color: Option<Color32>) -> io::Result<()> {
        if !self.apply(account_id, color) {
            return Ok(());
        }
        let json = self.to_json()?;
        write_file(&self.path, &json)
    }

    /// Changes the color in memory only. False if `account_id` is not a usable key.
    fn apply(&mut self, account_id: &str, color: Option<Color32>) -> bool {
        let key = normalize_account_id(account_id);
        if key.is_empty() {
            return false;
        }
        match color {
            Some(color) => self.colors.insert(key, color),
            None => self.colors.remove(&key),
        };
        true
    }

    fn to_json(&self) -> io::Result<String> {
        let raw: BTreeMap<&str, String> = self
            .colors
            .iter()
            .map(|(account_id, color)| (account_id.as_str(), format_hex(*color)))
            .collect();
        serde_json::to_string_pretty(&raw).map_err(io::Error::other)
    }
}

fn write_file(path: &std::path::Path, json: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Write beside the file and rename, so a crash never leaves half a file.
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, json)?;
    fs::rename(&temp_path, path)
}

fn colors_path() -> PathBuf {
    app_paths::config_root().join(COLORS_FILE_NAME)
}

static STORE: OnceLock<Mutex<AccountColorStore>> = OnceLock::new();

/// Reads the file on first use. Disk IO: only ever call this from a worker, never the UI thread.
fn shared_store() -> &'static Mutex<AccountColorStore> {
    STORE.get_or_init(|| Mutex::new(AccountColorStore::load(colors_path())))
}

/// Reads the saved colors on a worker. Until it finishes, [`color_for`] answers `None`.
pub fn warm() {
    launcher_runtime::spawn_blocking_detached(|| {
        shared_store();
    });
}

/// The color chosen for `account_id` (a player UUID), if any.
///
/// Called while drawing, so it never waits: before [`warm`] has finished, or while a change is being applied, it is `None`.
pub fn color_for(account_id: &str) -> Option<Color32> {
    STORE
        .get()?
        .try_lock()
        .ok()
        .and_then(|store| store.get(account_id))
}

/// Chooses (or with `None`, clears) the color for `account_id` and saves it to disk.
///
/// The change and the write happen on a worker; the new color shows on the next frame.
pub fn set_color(account_id: &str, color: Option<Color32>) {
    static WRITE_LOCK: Mutex<()> = Mutex::new(());
    let account_id = account_id.to_owned();
    launcher_runtime::spawn_blocking_detached(move || {
        // Writes go one at a time, each of the newest state, so two quick changes cannot interleave on the temp file.
        let _writing = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (path, json) = {
            let Ok(mut store) = shared_store().lock() else {
                return;
            };
            if !store.apply(&account_id, color) {
                return;
            }
            (store.path.clone(), store.to_json())
        };
        if let Err(error) = json.and_then(|json| write_file(&path, &json)) {
            tracing::warn!(
                target: "vertexlauncher/account_colors",
                error = %error,
                "Could not save the account colors."
            );
        }
    });
}

/// Whether `color` is one of the default palette colors.
pub fn is_palette_color(color: Color32) -> bool {
    PRIMARY_COLORS
        .iter()
        .chain(&SECONDARY_COLORS)
        .chain(&TERTIARY_COLORS)
        .any(|entry| entry.color == color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vertex-account-colors-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join(COLORS_FILE_NAME)
    }

    #[test]
    fn parses_hex_in_every_accepted_spelling() {
        let expected = Some(Color32::from_rgb(0x12, 0xab, 0xef));
        for text in ["#12abef", "#12ABEF", "12abef", "  #12abef  "] {
            assert_eq!(parse_color(text), expected, "{text:?}");
        }
        assert_eq!(parse_color("#f80"), Some(Color32::from_rgb(255, 136, 0)));
    }

    #[test]
    fn parses_rgb_function_with_commas_or_spaces() {
        let expected = Some(Color32::from_rgb(255, 136, 0));
        for text in [
            "rgb(255, 136, 0)",
            "rgb(255,136,0)",
            "RGB( 255 , 136 , 0 )",
            "rgb(255 136 0)",
        ] {
            assert_eq!(parse_color(text), expected, "{text:?}");
        }
    }

    #[test]
    fn rejects_malformed_colors() {
        for text in [
            "",
            "#",
            "#12",
            "#1234",
            "#12345g",
            "#1234567",
            "rgb(256, 0, 0)",
            "rgb(-1, 0, 0)",
            "rgb(1, 2)",
            "rgb(1, 2, 3, 4)",
            "rgb(1, 2, 3",
            "rgba(1, 2, 3, 4)",
            "red",
        ] {
            assert_eq!(parse_color(text), None, "{text:?} should not parse");
        }
    }

    #[test]
    fn hex_round_trips() {
        for color in [
            Color32::from_rgb(0, 0, 0),
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(1, 128, 254),
        ] {
            assert_eq!(parse_color(&format_hex(color)), Some(color));
        }
    }

    #[test]
    fn palette_has_every_primary_secondary_and_tertiary_color_once() {
        let all: Vec<_> = PRIMARY_COLORS
            .iter()
            .chain(&SECONDARY_COLORS)
            .chain(&TERTIARY_COLORS)
            .collect();
        assert_eq!(all.len(), 12);
        for (index, entry) in all.iter().enumerate() {
            assert!(
                all.iter()
                    .skip(index + 1)
                    .all(|other| other.color != entry.color && other.name != entry.name),
                "{} is duplicated",
                entry.name
            );
        }
    }

    #[test]
    fn contrasting_color_follows_brightness() {
        assert_eq!(
            contrasting_color(Color32::from_rgb(0xFD, 0xD8, 0x35)),
            Color32::BLACK
        );
        assert_eq!(
            contrasting_color(Color32::from_rgb(0x1E, 0x88, 0xE5)),
            Color32::WHITE
        );
        assert_eq!(contrasting_color(Color32::BLACK), Color32::WHITE);
    }

    #[test]
    fn colors_persist_by_uuid_across_loads() {
        let path = temp_path("persist");
        let uuid = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
        let mut store = AccountColorStore::load(path.clone());
        assert_eq!(store.get(uuid), None);
        store
            .set(uuid, Some(Color32::from_rgb(1, 2, 3)))
            .expect("save");

        // Dashed, dashless and upper-case spellings all find the same account.
        let reloaded = AccountColorStore::load(path.clone());
        for spelling in [
            uuid,
            "069a79f444e94726a5befca90e38aaf5",
            "069A79F4-44E9-4726-A5BE-FCA90E38AAF5",
        ] {
            assert_eq!(reloaded.get(spelling), Some(Color32::from_rgb(1, 2, 3)));
        }
        assert_eq!(reloaded.get("someone-else"), None);

        let text = fs::read_to_string(&path).expect("file");
        assert!(text.contains("#010203"), "stored as hex: {text}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn clearing_removes_the_entry_from_disk() {
        let path = temp_path("clear");
        let mut store = AccountColorStore::load(path.clone());
        store.set("abc", Some(Color32::from_rgb(255, 0, 0))).expect("save");
        store.set("abc", None).expect("save");
        assert_eq!(AccountColorStore::load(path.clone()).get("abc"), None);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_corrupt_file_is_set_aside_not_overwritten() {
        let path = temp_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).expect("dir");
        fs::write(&path, "{ not json").expect("write");
        let mut store = AccountColorStore::load(path.clone());
        assert_eq!(store.get("abc"), None);
        assert!(path.with_extension("json.corrupt").exists());
        store.set("abc", Some(Color32::from_rgb(0, 0, 255))).expect("save");
        assert_eq!(
            AccountColorStore::load(path.clone()).get("abc"),
            Some(Color32::from_rgb(0, 0, 255))
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unparseable_entries_are_skipped_on_load() {
        let path = temp_path("skip");
        fs::create_dir_all(path.parent().unwrap()).expect("dir");
        fs::write(&path, r##"{"good": "#ff0000", "bad": "nope"}"##).expect("write");
        let store = AccountColorStore::load(path.clone());
        assert_eq!(store.get("good"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(store.get("bad"), None);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
