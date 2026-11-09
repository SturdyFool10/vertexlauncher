//! Window chrome: the custom title bar (window controls, account menu), the sidebar (screens and instance
//! shortcuts) and the content area that hosts the active screen.

use instances::InstanceRecord;
use launcher_ui::screens::AppScreen;
use vertex_shell::sturdy::glam::{UVec2, Vec4};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, CornerRadius, Direction, ElementDesc,
    FloatTarget, Floating, Padding, Sizing, Ui,
};
use vertex_shell::sturdy::ui_kit::{self, PopupStyle};

use super::icons::Icon;
use super::images::ImageSource;
use super::kit::{self, Kit, MenuEntry, TextRole};
use super::{AppState, Dialog, accounts, screens};

pub const TITLE_BAR_ID: &str = "chrome::title-bar";
const MINIMIZE_ID: &str = "chrome::minimize";
const MAXIMIZE_ID: &str = "chrome::maximize";
const FULLSCREEN_ID: &str = "chrome::fullscreen";
const CLOSE_ID: &str = "chrome::close";
const ACCOUNT_ID: &str = "chrome::account";
/// Title-bar elements that must stay clickable instead of dragging the window.
pub const TITLE_BAR_CONTROLS: [&str; 6] = [
    FULLSCREEN_ID,
    MINIMIZE_ID,
    MAXIMIZE_ID,
    CLOSE_ID,
    ACCOUNT_ID,
    ACTIVE_USER_ID,
];
const TITLE_BAR_HEIGHT: f32 = 42.0;
/// Window-control icon size. A multiple of 4, so the rasterised texture is exactly the drawn size at 1x (the sidebar's
/// 22 px icons are scaled from a 24 px raster; the old 14 px here came from a 16 px one).
const WINDOW_ICON_SIZE: f32 = 16.0;
pub const SIDEBAR_WIDTH: f32 = 68.0;

/// The height below the title bar, inside the window's border: what the sidebar and the page area have.
fn body_height(ui: &Ui<'_>, border: f32) -> f32 {
    (ui.viewport_size().y - TITLE_BAR_HEIGHT - 2.0 * border).max(0.0)
}

/// Goes fullscreen in the chosen style, or back to a window when already fullscreen.
pub fn toggle_fullscreen(style: config::FullscreenStyle) {
    use vertex_shell::Fullscreen;
    if vertex_shell::fullscreen() != Fullscreen::Off {
        vertex_shell::request_fullscreen(Fullscreen::Off);
        return;
    }
    enter_fullscreen(style);
    // Fullscreen hides the window's controls, so say how to leave it. (Not when it was the "start in fullscreen"
    // setting that did it: that is a choice made in advance.)
    launcher_ui::notification::info!(
        "fullscreen",
        "Vertex is fullscreen. Press Escape (or F11) to leave fullscreen."
    );
}

/// Takes the window fullscreen in `style`.
pub fn enter_fullscreen(style: config::FullscreenStyle) {
    use vertex_shell::Fullscreen;
    vertex_shell::request_fullscreen(match style {
        config::FullscreenStyle::Borderless => Fullscreen::Borderless,
        config::FullscreenStyle::Exclusive => Fullscreen::Exclusive,
    });
}

/// The whole window: title bar on top, sidebar and the active screen below.
pub fn window(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, maximized: bool) {
    // Rounded corners need a see-through window; an opaque one would show square corners behind them.
    // A fullscreen window fills the screen exactly, like a maximized one: no rounded corners, no border.
    let maximized_or_fullscreen = maximized || vertex_shell::fullscreen() != vertex_shell::Fullscreen::Off;
    let rounded = !maximized_or_fullscreen
        && (state.config.window_transparency().is_translucent()
            || state.config.window_corner_radius() > 0);
    let radius = if rounded {
        f32::from(state.config.window_corner_radius())
    } else {
        0.0
    };
    // The window's border is the same 1 px on every side. It is drawn over the edge of the root, so everything inside
    // is inset by it, and the widths below, which come from the window's width, leave room for it on both sides.
    let border = if maximized_or_fullscreen { 0.0 } else { 1.0 };
    let _root = ui.element(
        &ElementDesc::new()
            .id("chrome::root".into())
            .width(Sizing::Grow)
            .height(Sizing::Grow)
            .direction(Direction::TopToBottom)
            // Inside the border, which is drawn over the element's edge, so children (the close button's hover fill)
            // cannot cover it.
            .padding(Padding::all(border as u16))
            .background(kit.palette.bg)
            .corner_radius(CornerRadius::all(radius))
            .border(Border::all(kit.palette.border_muted, border as u16)),
    );
    title_bar(ui, kit, state, maximized, radius);
    // Fixed to the window's width rather than growing: a fixed size caps the minimum size Clay passes up, so a screen
    // with something too wide cannot stretch the layout (and the title bar's buttons) past the window edge. (Clipping
    // would do that too, but a clipped axis is a scroll container, which stops squeezing its children at all.)
    // The height is fixed to the window's too, for the same reason as the width: a page that is taller than its space
    // (a long list of mods) must scroll inside it, never stretch the layout, and with it the page's scroll area, past
    // the bottom of the window.
    let _body = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fixed((ui.viewport_size().x - 2.0 * border).max(0.0)))
            .height(Sizing::Fixed(body_height(ui, border)))
            .direction(Direction::LeftToRight),
    );
    sidebar(ui, kit, state, radius);
    content(ui, kit, state, border);
}

