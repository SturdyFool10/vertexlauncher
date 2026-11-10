//! One instance: launch/stop and install progress on top, then the Content, Screenshot Gallery and Logs tabs.
//! Settings, exports and moving are dialogs (`instance_dialogs`). The state behind all of it is
//! `launcher_ui`'s `InstanceSession`, shared with the egui screen.


use vertex_i18n::tr;
use content_resolver::InstalledContentKind;
use instances::InstanceRecord;
use launcher_ui::notification;
use launcher_ui::screens::AppScreen;
use launcher_ui::screens::instance_api::{self as api, InstanceScreenTab};
use vertex_shell::sturdy::glam::Vec4;
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, Border, ChildAlignment, Clip, CornerRadius, Direction, ElementDesc,
    Padding, Sizing, TextStyle, TextWrap, Ui,
};
use vertex_shell::sturdy::ui_kit::{self, PopupStyle};

use super::super::icons::Icon;
use super::super::images::ImageSource;
use super::super::kit::{self, ButtonKind, Kit, TextRole};
use super::super::{AppState, Dialog};
use super::library;

const SCROLL_ID: &str = "instance::scroll";

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let Some(instance) = state
        .selected_instance_id
        .as_deref()
        .and_then(|id| state.instances.find(id))
        .cloned()
    else {
        let _scroll = kit.scroll(ui, SCROLL_ID, kit::SPACE_LG, kit::SPACE_XL);
        kit.text(ui, TextRole::Title, &tr!("Instance"));
        kit.text(
            ui,
            TextRole::Muted,
            &tr!("Select an instance from the left sidebar or click + to create one."),
        );
        return;
    };
    let id = instance.id.clone();
    state.ensure_session(&instance);
    let root = instances::instance_root_path(state.installations_root().as_path(), &instance);

    let _scroll = kit.scroll(ui, SCROLL_ID, kit::SPACE_LG, kit::SPACE_XL);
    header(ui, kit, state, &instance, &root);

    let Some(session) = state.sessions.get(&id) else {
        return;
    };
    if let Some(progress) = session.install_progress(&instance.name) {
        install_progress(ui, kit, &progress);
    }
    if let Some(message) = session.state.status_message.clone() {
        kit::label(
            ui,
            &message,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Muted)
            },
        );
    }

    tabs(ui, kit, state, &id);
    let tab = state
        .sessions
        .get(&id)
        .map_or(InstanceScreenTab::Content, |session| {
            session.state.active_tab
        });
    match tab {
        InstanceScreenTab::Content => content_tab(ui, kit, state, &instance, &root),
        InstanceScreenTab::ScreenshotGallery => screenshots_tab(ui, kit, state, &id, &root),
        InstanceScreenTab::Logs => logs_tab(ui, kit, state, &id, &root),
    }
}

fn header(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    instance: &InstanceRecord,
    root: &std::path::Path,
) {
    let id = instance.id.clone();
    {
        // Top line: the instance folder (open, then copy its path) at the left, game settings at the right.
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_XS)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        );
        kit.text(
            ui,
            TextRole::Muted,
            &tr!("Root Folder: {root}", root = root.display()),
        );
        if kit.icon_button(ui, "instance::folder", Icon::Folder, 30.0) {
            library::open_folder(state, &id);
        }
        kit.tooltip(
            ui,
            "instance::folder",
            &tr!("Open instance folder\n{root}", root = root.display()),
        );
        if kit.icon_button(ui, "instance::copy-root", Icon::Copy, 24.0) {
            let absolute = if root.is_absolute() {
                root.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(root))
                    .unwrap_or_else(|_| root.to_path_buf())
            };
            let text = absolute.display().to_string();
            let _ = ui
                .engine()
                .primary_window_control()
                .map(|mut window| window.set_clipboard_text(&text));
            notification::info!("instance/root", "Copied instance root path to the clipboard.");
        }
        kit.tooltip(
            ui,
            "instance::copy-root",
            &tr!("Copy absolute instance root path"),
        );
        kit.fill(ui);
        if kit.button(
            ui,
            "instance::game-settings",
            &tr!("Game settings"),
            ButtonKind::Neutral,
            true,
        ) {
            super::game_settings::open(state, &id);
        }
    }
    {
        let _header = kit.row(ui, kit::SPACE_MD);
        library::thumbnail(ui, kit, instance, "instance::thumb", 64.0);
        {
            let _title = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS),
            );
            kit.text(ui, TextRole::Title, &instance.name);
            kit.text(ui, TextRole::Muted, &library::loader_label(instance));
        }
        library::launch_button(ui, kit, state, instance, "instance::launch");
        if kit.button(
            ui,
            "instance::settings",
            &tr!("Instance settings"),
            ButtonKind::Neutral,
            true,
        ) {
            state.dialog = Some(Dialog::InstanceSettings(id.clone()));
        }
    }
}

