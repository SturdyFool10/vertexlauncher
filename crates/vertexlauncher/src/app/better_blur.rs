//! Finds the Better Blur DX KWin effect on Linux.
//!
//! Better Blur DX replaces KWin's blur. It blurs a window that asks for blur through the compositor's blur protocols
//! (which is how Vertex asks for it when Window Transparency is Blurred), so there is nothing to switch on in the
//! launcher; finding it lets the Settings screen say so and point at the effect's own settings. Its strength, noise,
//! saturation, brightness, contrast and refraction are global KWin settings: a client cannot set them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use launcher_runtime::WorkerChannel;

/// The effect's id in KWin: the name of its plugin library, which is also what `kwinrc` uses (`<id>Enabled`).
const PLUGIN_ID: &str = "better_blur_dx";
/// The Desktop Effects page, for `kcmshell6`. The effect's own settings are a plugin of that page (they install under
/// KWin's effect configs, not as a module of their own), so they open from the effect's configure button there.
const CONFIG_MODULE: &str = "kcm_kwin_effects";
const RECHECK_EVERY: Duration = Duration::from_secs(5);
/// The window class (Wayland app id) the effect matches Vertex's window by.
pub const WINDOW_CLASS: &str = "org.sturdyfool10.VertexLauncher";
/// The effect's settings group in `kwinrc`.
const CONFIG_GROUP: &str = "Effect-better-blur-dx";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BetterBlurDx {
    /// Not on this machine (or not a KDE session).
    #[default]
    NotFound,
    /// The plugin is installed but not switched on in Desktop Effects.
    Installed,
    /// Switched on, so KWin's blur is Better Blur DX.
    Active,
}

/// Whether `kwinrc` (the text of the file) has the effect switched on.
fn enabled_in(kwinrc: &str) -> bool {
    let key = format!("{PLUGIN_ID}Enabled");
    let mut in_plugins = false;
    for line in kwinrc.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            in_plugins = section == "Plugins";
        } else if in_plugins
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return value.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

/// The effect's `WindowClasses` lines and whether it blurs the listed windows (`BlurMatching`, on by default) or all the
/// others, read from the text of `kwinrc`. `None` classes means the key was never written (the effect's placeholders).
fn window_rules_in(kwinrc: &str) -> (Option<Vec<String>>, bool) {
    let mut in_group = false;
    let mut classes = None;
    let mut matching = true;
    for line in kwinrc.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            in_group = section == CONFIG_GROUP;
        } else if in_group && let Some((name, value)) = line.split_once('=') {
            match name.trim() {
                // A multi-line value is stored on one line with `\n` for the breaks.
                "WindowClasses" => {
                    classes = Some(value.trim().split("\\n").map(str::to_owned).filter(|class| !class.is_empty()).collect())
                }
                "BlurMatching" => matching = !value.trim().eq_ignore_ascii_case("false"),
                _ => {}
            }
        }
    }
    (classes, matching)
}

/// Whether one line of the effect's class list matches `class`: a plain class, or a `/regex/`.
fn class_line_matches(line: &str, class: &str) -> bool {
    if line.len() >= 2 && line.starts_with('/') && line.ends_with('/') {
        return regex::Regex::new(&line[1..line.len() - 1]).is_ok_and(|regex| regex.is_match(class));
    }
    line == class
}

/// Whether the effect, going by the text of `kwinrc`, blurs a window of `class`.
fn blurs_class(kwinrc: &str, class: &str) -> bool {
    let (classes, matching) = window_rules_in(kwinrc);
    let listed = classes.is_some_and(|lines| lines.iter().any(|line| class_line_matches(line, class)));
    listed == matching
}

/// Whether the plugin library is in any of the Qt plugin folders in `roots`.
fn installed_in(roots: &[PathBuf]) -> bool {
    let library = format!("{PLUGIN_ID}.so");
    roots.iter().any(|root| {
        ["kwin/effects/plugins", "kwin-x11/effects/plugins"]
            .iter()
            .any(|sub| root.join(sub).join(&library).is_file())
    })
}

