use egui::Ui;
use textui::{LabelOptions, TextUi};

use crate::{console, ui::style};

const CONSOLE_TEXT_TILE_CHARS: usize = 512;

pub fn render(ui: &mut Ui, text_ui: &mut TextUi) {
    let snapshot = console::snapshot();
    let lines = &snapshot.active_lines;
    let viewport_size = egui::vec2(
        ui.available_width().max(1.0),
        ui.available_height().max(1.0),
    );
    ui.allocate_ui_with_layout(
        viewport_size,
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.add_space(style::SPACE_LG);
            let inner_width = (ui.available_width() - style::SPACE_LG).max(1.0);
            ui.allocate_ui_with_layout(
                egui::vec2(inner_width, ui.available_height().max(1.0)),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    render_tabs_row(ui, text_ui, &snapshot);
                    ui.add_space(style::SPACE_SM);
                    let viewport_height = ui.available_height().max(1.0);
                    ui.set_min_height(viewport_height);
                    egui::ScrollArea::both()
                        .id_salt("console_scroll_area")
                        .auto_shrink([false, false])
                        .max_height(viewport_height)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            if lines.is_empty() {
                                let mut empty_style = style::muted(ui);
                                empty_style.wrap = false;
                                let _ = text_ui.label(
                                    ui,
                                    "console_empty",
                                    "No log entries yet.",
                                    &empty_style,
                                );
                                let _ = ui.allocate_exact_size(
                                    egui::vec2(1.0, (viewport_height - 24.0).max(1.0)),
                                    egui::Sense::hover(),
                                );
                                return;
                            }

                            let mut context = LogParseContext::default();
                            for (index, line) in lines.iter().enumerate() {
                                let resolved_level = resolve_log_level(line, &mut context);
                                let mut line_style = style::body(ui);
                                line_style.wrap = false;
                                line_style.color = color_for_level(ui, resolved_level);
                                line_style.padding = egui::Vec2::ZERO;
                                if matches!(resolved_level, Some(LogLevel::Error | LogLevel::Fatal))
                                {
                                    line_style.weight = 700;
                                }
                                render_tiled_console_line(ui, text_ui, index, line, &line_style);
                            }
                        });
                },
            );
            ui.add_space(style::SPACE_LG);
        },
    );
}

fn render_tabs_row(ui: &mut Ui, text_ui: &mut TextUi, snapshot: &console::ConsoleSnapshot) {
    let tab_style = textui::ButtonOptions {
        min_size: egui::vec2(80.0, 30.0),
        text_color: ui.visuals().text_color(),
        fill: ui.visuals().widgets.inactive.weak_bg_fill,
        fill_hovered: ui.visuals().widgets.hovered.bg_fill,
        fill_active: ui.visuals().widgets.active.bg_fill,
        fill_selected: ui.visuals().selection.bg_fill,
        stroke: ui.visuals().widgets.inactive.bg_stroke,
        ..textui::ButtonOptions::default()
    };
    egui::ScrollArea::horizontal()
        .id_salt("console_tabs")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(style::SPACE_SM, style::SPACE_SM);
                for tab in &snapshot.tabs {
                    let selected = tab.id == snapshot.active_tab_id;
                    if text_ui
                        .selectable_button(
                            ui,
                            ("console_tab", tab.id.as_str()),
                            tab.label.as_str(),
                            selected,
                            &tab_style,
                        )
                        .clicked()
                    {
                        console::set_active_tab(tab.id.as_str());
                    }
                }
            });
        });
}