fn install_progress(ui: &mut Ui<'_>, kit: &mut Kit, progress: &api::InstallProgressView) {
    let _card = kit.card(ui, "instance::install", kit::SPACE_XS);
    let mut label = progress.stage.clone();
    if progress.bytes_per_second > 0.0 {
        label.push_str(&tr!(" · {speed:.1} MiB/s", speed = progress.bytes_per_second / (1024.0 * 1024.0)));
    }
    if let Some(eta) = progress.eta_seconds {
        label.push_str(&tr!(" · ETA {eta}s", eta = eta));
    }
    if let Some(fraction) = progress.fraction {
        label.push_str(&format!(" · {:.0}%", fraction * 100.0));
    }
    kit.text(ui, TextRole::Body, &label);
    kit.progress(
        ui,
        "instance::install::bar",
        progress.fraction,
        kit::uptime(),
    );
    if !progress.message.is_empty() {
        kit.text(ui, TextRole::Muted, &progress.message);
    }
    if progress.total_files > 0 {
        let bytes = match progress.total_bytes {
            Some(total) => format!(
                "{} / {}",
                api::human_bytes(progress.downloaded_bytes),
                api::human_bytes(total)
            ),
            None => tr!("{size} downloaded", size = api::human_bytes(progress.downloaded_bytes)).into_owned(),
        };
        kit.text(
            ui,
            TextRole::Small,
            &tr!("Files: {downloaded_files}/{total_files} · {bytes}", downloaded_files = progress.downloaded_files, total_files = progress.total_files, bytes = bytes),
        );
    }
}

fn tabs(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState, id: &str) {
    let _row = kit.row(ui, kit::SPACE_SM);
    let current = state
        .sessions
        .get(id)
        .map_or(InstanceScreenTab::Content, |session| {
            session.state.active_tab
        });
    for (tab, label) in [
        (InstanceScreenTab::Content, tr!("Content")),
        (InstanceScreenTab::ScreenshotGallery, tr!("Screenshot Gallery")),
        (InstanceScreenTab::Logs, tr!("Logs")),
    ] {
        if kit.tab_sized(
            ui,
            &format!("instance::tab::{label}"),
            &label,
            current == tab,
            Sizing::Grow,
        ) && let Some(session) = state.sessions.get_mut(id)
        {
            session.state.active_tab = tab;
        }
    }
}

// -- Content --------------------------------------------------------------------------------------------------------

