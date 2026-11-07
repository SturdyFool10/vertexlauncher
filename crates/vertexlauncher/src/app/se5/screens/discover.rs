//! Discover: search Modrinth and CurseForge modpacks, open one, and create an instance from any of its versions.
//! The search, filtering, paging and install logic is `launcher_ui`'s `discover_api`; this draws it.


use launcher_ui::screens::{AppScreen, discover_api as api};
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, Border, ChildAlignment, CornerRadius, Direction, ElementDesc, Padding,
    Sizing, TextStyle, TextWrap, Ui,
};

use super::super::AppState;
use super::super::icons::Icon;
use super::super::images::ImageSource;
use super::super::kit::{self, ButtonKind, Kit, TextRole};

const SCROLL_ID: &str = "discover::scroll";
const LOAD_MORE_MARGIN: f32 = 700.0;

pub fn browse(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    api::tick(&mut state.discover);
    let _scroll = kit.scroll(ui, SCROLL_ID, kit::SPACE_LG, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, "Discover");
    filters(ui, kit, state);

    let status = api::status(&state.discover);
    if let Some(message) = &status.message {
        kit.text_wrapped(ui, TextRole::Muted, message);
    }
    for warning in &status.warnings {
        kit::label(
            ui,
            warning,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Danger)
            },
        );
    }
    if status.searching {
        kit.text_wrapped(ui, TextRole::Muted, "Loading modpacks...");
    }

    let entries = api::entries(&state.discover);
    if entries.is_empty() && !status.searching {
        kit.text_wrapped(
            ui,
            TextRole::Muted,
            "No modpacks matched the current search and filters.",
        );
        return;
    }

    let width = kit::content_width_in(ui, SCROLL_ID);
    let columns = kit::masonry_columns(width);
    // Exact column widths: columns that merely grow can be pushed past the right edge by their widest content.
    let column_width =
        ((width - f32::from(kit::SPACE_LG) * (columns as f32 - 1.0)) / columns as f32 - 6.0)
            .max(120.0);
    // Greedy masonry: each tile goes to the currently shortest column, by an estimate from its text length.
    let mut heights = vec![0.0_f32; columns];
    let mut buckets: Vec<Vec<&api::EntryView>> = vec![Vec::new(); columns];
    for entry in &entries {
        let shortest = heights
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(index, _)| index);
        heights[shortest] +=
            150.0 + (entry.summary.len() as f32 / 48.0).ceil().clamp(2.0, 6.0) * 18.0;
        buckets[shortest].push(entry);
    }
    let mut opened = None;
    {
        let _grid = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_LG),
        );
        for (column, bucket) in buckets.iter().enumerate() {
            let _column = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(column_width))
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_LG),
            );
            for entry in bucket {
                if tile(ui, kit, entry, column, column_width) {
                    opened = Some(entry.key.clone());
                }
            }
        }
    }
    if let Some(key) = opened
        && api::open_detail(&mut state.discover, &key)
    {
        state.open(AppScreen::DiscoverDetail);
    }
    if let Some(metrics) = ui.scroll_metrics(SCROLL_ID)
        && (-metrics.offset.y) + metrics.container_size.y
            >= metrics.content_size.y - LOAD_MORE_MARGIN
    {
        api::load_more(&mut state.discover);
    }
}

fn filters(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    let _card = kit.card(ui, "discover::filters", kit::SPACE_MD);
    let mut submit = false;
    let mut as_tag = false;
    {
        let _row = kit.row(ui, kit::SPACE_SM);
        let result = kit.text_field_ex(
            ui,
            "discover::query",
            api::query_mut(&mut state.discover),
            "Search modpacks and press Enter",
            false,
            Sizing::Grow,
        );
        if result.submitted {
            submit = true;
            as_tag = ui.engine().input().modifiers().shift;
        }
        if kit.button(ui, "discover::search", "Search", ButtonKind::Primary, true) {
            submit = true;
        }
    }
    if submit {
        api::submit(&mut state.discover, as_tag);
    }
    if !api::tags(&state.discover).is_empty() {
        let tags: Vec<String> = api::tags(&state.discover).to_vec();
        let remove = kit.chip_rows(ui, "discover::tags", &tags);
        if let Some(index) = remove {
            api::remove_tag(&mut state.discover, index);
        }
    }
    let current = api::filters(&state.discover);
    let (mut provider, mut loader, mut sort, mut game_version) = (
        current.provider,
        current.loader,
        current.sort,
        current.game_version,
    );
    {
        let _row = kit.row(ui, kit::SPACE_MD);
        if let Some(index) = kit.dropdown_grow(ui, "discover::provider", provider, &current.providers) {
            provider = index;
        }
        if let Some(index) = kit.dropdown_grow(ui, "discover::loader", loader, &current.loaders) {
            loader = index;
        }
        if let Some(index) = kit.dropdown_grow(ui, "discover::sort", sort, &current.sorts) {
            sort = index;
        }
        if let Some(index) =
            kit.dropdown_grow(ui, "discover::game-version", game_version, &current.game_versions)
        {
            game_version = index;
        }
    }
    api::set_filters(&mut state.discover, provider, loader, sort, game_version);
}