fn title_bar(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, maximized: bool, radius: f32) {
    {
        let _bar = ui.element(
            &ElementDesc::new()
                .id(TITLE_BAR_ID.into())
                .width(Sizing::Grow)
                .height(Sizing::Fixed(TITLE_BAR_HEIGHT))
                .padding(Padding {
                    left: kit::SPACE_LG,
                    ..Padding::default()
                })
                .child_gap(kit::SPACE_SM)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                .direction(Direction::LeftToRight)
                .background(kit.palette.bg_dark.with_w(kit.palette.bg.w))
                .corner_radius(CornerRadius {
                    top_left: radius,
                    top_right: radius,
                    bottom_left: 0.0,
                    bottom_right: 0.0,
                }),
        );
        kit.text(ui, TextRole::Body, "Vertex Launcher");
        if let Some(instance) = state
            .screen
            .eq(&AppScreen::Instance)
            .then(|| selected_name(state))
            .flatten()
        {
            kit.text(ui, TextRole::Muted, &format!("· {instance}"));
        }
        kit.fill(ui);
        active_user_button(ui, kit, state);
        account_button(ui, kit, state);
        // Fullscreen, the window's own controls are hidden (F11 leaves fullscreen).
        if vertex_shell::fullscreen() == vertex_shell::Fullscreen::Off {
            window_button(ui, kit, FULLSCREEN_ID, Some(Icon::ArrowsMaximize), false, 0.0);
            window_button(ui, kit, MINIMIZE_ID, Some(Icon::Minus), false, 0.0);
            window_button(
                ui,
                kit,
                MAXIMIZE_ID,
                maximized.then_some(Icon::Restore),
                false,
                0.0,
            );
            window_button(ui, kit, CLOSE_ID, Some(Icon::Close), true, radius);
        }
    }
    // The controls only exist outside fullscreen; asking about them there would put invisible buttons in the keyboard
    // focus order.
    let controls_shown = vertex_shell::fullscreen() == vertex_shell::Fullscreen::Off;
    if controls_shown && kit.clicked_with_default_cursor(ui, FULLSCREEN_ID) {
        toggle_fullscreen(state.config.fullscreen_style());
    }
    for id in [MINIMIZE_ID, MAXIMIZE_ID, CLOSE_ID] {
        if !controls_shown || !kit.clicked_with_default_cursor(ui, id) {
            continue;
        }
        let mut engine = ui.engine();
        if let Some(mut window) = engine.primary_window_control() {
            match id {
                MINIMIZE_ID => {
                    let _ = window.minimize();
                }
                MAXIMIZE_ID => {
                    let _ = window.toggle_maximized();
                }
                _ => window.request_close(),
            }
        }
    }
    kit.tooltip(
        ui,
        FULLSCREEN_ID,
        if vertex_shell::fullscreen() == vertex_shell::Fullscreen::Off {
            "Fullscreen (F11)"
        } else {
            "Exit fullscreen (F11)"
        },
    );
    kit.tooltip(ui, MINIMIZE_ID, "Minimize");
    kit.tooltip(
        ui,
        MAXIMIZE_ID,
        if maximized {
            "Restore down"
        } else {
            "Maximize"
        },
    );
    kit.tooltip(ui, CLOSE_ID, "Close");
    kit.tooltip(ui, ACCOUNT_ID, "Accounts and sign-in");
    accounts::account_menu(ui, kit, state, ACCOUNT_ID);
}