fn content_tab(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    instance: &InstanceRecord,
    root: &std::path::Path,
) {
    let id = instance.id.clone();
    let locked = state
        .sessions
        .get(&id)
        .is_some_and(|session| session.install_active());
    // File picker results.
    let added = state
        .file_dialogs
        .take("instance::add-local")
        .filter(|paths| !paths.is_empty());
    let patch = state
        .file_dialogs
        .take("instance::apply-patch")
        .and_then(|paths| paths.into_iter().next());
    if let Some(session) = state.sessions.get_mut(&id) {
        if let Some(paths) = added {
            api::content::add_local_files(&mut session.state, root, paths);
        }
        if let Some(path) = patch {
            api::content::apply_patch(&mut session.state, root, path);
        }
    }

    let mut open_browser = false;
    let mut add_files = false;
    let mut apply_patch = false;
    let mut refresh = false;
    {
        // One control: "Open Content Browser" and, chopped onto its right edge, the "+ ▾" menu for adding content.
        let (main, menu) = kit.split_button(
            ui,
            "instance::open-browser",
            &tr!("Open Content Browser"),
            "instance::add-menu",
            !locked,
        );
        if main {
            open_browser = true;
        }
        if menu {
            state.add_content_menu.toggle();
        }
        if locked {
            kit.tooltip(
                ui,
                "instance::open-browser",
                &tr!("Content Browser is unavailable while instance prep is running."),
            );
        }
        kit.tooltip(ui, "instance::add-menu", &tr!("Add content"));
    }
    let style = PopupStyle {
        background: kit.palette.bg_dark,
        border: kit.palette.border_muted,
        corner_radius: 12.0,
        padding: Padding::all(kit::SPACE_SM),
        z_index: kit::Z_POPUP,
        ..PopupStyle::default()
    };
    let mut menu = std::mem::take(&mut state.add_content_menu);
    kit.animated_popup(
        ui,
        "instance::add-popup",
        "instance::add-menu",
        &mut menu,
        &style,
        |kit, ui, popup, live| {
            let _column = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(240.0))
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS),
            );
            if kit.nav_item(ui, "instance::add-local", None, &tr!("Add local file(s)"), false) {
                if live {
                    add_files = true;
                    popup.close();
                }
            }
            if kit.nav_item(ui, "instance::add-patch", None, &tr!("Apply Patch"), false) {
                if live {
                    apply_patch = true;
                    popup.close();
                }
            }
            if kit.nav_item(
                ui,
                "instance::add-refresh",
                None,
                &tr!("Refresh local files"),
                false,
            ) {
                if live {
                    refresh = true;
                    popup.close();
                }
            }
        },
    );
    state.add_content_menu = menu;
    if add_files {
        state.file_dialogs.open(
            "instance::add-local",
            crate::app::se5::file_dialog::Pick::Files,
            crate::app::se5::file_dialog::Dialog::new().title(tr!("Add Local Content")),
        );
    }
    if apply_patch {
        state.file_dialogs.open(
            "instance::apply-patch",
            crate::app::se5::file_dialog::Pick::File,
            crate::app::se5::file_dialog::Dialog::new()
                .title(tr!("Apply Vertex Patch"))
                .filter(tr!("Vertex Modpack Patch"), &[vtmpack::VTMPATCH_EXTENSION]),
        );
    }
    if refresh && let Some(session) = state.sessions.get_mut(&id) {
        api::content::refresh(&mut session.state, root);
    }
    if open_browser {
        state.selected_instance_id = Some(id.clone());
        state.open(AppScreen::ContentBrowser);
        return;
    }

    let config = state.config.clone();
    let Some(session) = state.sessions.get_mut(&id) else {
        return;
    };
    let view = api::content::view(&mut session.state, root, &config);

    {
        let _row = kit.row(ui, kit::SPACE_SM);
        for kind in &view.kinds {
            if kit.tab_sized(
                ui,
                &format!("instance::kind::{}", kind.label()),
                kind.label(),
                *kind == view.kind,
                Sizing::Grow,
            ) {
                api::content::select_tab(&mut session.state, *kind);
            }
        }
    }
    if view.empty_message.is_some() && view.search.is_empty() && view.rows.is_empty() && !view.updatable_only {
        if let Some(message) = &view.empty_message {
            kit.text(ui, TextRole::Muted, message);
        }
        return;
    }
    let mut query = view.search.clone();
    if kit.text_field(
        ui,
        "instance::content-search",
        &mut query,
        &tr!("Search installed {kind} by name, file, version, or provider", kind = view.kind.label().to_lowercase()),
        false,
        Sizing::Grow,
    ) {
        api::content::set_search(&mut session.state, &query);
    }
    // With the updates filter on, the controls stay up whatever the list holds, so the filter can always be switched off.
    if let Some(message) = &view.empty_message
        && !view.updatable_only
    {
        kit.text(ui, TextRole::Muted, message);
        return;
    }

    let mut bulk = false;
    if let Some((label, tooltip)) = &view.bulk_update {
        if kit.button_icon(
            ui,
            "instance::bulk-update",
            Icon::Refresh,
            label,
            ButtonKind::Primary,
            !view.apply_in_flight,
            Sizing::Grow,
        ) {
            bulk = true;
        }
        kit.tooltip(ui, "instance::bulk-update", tooltip);
    }
    {
        let _row = kit.row(ui, kit::SPACE_SM);
        kit.text(ui, TextRole::Muted, &tr!("Items per page"));
        let sizes: Vec<String> = api::content::PAGE_SIZES
            .iter()
            .map(usize::to_string)
            .collect();
        let size_index = api::content::PAGE_SIZES
            .iter()
            .position(|size| *size == view.page_size)
            .unwrap_or(0);
        if let Some(index) = kit.dropdown(ui, "instance::page-size", size_index, &sizes, 90.0) {
            api::content::set_page_size(&mut session.state, api::content::PAGE_SIZES[index]);
        }
        kit.text(ui, TextRole::Muted, &tr!("Page"));
        let pages: Vec<String> = (1..=view.total_pages)
            .map(|page| {
                if page == view.page {
                    tr!("{page} / {total_pages}", page = page, total_pages = view.total_pages).into_owned()
                } else {
                    tr!("Page {page}", page = page).into_owned()
                }
            })
            .collect();
        if let Some(index) = kit.dropdown(
            ui,
            "instance::page",
            view.page.saturating_sub(1),
            &pages,
            120.0,
        ) {
            api::content::set_page(&mut session.state, index + 1);
        }
        kit.text(ui, TextRole::Muted, &view.count_label);
        // Pushes the filter to the right edge: the spacer grows, the Fit wrapper keeps the checkbox its natural width.
        let _spacer = ui.element(&ElementDesc::new().width(Sizing::Grow).height(Sizing::Fixed(1.0)));
        drop(_spacer);
        let _filter = ui.element(&ElementDesc::new().width(Sizing::Fit).height(Sizing::Fit));
        if let Some(on) = kit.checkbox(
            ui,
            "instance::updatable-only",
            &tr!("Show updatable files"),
            view.updatable_only,
        ) {
            api::content::set_updatable_only(&mut session.state, on);
        }
    }
    if let Some(message) = &view.empty_message {
        kit.text(ui, TextRole::Muted, message);
        return;
    }

    let is_mods = view.kind == InstalledContentKind::Mods;
    let mut delete = None;
    let mut toggle = None;
    let mut update = None;
    let mut open = None;
    for row in &view.rows {
        match content_row(ui, kit, row, is_mods, view.apply_in_flight) {
            RowAction::Delete => delete = Some(row.clone()),
            RowAction::Toggle => toggle = Some(row.clone()),
            RowAction::Update => update = Some(row.clone()),
            RowAction::Open => open = Some(row.clone()),
            RowAction::None => {}
        }
    }
    if bulk {
        api::content::bulk_update(&mut session.state, root, &config);
    }
    if let Some(row) = delete {
        api::content::delete(&mut session.state, root, &row);
    }
    if let Some(row) = toggle {
        api::content::toggle(&mut session.state, &row);
    }
    if let Some(row) = update {
        api::content::update(&mut session.state, root, &row);
    }
    if let Some(row) = open {
        match api::content::open_detail(&mut session.state, &row, locked) {
            Ok(()) => {
                state.selected_instance_id = Some(id.clone());
                state.open(AppScreen::ContentBrowser);
            }
            Err(message) => session.state.status_message = Some(message),
        }
    }
}

