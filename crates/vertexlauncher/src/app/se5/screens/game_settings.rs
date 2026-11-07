//! The in-game settings editor for an instance's `options.txt`: a live view of the file with edits written a moment
//! after you stop typing, plus the Resource Packs and Sync tabs. The schema logic is `game_settings_api`.

use launcher_ui::screens::game_settings_api::{
    self as api, Control, Group, PackOverride, Row, Session,
};
use vertex_shell::sturdy::Key;
use vertex_shell::sturdy::MouseButton;
use vertex_shell::sturdy::ui::{Clip, Direction, ElementDesc, Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::ModalStyle;

use super::super::AppState;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Group(Group),
    Packs,
    Sync,
}

/// The open dialog's state; lives in `AppState` while `Dialog::GameSettings` is showing.
pub struct GameSettingsState {
    session: Session,
    tab: Tab,
    search: String,
    /// The key bind being captured, and the frame it started on (so the click that started it is not the binding).
    capturing: Option<(String, u64)>,
    frame: u64,
}

/// Opens the dialog for an instance.
pub fn open(state: &mut AppState, instance_id: &str) {
    let Some(instance) = state.instances.find(instance_id).cloned() else {
        return;
    };
    let session = Session::open(&instance, state.installations_root().as_path());
    state.game_settings = Some(GameSettingsState {
        session,
        tab: Tab::Group(Group::Video),
        search: String::new(),
        capturing: None,
        frame: 0,
    });
    state.dialog = Some(super::super::Dialog::GameSettings(instance_id.to_owned()));
}

/// Returns false once the dialog should close.
pub fn dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, id: &str) -> bool {
    let AppState {
        game_settings,
        instances,
        config,
        ..
    } = state;
    let Some(gs) = game_settings.as_mut() else {
        return false;
    };
    if gs.session.instance_id() != id || instances.find(id).is_none() {
        gs.session.flush();
        *game_settings = None;
        return false;
    }
    gs.frame += 1;
    gs.session.poll(instances, config);
    let header = gs.session.header(instances, config);

    let viewport = ui.viewport_size();
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed((viewport.x * 0.92).clamp(520.0, 1100.0)),
        ..ModalStyle::default()
    };
    let mut close = false;
    // Escape cancels a key capture rather than closing the dialog.
    let was_capturing = gs.capturing.is_some();
    let response = kit::modal(ui, "game-settings", &style, true, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, &header.title);
        kit::label(
            ui,
            &header.subtitle,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        if header.running {
            kit::label(
                ui,
                "Minecraft is running. The game rewrites options.txt when you leave its options screens, so changes here can be overwritten; changes made in-game appear here as they are saved.",
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Warning)
                },
            );
        }
        if let Some(error) = &header.error {
            kit::label(
                ui,
                &format!("Couldn't save: {error}. Will retry."),
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Danger)
                },
            );
        }
        kit.text(
            ui,
            TextRole::Small,
            if header.saving {
                "Saving…"
            } else {
                "All changes saved"
            },
        );

        tab_strip(ui, kit, gs);
        kit.text_field(
            ui,
            "game-settings::search",
            &mut gs.search,
            "Search all settings",
            false,
            Sizing::Grow,
        );

        let height = (viewport.y - 380.0).clamp(220.0, 900.0);
        {
            let _scroll = ui.element(
                &ElementDesc::new()
                    .id("game-settings::scroll".into())
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(height))
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_SM)
                    .clip(Clip::vertical()),
            );
            kit.scrollbar(ui, "game-settings::scroll");
            body(ui, kit, gs, instances, config);
        }

        let _row = kit.row(ui, kit::SPACE_SM);
        kit.fill(ui);
        close = kit.button(
            ui,
            "game-settings::close",
            "Close",
            ButtonKind::Neutral,
            true,
        );
    });
    let done = close || ((response.dismissed || kit.back_pressed()) && !was_capturing);
    if done {
        // Save anything still pending before the view goes away.
        gs.session.flush();
        *game_settings = None;
        kit.forget("game-settings");
    }
    !done
}