fn plugin_roots(home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = std::env::var("QT_PLUGIN_PATH")
        .ok()
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    for fixed in [
        "/usr/lib/qt6/plugins",
        "/usr/lib64/qt6/plugins",
        "/usr/lib/x86_64-linux-gnu/qt6/plugins",
        "/usr/lib/aarch64-linux-gnu/qt6/plugins",
        "/usr/local/lib/qt6/plugins",
        "/usr/local/lib64/qt6/plugins",
    ] {
        roots.push(PathBuf::from(fixed));
    }
    if let Some(home) = home {
        roots.push(home.join(".local/lib/qt6/plugins"));
        roots.push(home.join(".local/lib64/qt6/plugins"));
    }
    roots
}

/// What a check found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Detection {
    pub status: BetterBlurDx,
    /// The effect's class list covers Vertex's window, so it blurs it.
    pub blurs_vertex: bool,
}

fn kwinrc_text() -> Option<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")))?;
    std::fs::read_to_string(config_home.join("kwinrc")).ok()
}

/// Looks for the effect. Reads a few files: call it from a worker.
pub fn detect() -> Detection {
    if !cfg!(target_os = "linux") {
        return Detection::default();
    }
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.to_ascii_uppercase().contains("KDE"));
    if !kde {
        return Detection::default();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let kwinrc = kwinrc_text();
    let enabled = kwinrc.as_deref().is_some_and(enabled_in);
    let status = if enabled {
        BetterBlurDx::Active
    } else if installed_in(&plugin_roots(home.as_deref())) {
        BetterBlurDx::Installed
    } else {
        BetterBlurDx::NotFound
    };
    let blurs_vertex = kwinrc.as_deref().is_some_and(|text| blurs_class(text, WINDOW_CLASS));
    Detection { status, blurs_vertex }
}

/// Puts Vertex's window class on the effect's list (or takes it off the list when the effect is set to blur everything
/// but the listed windows), then tells KWin to reload the effect. Runs `kwriteconfig6` and `dbus-send`: call it from a
/// worker. The effect is the user's own KWin setup, so this is only done when they ask for it.
pub fn cover_vertex() -> Result<(), String> {
    let kwinrc = kwinrc_text().unwrap_or_default();
    let (classes, matching) = window_rules_in(&kwinrc);
    let mut lines: Vec<String> = classes.unwrap_or_default();
    lines.retain(|line| !class_line_matches(line, WINDOW_CLASS));
    if matching {
        lines.push(WINDOW_CLASS.to_owned());
    }
    let status = std::process::Command::new("kwriteconfig6")
        .args(["--file", "kwinrc", "--group", CONFIG_GROUP, "--key", "WindowClasses"])
        .arg(lines.join("\n"))
        .status()
        .map_err(|error| format!("could not run kwriteconfig6: {error}"))?;
    if !status.success() {
        return Err("kwriteconfig6 did not accept the change".to_owned());
    }
    // A failed reload is not fatal: the effect picks the list up the next time KWin reads its settings.
    let _ = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.kde.KWin",
            "--type=method_call",
            "/Effects",
            "org.kde.kwin.Effects.reconfigureEffect",
        ])
        .arg(format!("string:{PLUGIN_ID}"))
        .status();
    Ok(())
}

/// Opens Desktop Effects in System Settings, where the effect has its configure button.
pub fn open_settings() {
    launcher_runtime::spawn_blocking_detached(|| {
        if let Err(error) = std::process::Command::new("kcmshell6").arg(CONFIG_MODULE).spawn() {
            tracing::warn!(target: "vertexlauncher/better_blur", "could not open the Desktop Effects settings: {error}");
            launcher_ui::notification::warn!("settings", "Could not open the Desktop Effects settings: {}", error);
        }
    });
}

/// What the UI keeps: the latest answer, refreshed in the background.
#[derive(Default)]
pub struct BetterBlurWatch {
    status: BetterBlurDx,
    blurs_vertex: bool,
    checked: Option<Instant>,
    in_flight: bool,
    /// Vertex has already been added to the class list for this choice of Better Blur DX, so a failure is not retried
    /// every few seconds.
    cover_attempted: bool,
    /// A check has finished, so `status` is an answer and not just the default.
    settled: bool,
    results: WorkerChannel<Detection>,
}

impl BetterBlurWatch {
    pub fn status(&self) -> BetterBlurDx {
        self.status
    }

    /// Whether the effect is on this machine (installed or on). Offer it as a choice only then.
    pub fn available(&self) -> bool {
        self.status != BetterBlurDx::NotFound
    }

