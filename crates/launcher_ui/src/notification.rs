use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use egui::{self, Color32, CornerRadius, Frame, Layout, Margin, Stroke};
use textui::{LabelOptions, TextUi};

use crate::{assets, privacy};

const NOTIFICATION_TTL: Duration = Duration::from_secs(7);
const PROGRESS_NOTIFICATION_STALE_TTL: Duration = Duration::from_secs(14);
const NOTIFICATION_MAX_STACK: usize = 8;
const NOTIFICATION_EXPIRY_BAR_HEIGHT: f32 = 4.0;

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
    pub spinner: bool,
    pub replace_key: Option<String>,
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
    let streamer_mode = STREAMER_MODE_ENABLED.load(Ordering::Relaxed);
    let source = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &source.into()).into_owned(),
    );
    let message = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &message.into()).into_owned(),
    );
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
    if let Err(err) = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        spinner: false,
        replace_key: None,
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue notification."
        );
    }
}

pub fn emit_progress(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    progress: f32,
) {
    let streamer_mode = STREAMER_MODE_ENABLED.load(Ordering::Relaxed);
    let source = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &source.into()).into_owned(),
    );
    let message = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &message.into()).into_owned(),
    );
    let progress = progress.clamp(0.0, 1.0);
    tracing::info!(
        target: "notification",
        source = %source,
        message = %message,
        progress,
        "notification progress update"
    );
    if let Err(err) = center().tx.send(Notification {
        severity,
        source: source.clone(),
        message,
        progress: Some(progress),
        spinner: false,
        replace_key: Some(source.clone()),
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            source = %source,
            progress,
            error = %err,
            "Failed to enqueue progress notification."
        );
    }
}

pub fn emit_spinner(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    replace_key: impl Into<String>,
) {
    let streamer_mode = STREAMER_MODE_ENABLED.load(Ordering::Relaxed);
    let source = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &source.into()).into_owned(),
    );
    let message = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &message.into()).into_owned(),
    );
    let replace_key = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &replace_key.into()).into_owned(),
    );
    tracing::info!(
        target: "notification",
        source = %source,
        message = %message,
        replace_key = %replace_key,
        spinner = true,
        "notification spinner update"
    );
    if let Err(err) = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        spinner: true,
        replace_key: Some(replace_key),
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue spinner notification."
        );
    }
}

pub fn emit_replace(
    severity: Severity,
    source: impl Into<String>,
    message: impl Into<String>,
    replace_key: impl Into<String>,
) {
    let streamer_mode = STREAMER_MODE_ENABLED.load(Ordering::Relaxed);
    let source = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &source.into()).into_owned(),
    );
    let message = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &message.into()).into_owned(),
    );
    let replace_key = privacy::sanitize_text_for_log(
        &privacy::redact_sensitive_text(streamer_mode, &replace_key.into()).into_owned(),
    );
    tracing::info!(
        target: "notification",
        source = %source,
        message = %message,
        replace_key = %replace_key,
        "notification replacement update"
    );
    if let Err(err) = center().tx.send(Notification {
        severity,
        source,
        message,
        progress: None,
        spinner: false,
        replace_key: Some(replace_key),
        when: Instant::now(),
    }) {
        tracing::error!(
            target: "notification",
            error = %err,
            "Failed to enqueue replacement notification."
        );
    }
}