const ACTIVE_USER_ID: &str = "chrome::active-user";

/// While the signed-in account is playing, a button that opens that game's console.
fn active_user_button(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let Some(account) = state
        .auth
        .active_launch_context()
        .map(|context| context.account_key)
    else {
        return;
    };
    let Some(instance_root) = installation::running_instance_for_account(&account) else {
        return;
    };
    let hovered = ui.hovered(ACTIVE_USER_ID);
    {
        let _button = ui.element(
            &ElementDesc::new()
                .id(ACTIVE_USER_ID.into())
                .width(Sizing::Fit)
                .height(Sizing::Fixed(30.0))
                .padding(Padding::symmetric(kit::SPACE_MD, 0))
                .child_gap(kit::SPACE_SM)
                .align(ChildAlignment::center())
                .background(if hovered {
                    kit.palette.hovered_fill()
                } else {
                    kit.palette.bg_light
                })
                .corner_radius(CornerRadius::all(8.0)),
        );
        kit.icon(ui, Icon::Terminal2, 16.0, kit.palette.text);
        launcher_ui::privacy::without_name_scrubbing(|| {
            kit.text(ui, TextRole::Small, "player active")
        });
    }
    kit.tooltip(
        ui,
        ACTIVE_USER_ID,
        "Open the console of the game you are playing",
    );
    if kit.clicked(ui, ACTIVE_USER_ID) {
        state.open(AppScreen::Console);
        if !launcher_ui::console::activate_tab_for_instance_root(&instance_root) {
            let name = state.auth.display_name().map(str::to_owned);
            let _ = launcher_ui::console::activate_tab_for_user(Some(&account), name.as_deref());
        }
    }
}

fn screen_icon(screen: AppScreen) -> Icon {
    match screen {
        AppScreen::Home => Icon::Home,
        AppScreen::Library | AppScreen::Instance => Icon::Library,
        AppScreen::Discover | AppScreen::DiscoverDetail | AppScreen::ContentBrowser => {
            Icon::Discover
        }
        AppScreen::Skins => Icon::Skins,
        AppScreen::Settings => Icon::Settings,
        AppScreen::Legal => Icon::Legal,
        AppScreen::Console => Icon::Terminal,
    }
}

fn selected_name(state: &AppState) -> Option<String> {
    let id = state.selected_instance_id.as_deref()?;
    state
        .instances
        .find(id)
        .map(|instance| instance.name.clone())
}

/// `None` draws the plain square of a maximize button.
///
/// `top_right_radius` rounds the button's top-right corner to follow the window's, so its hover fill stays inside it.
fn window_button(
    ui: &mut Ui<'_>,
    kit: &Kit,
    id: &str,
    icon: Option<Icon>,
    danger: bool,
    top_right_radius: f32,
) {
    let hovered = ui.hovered(id);
    let background = match (hovered, danger) {
        (true, true) => kit.palette.danger,
        (true, false) => kit.palette.hovered_fill(),
        _ => Vec4::ZERO,
    };
    let _button = ui.element(
        &ElementDesc::new()
            .id(id.into())
            .width(Sizing::Fixed(46.0))
            .height(Sizing::Grow)
            .align(ChildAlignment::center())
            .background(background)
            .corner_radius(CornerRadius {
                top_right: (top_right_radius - 1.0).max(0.0),
                ..CornerRadius::default()
            }),
    );
    let color = if hovered && danger {
        kit.palette.on_primary
    } else {
        kit.palette.text
    };
    match icon {
        Some(icon) => kit.icon(ui, icon, WINDOW_ICON_SIZE, color),
        None => drop(
            ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(11.0))
                    .height(Sizing::Fixed(11.0))
                    .border(Border::all(color, 1))
                    .corner_radius(CornerRadius::all(2.0)),
            ),
        ),
    }
}

