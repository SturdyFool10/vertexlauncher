use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use egui::{self, Color32, CornerRadius, Frame, Margin, Stroke};
use textui::{LabelOptions, TextUi};

use crate::assets;

const NOTIFICATION_TTL: Duration = Duration::from_secs(7);
const NOTIFICATION_MAX_STACK: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Log,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub severity: Severity,
    pub source: String,
    pub message: String,
    pub progress: Option<f32>,
    pub replace_by_source: bool,
    pub when: Instant,
}

#[derive(Debug, Clone)]
struct NotificationEntry {
    severity: Severity,
    source: String,
    message: String,
    progress: Option<f32>,
    count: u32,
    last_seen: Instant,
}

#[derive(Default)]
struct NotificationStore {
    entries: Vec<NotificationEntry>,
}

struct NotificationCenter {
    tx: mpsc::Sender<Notification>,
    rx: Mutex<mpsc::Receiver<Notification>>,
    store: Mutex<NotificationStore>,
}

static NOTIFICATION_CENTER: OnceLock<NotificationCenter> = OnceLock::new();

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

fn icon_for_severity(severity: Severity) -> &'static [u8] {
    match severity {
        Severity::Log => assets::LOG_SVG,
        Severity::Info => assets::INFO_CIRCLE_SVG,
        Severity::Warn => assets::WARN_SVG,
        Severity::Error => assets::ERROR_SVG,
    }
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Log => "Log",
        Severity::Info => "Info",
        Severity::Warn => "Warn",
        Severity::Error => "Error",
    }
}

pub fn emit(severity: Severity, source: impl Into<String>, message: impl Into<String>) {
    let source = source.into();
    let message = message.into();
    match severity {
        Severity::Log => {
            tracing::debug!(target: "notification", source = %source, message = %message)
        }
        Severity::Info => {
            tracing::info!(target: "notification", source = %source, message = %message)
        }
        Severity::Warn => {
            tracing::warn!(target: "notification", source = %source, message = %message)
        }
        Severity::Error => {
            tracing::error!(target: "notification", source = %source, message = %message)
        }
    }
    let _ = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        replace_by_source: false,
        when: Instant::now(),
    });
}

pub fn emit_progress(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
) {
    let source = source.into();
    let message = message.into();
    let progress = progress.clamp(0.0, 1.0);
    let _ = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: Some(progress),
        replace_by_source: true,
        when: Instant::now(),
    });
}

fn drain_notifications() {
    let center = center();
    let Ok(rx) = center.rx.lock() else {
        return;
    };
    let Ok(mut store) = center.store.lock() else {
        return;
    };

    while let Ok(notif) = rx.try_recv() {
        if notif.replace_by_source {
            if let Some(existing) = store
                .entries
                .iter_mut()
                .find(|entry| entry.severity == notif.severity && entry.source == notif.source)
            {
                existing.message = notif.message;
                existing.progress = notif.progress;
                existing.count = 1;
                existing.last_seen = notif.when;
                continue;
            }
        } else if let Some(existing) = store.entries.iter_mut().find(|entry| {
            entry.severity == notif.severity
                && entry.source == notif.source
                && entry.message == notif.message
        }) {
            existing.count = existing.count.saturating_add(1);
            existing.last_seen = notif.when;
            continue;
        }

        store.entries.push(NotificationEntry {
            severity: notif.severity,
            source: notif.source,
            message: notif.message,
            progress: notif.progress,
            count: 1,
            last_seen: notif.when,
        });
    }

    let now = Instant::now();
    store
        .entries
        .retain(|entry| now.saturating_duration_since(entry.last_seen) < NOTIFICATION_TTL);
    store.entries.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    if store.entries.len() > NOTIFICATION_MAX_STACK {
        store.entries.truncate(NOTIFICATION_MAX_STACK);
    }
}

