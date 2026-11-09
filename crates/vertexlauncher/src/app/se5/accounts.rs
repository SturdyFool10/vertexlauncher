//! Microsoft account UI: the title-bar account menu and the sign-in dialog. The flows themselves are `AuthState`'s,
//! shared with the egui front end.

use vertex_shell::sturdy::ui::{Sizing, TextStyle, TextWrap, Ui};
use vertex_shell::sturdy::ui_kit::{self, ModalStyle, PopupStyle};

use launcher_ui::account_colors::{self, PaletteColor};
use launcher_ui::account_text::mention;
use launcher_ui::geom::Color32;
use vertex_shell::sturdy::glam::{UVec2, Vec4};
use vertex_shell::sturdy::ui::{
    Border, ChildAlignment, CornerRadius, Direction, ElementDesc,
};

use super::AppState;
use super::icons::Icon;
use super::images::ImageSource;
use super::kit::{self, ButtonKind, Kit, TextRole};

const ACCOUNT_MENU_ID: &str = "accounts::menu";

/// What the title-bar account button says. Streamer mode hides the account name.
pub fn active_account_label(state: &AppState) -> String {
    if state.auth.sign_in_in_progress() {
        return "Signing in…".to_owned();
    }
    match state.auth.display_name() {
        Some(_) if state.streamer_mode() => "Signed in".to_owned(),
        Some(name) => name.to_owned(),
        None => "Sign in".to_owned(),
    }
}

/// The colour editor and the like that live inside the account menu.
#[derive(Default)]
pub struct AccountsView {
    /// The account whose colour editor is open.
    editing: Option<String>,
    /// The custom colour being typed.
    input: String,
    invalid: bool,
}

fn vec4(color: Color32) -> Vec4 {
    Vec4::new(
        f32::from(color.r()) / 255.0,
        f32::from(color.g()) / 255.0,
        f32::from(color.b()) / 255.0,
        1.0,
    )
}

/// What the account menu was asked to do this frame.
#[derive(Default)]
struct MenuActions {
    select: Option<String>,
    remove: Option<String>,
    refresh: Option<String>,
    webview: bool,
    browser: bool,
    device_code: bool,
    cancel_device_code: bool,
}

fn pending_auth_label(state: &AppState) -> &'static str {
    if state.auth.token_refresh_in_progress() {
        "Refreshing session..."
    } else if state.auth.sign_in_in_progress() {
        "Signing in with Microsoft..."
    } else {
        "Authenticating..."
    }
}