fn tab_strip(ui: &mut Ui<'_>, kit: &mut Kit, gs: &mut GameSettingsState) {
    let _row = ui.element(
        &ElementDesc::new()
            .id("game-settings::tabs".into())
            .width(Sizing::Grow)
            .height(Sizing::Fit)
            .direction(Direction::LeftToRight)
            .child_gap(kit::SPACE_SM)
            .clip(Clip::horizontal()),
    );
    let mut tabs: Vec<(Tab, String, String)> = Group::ALL
        .into_iter()
        .map(|g| {
            (
                Tab::Group(g),
                g.label().to_owned(),
                format!("game-settings::tab::{}", g.slug()),
            )
        })
        .collect();
    tabs.push((
        Tab::Packs,
        "Resource Packs".to_owned(),
        "game-settings::tab::packs".to_owned(),
    ));
    tabs.push((
        Tab::Sync,
        "Sync".to_owned(),
        "game-settings::tab::sync".to_owned(),
    ));
    for (tab, label, id) in tabs {
        let kind = if gs.tab == tab {
            ButtonKind::Primary
        } else {
            ButtonKind::Neutral
        };
        if kit.button(ui, &id, &label, kind, true) {
            gs.tab = tab;
            gs.capturing = None;
        }
    }
}

/// Search results when searching, else the selected tab.
fn body(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    gs: &mut GameSettingsState,
    instances: &mut instances::InstanceStore,
    config: &config::Config,
) {
    if !gs.search.trim().is_empty() {
        return rows(ui, kit, gs, Group::Video);
    }
    match gs.tab {
        Tab::Group(group) => rows(ui, kit, gs, group),
        Tab::Packs => packs_tab(ui, kit, gs, instances, config),
        Tab::Sync => sync_tab(
            ui,
            kit,
            instances,
            config,
            gs.session.instance_id().to_owned(),
        ),
    }
}