enum RowAction {
    None,
    Open,
    Delete,
    Toggle,
    Update,
}

fn content_row(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    row: &api::content::ContentRow,
    is_mods: bool,
    apply_busy: bool,
) -> RowAction {
    let id = format!("instance::row::{}", row.lookup_key);
    let delete_id = format!("{id}::delete");
    let toggle_id = format!("{id}::toggle");
    let update_id = format!("{id}::update");
    let hovered = ui.hovered(&id);
    let mut toggled = false;
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
                .border(Border::all(kit.palette.border_muted, 1)),
        );
        match row.icon_url.clone().filter(|url| !url.trim().is_empty()) {
            Some(url) if !kit.images.failed(&format!("remote::{url}")) => {
                let key = format!("remote::{url}");
                kit.image(
                    ui,
                    &format!("{id}::icon"),
                    &key,
                    || ImageSource::Url(url),
                    48.0,
                    48.0,
                    8.0,
                );
            }
            _ => {
                let _box = ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Fixed(48.0))
                        .height(Sizing::Fixed(48.0))
                        .align(ChildAlignment::center())
                        .background(kit.palette.bg_light)
                        .corner_radius(CornerRadius::all(8.0)),
                );
                kit.icon(ui, Icon::Library, 22.0, kit.palette.text_muted);
            }
        }
        {
            let _text = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS)
                    // Never wider than the space left of the actions, so they stay on the card's right edge.
                    .clip(vertex_shell::sturdy::ui::Clip::horizontal()),
            );
            kit::label(
                ui,
                &row.display_name,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Heading)
                },
            );
            {
                let _badges = kit.row(ui, kit::SPACE_SM);
                badge(ui, kit, &row.platform, kit.palette.primary.with_w(0.3));
                if let Some(version) = &row.version {
                    badge(ui, kit, version, kit.palette.bg_light);
                }
                if let Some(update) = &row.update_label {
                    let hovered = ui.hovered(&update_id);
                    {
                        let _update = ui.element(
                            &ElementDesc::new()
                                .id(update_id.clone())
                                .width(Sizing::Fit)
                                .height(Sizing::Fixed(22.0))
                                .padding(Padding::symmetric(8, 0))
                                .align(ChildAlignment::center())
                                .background(kit.palette.success.with_w(if hovered {
                                    0.45
                                } else {
                                    0.28
                                }))
                                .corner_radius(CornerRadius::all(7.0)),
                        );
                        kit.text(ui, TextRole::Small, update);
                    }
                }
                if let Some(warning) = &row.warning {
                    kit.icon(ui, Icon::Warn, 16.0, kit.palette.warning);
                    kit.text(ui, TextRole::Small, warning);
                }
            }
            // Card width minus padding, gaps, the icon and the actions.
            let room = ui.element_bounds(&id).map_or(620.0, |(_, size)| size.x)
                - 2.0 * f32::from(kit::SPACE_MD)
                - 2.0 * f32::from(kit::SPACE_MD)
                - 48.0
                - 100.0;
            kit.text_clipped(
                ui,
                TextRole::Muted,
                &row.description,
                room.clamp(120.0, 900.0),
            );
        }
        {
            // Fit-width, not `kit.row` (which grows): two growing siblings would split the card in half.
            let _actions = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fit)
                    .height(Sizing::Fit)
                    .direction(Direction::LeftToRight)
                    .child_gap(kit::SPACE_SM)
                    .align(ChildAlignment::new(AlignX::Right, AlignY::Center)),
            );
            if is_mods
                && kit
                    .switch(ui, &toggle_id, !row.disabled, !apply_busy)
                    .is_some()
            {
                toggled = true;
            }
            let hovered = ui.hovered(&delete_id);
            let _delete = ui.element(
                &ElementDesc::new()
                    .id(delete_id.clone())
                    .width(Sizing::Fixed(30.0))
                    .height(Sizing::Fixed(30.0))
                    .align(ChildAlignment::center())
                    .background(if hovered {
                        kit.palette.danger.with_w(0.3)
                    } else {
                        kit.palette.bg_light
                    })
                    .corner_radius(CornerRadius::all(8.0)),
            );
            kit.icon(ui, Icon::Trash, 16.0, kit.palette.danger);
        }
    }
    if is_mods {
        kit.tooltip(
            ui,
            &toggle_id,
            &if row.disabled {
                tr!("Enable mod")
            } else {
                tr!("Disable mod")
            },
        );
    }
    kit.tooltip(ui, &delete_id, &tr!("Delete this content"));
    if row.update_label.is_some() {
        kit.tooltip(ui, &update_id, &tr!("Update to this version"));
    }
    if toggled {
        RowAction::Toggle
    } else if !apply_busy && kit.clicked_plain(ui, &delete_id) {
        RowAction::Delete
    } else if row.update_label.is_some() && !apply_busy && kit.clicked_plain(ui, &update_id) {
        RowAction::Update
    } else if kit.clicked(ui, &id)
        && !ui.hovered(&delete_id)
        && !ui.hovered(&update_id)
        && !ui.hovered(&toggle_id)
    {
        RowAction::Open
    } else {
        RowAction::None
    }
}