/// The popup under the account button: who is signed in, the saved accounts (select, refresh, remove, colour), and
/// the ways to sign in.
pub fn account_menu(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, anchor_id: &str) {
    let style = PopupStyle {
        background: kit.palette.bg_dark,
        border: kit.palette.border_muted,
        corner_radius: 12.0,
        padding: vertex_shell::sturdy::ui::Padding::all(kit::SPACE_MD),
        z_index: kit::Z_POPUP,
        ..PopupStyle::default()
    };
    let entries = state.auth.account_entries();
    let streamer = state.streamer_mode();
    let prompt = state.auth.device_code_prompt().cloned();
    let status = state.auth.status_message().map(str::to_owned);
    let busy = state.auth.auth_busy();
    let pending = pending_auth_label(state);
    let name = state.auth.display_name().map(str::to_owned);
    let mut actions = MenuActions::default();
    let mut view = std::mem::take(&mut state.accounts_view);
    let mut menu = std::mem::take(&mut state.account_menu);
    ui_kit::popup(
        ui,
        ACCOUNT_MENU_ID,
        anchor_id,
        &mut menu,
        &style,
        |ui, _popup| {
            let _column = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fit)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_SM),
            );
            // The menu is at least this wide and grows to fit longer names and buttons.
            drop(
                ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Fixed(340.0))
                        .height(Sizing::Fixed(0.0)),
                ),
            );
            // Header.
            match &name {
                Some(name) => {
                    let active_id = entries
                        .iter()
                        .find(|entry| entry.is_active)
                        .map_or("", |entry| entry.profile_id.as_str());
                    kit.account_label(
                        ui,
                        TextRole::Strong,
                        &format!("Signed in as {}", mention(active_id, name)),
                        streamer,
                    );
                }
                None => kit.text(ui, TextRole::Strong, "No Microsoft account signed in"),
            }
            if prompt.is_none()
                && let Some(status) = &status
            {
                kit::label(
                    ui,
                    status,
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Muted)
                    },
                );
            }
            if let Some(prompt) = &prompt {
                device_code_section(ui, kit, prompt, &mut actions);
                return;
            }
            if !entries.is_empty() {
                kit.text(
                    ui,
                    TextRole::Muted,
                    "Saved accounts (right-click to set a color)",
                );
                let _list = kit.card(ui, "accounts::list", kit::SPACE_XS);
                for (index, entry) in entries.iter().enumerate() {
                    let shown = if streamer {
                        format!("Account {}", index + 1)
                    } else {
                        entry.display_name.clone()
                    };
                    let label = if entry.is_active && entry.is_failed {
                        format!("{shown} (Failed)")
                    } else if entry.is_active {
                        format!("{shown} (Active)")
                    } else {
                        shown
                    };
                    let row_id = format!("{ACCOUNT_MENU_ID}::account::{}", entry.profile_id);
                    let tint = streamer
                        .then(|| account_colors::color_for(&entry.profile_id))
                        .flatten();
                    let mut open_editor = false;
                    {
                        let _row = kit.row(ui, kit::SPACE_XS);
                        // The account's head, drawn by the engine from its skin (nothing in streamer mode).
                        if !streamer
                            && let Some(texture) = entry.skin_url.as_deref().and_then(|url| {
                                kit.heads.head(&mut ui.engine(), url, UVec2::new(88, 88))
                            })
                        {
                            ui.image(
                                &ElementDesc::new()
                                    .id(format!("{ACCOUNT_MENU_ID}::head::{}", entry.profile_id))
                                    .width(Sizing::Fixed(24.0))
                                    .height(Sizing::Fixed(24.0))
                                    .corner_radius(CornerRadius::all(5.0)),
                                texture,
                            );
                        }
                        if let Some(color) = tint {
                            // A coloured row: the account's colour behind its name, readable on top.
                            let ink = vec4(account_colors::contrasting_color(color));
                            let hovered = ui.hovered(&row_id);
                            let _bg = ui.element(
                                &ElementDesc::new()
                                    .id(row_id.clone())
                                    .width(Sizing::Grow)
                                    .height(Sizing::Fixed(kit::CONTROL_HEIGHT))
                                    .padding(vertex_shell::sturdy::ui::Padding::symmetric(
                                        kit::SPACE_MD,
                                        0,
                                    ))
                                    .align(ChildAlignment::new(
                                        vertex_shell::sturdy::ui::AlignX::Left,
                                        vertex_shell::sturdy::ui::AlignY::Center,
                                    ))
                                    .background(vec4(color) * if hovered { 0.9 } else { 1.0 })
                                    .corner_radius(CornerRadius::all(10.0))
                                    .border(if entry.is_active {
                                        Border::all(ink, 2)
                                    } else {
                                        Border::default()
                                    }),
                            );
                            kit.text_colored(ui, &label, ink, 1.0);
                        } else if kit.nav_item_ex(
                            ui,
                            &row_id,
                            Some(Icon::User),
                            &label,
                            entry.is_active,
                            false,
                        ) {
                            actions.select = Some(entry.profile_id.clone());
                        }
                        if tint.is_some() && kit.clicked(ui, &row_id) {
                            actions.select = Some(entry.profile_id.clone());
                        }
                        if kit.secondary(ui, &row_id).is_some() {
                            open_editor = true;
                        }
                        if entry.is_failed
                            && kit.button(
                                ui,
                                &format!("{ACCOUNT_MENU_ID}::refresh::{}", entry.profile_id),
                                "Refresh",
                                ButtonKind::Neutral,
                                !busy,
                            )
                        {
                            actions.refresh = Some(entry.profile_id.clone());
                        }
                        if kit.button(
                            ui,
                            &format!("{ACCOUNT_MENU_ID}::remove::{}", entry.profile_id),
                            "Remove",
                            ButtonKind::Neutral,
                            true,
                        ) {
                            actions.remove = Some(entry.profile_id.clone());
                        }
                    }
                    if open_editor {
                        view.editing = if view.editing.as_deref() == Some(entry.profile_id.as_str())
                        {
                            None
                        } else {
                            Some(entry.profile_id.clone())
                        };
                        view.input.clear();
                        view.invalid = false;
                    }
                    if view.editing.as_deref() == Some(entry.profile_id.as_str())
                        && color_editor(
                            ui,
                            kit,
                            &entry.profile_id,
                            &entry.display_name,
                            streamer,
                            &mut view,
                        )
                    {
                        view.editing = None;
                    }
                }
            }
            kit.separator(ui);
            if busy {
                kit.button_sized(
                    ui,
                    &format!("{ACCOUNT_MENU_ID}::pending"),
                    pending,
                    ButtonKind::Neutral,
                    false,
                    Sizing::Grow,
                );
            } else {
                let _row = kit.row(ui, kit::SPACE_SM);
                if kit.button_sized(
                    ui,
                    &format!("{ACCOUNT_MENU_ID}::browser"),
                    "Browser Sign-in",
                    ButtonKind::Primary,
                    true,
                    Sizing::Grow,
                ) {
                    actions.browser = true;
                }
                if kit.button_sized(
                    ui,
                    &format!("{ACCOUNT_MENU_ID}::webview"),
                    "Webview Sign-in",
                    ButtonKind::Neutral,
                    true,
                    Sizing::Grow,
                ) {
                    actions.webview = true;
                }
                if kit.button_sized(
                    ui,
                    &format!("{ACCOUNT_MENU_ID}::device"),
                    "Device Code Sign-in",
                    ButtonKind::Neutral,
                    true,
                    Sizing::Grow,
                ) {
                    actions.device_code = true;
                }
            }
        },
    );
    state.account_menu = menu;
    state.accounts_view = view;
    apply_actions(state, actions);
}

