#[path = "notification/emitters.rs"]
mod emitters;
#[path = "notification/natural_language.rs"]
pub mod natural_language;
#[path = "notification/store.rs"]
mod store;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use config::NotificationEasing;


pub use emitters::{
    NotificationBuilder, emit, emit_progress, emit_replace, emit_spinner, emit_with_actions,
    emit_with_icon, retract_from_stack,
};
pub use natural_language::{count_word, join_natural};
use store::{drain_notifications, entry_fade_opacity, entry_ttl};

const NOTIFICATION_TTL: Duration = Duration::from_secs(7);
/// Notifications carrying buttons stay up longer so there is time to reach them.
const NOTIFICATION_ACTION_TTL: Duration = Duration::from_secs(15);
const PROGRESS_NOTIFICATION_STALE_TTL: Duration = Duration::from_secs(14);
const NOTIFICATION_MAX_STACK: usize = 8;


/// User-configurable presentation settings for notification popups.
#[derive(Debug, Clone, Copy)]
pub struct NotificationDisplaySettings {
    pub expiry_bars_empty_left: bool,
    pub fade_out: Duration,
    pub easing: NotificationEasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Log,
    Info,
    Warn,
    Error,
}

/// A button attached to a notification. The callback runs on the UI thread when it is clicked; anything that needs
/// the front end goes through [`crate::app_commands`].
#[derive(Clone)]
pub struct NotificationAction {
    pub label: String,
    pub callback: Arc<dyn Fn() + Send + Sync>,
    /// Draws the label in the theme's danger color, for destructive actions.
    pub danger: bool,
}

impl NotificationAction {
    pub fn new(
        label: impl Into<String>,
        callback: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            callback: Arc::new(callback),
            danger: false,
        }
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

impl std::fmt::Debug for NotificationAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationAction")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// How a notification with its own details merges into similar ones that arrive while it is
/// still showing, instead of piling up as separate popups.
///
/// Notifications sharing a `key` become one popup. Each arrival adds its `items`, and the
/// message is rewritten from *all* items by `render`, so the text stays natural ("got two new
/// capes: A and B") rather than being appended to. Notifications with no details of their own
/// need no spec: identical ones collapse automatically and show an "xN" badge by the icon.
#[derive(Clone)]
pub struct StackSpec {
    pub key: String,
    pub items: Vec<String>,
    pub render: Arc<dyn Fn(&[String]) -> String + Send + Sync>,
    /// Takes `items` back out of the stack instead of adding them (see [`retract_from_stack`]).
    pub remove: bool,
}

impl std::fmt::Debug for StackSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StackSpec")
            .field("key", &self.key)
            .field("items", &self.items)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub severity: Severity,
    pub source: String,
    pub message: String,
    pub progress: Option<f32>,
    pub spinner: bool,
    pub replace_key: Option<String>,
    pub actions: Vec<NotificationAction>,
    /// SVG shown instead of the severity icon.
    pub icon: Option<&'static [u8]>,
    pub stack: Option<StackSpec>,
    pub when: Instant,
}

#[derive(Debug, Clone)]
struct NotificationEntry {
    id: u64,
    severity: Severity,
    source: String,
    message: String,
    progress: Option<f32>,
    spinner: bool,
    replace_key: Option<String>,
    actions: Vec<NotificationAction>,
    icon: Option<&'static [u8]>,
    stack: Option<StackSpec>,
    count: u32,
    last_seen: Instant,
}

#[derive(Default)]
struct NotificationStore {
    entries: Vec<NotificationEntry>,
    next_id: u64,
}

struct NotificationCenter {
    tx: mpsc::Sender<Notification>,
    rx: Mutex<mpsc::Receiver<Notification>>,
    store: Mutex<NotificationStore>,
}

static NOTIFICATION_CENTER: OnceLock<NotificationCenter> = OnceLock::new();
static STREAMER_MODE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_streamer_mode(enabled: bool) {
    STREAMER_MODE_ENABLED.store(enabled, Ordering::Relaxed);
}

fn streamer_mode_enabled() -> bool {
    STREAMER_MODE_ENABLED.load(Ordering::Relaxed)
}

fn center() -> &'static NotificationCenter {
    NOTIFICATION_CENTER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Notification>();
        NotificationCenter {
            tx,
            rx: Mutex::new(rx),
            store: Mutex::new(NotificationStore::default()),
        }
    })
}





#[macro_export]
macro_rules! __notification_log {
    ($source:expr, $fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Log,
            $source,
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
    ($fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Log,
            module_path!(),
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
}

#[macro_export]
macro_rules! __notification_info {
    ($source:expr, $fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Info,
            $source,
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
    ($fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Info,
            module_path!(),
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
}

#[macro_export]
macro_rules! __notification_warn {
    ($source:expr, $fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Warn,
            $source,
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
    ($fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Warn,
            module_path!(),
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
}

#[macro_export]
macro_rules! __notification_error {
    ($source:expr, $fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Error,
            $source,
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
    ($fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Error,
            module_path!(),
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
        )
    };
}

pub use crate::__notification_error as error;
pub use crate::__notification_info as info;
pub use crate::__notification_log as log;
pub use crate::__notification_warn as warn;

#[macro_export]
macro_rules! __notification_progress {
    ($severity:expr, $source:expr, $progress:expr, $fmt:literal $(, $name:ident = $value:expr )* $(,)?) => {
        $crate::notification::emit_progress(
            $severity,
            $source,
            $crate::i18n::tr!($fmt $(, $name = $value)*).into_owned(),
            $progress,
        )
    };
}

pub use crate::__notification_progress as progress;

/// One notification popup as plain data, for front ends that draw their own popups.
#[derive(Debug, Clone)]
pub struct NotificationView {
    pub id: u64,
    pub severity: Severity,
    pub source: String,
    pub message: String,
    pub progress: Option<f32>,
    pub spinner: bool,
    /// How many identical notifications were folded into this one (the "xN" badge).
    pub count: u32,
    pub actions: Vec<NotificationAction>,
    /// 1 while the popup is live, falling to 0 as it fades out.
    pub opacity: f32,
    /// Share of the popup's lifetime left, for the expiry bar.
    pub remaining: f32,
}

/// Absorbs newly emitted notifications and returns the popups to show, newest first. Mirrors what
/// [`render_popups`] shows, including the streamer-mode name scrubbing done by the emitters.
pub fn visible_notifications(settings: &NotificationDisplaySettings) -> Vec<NotificationView> {
    drain_notifications(settings.fade_out);
    let Ok(store) = center().store.lock() else {
        return Vec::new();
    };
    let now = Instant::now();
    store
        .entries
        .iter()
        .map(|entry| {
            let ttl = entry_ttl(entry);
            let remaining = ttl.saturating_sub(now.saturating_duration_since(entry.last_seen));
            NotificationView {
                id: entry.id,
                severity: entry.severity,
                source: entry.source.clone(),
                message: entry.message.clone(),
                progress: entry.progress,
                spinner: entry.spinner,
                count: entry.count,
                actions: entry.actions.clone(),
                opacity: entry_fade_opacity(entry, now, settings),
                remaining: (remaining.as_secs_f32() / ttl.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0),
            }
        })
        .collect()
}

/// Removes a popup now (its close button).
pub fn dismiss(id: u64) {
    if let Ok(mut store) = center().store.lock() {
        store.entries.retain(|entry| entry.id != id);
    }
}