/// One result card. Returns true when it was clicked.
fn tile(ui: &mut Ui<'_>, kit: &mut Kit, entry: &api::EntryView, column: usize, width: f32) -> bool {
    let id = format!("discover::tile::{}", entry.key);
    let hovered = ui.hovered(&id);
    // Room for text inside the card: its padding and border come off the fixed width.
    let inner = width - f32::from(kit::SPACE_MD) * 2.0 - 2.0;
    let titles_width = inner - 56.0 - f32::from(kit::SPACE_MD);
    {
        let _card = ui.element(
            &ElementDesc::new()
                .id(id.clone())
                // A set width, growing only downwards; nothing inside may widen it.
                .width(Sizing::Fixed(width))
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(kit::SPACE_SM)
                .padding(Padding::all(kit::SPACE_MD))
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
        {
            let _head = kit.row(ui, kit::SPACE_MD);
            project_icon(ui, kit, &format!("{id}::icon"), entry, 56.0, SCROLL_ID);
            let _titles = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Fixed(titles_width))
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(2),
            );
            kit::wrapped_label(ui, &entry.name, &kit.style(TextRole::Heading), titles_width);
            if let Some(author) = &entry.author {
                kit::wrapped_label(
                    ui,
                    &format!("by {author}"),
                    &kit.style(TextRole::Muted),
                    titles_width,
                );
            }
        }
        kit::wrapped_label(ui, &entry.summary, &kit.style(TextRole::Body), inner);
        badges(ui, kit, &entry.sources, inner);
        if let Some(downloads) = &entry.downloads {
            kit::wrapped_label(
                ui,
                &format!("Downloads: {downloads}"),
                &kit.style(TextRole::Muted),
                inner,
            );
        }
        if let Some(updated) = &entry.updated {
            kit::wrapped_label(
                ui,
                &format!("Updated: {updated}"),
                &kit.style(TextRole::Muted),
                inner,
            );
        }
    }
    let _ = column;
    kit.clicked(ui, &id)
}

/// Source badges in rows that break before one would cross `max_width`, so they wrap instead of overflowing.
fn badges(ui: &mut Ui<'_>, kit: &Kit, sources: &[&str], max_width: f32) {
    let style = kit.style(TextRole::Small);
    let gap = f32::from(kit::SPACE_SM);
    let mut rows: Vec<Vec<&str>> = vec![Vec::new()];
    let mut used = 0.0_f32;
    for source in sources {
        let width = ui.measure_text(&style, source).x + 16.0;
        let row = rows.last_mut().expect("rows is never empty");
        if !row.is_empty() && used + gap + width > max_width {
            rows.push(vec![*source]);
            used = width;
        } else {
            used += if row.is_empty() { width } else { gap + width };
            row.push(*source);
        }
    }
    let _column = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fixed(max_width))
            .height(Sizing::Fit)
            .direction(Direction::TopToBottom)
            .child_gap(kit::SPACE_SM),
    );
    for row in rows.iter().filter(|row| !row.is_empty()) {
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_SM),
        );
        for source in row {
            badge(ui, kit, source, max_width);
        }
    }
}

fn badge(ui: &mut Ui<'_>, kit: &Kit, label: &str, max_width: f32) {
    let _badge = ui.element(
        &ElementDesc::new()
            .width(Sizing::Fit)
            .height(Sizing::Fixed(24.0))
            .padding(Padding::symmetric(8, 0))
            .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
            .background(kit.palette.bg_light)
            .corner_radius(CornerRadius::all(8.0)),
    );
    // One line (the badge is a fixed-height pill); a label too long for the card on its own is shortened instead.
    kit.text_clipped(ui, TextRole::Small, label, max_width - 16.0);
}

/// A project's icon from its URL, or the Discover glyph when it has none (or while it loads).
fn project_icon(
    ui: &mut Ui<'_>,
    kit: &mut Kit,
    id: &str,
    entry: &api::EntryView,
    size: f32,
    scroll_id: &str,
) {
    match entry.icon_url.clone().filter(|url| !url.trim().is_empty()) {
        Some(url) if kit.near_viewport(ui, scroll_id, id, 600.0) => {
            let key = format!("remote::{url}");
            if kit.images.failed(&key) {
                placeholder(ui, kit, id, size);
            } else {
                kit.image(ui, id, &key, || ImageSource::Url(url), size, size, 10.0);
            }
        }
        _ => placeholder(ui, kit, id, size),
    }
}