fn rows(ui: &mut Ui<'_>, kit: &mut Kit, gs: &mut GameSettingsState, group: Group) {
    let searching = !gs.search.trim().is_empty();
    let list = gs.session.rows(group, &gs.search);
    if list.is_empty() && searching {
        kit.text(ui, TextRole::Muted, "No settings match your search.");
        return;
    }
    // Capture one key press per frame, for the bind that is waiting for one.
    let captured = capture(ui, gs);
    for row in list {
        match row {
            Row::Heading(text) => {
                kit.gap(ui, f32::from(kit::SPACE_MD));
                kit.text(ui, TextRole::Heading, &text);
            }
            Row::Subheading(text) => {
                kit.gap(ui, f32::from(kit::SPACE_LG));
                kit.text(ui, TextRole::Heading, &text);
            }
            Row::Setting {
                key,
                label,
                tip,
                control,
            } => setting(
                ui,
                kit,
                gs,
                &key,
                &label,
                tip.as_deref(),
                control,
                captured.as_ref(),
            ),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn setting(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    gs: &mut GameSettingsState,
    key: &str,
    label: &str,
    tip: Option<&str>,
    control: Control,
    captured: Option<&Captured>,
) {
    let id = format!("game-settings::{key}");
    match control {
        Control::Toggle(mut on) => {
            if kit.toggle_setting(ui, &id, label, tip, &mut on) {
                gs.session.set_text(key, &on.to_string());
            }
        }
        Control::Int { value, min, max } => {
            if let Some(next) = stepper(
                ui,
                kit,
                &id,
                label,
                tip,
                f64::from(value),
                f64::from(min),
                f64::from(max),
                1.0,
                |v| format!("{v:.0}"),
            ) {
                gs.session.set_text(key, &(next.round() as i32).to_string());
            }
        }
        Control::Float {
            value,
            min,
            max,
            step,
            percent,
        } => {
            let next = if percent {
                kit.slider_setting(ui, &id, label, tip, f64::from(value), 0.0, 1.0, 0.0, |v| {
                    format!("{:.0}%", v * 100.0)
                })
            } else {
                stepper(
                    ui,
                    kit,
                    &id,
                    label,
                    tip,
                    f64::from(value),
                    f64::from(min),
                    f64::from(max),
                    f64::from(step),
                    |v| format!("{}", (v * 100.0).round() / 100.0),
                )
            };
            if let Some(next) = next {
                gs.session.set_float(key, next as f32);
            }
        }
        Control::Choice {
            labels,
            stored,
            selected,
        } => {
            let mut chosen = None;
            kit.setting_row(ui, &id, label, tip, |kit, ui| {
                chosen = kit.dropdown(ui, &format!("{id}::dropdown"), selected, &labels, 360.0)
            });
            if let Some(value) = chosen.and_then(|index| stored.get(index)) {
                gs.session.set_text(key, value);
            }
        }
        Control::Text(mut text) => {
            if kit.text_setting(ui, &id, label, tip, &mut text, "", false) {
                gs.session.set_text(key, &text);
            }
        }
        Control::List(summary) => kit.text(ui, TextRole::Muted, &format!("{label}: {summary}")),
        Control::Raw(mut text) => {
            if kit.text_setting(ui, &id, label, tip, &mut text, "", false) {
                gs.session.set_raw(key, &text);
            }
        }
        Control::Keybind { shown, conflict } => {
            let capturing = gs.capturing.as_ref().is_some_and(|(k, _)| k == key);
            {
                let _row = kit.row(ui, kit::SPACE_SM);
                kit.text(ui, TextRole::Body, label);
                kit.fill(ui);
                kit.text(ui, TextRole::Strong, &shown);
                if kit.button(
                    ui,
                    &format!("{id}::unbind"),
                    "Unbind",
                    ButtonKind::Neutral,
                    true,
                ) {
                    gs.session.set_keybind(key, "key.keyboard.unknown");
                }
                if kit.button(
                    ui,
                    &format!("{id}::capture"),
                    if capturing {
                        "Press a key…"
                    } else {
                        "Change"
                    },
                    ButtonKind::Neutral,
                    true,
                ) {
                    gs.capturing = (!capturing).then(|| (key.to_owned(), gs.frame));
                }
            }
            if let Some(conflict) = conflict {
                kit::label(
                    ui,
                    &conflict,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Warning)
                    },
                );
            }
            if capturing {
                if let Some(captured) = captured {
                    if let Captured::Binding(name) = captured {
                        gs.session.set_keybind(key, name);
                    }
                    gs.capturing = None;
                }
            }
        }
    }
}

/// A `−  value  +` row. Returns the new value when a button was pressed.
#[allow(clippy::too_many_arguments)]
fn stepper(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    label: &str,
    tip: Option<&str>,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    format: impl Fn(f64) -> String,
) -> Option<f64> {
    let mut next = None;
    kit.setting_row(ui, id, label, tip, |kit, ui| {
        if kit.button(
            ui,
            &format!("{id}::minus"),
            "−",
            ButtonKind::Neutral,
            value > min,
        ) {
            next = Some((value - step).max(min));
        }
        // Typing a number is the precise way: the box takes only what a number can hold, and the value is clamped.
        let shown = format(value);
        let (number, suffix) = shown.split_at(
            shown
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .unwrap_or(shown.len()),
        );
        if let Some(typed) = kit.number_field(
            ui,
            &format!("{id}::value"),
            number,
            84.0,
            step.fract() != 0.0 || number.contains('.'),
            min < 0.0,
        ) {
            next = Some(typed.clamp(min, max));
        }
        if !suffix.trim().is_empty() {
            kit.text(ui, TextRole::Muted, suffix.trim());
        }
        if kit.button(
            ui,
            &format!("{id}::plus"),
            "+",
            ButtonKind::Neutral,
            value < max,
        ) {
            next = Some((value + step).min(max));
        }
    });
    next
}

// -- Key capture ----------------------------------------------------------------------------------------------------

enum Captured {
    Binding(String),
    Cancelled,
}

/// The first key or mouse press this frame while a bind is waiting for one. Escape cancels.
fn capture(ui: &mut Ui<'_>, gs: &GameSettingsState) -> Option<Captured> {
    let (_, started) = gs.capturing.as_ref()?;
    if gs.frame <= *started {
        return None;
    }
    let engine = ui.engine();
    let input = engine.input();
    if input.key_just_pressed(Key::Escape) {
        return Some(Captured::Cancelled);
    }
    for (key, name) in KEY_NAMES {
        if input.key_just_pressed(*key) {
            return Some(Captured::Binding(format!("key.keyboard.{name}")));
        }
    }
    for (button, name) in [
        (MouseButton::Left, "left"),
        (MouseButton::Right, "right"),
        (MouseButton::Middle, "middle"),
        (MouseButton::Extra1, "4"),
        (MouseButton::Extra2, "5"),
    ] {
        if input.mouse_just_pressed(button) {
            return Some(Captured::Binding(format!("key.mouse.{name}")));
        }
    }
    None
}

/// Minecraft 1.13+ names of the keys a bind can use. Modifiers come last so a combination binds its main key.
const KEY_NAMES: &[(Key, &str)] = &[
    (Key::A, "a"),
    (Key::B, "b"),
    (Key::C, "c"),
    (Key::D, "d"),
    (Key::E, "e"),
    (Key::F, "f"),
    (Key::G, "g"),
    (Key::H, "h"),
    (Key::I, "i"),
    (Key::J, "j"),
    (Key::K, "k"),
    (Key::L, "l"),
    (Key::M, "m"),
    (Key::N, "n"),
    (Key::O, "o"),
    (Key::P, "p"),
    (Key::Q, "q"),
    (Key::R, "r"),
    (Key::S, "s"),
    (Key::T, "t"),
    (Key::U, "u"),
    (Key::V, "v"),
    (Key::W, "w"),
    (Key::X, "x"),
    (Key::Y, "y"),
    (Key::Z, "z"),
    (Key::Digit0, "0"),
    (Key::Digit1, "1"),
    (Key::Digit2, "2"),
    (Key::Digit3, "3"),
    (Key::Digit4, "4"),
    (Key::Digit5, "5"),
    (Key::Digit6, "6"),
    (Key::Digit7, "7"),
    (Key::Digit8, "8"),
    (Key::Digit9, "9"),
    (Key::F1, "f1"),
    (Key::F2, "f2"),
    (Key::F3, "f3"),
    (Key::F4, "f4"),
    (Key::F5, "f5"),
    (Key::F6, "f6"),
    (Key::F7, "f7"),
    (Key::F8, "f8"),
    (Key::F9, "f9"),
    (Key::F10, "f10"),
    (Key::F11, "f11"),
    (Key::F12, "f12"),
    (Key::F13, "f13"),
    (Key::F14, "f14"),
    (Key::F15, "f15"),
    (Key::F16, "f16"),
    (Key::F17, "f17"),
    (Key::F18, "f18"),
    (Key::F19, "f19"),
    (Key::F20, "f20"),
    (Key::F21, "f21"),
    (Key::F22, "f22"),
    (Key::F23, "f23"),
    (Key::F24, "f24"),
    (Key::Up, "up"),
    (Key::Down, "down"),
    (Key::Left, "left"),
    (Key::Right, "right"),
    (Key::Space, "space"),
    (Key::Enter, "enter"),
    (Key::Tab, "tab"),
    (Key::Backspace, "backspace"),
    (Key::Insert, "insert"),
    (Key::Delete, "delete"),
    (Key::Home, "home"),
    (Key::End, "end"),
    (Key::PageUp, "page.up"),
    (Key::PageDown, "page.down"),
    (Key::Minus, "minus"),
    (Key::Equal, "equal"),
    (Key::LeftBracket, "left.bracket"),
    (Key::RightBracket, "right.bracket"),
    (Key::Semicolon, "semicolon"),
    (Key::Apostrophe, "apostrophe"),
    (Key::Comma, "comma"),
    (Key::Period, "period"),
    (Key::Slash, "slash"),
    (Key::Backslash, "backslash"),
    (Key::GraveAccent, "grave.accent"),
    (Key::Numpad0, "keypad.0"),
    (Key::Numpad1, "keypad.1"),
    (Key::Numpad2, "keypad.2"),
    (Key::Numpad3, "keypad.3"),
    (Key::Numpad4, "keypad.4"),
    (Key::Numpad5, "keypad.5"),
    (Key::Numpad6, "keypad.6"),
    (Key::Numpad7, "keypad.7"),
    (Key::Numpad8, "keypad.8"),
    (Key::Numpad9, "keypad.9"),
    (Key::NumpadDecimal, "keypad.decimal"),
    (Key::NumpadDivide, "keypad.divide"),
    (Key::NumpadMultiply, "keypad.multiply"),
    (Key::NumpadSubtract, "keypad.subtract"),
    (Key::NumpadAdd, "keypad.add"),
    (Key::NumpadEnter, "keypad.enter"),
    (Key::NumpadEqual, "keypad.equal"),
    (Key::CapsLock, "caps.lock"),
    (Key::ScrollLock, "scroll.lock"),
    (Key::NumLock, "num.lock"),
    (Key::PrintScreen, "print.screen"),
    (Key::Pause, "pause"),
    (Key::Menu, "menu"),
    (Key::LeftShift, "left.shift"),
    (Key::RightShift, "right.shift"),
    (Key::LeftControl, "left.control"),
    (Key::RightControl, "right.control"),
    (Key::LeftAlt, "left.alt"),
    (Key::RightAlt, "right.alt"),
    (Key::LeftSuper, "left.win"),
    (Key::RightSuper, "right.win"),
];

// -- Resource packs -------------------------------------------------------------------------------------------------

fn packs_tab(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    gs: &mut GameSettingsState,
    instances: &mut instances::InstanceStore,
    config: &config::Config,
) {
    kit::label(
        ui,
        "Each pack declares which Minecraft versions it supports. You know your setup better than the metadata: mark a pack as working (or not) for this instance and sync will follow that instead. Overrides only affect sync; they don't change the pack or what the game allows.",
        &TextStyle {
            wrap: TextWrap::Words,
            ..kit.style(TextRole::Muted)
        },
    );
    if !instances.game_settings_sync.resource_packs {
        kit::label(
            ui,
            "Resource pack sync is off. Turn it on in the Sync tab; overrides are remembered either way.",
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Warning)
            },
        );
    }
    kit.gap(ui, f32::from(kit::SPACE_MD));
    let packs = gs.session.packs(instances, config);
    if packs.is_empty() {
        kit.text(ui, TextRole::Muted, "No resource packs found.");
        return;
    }
    let mut change = None;
    for pack in &packs {
        kit.text(ui, TextRole::Strong, &pack.name);
        kit::label(
            ui,
            &pack.detail,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
        {
            let _row = kit.row(ui, kit::SPACE_SM);
            for (label, target) in [
                ("Works here", Some(PackOverride::Allow)),
                ("Doesn't work here", Some(PackOverride::Deny)),
                ("Use pack's metadata", None),
            ] {
                let selected = pack.verdict == target;
                let kind = if selected {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Neutral
                };
                if kit.button(
                    ui,
                    &format!("game-settings::pack::{}::{label}", pack.name),
                    label,
                    kind,
                    true,
                ) && !selected
                {
                    change = Some((pack.name.clone(), target));
                }
            }
        }
        kit.gap(ui, f32::from(kit::SPACE_SM));
    }
    if let Some((name, target)) = change {
        gs.session
            .set_pack_override(instances, config, &name, target);
    }
}

