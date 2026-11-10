//! Content Browser: find mods, resource packs, shaders and data packs for the selected instance and install them.
//! Search, filtering, paging, installed-state and the install queue are `launcher_ui`'s `content_browser_api`.


use launcher_ui::screens::AppScreen;
use launcher_ui::screens::content_browser_api as api;
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, Border, ChildAlignment, Clip, CornerRadius, Direction, ElementDesc, Padding,
    Sizing, TextStyle, TextWrap, Ui,
};

use super::super::AppState;
use super::super::icons::Icon;
use super::super::images::ImageSource;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

const SCROLL_ID: &str = "content::scroll";

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let Some(instance) = state
        .selected_instance_id
        .as_deref()
        .and_then(|id| state.instances.find(id))
        .cloned()
    else {
        let _scroll = kit.scroll(ui, SCROLL_ID, kit::SPACE_LG, kit::SPACE_XL);
        kit.text(ui, TextRole::Title, "Content Browser");
        kit.text(
            ui,
            TextRole::Muted,
            "Select an instance first. Content Browser installs into the selected instance.",
        );
        return;
    };
    let root = state.installations_root();
    let instance_root = instances::instance_root_path(root.as_path(), &instance);
    if api::tick(&mut state.content, &instance, root.as_path()) {
    }

    let _scroll = kit.scroll(ui, SCROLL_ID, kit::SPACE_LG, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, "Content Browser");
    kit.text(ui, TextRole::Small, &api::context_line(&instance));
    let (message, warnings) = api::status(&state.content);
    if let Some(message) = message {
        kit.text(ui, TextRole::Muted, &message);
    }
    for warning in warnings {
        kit::label(
            ui,
            &warning,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Danger)
            },
        );
    }

    if api::is_detail(&state.content) {
        detail(ui, kit, state, &instance_root);
    } else {
        browse(ui, kit, state, &instance_root);
    }

    let _row = kit.row(ui, kit::SPACE_SM);
    if api::is_detail(&state.content)
        && kit.button(
            ui,
            "content::back-browser",
            "Back to Mod Browser",
            ButtonKind::Neutral,
            true,
        )
    {
        api::back_to_browse(&mut state.content);
    }
    if kit.button(
        ui,
        "content::back-instance",
        "Back to Instance",
        ButtonKind::Neutral,
        true,
    ) {
        state.open(AppScreen::Instance);
    }
}

fn browse(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, instance_root: &std::path::Path) {
    controls(ui, kit, state);
    if let Some(progress) = api::searching(&state.content) {
        kit.text(ui, TextRole::Muted, &progress);
        kit.progress(
            ui,
            "content::search-progress",
            api::search_fraction(&state.content),
            kit::uptime(),
        );
    }
    let groups = api::results(&mut state.content, instance_root);
    if groups.is_empty() && api::searching(&state.content).is_none() {
        kit.text_wrapped(ui, TextRole::Muted, api::empty_message(&state.content));
    }
    let mut open = None;
    let mut install = Vec::new();
    for group in &groups {
        kit.text(ui, TextRole::Heading, &group.heading);
        for tile in &group.tiles {
            let (opened, quick) = result_tile(ui, kit, tile);
            if opened {
                open = Some(tile.key.clone());
            }
            if quick {
                install.push(tile.key.clone());
            }
        }
    }
    for key in install {
        api::quick_install(&mut state.content, &key);
    }
    if let Some(key) = open {
        api::open_detail(&mut state.content, &key);
    }
    pagination(ui, kit, state);
}