fn apply_actions(state: &mut AppState, actions: MenuActions) {
    if let Some(profile_id) = actions.select {
        state.auth.select_account(&profile_id);
    }
    if let Some(profile_id) = actions.remove {
        state.auth.remove_account(&profile_id);
    }
    if let Some(profile_id) = actions.refresh {
        state.auth.refresh_account_token(&profile_id);
    }
    if actions.webview {
        state.auth.start_webview_sign_in();
    }
    if actions.browser {
        let theme = state.theme.clone();
        state.auth.start_system_browser_sign_in(&theme);
    }
    if actions.device_code {
        state.auth.start_device_code_sign_in();
    }
    if actions.cancel_device_code {
        state.auth.cancel_device_code_sign_in();
    }
}

/// The device-code step: a QR code for the verification page, the code with a copy button, and the way out to the
/// browser flow. Draws no account names, so it is exempt from name scrubbing.
fn device_code_section(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    prompt: &auth::DeviceCodePrompt,
    actions: &mut MenuActions,
) {
    launcher_ui::privacy::without_name_scrubbing(|| {
        kit::label(
            ui,
            "Scan this QR code to continue with device code on another device, or use browser sign-in on this device to avoid typing the code manually.",
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Body)
            },
        );
        let url = prompt.verification_url();
        if let Some(png) = launcher_ui::qr::render_png(&url) {
            let png: std::sync::Arc<[u8]> = png.into();
            let _center = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .align(ChildAlignment::center()),
            );
            kit.image(
                ui,
                "accounts::qr",
                &format!("qr::{url}"),
                || ImageSource::Bytes(png),
                200.0,
                200.0,
                8.0,
            );
        }
        {
            let _row = kit.row(ui, kit::SPACE_SM);
            kit.fill(ui);
            kit.selectable_text(
                ui,
                "accounts::user-code",
                &prompt.user_code,
                Sizing::Fixed(220.0),
            );
            if kit.button(
                ui,
                "accounts::copy-user-code",
                "Copy code",
                ButtonKind::Neutral,
                true,
            ) {
                launcher_ui::app_commands::push(launcher_ui::app_commands::AppCommand::CopyText(
                    prompt.user_code.clone(),
                ));
            }
            kit.fill(ui);
        }
        let _row = kit.row(ui, kit::SPACE_SM);
        if kit.button_sized(
            ui,
            "accounts::device-cancel",
            "Cancel",
            ButtonKind::Neutral,
            true,
            Sizing::Grow,
        ) {
            actions.cancel_device_code = true;
        }
        if kit.button_sized(
            ui,
            "accounts::device-browser",
            "Use Browser Sign-in",
            ButtonKind::Primary,
            true,
            Sizing::Grow,
        ) {
            actions.browser = true;
        }
    });
}

/// The colour editor under an account row. Returns true when it should close.
fn color_editor(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    account_id: &str,
    account_name: &str,
    streamer: bool,
    view: &mut AccountsView,
) -> bool {
    let mut done = false;
    let current = account_colors::color_for(account_id);
    let _editor = kit.column(ui, kit::SPACE_XS);
    kit.account_label(
        ui,
        TextRole::Small,
        &format!("Color for {}", mention(account_id, account_name)),
        streamer,
    );
    for (name, colors) in [
        ("Primary", &account_colors::PRIMARY_COLORS[..]),
        ("Secondary", &account_colors::SECONDARY_COLORS[..]),
        ("Tertiary", &account_colors::TERTIARY_COLORS[..]),
    ] {
        kit.text(ui, TextRole::Small, name);
        let _row = kit.row(ui, kit::SPACE_XS);
        for PaletteColor { name, color } in colors {
            let id = format!("accounts::swatch::{account_id}::{name}");
            if swatch(ui, kit, &id, Some(*color), current == Some(*color)) {
                account_colors::set_color(account_id, Some(*color));
            }
        }
    }
    kit.text(ui, TextRole::Small, "Custom");
    {
        let _row = kit.row(ui, kit::SPACE_XS);
        kit.text_field(
            ui,
            &format!("accounts::custom-color::{account_id}"),
            &mut view.input,
            "#rrggbb",
            false,
            Sizing::Fixed(150.0),
        );
        if kit.button(
            ui,
            &format!("accounts::custom-set::{account_id}"),
            "Set",
            ButtonKind::Neutral,
            true,
        ) {
            match account_colors::parse_color(&view.input) {
                Some(color) => {
                    account_colors::set_color(account_id, Some(color));
                    view.invalid = false;
                }
                None => view.invalid = true,
            }
        }
    }
    if view.invalid {
        kit.text(ui, TextRole::Danger, "Use #rrggbb or rgb(r, g, b)");
    }
    let _row = kit.row(ui, kit::SPACE_XS);
    if kit.button(
        ui,
        &format!("accounts::clear-color::{account_id}"),
        "Clear color",
        ButtonKind::Neutral,
        current.is_some(),
    ) {
        account_colors::set_color(account_id, None);
    }
    if kit.button(
        ui,
        &format!("accounts::color-done::{account_id}"),
        "Done",
        ButtonKind::Primary,
        true,
    ) {
        done = true;
    }
    done
}