fn badge(ui: &mut Ui<'_>, kit: &Kit, label: &str, background: Vec4) {
    let _badge = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fit)
            .height(Sizing::Fixed(22.0))
            .padding(Padding::symmetric(8, 0))
            .align(ChildAlignment::center())
            .background(background)
            .corner_radius(CornerRadius::all(7.0)),
    );
    kit.text(ui, TextRole::Small, label);
}

// -- Screenshots ----------------------------------------------------------------------------------------------------

fn screenshots_tab(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    id: &str,
    root: &std::path::Path,
) {
    let displayed_time = state.config.displayed_time().clone();
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    api::screenshots_tick(&mut session.state, root);
    if session.state.screenshot_scan_in_flight {
    }
    let shots = session.state.screenshots.clone();
    kit.text(
        ui,
        TextRole::Heading,
        &tr!("Screenshots ({count})", count = shots.len()),
    );
    if shots.is_empty() {
        kit.text(
            ui,
            TextRole::Muted,
            &tr!("No screenshots found under this instance's screenshots folder."),
        );
        return;
    }
    let width = kit::content_width_in(ui, SCROLL_ID);
    let gap = f32::from(kit::SPACE_MD);
    let columns = kit::masonry_columns(width);
    let tile_width = (width - gap * (columns as f32 - 1.0)) / columns as f32;
    let mut heights = vec![0.0_f32; columns];
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); columns];
    for (index, shot) in shots.iter().enumerate() {
        let shortest = heights
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(column, _)| column);
        heights[shortest] += tile_width * shot.height as f32 / shot.width.max(1) as f32 + gap;
        buckets[shortest].push(index);
    }
    let mut open = None;
    let mut delete = None;
    {
        let _grid = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_MD),
        );
        for bucket in &buckets {
            let _column = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_MD),
            );
            for &index in bucket {
                let shot = &shots[index];
                let tile_id = format!("instance::shot::{index}");
                let height = tile_width * shot.height as f32 / shot.width.max(1) as f32;
                let key = format!("shot::{}::{:?}", shot.path.display(), shot.modified_at_ms);
                if kit.near_viewport(ui, SCROLL_ID, &tile_id, 800.0) {
                    let path = shot.path.clone();
                    kit.image(
                        ui,
                        &tile_id,
                        &key,
                        || ImageSource::File(path),
                        tile_width,
                        height,
                        10.0,
                    );
                } else {
                    drop(
                        ui.element(
                            &ElementDesc::new()
                                .id(tile_id.clone())
                                .width(Sizing::Grow)
                                .height(Sizing::Fixed(height)),
                        ),
                    );
                }
                super::viewer::tile_taken(ui, kit, &tile_id, &shot.path, &displayed_time);
                match super::viewer::tile_actions(ui, kit, &tile_id) {
                    Some(super::viewer::TileAction::Copy) => {
                        launcher_ui::app_commands::push(
                            launcher_ui::app_commands::AppCommand::CopyImage(shot.path.clone()),
                        );
                    }
                    Some(super::viewer::TileAction::Delete) => {
                        delete = Some(shot.path.clone());
                    }
                    None => {
                        if kit.clicked(ui, &tile_id) {
                            open = Some(shot.path.clone());
                        }
                    }
                }
            }
        }
    }
    if let Some(path) = delete {
        state.confirming_screenshot_delete = Some(format!("viewer-confirm::{}", path.display()));
        state.dialog = Some(Dialog::InstanceScreenshot(id.to_owned(), path));
    } else if let Some(path) = open {
        state.dialog = Some(Dialog::InstanceScreenshot(id.to_owned(), path));
    }
}