// -- Sync -----------------------------------------------------------------------------------------------------------

fn sync_tab(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    instances: &mut instances::InstanceStore,
    config: &config::Config,
    id: String,
) {
    kit::label(
        ui,
        "Keep these settings in step across instances. The most recently changed instance wins; its settings are translated to each other instance's Minecraft version (renamed options, key codes, value formats). Settings that belong to this machine, like the window size or audio device, are never copied, and running instances are left alone until they close.",
        &TextStyle {
            wrap: TextWrap::Words,
            ..kit.style(TextRole::Muted)
        },
    );
    kit.gap(ui, f32::from(kit::SPACE_MD));
    let mut member = instances.game_settings_sync.instances.contains(&id);
    if kit.toggle_setting(
        ui,
        "game-settings::sync::member",
        "Sync this instance's settings",
        Some("Instances with this turned on share the settings chosen below."),
        &mut member,
    ) {
        if member {
            instances.game_settings_sync.instances.insert(id.clone());
        } else {
            instances.game_settings_sync.instances.remove(&id);
        }
    }
    let participants = instances.game_settings_sync.instances.len();
    kit.text(
        ui,
        TextRole::Muted,
        &if participants < 2 {
            "Turn this on for at least two instances for anything to sync.".to_owned()
        } else {
            format!("{participants} instances take part.")
        },
    );

    kit.gap(ui, f32::from(kit::SPACE_MD));
    kit.text(ui, TextRole::Heading, "Settings to sync");
    for group in api::syncable_groups() {
        let mut on = instances
            .game_settings_sync
            .categories
            .contains(group.slug());
        if kit.toggle_setting(
            ui,
            &format!("game-settings::sync::group::{}", group.slug()),
            group.label(),
            Some("Keep this group of game options identical across your instances."),
            &mut on,
        ) {
            if on {
                instances
                    .game_settings_sync
                    .categories
                    .insert(group.slug().to_owned());
            } else {
                instances.game_settings_sync.categories.remove(group.slug());
            }
        }
    }

    kit.gap(ui, f32::from(kit::SPACE_MD));
    kit.text(ui, TextRole::Heading, "Mod settings to sync");
    kit::label(
        ui,
        "Only instances that already have the mod's config file take part. Thread counts, server identity and other machine-specific keys are skipped.",
        &TextStyle {
            wrap: TextWrap::Words,
            ..kit.style(TextRole::Muted)
        },
    );
    for (mod_id, name) in api::available_mod_configs() {
        let mut on = instances.game_settings_sync.mod_configs.contains(mod_id);
        if kit.toggle_setting(
            ui,
            &format!("game-settings::sync::mod::{mod_id}"),
            name,
            Some("Keep this mod's options identical across your instances."),
            &mut on,
        ) {
            if on {
                instances
                    .game_settings_sync
                    .mod_configs
                    .insert(mod_id.to_owned());
            } else {
                instances.game_settings_sync.mod_configs.remove(mod_id);
            }
        }
    }

    kit.gap(ui, f32::from(kit::SPACE_MD));
    let mut packs = instances.game_settings_sync.resource_packs;
    if kit.toggle_setting(
        ui,
        "game-settings::sync::packs",
        "Sync resource packs",
        Some("Shares packs between these instances where their metadata says they work (or you've said they do), and keeps which packs are enabled in step. Manage per-pack exceptions on the Resource Packs tab."),
        &mut packs,
    ) {
        instances.game_settings_sync.resource_packs = packs;
    }
    kit.gap(ui, f32::from(kit::SPACE_MD));
    if kit.button(
        ui,
        "game-settings::sync::now",
        "Sync now",
        ButtonKind::Neutral,
        true,
    ) {
        api::sync_now(instances, config);
    }
}