fn placeholder(ui: &mut Ui<'_>, kit: &Kit, id: &str, size: f32) {
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

pub fn detail(ui: &mut Ui<'_>, kit: &mut Kit, state: &mut AppState) {
    if api::detail_tick(&mut state.discover) {
    }
    let _scroll = kit.scroll(ui, "discover-detail::scroll", kit::SPACE_LG, kit::SPACE_XL);
    let Some(view) = api::detail(&state.discover) else {
        kit.text_wrapped(ui, TextRole::Muted, "No modpack selected.");
        if kit.button(
            ui,
            "discover-detail::back",
            "Back to Discover",
            ButtonKind::Neutral,
            true,
        ) {
            state.open(AppScreen::Discover);
        }
        return;
    };
    {
        let _row = kit.row(ui, kit::SPACE_SM);
        if kit.button(
            ui,
            "discover-detail::back",
            "Back to Discover",
            ButtonKind::Neutral,
            true,
        ) {
            state.open(AppScreen::Discover);
        }
        if view.sources.len() > 1
            && let Some(index) = kit.dropdown(
                ui,
                "discover-detail::source",
                view.selected_source,
                &view.sources,
                180.0,
            )
        {
            api::detail_select_source(&mut state.discover, index);
        }
    }

    {
        let _card = kit.card(ui, "discover-detail::card", kit::SPACE_SM);
        {
            let _head = kit.row(ui, kit::SPACE_LG);
            project_icon(
                ui,
                kit,
                "discover-detail::icon",
                &view.entry,
                96.0,
                "discover-detail::scroll",
            );
            let _titles = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(kit::SPACE_XS),
            );
            kit::label(
                ui,
                &view.entry.name,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Title)
                },
            );
            if let Some(author) = &view.entry.author {
                kit::label(
                    ui,
                    &format!("by {author}"),
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Muted)
                    },
                );
            }
        }
        kit::label(
            ui,
            &view.entry.summary,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Body)
            },
        );
        if let Some(url) = &view.project_url
            && kit.button(
                ui,
                "discover-detail::project",
                "Open project page",
                ButtonKind::Neutral,
                true,
            )
            && let Err(err) = launcher_ui::desktop::open_url(url)
        {
            launcher_ui::notification::error!("discover", "Could not open the browser: {}", err);
        }
    }

    if let Some(error) = &view.install_error {
        kit::label(
            ui,
            error,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Danger)
            },
        );
    }
    if view.install_in_flight {
        kit.text_wrapped(
            ui,
            TextRole::Muted,
            view.install_message
                .as_deref()
                .unwrap_or("Installing modpack..."),
        );
        kit.progress(
            ui,
            "discover-detail::install",
            view.install_progress,
            kit::uptime(),
        );
    }

    if view.versions_loading {
        kit.text_wrapped(ui, TextRole::Muted, "Loading modpack versions...");
        return;
    }
    if let Some(error) = &view.versions_error {
        kit::label(
            ui,
            error,
            &TextStyle {
                wrap: TextWrap::Words,
                ..kit.style(TextRole::Danger)
            },
        );
        return;
    }
    let mut install = None;
    for version in &view.versions {
        let _card = kit.card(
            ui,
            &format!("discover-detail::version::{}", version.id),
            kit::SPACE_SM,
        );
        let _row = kit.row(ui, kit::SPACE_MD);
        {
            let _info = ui.element(
                &ElementDesc::new()
                    .width(Sizing::Grow)
                    .height(Sizing::Fit)
                    .direction(Direction::TopToBottom)
                    .child_gap(2),
            );
            kit::label(
                ui,
                &version.name,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Heading)
                },
            );
            if let Some(published) = &version.published {
                kit.text_wrapped(ui, TextRole::Muted, &format!("Published: {published}"));
            }
            if !version.loaders.is_empty() {
                kit.text_wrapped(
                    ui,
                    TextRole::Muted,
                    &format!("Loaders: {}", version.loaders.join(", ")),
                );
            }
            if !version.game_versions.is_empty() {
                let preview = version
                    .game_versions
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                kit.text_wrapped(ui, TextRole::Muted, &format!("Game versions: {preview}"));
            }
            if let Some(downloads) = &version.downloads {
                kit::label(
                    ui,
                    &format!("Downloads: {downloads}"),
                    &TextStyle {
                        wrap: TextWrap::Words,
                        ..kit.style(TextRole::Muted)
                    },
                );
            }
        }
        if kit.button(
            ui,
            &format!("discover-detail::create::{}", version.id),
            "Create Instance",
            ButtonKind::Primary,
            !view.install_in_flight,
        ) {
            install = Some(version.id.clone());
        }
    }
    if let Some(version_id) = install
        && let Some(request) = api::detail_install(&mut state.discover, &version_id)
    {
        state.start_discover_install(request);
    }
}