fn controls(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let _card = kit.card(ui, "content::controls", kit::SPACE_MD);
    let mut as_tag = false;
    let mut submit = false;
    {
        let _row = kit.row(ui, kit::SPACE_SM);
        let result = kit.text_field_ex(
            ui,
            "content::query",
            api::query_mut(&mut state.content),
            "Search project names, summaries, and tags. Press Enter to search",
            false,
            Sizing::Grow,
        );
        if result.submitted {
            submit = true;
            as_tag = ui.engine().input().modifiers().shift;
        }
        if kit.button(ui, "content::search", "Search", ButtonKind::Primary, true) {
            submit = true;
        }
    }
    if submit {
        api::submit(&mut state.content, as_tag);
    }
    if !api::tags(&state.content).is_empty() {
        let tags = api::tags(&state.content).to_vec();
        let remove = kit.chip_rows(ui, "content::tags", &tags);
        if let Some(index) = remove {
            api::remove_tag(&mut state.content, index);
        }
    }
    let current = api::controls(&state.content);
    let (mut version, mut scope, mut sort, mut loader) =
        (current.version, current.scope, current.sort, current.loader);
    {
        // One line: the four dropdowns split its width equally.
        let _row = kit.row(ui, kit::SPACE_MD);
        if let Some(index) = kit.dropdown_grow(ui, "content::version", version, &current.versions) {
            version = index;
        }
        if let Some(index) = kit.dropdown_grow(ui, "content::scope", scope, &current.scopes) {
            scope = index;
        }
        if let Some(index) = kit.dropdown_grow(ui, "content::sort", sort, &current.sorts) {
            sort = index;
        }
        if let Some(index) = kit.dropdown_grow(ui, "content::loader", loader, &current.loaders) {
            loader = index;
        }
    }
    api::apply_controls(&mut state.content, version, scope, sort, loader);
}

/// Returns (opened, quick install requested).
fn result_tile(ui: &mut Ui<'_>, kit: &mut Kit, tile: &api::TileView) -> (bool, bool) {
    let id = format!("content::tile::{}", tile.key);
    let info_id = format!("{id}::info");
    let install_id = format!("{id}::install");
    let hovered = ui.hovered(&id);
    {
        let _card = ui.element(
            &ElementDesc::new()
                .id(id.clone())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_MD)
                .padding(Padding::all(kit::SPACE_MD))
                .align(ChildAlignment::new(AlignX::Left, AlignY::Top))
                .background(if hovered {
                    kit.palette.bg_light
                } else {
                    kit.palette.bg
                })
                .corner_radius(CornerRadius::all(kit::CORNER))
                .border(Border::all(
                    if hovered {
                        kit.palette.border
                    } else {
                        kit.palette.border_muted
                    },
                    1,
                )),
        );
        content_icon(ui, kit, &format!("{id}::icon"), tile, 56.0, SCROLL_ID);
        {
            let _text = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS)
                    // Never wider than the space left of the actions, so they stay on the card's right edge.
                    .clip(Clip::horizontal()),
            );
            {
                let _title = kit.row(ui, kit::SPACE_SM);
                kit::label(ui, &tile.name, &kit.style(TextRole::Heading));
                chips(ui, kit, tile);
            }
            kit::label(
                ui,
                &tile.summary,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Muted)
                },
            );
        }
        {
            // Fit-width, not `kit.row` (which grows): two growing siblings would split the card in half.
            let _actions = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fit)
                    .height(Sizing::Fit)
                    .direction(Direction::LeftToRight)
                    .child_gap(kit::SPACE_XS)
                    .align(ChildAlignment::new(AlignX::Right, AlignY::Center)),
            );
            icon_button(ui, kit, &info_id, Icon::Settings, true);
            if tile.downloading {
                kit.progress(ui, &format!("{id}::progress"), None, kit::uptime());
            } else if tile.queued {
                icon_button(ui, kit, &install_id, Icon::Download, false);
            } else {
                icon_button(
                    ui,
                    kit,
                    &install_id,
                    if tile.installed.is_some() {
                        Icon::Check
                    } else {
                        Icon::Download
                    },
                    tile.installed.is_none(),
                );
            }
        }
    }
    kit.tooltip(ui, &info_id, "Open mod details");
    if tile.queued && !tile.downloading {
        kit.tooltip(ui, &install_id, "Queued: installs after the downloads ahead of it");
    } else if !tile.downloading {
        match &tile.installed {
            Some(installed) => kit.tooltip(
                ui,
                &install_id,
                &format!("Already installed in this instance\nCurrent: {installed}"),
            ),
            None => kit.tooltip(ui, &install_id, "Quick install latest compatible version"),
        }
    }
    let install = tile.installed.is_none() && !tile.downloading && !tile.queued && kit.clicked(ui, &install_id);
    let open =
        kit.clicked(ui, &info_id) || (kit.clicked(ui, &id) && !install && !ui.hovered(&install_id));
    (open, install)
}