fn color_for_level(ui: &Ui, level: Option<LogLevel>) -> egui::Color32 {
    match level {
        Some(LogLevel::Fatal | LogLevel::Error) => ui.visuals().error_fg_color,
        Some(LogLevel::Warn) => ui.visuals().warn_fg_color,
        Some(LogLevel::Info) => ui.visuals().hyperlink_color,
        Some(LogLevel::Debug | LogLevel::Trace) => ui.visuals().weak_text_color(),
        None => ui.visuals().text_color(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

#[derive(Clone, Debug, Default)]
struct LogParseContext {
    in_error_trace: bool,
}

fn resolve_log_level(line: &str, context: &mut LogParseContext) -> Option<LogLevel> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        context.in_error_trace = false;
        return None;
    }

    if let Some(level) = detect_log_level(line) {
        context.in_error_trace = matches!(level, LogLevel::Error | LogLevel::Fatal);
        return Some(level);
    }

    if is_stacktrace_line(trimmed)
        || (context.in_error_trace && is_stacktrace_continuation_line(trimmed))
    {
        context.in_error_trace = true;
        return Some(LogLevel::Error);
    }

    if trimmed.starts_with('[') {
        context.in_error_trace = false;
    }
    None
}

fn detect_log_level(line: &str) -> Option<LogLevel> {
    if let Some(level) = parse_minecraft_log_level(line) {
        return Some(level);
    }
    parse_generic_log_level(line)
}

fn is_stacktrace_line(trimmed: &str) -> bool {
    trimmed.starts_with("at ")
        || trimmed.starts_with("Caused by:")
        || trimmed.starts_with("Suppressed:")
        || trimmed.starts_with("Exception in thread ")
        || (trimmed.starts_with("... ") && trimmed.ends_with(" more"))
        || trimmed.contains("Exception:")
        || trimmed.ends_with("Exception")
        || trimmed.contains("Error:")
        || trimmed.ends_with("Error")
}

fn is_stacktrace_continuation_line(trimmed: &str) -> bool {
    trimmed.starts_with('\t')
        || trimmed.starts_with("com.")
        || trimmed.starts_with("net.")
        || trimmed.starts_with("org.")
        || trimmed.starts_with("java.")
        || trimmed.starts_with("javax.")
        || trimmed.starts_with("kotlin.")
        || trimmed.starts_with('#')
}

fn parse_minecraft_log_level(line: &str) -> Option<LogLevel> {
    // Vanilla/Forge-like game logs usually look like:
    // [20:29:39] [main/WARN]: ...
    // [20:29:39] [Render thread/INFO] [pkg.Logger/]: ...
    if !line.starts_with('[') {
        return None;
    }
    let first_close = line.find(']')?;
    if first_close < 2 {
        return None;
    }
    let timestamp = &line[1..first_close];
    if !looks_like_minecraft_timestamp(timestamp) {
        return None;
    }
    let after_timestamp = line.get(first_close + 1..)?;
    if !after_timestamp.starts_with(" [") {
        return None;
    }
    let second = after_timestamp.get(2..)?;
    let second_close = second.find(']')?;
    let thread_and_level = &second[..second_close];
    if let Some((_, level_token)) = thread_and_level.rsplit_once('/')
        && let Some(level) = parse_level_token(level_token)
    {
        return Some(level);
    }

    // User requested Minecraft logs default to INFO when level token is absent/unrecognized.
    Some(LogLevel::Info)
}

fn parse_generic_log_level(line: &str) -> Option<LogLevel> {
    for (token, level) in [
        ("FATAL", LogLevel::Fatal),
        ("ERROR", LogLevel::Error),
        ("WARN", LogLevel::Warn),
        ("INFO", LogLevel::Info),
        ("DEBUG", LogLevel::Debug),
        ("TRACE", LogLevel::Trace),
    ] {
        if line.contains(&format!("][{token}]["))
            || line.contains(&format!("][{token}]:"))
            || line.contains(&format!("/{token}]"))
            || line.contains(&format!("/{token}]:"))
        {
            return Some(level);
        }
    }
    None
}

fn parse_level_token(token: &str) -> Option<LogLevel> {
    match token.trim() {
        "TRACE" => Some(LogLevel::Trace),
        "DEBUG" => Some(LogLevel::Debug),
        "INFO" => Some(LogLevel::Info),
        "WARN" => Some(LogLevel::Warn),
        "ERROR" => Some(LogLevel::Error),
        "FATAL" => Some(LogLevel::Fatal),
        _ => None,
    }
}

fn looks_like_minecraft_timestamp(value: &str) -> bool {
    // Typical game output uses HH:mm:ss
    let mut parts = value.split(':');
    let Some(hours) = parts.next() else {
        return false;
    };
    let Some(minutes) = parts.next() else {
        return false;
    };
    let Some(seconds) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [hours, minutes, seconds]
        .iter()
        .all(|part| part.len() == 2 && part.as_bytes().iter().all(u8::is_ascii_digit))
}

fn render_tiled_console_line(
    ui: &mut Ui,
    text_ui: &mut TextUi,
    line_index: usize,
    line: &str,
    line_style: &LabelOptions,
) {
    if line.is_empty() {
        let _ = text_ui.label_async(ui, ("console_line", line_index, 0usize), "", line_style);
        return;
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut start = 0usize;
        let mut tile_index = 0usize;
        while start < line.len() {
            let end = tile_end_at_char_limit(line, start, CONSOLE_TEXT_TILE_CHARS);
            let segment = &line[start..end];
            let _ = text_ui.label_async(
                ui,
                ("console_line", line_index, tile_index),
                segment,
                line_style,
            );
            start = end;
            tile_index = tile_index.saturating_add(1);
        }
    });
}

fn tile_end_at_char_limit(line: &str, start: usize, max_chars: usize) -> usize {
    line[start..]
        .char_indices()
        .nth(max_chars)
        .map(|(offset, _)| start + offset)
        .unwrap_or(line.len())
}