/// A clickable colour square; `None` is the "no colour" box. A dot marks the colour in use.
fn swatch(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    color: Option<Color32>,
    selected: bool,
) -> bool {
    {
        let fill = color.map_or(kit.palette.bg_light, vec4);
        let _box = ui.element(
            &ElementDesc::new()
                .id(id.into())
                .width(Sizing::Fixed(24.0))
                .height(Sizing::Fixed(24.0))
                .align(ChildAlignment::center())
                .background(fill)
                .corner_radius(CornerRadius::all(4.0))
                .border(Border::all(kit.palette.border_muted, 1)),
        );
        if selected {
            let ink = color.map_or(kit.palette.text, |color| {
                vec4(account_colors::contrasting_color(color))
            });
            let _dot = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(8.0))
                    .height(Sizing::Fixed(8.0))
                    .background(ink)
                    .corner_radius(CornerRadius::all(4.0)),
            );
        }
    }
    kit.clicked(ui, id)
}

/// The sign-in dialog. Returns false once it should close.
pub fn sign_in_dialog(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) -> bool {
    let style = ModalStyle {
        background: kit.palette.bg_dark,
        corner_radius: kit::CORNER,
        width: Sizing::Fixed(540.0),
        ..ModalStyle::default()
    };
    let mut keep_open = true;
    let in_progress = state.auth.sign_in_in_progress();
    let prompt = state.auth.device_code_prompt().cloned();
    let status = state.auth.status_message().map(str::to_owned);
    let mut actions = MenuActions::default();
    let mut close = false;
    let response = kit::modal(ui, "accounts::sign-in", &style, !in_progress, |ui| {
        let _column = kit.column(ui, kit::SPACE_MD);
        kit.text(ui, TextRole::Heading, "Add a Microsoft account");
        match &prompt {
            Some(prompt) => device_code_section(ui, kit, prompt, &mut actions),
            None if in_progress => {
                kit.text(ui, TextRole::Body, "Finish signing in in your browser…");
                kit.progress(ui, "accounts::progress", None, kit::uptime());
            }
            None => {
                kit::label(
                    ui,
                    "Sign in with the Microsoft account that owns Minecraft: Java Edition.",
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Muted)
                    },
                );
                let _row = kit.row(ui, kit::SPACE_SM);
                if kit.button(
                    ui,
                    "accounts::browser",
                    "Browser Sign-in",
                    ButtonKind::Primary,
                    true,
                ) {
                    actions.browser = true;
                }
                if kit.button(
                    ui,
                    "accounts::webview",
                    "Webview Sign-in",
                    ButtonKind::Neutral,
                    true,
                ) {
                    actions.webview = true;
                }
                if kit.button(
                    ui,
                    "accounts::device",
                    "Device Code Sign-in",
                    ButtonKind::Neutral,
                    true,
                ) {
                    actions.device_code = true;
                }
                kit.fill(ui);
                if kit.button(ui, "accounts::close", "Close", ButtonKind::Neutral, true) {
                    close = true;
                }
            }
        }
        if let Some(status) = &status {
            kit::label(
                ui,
                status,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Small)
                },
            );
        }
    });
    apply_actions(state, actions);
    if close {
        keep_open = false;
    }
    // Close once a flow that ran while the dialog was open finishes with an account signed in.
    let finished = state.sign_in_running && !in_progress && state.auth.display_name().is_some();
    state.sign_in_running = in_progress || state.auth.sign_in_in_progress();
    if (response.dismissed || kit.back_pressed()) || finished {
        keep_open = false;
        state.sign_in_running = false;
    }
    keep_open
}