    /// Whether the effect's class list covers Vertex's window.
    pub fn blurs_vertex(&self) -> bool {
        self.blurs_vertex
    }

    /// Adds Vertex's window class to the effect's list in the background, then checks again at once.
    pub fn cover_vertex(&mut self) {
        let tx = self.results.sender();
        launcher_runtime::spawn_blocking_detached(move || {
            match cover_vertex() {
                Ok(()) => launcher_ui::notification::info!("settings", "Added Vertex to Better Blur DX's window classes."),
                Err(error) => {
                    tracing::warn!(target: "vertexlauncher/better_blur", "could not add Vertex to Better Blur DX: {error}");
                    launcher_ui::notification::warn!("settings", "Could not add Vertex to Better Blur DX: {}", error);
                }
            }
            let _ = tx.send(detect());
            vertex_shell::wake();
        });
    }

    /// Call every frame with whether Better Blur DX is the chosen blur. Choosing it is the request to blur Vertex with
    /// it, so when the effect is on but does not list Vertex's window class, the class is added (once; the Settings
    /// button is there to try again).
    pub fn ensure_covered(&mut self, chosen: bool) {
        if !chosen || self.blurs_vertex {
            self.cover_attempted = false;
            return;
        }
        if self.settled && self.status == BetterBlurDx::Active && !self.cover_attempted {
            self.cover_attempted = true;
            self.cover_vertex();
        }
    }

    /// Whether the first check has finished: before that `available` is not an answer yet.
    pub fn settled(&self) -> bool {
        self.settled
    }

    /// Call every frame. Picks up the last answer and starts a new check now and then (never on the UI thread).
    pub fn poll(&mut self) {
        if !cfg!(target_os = "linux") {
            return;
        }
        for found in self.results.drain().items {
            self.in_flight = false;
            self.settled = true;
            self.status = found.status;
            self.blurs_vertex = found.blurs_vertex;
        }
        let due = self.checked.is_none_or(|checked| checked.elapsed() >= RECHECK_EVERY);
        if due && !self.in_flight {
            self.in_flight = true;
            self.checked = Some(Instant::now());
            let tx = self.results.sender();
            launcher_runtime::spawn_blocking_detached(move || {
                let _ = tx.send(detect());
                vertex_shell::wake();
            });
        }
        vertex_shell::wake_after(RECHECK_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_effect_counts_as_enabled_only_in_the_plugins_section() {
        assert!(enabled_in("[Plugins]\nblurEnabled=false\nbetter_blur_dxEnabled=true\n"));
        assert!(!enabled_in("[Plugins]\nbetter_blur_dxEnabled=false\n"));
        assert!(!enabled_in("[Plugins]\nblurEnabled=true\n"));
        assert!(!enabled_in("[Other]\nbetter_blur_dxEnabled=true\n[Plugins]\n"));
        assert!(enabled_in("[Compositing]\nx=1\n\n[Plugins]\n better_blur_dxEnabled = True \n"));
    }

    #[test]
    fn the_class_list_decides_whether_vertex_is_blurred() {
        let class = WINDOW_CLASS;
        let rc = |body: &str| format!("[Plugins]\nx=1\n\n[{CONFIG_GROUP}]\n{body}\n");
        assert!(blurs_class(&rc(&format!("WindowClasses=firefox\\n{class}")), class));
        assert!(!blurs_class(&rc("WindowClasses=firefox"), class));
        assert!(blurs_class(&rc("WindowClasses=/^org\\.sturdyfool10\\./"), class));
        // Set to blur everything but the listed windows.
        assert!(!blurs_class(&rc(&format!("WindowClasses={class}\nBlurMatching=false")), class));
        assert!(blurs_class(&rc("WindowClasses=firefox\nBlurMatching=false"), class));
        // Never configured: nothing is listed.
        assert!(!blurs_class("[Plugins]\nx=1\n", class));
    }

    #[test]
    fn the_plugin_is_found_in_either_kwin_effect_folder() {
        let root = std::env::temp_dir().join(format!("vertex-bbdx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!installed_in(&[root.clone()]));
        let folder = root.join("kwin/effects/plugins");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("better_blur_dx.so"), b"").unwrap();
        assert!(installed_in(&[root.clone()]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