// -- Logs -----------------------------------------------------------------------------------------------------------

fn logs_tab(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    state: &mut AppState,
    id: &str,
    root: &std::path::Path,
) {
    let Some(session) = state.sessions.get_mut(id) else {
        return;
    };
    api::logs_tick(&mut session.state, root);
    kit.text(ui, TextRole::Heading, &tr!("Logs"));
    kit.text(
        ui,
        TextRole::Muted,
        &tr!("Select a log file to view it with the same highlighting rules as the live console."),
    );
    if session.state.logs.is_empty() {
        kit.text(
            ui,
            TextRole::Muted,
            &tr!("No log files found under this instance's logs folder."),
        );
        return;
    }
    let logs = session.state.logs.clone();
    let selected = session.state.selected_log_path.clone();
    let mut pick = None;
    {
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fixed(520.0))
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_MD),
        );
        {
            let _list = ui.element(
                &ElementDesc::new()
                    .id("instance::logs-list".into())
                    .width(Sizing::Fixed(300.0))
                    .height(Sizing::Grow)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS)
                    .clip(Clip::vertical()),
            );
            for log in &logs {
                let mut label = log.file_name.clone();
                if log.size_bytes > 0 {
                    label.push_str(&format!(
                        " · {} · {}",
                        api::log_size_label(log.size_bytes),
                        api::time_ago(log.modified_at_ms)
                    ));
                }
                if kit.nav_item(
                    ui,
                    &format!("instance::log::{}", log.path.display()),
                    None,
                    &label,
                    selected.as_ref() == Some(&log.path),
                ) {
                    pick = Some(log.path.clone());
                }
            }
        }
        {
            let _viewer = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Grow)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_SM),
            );
            match selected.as_ref() {
                Some(path) => {
                    kit.text(
                        ui,
                        TextRole::Body,
                        &path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    );
                    let mut details = path.display().to_string();
                    if session.state.loaded_log_truncated {
                        details.push_str(&tr!(" | showing last {count} lines", count = api::MAX_LOG_LINES));
                    }
                    kit.text_clipped(ui, TextRole::Small, &details, 640.0);
                    if session.state.log_load_in_flight {
                        kit.text(ui, TextRole::Muted, &tr!("Loading log contents..."));
                    }
                    if let Some(error) = &session.state.loaded_log_error {
                        kit::label(
                            ui,
                            error,
                            &TextStyle {
                                wrap: TextWrap::Words,
                                ..kit.style(TextRole::Danger)
                            },
                        );
                    } else {
                        super::console::log_lines(
                            ui,
                            kit,
                            "instance::log-lines",
                            &session.state.loaded_log_lines,
                            &tr!("Log is empty."),
                        );
                    }
                }
                None => kit.text(
                    ui,
                    TextRole::Muted,
                    &tr!("Select a log file from the left to view it."),
                ),
            }
        }
    }
    if let Some(path) = pick {
        api::select_log(&mut session.state, path);
    }
}