fn account_button(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let label = accounts::active_account_label(state);
    let hovered = ui.hovered(ACCOUNT_ID);
    {
        let _button = ui.element(
            &ElementDesc::new()
                .id(ACCOUNT_ID.into())
                .width(Sizing::Fit)
                .height(Sizing::Fixed(30.0))
                .padding(Padding::symmetric(kit::SPACE_MD, 0))
                .align(ChildAlignment::center())
                .background(if hovered {
                    kit.palette.hovered_fill()
                } else {
                    kit.palette.bg_light
                })
                .corner_radius(CornerRadius::all(10.0)),
        );
        if !state.streamer_mode() {
            // The head is drawn by the engine from the skin; until that is ready (or if it cannot be), the flat avatar
            // picture stands in.
            let head = state.auth.active_skin_url().and_then(|url| {
                kit.heads.head(&mut ui.engine(), &url, UVec2::new(88, 88))
            });
            match head {
                Some(texture) => ui.image(
                    &ElementDesc::new()
                        .id("chrome::avatar".into())
                        .width(Sizing::Fixed(22.0))
                        .height(Sizing::Fixed(22.0))
                        .corner_radius(CornerRadius::all(5.0)),
                    texture,
                ),
                None => {
                    if let Some(avatar) = state
                        .auth
                        .avatar_png()
                        .map(std::sync::Arc::<[u8]>::from)
                    {
                        let key = format!(
                            "avatar::{}",
                            state.auth.active_account_key().unwrap_or_default()
                        );
                        kit.image(
                            ui,
                            "chrome::avatar",
                            &key,
                            || ImageSource::Bytes(avatar),
                            22.0,
                            22.0,
                            5.0,
                        );
                    }
                }
            }
        }
        kit.text_clipped(ui, TextRole::Body, &label, 220.0);
    }
    if kit.clicked(ui, ACCOUNT_ID) {
        state.account_menu.toggle();
    }
}

// -- Sidebar ------------------------------------------------------------------------------------------------------

fn sidebar(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, radius: f32) {
    {
        let _sidebar = ui.element(
            &ElementDesc::new()
                .id("chrome::sidebar".into())
                .width(Sizing::Fixed(SIDEBAR_WIDTH))
                .height(Sizing::Grow)
                .padding(Padding::all(kit::SPACE_MD))
                .child_gap(kit::SPACE_XS)
                .direction(Direction::TopToBottom)
                .background(kit.palette.bg_dark.with_w(kit.palette.bg.w))
                .corner_radius(CornerRadius {
                    bottom_left: radius,
                    ..CornerRadius::all(0.0)
                })
                // No border of its own: the line between the sidebar and the page is drawn by `page_edge`, which
                // bends it round the page's rounded corner instead of running it straight up past the curve.
                .border(Border::default()),
        );
        for screen in AppScreen::FIXED_NAV {
            let selected = state.screen == screen
                || (screen == AppScreen::Discover && state.screen == AppScreen::DiscoverDetail);
            let id = format!("nav::{}", screen.label());
            if kit.nav_icon(ui, &id, screen_icon(screen), selected) {
                state.open(screen);
            }
            kit.tooltip(ui, &id, screen.label());
        }
        kit.separator(ui);
        {
            let _create = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .align(ChildAlignment::center()),
            );
            if kit.nav_icon(
                ui,
                "nav::create-instance",
                Icon::Plus,
                state.profile_menu.is_open(),
            ) {
                state.profile_menu.toggle();
            }
        }
        kit.tooltip(ui, "nav::create-instance", "Profile actions");
        {
            let _list = ui.element(
                &ElementDesc::new()
                    .id("nav::instances".into())
                    .width(Sizing::Grow)
                    .height(Sizing::Grow)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS)
                    .clip(vertex_shell::sturdy::ui::Clip::vertical()),
            );
            let shortcuts: Vec<InstanceRecord> = state.instances.instances.clone();
            for instance in shortcuts {
                let id = instance.id.clone();
                let element_id = format!("nav::instance::{id}");
                let selected = state.screen == AppScreen::Instance
                    && state.selected_instance_id.as_deref() == Some(id.as_str());
                if kit.nav_thumbnail(ui, &element_id, selected, |ui, kit| {
                    screens::library::thumbnail(
                        ui,
                        kit,
                        &instance,
                        &format!("{element_id}::thumb"),
                        34.0,
                    )
                }) {
                    state.open_instance(&id);
                }
                kit.tooltip(ui, &element_id, &instance.name);
                if let Some(at) = kit.secondary(ui, &element_id) {
                    state.instance_menu_target = Some(id.clone());
                    kit.open_context_menu("chrome::instance-menu", &mut state.instance_menu, at);
                }
            }
        }
    }
    profile_actions_menu(ui, kit, state);
    instance_context_menu(ui, kit, state);
}