fn drain_notifications() {
    let center = center();
    let Ok(rx) = center.rx.lock() else {
        tracing::error!(
            target: "notification",
            "Notification receiver mutex was poisoned while draining notifications."
        );
        return;
    };
    let Ok(mut store) = center.store.lock() else {
        tracing::error!(
            target: "notification",
            "Notification store mutex was poisoned while draining notifications."
        );
        return;
    };

    while let Ok(notif) = rx.try_recv() {
        if let Some(replace_key) = notif.replace_key.as_deref() {
            if let Some(existing) = store
                .entries
                .iter_mut()
                .find(|entry| entry.replace_key.as_deref() == Some(replace_key))
            {
                existing.severity = notif.severity;
                existing.source = notif.source;
                existing.message = notif.message;
                existing.progress = notif.progress;
                existing.spinner = notif.spinner;
                existing.count = 1;
                existing.replace_key = notif.replace_key;
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

        let entry_id = store.next_id;
        store.entries.push(NotificationEntry {
            id: entry_id,
            severity: notif.severity,
            source: notif.source,
            message: notif.message,
            progress: notif.progress,
            spinner: notif.spinner,
            replace_key: notif.replace_key,
            count: 1,
            last_seen: notif.when,
        });
        store.next_id = entry_id.saturating_add(1);
    }

    let now = Instant::now();
    store.entries.retain(|entry| {
        let ttl = if entry.progress.is_some() || entry.spinner {
            PROGRESS_NOTIFICATION_STALE_TTL
        } else {
            NOTIFICATION_TTL
        };
        now.saturating_duration_since(entry.last_seen) < ttl
    });
    store.entries.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    if store.entries.len() > NOTIFICATION_MAX_STACK {
        store.entries.truncate(NOTIFICATION_MAX_STACK);
    }
}

pub fn render_popups(
    ctx: &egui::Context,
    text_ui: &mut TextUi,
    expiry_bars_empty_left: bool,
    suppressed_progress_source: Option<&str>,
) {
    drain_notifications();

    let entries = {
        let Ok(store) = center().store.lock() else {
            return;
        };
        if store.entries.is_empty() {
            return;
        }
        store
            .entries
            .iter()
            .filter(|entry| {
                suppressed_progress_source.is_none_or(|source| {
                    let suppressible = entry.progress.is_some() || entry.spinner;
                    !suppressible
                        || (entry.source != source && entry.replace_key.as_deref() != Some(source))
                })
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    if entries.is_empty() {
        return;
    }

    egui::Area::new(egui::Id::new("notification_stack_area"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-14.0, 48.0))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(360.0);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 8.0);
            let now = Instant::now();
            let mut dismissed_ids = Vec::new();

            for (index, entry) in entries.iter().enumerate() {
                let base = ui.visuals().window_fill;
                let frame = Frame::new()
                    .fill(Color32::from_rgba_premultiplied(
                        base.r(),
                        base.g(),
                        base.b(),
                        255,
                    ))
                    .stroke(Stroke::new(1.0, ui.visuals().window_stroke.color))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(10));

                frame.show(ui, |ui| {
                    if entry.progress.is_none() && !entry.spinner {
                        let elapsed = now.saturating_duration_since(entry.last_seen);
                        let remaining = NOTIFICATION_TTL.saturating_sub(elapsed);
                        let expiry_progress = (remaining.as_secs_f32()
                            / NOTIFICATION_TTL.as_secs_f32())
                        .clamp(0.0, 1.0);
                        let expiry_fill = severity_accent_fill(ui, entry.severity);
                        let bar_width = ui.available_width().max(1.0);
                        let (bar_rect, _) = ui.allocate_exact_size(
                            egui::vec2(bar_width, NOTIFICATION_EXPIRY_BAR_HEIGHT),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(
                            bar_rect,
                            CornerRadius::same(2),
                            ui.visuals().widgets.inactive.bg_fill,
                        );
                        if expiry_progress > 0.0 {
                            let fill_width = bar_rect.width() * expiry_progress;
                            let filled_rect = if expiry_bars_empty_left {
                                egui::Rect::from_min_max(
                                    egui::pos2(bar_rect.max.x - fill_width, bar_rect.min.y),
                                    bar_rect.max,
                                )
                            } else {
                                egui::Rect::from_min_max(
                                    bar_rect.min,
                                    egui::pos2(bar_rect.min.x + fill_width, bar_rect.max.y),
                                )
                            };
                            ui.painter().rect_filled(
                                filled_rect,
                                CornerRadius::same(2),
                                expiry_fill,
                            );
                        }
                        ui.add_space(8.0);
                    }

                    ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                        let close_button = notification_icon_button(
                            ui,
                            &format!("notif-close-{}", entry.id),
                            assets::X_SVG,
                            "Dismiss notification",
                        );
                        if close_button.clicked() {
                            dismissed_ids.push(entry.id);
                        }
                        ui.add_space(6.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width().max(0.0), 0.0),
                            Layout::left_to_right(egui::Align::TOP),
                            |ui| {
                                let icon_size = 16.0;
                                let icon = themed_svg_image(
                                    &format!(
                                        "notif-icon-{}-{}",
                                        index,
                                        severity_name(entry.severity)
                                    ),
                                    icon_for_severity(entry.severity),
                                    ui.visuals().text_color(),
                                    icon_size,
                                );
                                let icon_rect = egui::Rect::from_min_size(
                                    ui.cursor().min + egui::vec2(0.0, 2.0),
                                    egui::vec2(icon_size, icon_size),
                                );
                                ui.put(icon_rect, icon);
                                ui.add_space(6.0);

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
                                source_style.wrap = true;
                                source_style.weight = 700;
                                source_style.font_size = 15.0;
                                source_style.line_height = 20.0;
                                let _ = text_ui.label(
                                    ui,
                                    ("notif-source", index),
                                    &format!(
                                        "{} · {}",
                                        severity_name(entry.severity),
                                        entry.source
                                    ),
                                    &source_style,
                                );
                            },
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
                            egui::ProgressBar::new(progress).desired_width(ui.available_width()),
                        );
                        let _ = text_ui.label(
                            ui,
                            ("notif-progress-message", index),
                            overlay.as_str(),
                            &message_style,
                        );
                    } else if entry.spinner {
                        ui.horizontal_wrapped(|ui| {
                            ui.spinner();
                            ui.add_space(6.0);
                            let _ = text_ui.label(
                                ui,
                                ("notif-spinner-message", index),
                                entry.message.as_str(),
                                &message_style,
                            );
                        });
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

            if !dismissed_ids.is_empty()
                && let Ok(mut store) = center().store.lock()
            {
                store
                    .entries
                    .retain(|entry| !dismissed_ids.contains(&entry.id));
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

fn severity_accent_fill(ui: &egui::Ui, severity: Severity) -> Color32 {
    match severity {
        Severity::Log | Severity::Info => ui.visuals().selection.bg_fill,
        Severity::Warn => ui.visuals().warn_fg_color,
        Severity::Error => ui.visuals().error_fg_color,
    }
}

fn notification_icon_button(
    ui: &mut egui::Ui,
    icon_id: &str,
    svg_bytes: &[u8],
    tooltip: &str,
) -> egui::Response {
    let button_size = egui::vec2(22.0, 22.0);
    let icon_size = 12.0;
    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        ui.visuals().widgets.active.bg_fill
    } else if response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.weak_bg_fill
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(7),
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let image = themed_svg_image(icon_id, svg_bytes, ui.visuals().text_color(), icon_size);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(icon_size, icon_size));
    let _ = ui.put(icon_rect, image);

    response.on_hover_text(tooltip)
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