pub fn render_popups(ctx: &egui::Context, text_ui: &mut TextUi) {
    drain_notifications();

    let entries = {
        let Ok(store) = center().store.lock() else {
            return;
        };
        if store.entries.is_empty() {
            return;
        }
        store.entries.clone()
    };

    egui::Area::new(egui::Id::new("notification_stack_area"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-14.0, 48.0))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(360.0);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 8.0);

            for (index, entry) in entries.iter().enumerate() {
                let frame = Frame::new()
                    .fill(ui.visuals().widgets.noninteractive.bg_fill)
                    .stroke(Stroke::new(
                        1.0,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(10));

                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let icon_size = 16.0;
                        let icon = themed_svg_image(
                            &format!("notif-icon-{}-{}", index, severity_name(entry.severity)),
                            icon_for_severity(entry.severity),
                            ui.visuals().text_color(),
                            icon_size,
                        );
                        let icon_rect = egui::Rect::from_min_size(
                            ui.cursor().min + egui::vec2(0.0, 2.0),
                            egui::vec2(icon_size, icon_size),
                        );
                        ui.put(icon_rect, icon);
                        ui.add_space(icon_size + 6.0);

                        if entry.count > 1 {
                            let mut count_style = LabelOptions::default();
                            count_style.color = ui.visuals().weak_text_color();
                            count_style.wrap = false;
                            count_style.font_size = 14.0;
                            count_style.line_height = 18.0;
                            let _ = text_ui.label(
                                ui,
                                ("notif-count", index),
                                &format!("x{}", entry.count),
                                &count_style,
                            );
                            ui.add_space(6.0);
                        }

                        let mut source_style = LabelOptions::default();
                        source_style.color = ui.visuals().text_color();
                        source_style.wrap = false;
                        source_style.weight = 700;
                        source_style.font_size = 15.0;
                        source_style.line_height = 20.0;
                        let _ = text_ui.label(
                            ui,
                            ("notif-source", index),
                            &format!("{} · {}", severity_name(entry.severity), entry.source),
                            &source_style,
                        );
                    });

                    ui.add_space(4.0);
                    let mut message_style = LabelOptions::default();
                    message_style.color = ui.visuals().text_color();
                    message_style.wrap = true;
                    message_style.font_size = 14.0;
                    message_style.line_height = 18.0;
                    if let Some(progress) = entry.progress {
                        ui.add_space(6.0);
                        let overlay = format!("{}  {:.0}%", entry.message, progress * 100.0);
                        ui.add(
                            egui::ProgressBar::new(progress)
                                .text(overlay)
                                .desired_width(ui.available_width()),
                        );
                    } else {
                        let _ = text_ui.label(
                            ui,
                            ("notif-message", index),
                            entry.message.as_str(),
                            &message_style,
                        );
                    }
                });
            }
        });

    ctx.request_repaint();
}

fn themed_svg_image(
    icon_id: &str,
    svg_bytes: &[u8],
    color: Color32,
    icon_size: f32,
) -> egui::Image<'static> {
    let themed_svg = apply_svg_color(svg_bytes, color);
    let mut hasher = DefaultHasher::new();
    icon_id.hash(&mut hasher);
    color.hash(&mut hasher);
    let uri = format!(
        "bytes://vertex-notification-icons/{:x}.svg",
        hasher.finish()
    );
    egui::Image::from_bytes(uri, themed_svg).fit_to_exact_size(egui::vec2(icon_size, icon_size))
}

fn apply_svg_color(svg_bytes: &[u8], color: Color32) -> Vec<u8> {
    let color_hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    String::from_utf8_lossy(svg_bytes)
        .replace("currentColor", &color_hex)
        .into_bytes()
}

#[macro_export]
macro_rules! __notification_log {
    ($source:expr, $fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Log,
            $source,
            format!($fmt $(, $arg )*),
        )
    };
    ($fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Log,
            module_path!(),
            format!($fmt $(, $arg )*),
        )
    };
}

#[macro_export]
macro_rules! __notification_info {
    ($source:expr, $fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Info,
            $source,
            format!($fmt $(, $arg )*),
        )
    };
    ($fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Info,
            module_path!(),
            format!($fmt $(, $arg )*),
        )
    };
}

#[macro_export]
macro_rules! __notification_warn {
    ($source:expr, $fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Warn,
            $source,
            format!($fmt $(, $arg )*),
        )
    };
    ($fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Warn,
            module_path!(),
            format!($fmt $(, $arg )*),
        )
    };
}

#[macro_export]
macro_rules! __notification_error {
    ($source:expr, $fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Error,
            $source,
            format!($fmt $(, $arg )*),
        )
    };
    ($fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit(
            $crate::notification::Severity::Error,
            module_path!(),
            format!($fmt $(, $arg )*),
        )
    };
}

pub use crate::__notification_error as error;
pub use crate::__notification_info as info;
pub use crate::__notification_log as log;
pub use crate::__notification_warn as warn;

#[macro_export]
macro_rules! __notification_progress {
    ($severity:expr, $source:expr, $progress:expr, $fmt:literal $(, $arg:expr )* $(,)?) => {
        $crate::notification::emit_progress(
            $severity,
            $source,
            format!($fmt $(, $arg )*),
            $progress,
        )
    };
}

pub use crate::__notification_progress as progress;