fn chips(ui: &mut Ui<'_>, kit: &Kit, tile: &api::TileView) {
    chip(ui, kit, tile.kind, false);
    for source in &tile.sources {
        chip(ui, kit, source, false);
    }
    if let Some(installed) = &tile.installed {
        chip(ui, kit, installed, true);
    }
}

fn chip(ui: &mut Ui<'_>, kit: &Kit, label: &str, accent: bool) {
    let _chip = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fit)
            .height(Sizing::Fixed(22.0))
            .padding(Padding::symmetric(6, 0))
            .align(ChildAlignment::center())
            .background(if accent {
                kit.palette.success.with_w(0.25)
            } else {
                kit.palette.bg_light
            })
            .corner_radius(CornerRadius::all(6.0)),
    );
    kit.text(ui, TextRole::Small, label);
}

fn icon_button(ui: &mut Ui<'_>, kit: &Kit, id: &str, icon: Icon, enabled: bool) {
    let hovered = enabled && ui.hovered(id);
    let _button = ui.element(
        &ElementDesc::new()
            .id(id.into())
            .width(Sizing::Fixed(30.0))
            .height(Sizing::Fixed(30.0))
            .align(ChildAlignment::center())
            .background(if hovered {
                kit.palette.hovered_fill()
            } else {
                kit.palette.bg_light
            })
            .corner_radius(CornerRadius::all(8.0)),
    );
    kit.icon(
        ui,
        icon,
        16.0,
        if enabled {
            kit.palette.text
        } else {
            kit.palette.text_muted
        },
    );
}

fn content_icon(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    tile: &api::TileView,
    size: f32,
    scroll_id: &str,
) {
    match tile.icon_url.clone().filter(|url| !url.trim().is_empty()) {
        Some(url)
            if kit.near_viewport(ui, scroll_id, id, 600.0)
                && !kit.images.failed(&format!("remote::{url}")) =>
        {
            let key = format!("remote::{url}");
            kit.image(ui, id, &key, || ImageSource::Url(url), size, size, 10.0);
        }
        _ => {
            let _box = ui.element(
                &ElementDesc::new()
                    .id(id.into())
                    .width(Sizing::Fixed(size))
                    .height(Sizing::Fixed(size))
                    .align(ChildAlignment::center())
                    .background(kit.palette.bg_light)
                    .corner_radius(CornerRadius::all(10.0)),
            );
            kit.icon(ui, Icon::Discover, size * 0.5, kit.palette.text_muted);
        }
    }
}

fn pagination(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let (page, searching) = api::page(&state.content);
    let mut target = None;
    let _row = kit.row(ui, kit::SPACE_SM);
    if kit.button(
        ui,
        "content::prev",
        "Previous",
        ButtonKind::Neutral,
        !searching && page > 1,
    ) {
        target = Some(page - 1);
    }
    kit.text(ui, TextRole::Muted, "Page");
    let mut text = page.to_string();
    kit.number_text_field(ui, "content::page", &mut text, 72.0, false, false);
    let typed = text.trim().parse::<u32>().ok();
    if kit.button(
        ui,
        "content::go",
        "Go",
        ButtonKind::Neutral,
        !searching && typed.is_some_and(|value| value != page),
    ) {
        target = typed;
    }
    if kit.button(
        ui,
        "content::next",
        "Next",
        ButtonKind::Neutral,
        !searching && api::has_next_page(&state.content),
    ) {
        target = Some(page + 1);
    }
    kit.text(ui, TextRole::Muted, &format!("Current: {page}"));
    drop(_row);
    if let Some(page) = target {
        api::go_to_page(&mut state.content, page);
    }
}

