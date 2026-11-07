//! Notification popups, top-right under the title bar, newest first: the SE5 view of `launcher_ui::notification`'s store,
//! with the user's expiry-bar direction, fade time and easing. Every card has the same padding on all sides; the
//! severity shows as a strip down its left edge.

use std::time::Duration;

use config::Config;
use launcher_ui::notification::{self, NotificationDisplaySettings, Severity};
use vertex_shell::sturdy::glam::{Vec2, Vec4};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, CornerRadius, Direction,
    ElementDesc, FloatTarget, Floating, Padding, Sizing, TextStyle, TextWrap, Ui,
};

use super::kit::{self, ButtonKind, Kit, TextRole};

const WIDTH: f32 = 360.0;
/// Space between the window edge and the stack, and the same padding inside each card.
const MARGIN: f32 = 16.0;
const PADDING: u16 = 14;
/// The title bar's height, so the stack starts just below it.
const TITLE_BAR: f32 = 40.0;

pub fn show(ui: &mut Ui<'_>, kit: &mut Kit, config: &Config) {
    let settings = NotificationDisplaySettings {
        expiry_bars_empty_left: config.notification_expiry_bars_empty_left(),
        fade_out: Duration::from_secs_f32(config.notification_fade_out_seconds().max(0.0)),
        easing: config.notification_easing(),
    };
    let toasts = notification::visible_notifications(&settings);
    if toasts.is_empty() {
        return;
    }
    // Expiry bars and fades animate; keep frames coming while any popup is up.

    let _stack = ui.element(
        &ElementDesc::new()
            .id("toasts".into())
            .width(Sizing::Fixed(WIDTH))
            .height(Sizing::Fit)
            .direction(Direction::TopToBottom)
            .child_gap(kit::SPACE_SM)
            .floating(Some(Floating {
                element_attach_point: AttachPoint::RightTop,
                parent_attach_point: AttachPoint::RightTop,
                offset: Vec2::new(-MARGIN, TITLE_BAR + MARGIN * 0.5),
                z_index: 1500,
                // The cards are in front of the page, so they take the pointer: what they cover is not clickable.
                capture_pointer: true,
                ..Floating::new(FloatTarget::Root)
            })),
    );
    let mut dismissed = Vec::new();
    for toast in toasts.iter() {
        let accent = severity_color(kit, toast.severity);
        let fade = toast.opacity.clamp(0.0, 1.0);
        let id = format!("toast::{}", toast.id);
        {
            // The card: a severity strip, then the content with equal padding on every side.
            let _card = ui.element(
                &ElementDesc::new()
                    .id(id.clone())
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::LeftToRight)
                    .background(
                        kit.palette
                            .bg_dark
                            .with_w(kit.palette.bg_dark.w * 0.97 * fade),
                    )
                    .corner_radius(CornerRadius::all(kit::CORNER))
                    .border(Border::all(kit.palette.border_muted.with_w(fade), 1)),
            );
            // An inset pill, so it never touches the card's rounded corners or border.
            {
                let _gutter = ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Fit)
                        .height(Sizing::Grow)
                        .padding(Padding {
                            left: 7,
                            right: 0,
                            top: 9,
                            bottom: 9,
                        }),
                );
                drop(
                    ui.element(
                        &ElementDesc::new()
                            .width(Sizing::Fixed(3.0))
                            .height(Sizing::Grow)
                            .background(accent.with_w(fade))
                            .corner_radius(CornerRadius::all(1.5)),
                    ),
                );
            }
            let _content = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_SM)
                    .padding(Padding::all(PADDING)),
            );
            {
                let _header = ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Grow)
                        .height(Sizing::Fit)
                        .direction(Direction::LeftToRight)
                        .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
                );
                let title = if toast.count > 1 {
                    format!("{} ×{}", severity_name(toast.severity), toast.count)
                } else {
                    severity_name(toast.severity).to_owned()
                };
                kit.text_colored(ui, &title, accent.with_w(fade), 0.85);
                kit.fill(ui);
                let close = format!("{id}::close");
                {
                    let _close = ui.element(
                        &ElementDesc::new()
                            .id(close.clone())
                            .width(Sizing::Fixed(22.0))
                            .height(Sizing::Fixed(22.0))
                            .align(ChildAlignment::center())
                            .corner_radius(CornerRadius::all(6.0))
                            .background(if ui.hovered(&close) {
                                kit.palette.hovered_fill()
                            } else {
                                Vec4::ZERO
                            }),
                    );
                    kit.text_colored(ui, "✕", kit.palette.text_muted.with_w(fade), 0.8);
                }
                if kit.clicked_plain(ui, &close) {
                    dismissed.push(toast.id);
                }
            }
            // The card's text width: the stack's width less the border, the accent strip and both paddings.
            kit::with_text_width(WIDTH - 2.0 - 10.0 - 2.0 * f32::from(PADDING), || {
                kit::label(
                    ui,
                    &toast.message,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        color: kit.palette.text.with_w(fade),
                        ..kit.style(TextRole::Body)
                    },
                )
            });
            if toast.progress.is_some() || toast.spinner {
                kit.progress(
                    ui,
                    &format!("{id}::progress"),
                    toast.progress,
                    kit::uptime(),
                );
            }
            if !toast.actions.is_empty() {
                let _actions = kit.row(ui, kit::SPACE_SM);
                for (index, action) in toast.actions.iter().enumerate() {
                    let kind = if action.danger {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Neutral
                    };
                    if kit.button(
                        ui,
                        &format!("{id}::action::{index}"),
                        &action.label,
                        kind,
                        true,
                    ) {
                        (action.callback)();
                        dismissed.push(toast.id);
                    }
                }
            }
            expiry_bar(
                ui,
                kit,
                toast.remaining,
                settings.expiry_bars_empty_left,
                accent.with_w(0.8 * fade),
            );
        }
    }
    for id in dismissed {
        notification::dismiss(id);
    }
}

fn expiry_bar(ui: &mut Ui<'_>, kit: &Kit, remaining: f32, empty_left: bool, color: Vec4) {
    let _track = ui.element(
        &ElementDesc::new()
            .width(Sizing::Grow)
            .height(Sizing::Fixed(3.0))
            .direction(Direction::LeftToRight)
            .align(ChildAlignment::new(
                if empty_left {
                    AlignX::Right
                } else {
                    AlignX::Left
                },
                AlignY::Center,
            ))
            .background(kit.palette.bg_light.with_w(0.5))
            .corner_radius(CornerRadius::all(1.5)),
    );
    drop(
        ui.element(
            &ElementDesc::new()
                .width(Sizing::Percent(remaining.clamp(0.0, 1.0)))
                .height(Sizing::Grow)
                .background(color)
                .corner_radius(CornerRadius::all(1.5)),
        ),
    );
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Log => "Log",
        Severity::Info => "Info",
        Severity::Warn => "Warning",
        Severity::Error => "Error",
    }
}

fn severity_color(kit: &Kit, severity: Severity) -> Vec4 {
    match severity {
        Severity::Log => kit.palette.text_muted,
        Severity::Info => kit.palette.info,
        Severity::Warn => kit.palette.warning,
        Severity::Error => kit.palette.danger,
    }
}