/// The menu under the sidebar's "+" button.
fn profile_actions_menu(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let style = PopupStyle {
        background: kit.palette.bg_dark,
        border: kit.palette.border_muted,
        corner_radius: 10.0,
        padding: Padding::all(kit::SPACE_SM),
        z_index: kit::Z_POPUP,
        ..PopupStyle::default()
    };
    let mut menu = std::mem::take(&mut state.profile_menu);
    let mut choice = None;
    kit.animated_popup(
        ui,
        "chrome::profile-menu",
        "nav::create-instance",
        &mut menu,
        &style,
        |kit, ui, popup, live| {
            let _column = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fit)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS),
            );
            // At least this wide; longer entries widen the menu.
            drop(
                ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Fixed(200.0))
                        .height(Sizing::Fixed(0.0)),
                ),
            );
            for (index, label) in [
                "Create from scratch",
                "Import profile",
                "Link existing Vertex profile",
            ]
            .into_iter()
            .enumerate()
            {
                if kit.nav_item_ex(
                    ui,
                    &format!("chrome::profile-menu::{index}"),
                    Some(Icon::Plus),
                    label,
                    false,
                    false,
                ) && live
                {
                    choice = Some(index);
                    popup.close();
                }
            }
        },
    );
    state.profile_menu = menu;
    match choice {
        Some(0) => state.dialog = Some(Dialog::CreateInstance),
        Some(1) => state.dialog = Some(Dialog::Import),
        Some(2) => super::link_profile::start(state),
        _ => {}
    }
}

fn instance_context_menu(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let Some(target) = state.instance_menu_target.clone() else {
        return;
    };
    let items = [
        MenuEntry::new("Open instance menu").icon(Icon::Library),
        MenuEntry::new("Open folder").icon(Icon::Folder),
        MenuEntry::new("Copy command line").icon(Icon::Terminal),
        MenuEntry::new("Copy Steam launch options").icon(Icon::Steam),
        MenuEntry::new("Delete instance").icon(Icon::Trash).danger(),
    ];
    if let Some(choice) = kit.context_menu(
        ui,
        "chrome::instance-menu",
        &mut state.instance_menu,
        &items,
    ) {
        match choice {
            0 => state.open_instance(&target),
            1 => screens::library::open_folder(state, &target),
            2 => screens::library::copy_quick_launch(ui, state, &target, false),
            3 => screens::library::copy_quick_launch(ui, state, &target, true),
            _ => state.dialog = Some(Dialog::ConfirmDelete(target.clone())),
        }
    }
    // Keep the target while the menu folds away, or it would vanish instead of animating closed.
    if !state.instance_menu.is_open() && !kit.menu_visible("chrome::instance-menu") {
        state.instance_menu_target = None;
    }
}

// -- Content ------------------------------------------------------------------------------------------------------

/// Radius of the rounded inside corner where the title bar meets the sidebar.
const INNER_CORNER: f32 = 14.0;

/// The line that frames the page: down the sidebar's right side, round the page's rounded corner, and along the title
/// bar's bottom edge to the window's right side, hugging the outside of the page's outline the whole way.
fn page_edge(ui: &mut Ui<'_>, kit: &Kit, border: f32) {
    use vertex_shell::sturdy::glam::Vec2;
    let r = INNER_CORNER;
    let page_width = (ui.viewport_size().x - 2.0 * border - SIDEBAR_WIDTH).max(r);
    let page_height = (ui.viewport_size().y - TITLE_BAR_HEIGHT - 2.0 * border).max(r);
    // The box starts one pixel above and to the left of the page, so the one-pixel line sits just outside it. The page's
    // corner is a circle of radius `r` centred at (r + 1, r + 1) here; the line is half a pixel outside that.
    let (width, height) = (page_width + 1.0, page_height + 1.0);
    let centre = Vec2::splat(r + 1.0);
    let mut points = vec![Vec2::new(0.5, height), Vec2::new(0.5, centre.y)];
    const STEPS: usize = 32;
    for step in 1..STEPS {
        let angle = std::f32::consts::FRAC_PI_2 * step as f32 / STEPS as f32;
        points.push(centre - Vec2::new(angle.cos(), angle.sin()) * (r + 0.5));
    }
    points.push(Vec2::new(centre.x, 0.5));
    points.push(Vec2::new(width, 0.5));
    // A floating box that holds the line, rather than the line's own leaf element being the floating one.
    let _box = ui.element(
        &ElementDesc::new()
            .id("chrome::page-edge".into())
            .width(Sizing::Fixed(width))
            .height(Sizing::Fixed(height))
            .floating(Some(Floating {
                element_attach_point: AttachPoint::LeftTop,
                parent_attach_point: AttachPoint::LeftTop,
                offset: Vec2::new(-1.0, -1.0),
                z_index: 1,
                capture_pointer: false,
                ..Floating::new(FloatTarget::Parent)
            })),
    );
    ui.stroke_polyline(
        &ElementDesc::new()
            .id("chrome::page-edge::line".into())
            .width(Sizing::Fixed(width))
            .height(Sizing::Fixed(height)),
        &points,
        vertex_shell::sturdy::ui::StrokeStyle {
            color: kit.palette.border_muted,
            width: 1.0,
            // Soft edges: the default is none, which draws the curve jagged.
            feather_px: 1.0,
            ..Default::default()
        },
    );
}