fn detail(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, instance_root: &std::path::Path) {
    let Some(view) = api::detail(&mut state.content, instance_root) else {
        kit.text(ui, TextRole::Muted, "No content item selected.");
        return;
    };
    {
        let _card = kit.card(ui, "content::detail-card", kit::SPACE_SM);
        let _head = kit.row(ui, kit::SPACE_LG);
        content_icon(ui, kit, "content::detail-icon", &view.tile, 96.0, SCROLL_ID);
        let _text = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_SM),
        );
        kit::label(
            ui,
            &view.tile.name,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Title)
            },
        );
        let _chips = kit.row(ui, kit::SPACE_SM);
        chips(ui, kit, &view.tile);
    }
    {
        let _tabs = kit.row(ui, kit::SPACE_SM);
        for (index, label) in ["Overview", "Versions"].iter().enumerate() {
            if kit.nav_item(
                ui,
                &format!("content::tab::{index}"),
                None,
                label,
                view.tab == index,
            ) {
                api::set_detail_tab(&mut state.content, index);
            }
        }
    }
    if view.tab == 0 {
        let _card = kit.card(ui, "content::overview", kit::SPACE_SM);
        kit::label(
            ui,
            &view.body,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Body)
            },
        );
        return;
    }
    let _card = kit.card(ui, "content::versions", kit::SPACE_MD);
    let (mut loader, mut version) = (view.loader, view.version);
    kit.setting_row(
        ui,
        "content::detail-loader-row",
        "Loader",
        None,
        |kit, ui| {
            if let Some(index) =
                kit.dropdown(ui, "content::detail-loader", loader, &view.loaders, 220.0)
            {
                loader = index;
            }
        },
    );
    kit.setting_row(
        ui,
        "content::detail-version-row",
        "Minecraft Version",
        None,
        |kit, ui| {
            if let Some(index) = kit.dropdown(
                ui,
                "content::detail-version",
                version,
                &view.versions,
                220.0,
            ) {
                version = index;
            }
        },
    );
    api::set_detail_filters(&mut state.content, loader, version);
    if let Some(error) = &view.error {
        kit::label(
            ui,
            error,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Danger)
            },
        );
    }
    if view.loading {
        kit.text(ui, TextRole::Muted, "Loading versions...");
    } else if view.rows.is_empty() {
        kit.text(
            ui,
            TextRole::Muted,
            "No versions match the current filters.",
        );
    }
    let mut queue = None;
    for row in &view.rows {
        let id = format!("content::version::{}", row.id);
        let _row = ui.element(
            &ElementDesc::new()
                .id(id.clone())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_MD)
                .padding(Padding::all(kit::SPACE_MD))
                .background(kit.palette.bg_light)
                .corner_radius(CornerRadius::all(10.0)),
        );
        {
            let _info = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS),
            );
            kit::label(
                ui,
                &row.name,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Heading)
                },
            );
            {
                let _chips = kit.row(ui, kit::SPACE_XS);
                chip(ui, kit, row.source, false);
                for loader in &row.loaders {
                    chip(ui, kit, loader, false);
                }
                for game_version in &row.game_versions {
                    chip(ui, kit, game_version, false);
                }
            }
            kit.text(ui, TextRole::Small, &row.file_line);
        }
        let (icon, enabled) = match row.action {
            api::RowAction::Installed => (Icon::Check, false),
            api::RowAction::Switch => (Icon::Refresh, true),
            api::RowAction::Download => (Icon::Download, true),
        };
        let button_id = format!("{id}::action");
        icon_button(ui, kit, &button_id, icon, enabled);
        if enabled && kit.clicked(ui, &button_id) {
            queue = Some((row.id.clone(), row.action));
        }
    }
    if let Some((id, action)) = queue {
        api::queue_version(&mut state.content, &id, action);
    }
}