/// Rounds the corner of the page area where the title bar and the sidebar meet: the two dark bars form an L around the
/// page, and its inside corner is filled in the bars' colour with a quarter circle cut out, so the page looks rounded
/// there. Call it inside the page area's element.
fn inner_corner(ui: &mut Ui<'_>, kit: &Kit, border: f32) {
    use std::f32::consts::PI;
    let r = INNER_CORNER;
    page_edge(ui, kit, border);
    let _square = ui.element(
        &ElementDesc::new()
            .id("chrome::inner-corner".into())
            .width(Sizing::Fixed(r))
            .height(Sizing::Fixed(r))
            // The ring segment reaches past the square; clipping to it keeps it to the corner's own area.
            .clip(vertex_shell::sturdy::ui::Clip::both())
            .floating(Some(Floating {
                element_attach_point: AttachPoint::LeftTop,
                parent_attach_point: AttachPoint::LeftTop,
                z_index: 1,
                capture_pointer: false,
                ..Floating::new(FloatTarget::Parent)
            })),
    );
    ui.fill_sectors(
        &ElementDesc::new()
            .id("chrome::inner-corner::fill".into())
            .width(Sizing::Fixed(r))
            .height(Sizing::Fixed(r)),
        &[vertex_shell::sturdy::ui::Sector {
            center: vertex_shell::sturdy::glam::Vec2::new(r, r),
            inner_radius: r,
            outer_radius: r * 1.5,
            start_angle: PI,
            end_angle: PI * 1.5,
            // The same colour the title bar and the sidebar are drawn in.
            color: kit.palette.bg_dark.with_w(kit.palette.bg.w),
            feather_px: 1.0,
        }],
    );
}

fn content(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, border: f32) {
    // Pages run to the very bottom of the window and are clipped there. Clipping is rectangular, so it cannot follow
    // the window's rounded bottom-right corner, but nothing needs it to: a page keeps SPACE_XL (24 px) clear of the
    // right edge, and a corner of the largest radius (28 px) is still clear of anything that far in. (The bottom-left
    // corner belongs to the sidebar.) The corner therefore never changes a page's layout.
    let _content = ui.element(
        &ElementDesc::new()
            .id("chrome::content".into())
            .width(Sizing::Fixed(
                (ui.viewport_size().x - 2.0 * border - SIDEBAR_WIDTH).max(0.0),
            ))
            .height(Sizing::Fixed(body_height(ui, border)))
            .direction(Direction::TopToBottom),
    );
    inner_corner(ui, kit, border);
    if state.screen != AppScreen::Skins {
        // The preview texture is only kept while the screen is showing.
        state.skins_view.hidden(ui);
    }
    match state.screen {
        AppScreen::Library => screens::library::render(ui, kit, state),
        AppScreen::Instance => screens::instance::render(ui, kit, state),
        // Static launcher text only; nothing here can show an account name.
        AppScreen::Settings => launcher_ui::privacy::without_name_scrubbing(|| {
            screens::settings::render(ui, kit, state)
        }),
        AppScreen::Console => screens::console::render(ui, kit, state),
        AppScreen::Legal => {
            launcher_ui::privacy::without_name_scrubbing(|| screens::legal::render(ui, kit))
        }
        AppScreen::Home => screens::home::render(ui, kit, state),
        AppScreen::Discover => screens::discover::browse(ui, kit, state),
        AppScreen::DiscoverDetail => screens::discover::detail(ui, kit, state),
        AppScreen::ContentBrowser => screens::content_browser::render(ui, kit, state),
        AppScreen::Skins => screens::skins::render(ui, kit, state),
    }
}
