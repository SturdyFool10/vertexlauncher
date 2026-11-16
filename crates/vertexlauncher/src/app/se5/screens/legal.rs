//! Legal: Vertex's own licence and the licences of the fonts and icons it bundles, as an accordion: open a notice to read its full text.

use vertex_i18n::{tr, trn};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use super::super::markdown::Document;

use vertex_shell::sturdy::ui::{
    AlignX, AlignY, ChildAlignment, Clip, CornerRadius, Direction, ElementDesc, Padding,
    Sizing, TextStyle, TextWrap, Ui,
};

use super::super::icons::{self, Icon};
use super::super::kit::{self, Kit, TextRole};

/// One notice as shown: the projects it covers, the licence type(s) they share, and the licence text.
struct Notice {
    names: Vec<String>,
    licenses: Vec<String>,
    source: String,
    text: String,
}

impl Notice {
    /// "Tabler Icons (MIT)", "fmt, spdlog and glm (MIT)", "stb (MIT or Public Domain)"-style heading.
    fn title(&self) -> String {
        let names = launcher_ui::notification::natural_language::join_natural(&self.names);
        if self.licenses.is_empty() {
            names
        } else {
            tr!("{names} ({licenses})", names = names, licenses = launcher_ui::notification::natural_language::join_natural(&self.licenses)).into_owned()
        }
    }
}

/// Vertex's own licence, and its bundled fonts and icons: (projects, licence, file, text).
const BUNDLED: &[(&[&str], &str, &str, &str)] = &[
    (
        &["Vertex Launcher"],
        "GPL-3.0",
        "LICENSE.md",
        include_str!("../../../../../../LICENSE.md"),
    ),
    (
        &["SturdyEngine"],
        "MIT",
        "legal/STURDY_ENGINE_LICENSE.txt",
        include_str!("../../../../../launcher_ui/src/legal/STURDY_ENGINE_LICENSE.txt"),
    ),
    (
        &["Tabler Icons"],
        "MIT",
        "legal/TABLER_LICENSE.txt",
        include_str!("../../../../../launcher_ui/src/legal/TABLER_LICENSE.txt"),
    ),
    (
        &["Maple Mono NF", "Maple Mono NF CN"],
        "SIL OFL 1.1",
        "legal/MAPLE_LICENSE.txt",
        include_str!("../../../../../launcher_ui/src/legal/MAPLE_LICENSE.txt"),
    ),
    (
        &["Noto Color Emoji"],
        "SIL OFL 1.1",
        "legal/NOTO_EMOJI_LICENSE.txt",
        include_str!("../../../../../launcher_ui/src/legal/NOTO_EMOJI_LICENSE.txt"),
    ),
];

/// The licence type(s) a licence text declares, which the engine's own list does not name. An SPDX identifier in the
/// text wins; otherwise the well-known wording of each licence is recognised. Empty when nothing is recognised, so no
/// label is shown rather than a wrong one.
fn detect_licenses(text: &str) -> Vec<String> {
    let lower = normalize(text).to_lowercase();
    let has = |needle: &str| lower.contains(needle);
    if let Some(at) = lower.find("spdx-license-identifier:") {
        let id = &text_after(text, at + "spdx-license-identifier:".len());
        if !id.is_empty() {
            return vec![id.clone()];
        }
    }
    // libtiff's own permissive licence quotes BSD wording; it is one licence, not two.
    if has("libtiff license") {
        return vec!["libtiff".to_owned()];
    }
    let mut found: Vec<&str> = Vec::new();
    if has("apache license") && has("version 2.0") {
        found.push("Apache-2.0");
    }
    if has("permission is hereby granted") {
        found.push("MIT");
    }
    if has("provided 'as-is'") && has("altered source versions") {
        found.push("zlib");
    }
    if has("redistribution and use in source and binary forms") || has("2-clauses bsd") || has("3-clause bsd") {
        if has("2-clause") {
            found.push("BSD 2-Clause");
        } else if has("neither the name") || has("endorse or promote") || has("3-clause") {
            found.push("BSD 3-Clause");
        } else {
            found.push("BSD 2-Clause");
        }
    }
    if has("public domain") || has("unlicense") {
        found.push("Public Domain");
    }
    if has("happy bunny license") {
        found.push("Happy Bunny");
    }
    if has("freetype license") {
        found.push("FreeType License");
    }
    if has("gnu free documentation license") {
        found.push("GNU FDL 1.3");
    }
    if has("boost software license") {
        found.push("BSL 1.0");
    }
    if has("mozilla public license") {
        found.push("MPL 2.0");
    }
    found.into_iter().map(str::to_owned).collect()
}

/// The first word-ish token of `text` starting at byte `from` of its normalized, lowercased form: the identifier after an
/// SPDX tag, kept in the text's own capitalisation.
fn text_after(text: &str, from: usize) -> String {
    // `from` indexes the lowercased, whitespace-collapsed copy, which is as long as the original's collapsed form.
    let collapsed = normalize(text);
    collapsed
        .get(from..)
        .map(|rest| {
            let rest = rest.trim_start();
            let end = rest.find(|c: char| c == '\n' || c == '*').unwrap_or(rest.len());
            rest[..end].split(" Licensed ").next().unwrap_or("").trim().to_owned()
        })
        .unwrap_or_default()
}

/// Vertex's notices plus SturdyEngine5's own list of what it embeds, with projects that share the same licence text
/// folded into one entry so nothing is shown twice.
fn notices() -> &'static [Notice] {
    static NOTICES: OnceLock<Vec<Notice>> = OnceLock::new();
    NOTICES.get_or_init(|| {
        let mut notices: Vec<Notice> = Vec::new();
        let mut add = |names: Vec<String>, licenses: Vec<String>, source: String, text: String| {
            let key = normalize(&text);
            match notices
                .iter_mut()
                .find(|notice| normalize(&notice.text) == key)
            {
                Some(existing) => {
                    for name in names {
                        if !existing.names.iter().any(|known| known.eq_ignore_ascii_case(&name)) {
                            existing.names.push(name);
                        }
                    }
                }
                // Windows line endings would reach the text layout as stray control characters.
                None => notices.push(Notice { names, licenses, source, text: text.replace("\r\n", "\n").replace('\r', "\n") }),
            }
        };
        for (names, license, source, text) in BUNDLED {
            add(
                names.iter().map(|name| (*name).to_owned()).collect(),
                vec![(*license).to_owned()],
                (*source).to_owned(),
                (*text).to_owned(),
            );
        }
        for license in vertex_shell::sturdy::images::third_party_licenses() {
            let licenses = detect_licenses(&license.text);
            let names = license.project.split(", ").map(str::to_owned).collect();
            add(names, licenses, license.license_file, license.text);
        }
        notices
    })
}

/// A licence text with whitespace runs collapsed, so copies that differ only in line wrapping count as the same.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Which notices are open, by index; it survives leaving the screen.
static OPEN: Mutex<BTreeSet<usize>> = Mutex::new(BTreeSet::new());

/// Which notices are shown as Markdown rather than as plain text, by index. Off until switched on, per notice.
static MARKDOWN: Mutex<BTreeSet<usize>> = Mutex::new(BTreeSet::new());

thread_local! {
    /// The parsed document of each notice shown as Markdown (parsed once, on first use; the texts never change).
    static VIEWS: RefCell<HashMap<usize, Document>> = RefCell::new(HashMap::new());
}

pub fn render(ui: &mut Ui<'_>, kit: &mut Kit) {
    let _scroll = kit.scroll(ui, "legal::scroll", kit::SPACE_LG, kit::SPACE_XL);
    kit.text(ui, TextRole::Title, &tr!("Legal"));
    {
        let _header = kit.card(ui, "legal::header", kit::SPACE_SM);
        kit.text(ui, TextRole::Heading, &tr!("Included Notices"));
        kit.separator(ui);
        kit.text(
            ui,
            TextRole::Body,
            &tr!("Open a notice below to read its full legal text."),
        );
        kit.text(ui, TextRole::Small, &trn!(notices().len(), "{count} notice", "{count} notices"));
    }
    for (index, notice) in notices().iter().enumerate() {
        let open = OPEN.lock().is_ok_and(|open| open.contains(&index));
        let card_id = format!("legal::notice::{index}");
        let _card = kit.card(ui, &card_id, kit::SPACE_SM);
        let (progress, eased) = kit.animate_collapse(&card_id, open);
        let header_id = format!("legal::notice::{index}::header");
        let markdown = MARKDOWN.lock().is_ok_and(|on| on.contains(&index));
        let _top = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .child_gap(kit::SPACE_SM)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        );
        {
            let hovered = ui.hovered(&header_id);
            let _header = ui.element(
                &ElementDesc::new()
                    .id(header_id.clone())
                    .width(Sizing::Grow)
                    .height(Sizing::Fixed(34.0))
                    .direction(Direction::LeftToRight)
                    .child_gap(kit::SPACE_SM)
                    .padding(Padding::symmetric(kit::SPACE_SM, 0))
                    .align(ChildAlignment::new(AlignX::Left, AlignY::Center))
                    .background(if hovered {
                        kit.palette.hovered_fill()
                    } else {
                        vertex_shell::sturdy::glam::Vec4::ZERO
                    })
                    .corner_radius(CornerRadius::all(8.0)),
            );
            // Right when closed, a clockwise quarter-turn to down when open (the down chevron turned 270 degrees points
            // right), so both resting states are the plain chevrons.
            icons::draw_turned(
                ui,
                Icon::ChevronDown,
                270.0 + 90.0 * eased,
                16.0,
                kit.palette.text,
                kit.svg_supersample,
            );
            kit::label(
                ui,
                &notice.title(),
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Strong)
                },
            );
        }
        // A radial toggle at the right of the heading, outside the heading so pressing it does not open or close the notice.
        {
            let _toggle = ui.element(&ElementDesc::new().width(Sizing::Fit).height(Sizing::Fixed(34.0)));
            if kit.radio(ui, &format!("legal::notice::{index}::markdown"), &tr!("Enable Markdown"), markdown)
                && let Ok(mut on) = MARKDOWN.lock()
                && !on.remove(&index)
            {
                on.insert(index);
            }
        }
        drop(_top);
        if kit.clicked(ui, &header_id) {
            if let Ok(mut open) = OPEN.lock()
                && !open.remove(&index)
            {
                open.insert(index);
            }
        }
        kit.collapse_body(ui, &card_id, progress, eased, kit::SPACE_SM, |ui, kit| {
            kit::label(
                ui,
                &notice.source,
                &TextStyle {
                    wrap: TextWrap::Words,
                    ..kit.style(TextRole::Small)
                },
            );
            kit.separator(ui);
            let text_id = format!("legal::notice::{index}::text");
            {
                let _text = ui.element(
                    &ElementDesc::new()
                        .id(text_id.clone())
                        .width(Sizing::Grow)
                        .height(Sizing::Fixed(
                            (ui.viewport_size().y * 0.45).clamp(180.0, 480.0),
                        ))
                        .direction(Direction::TopToBottom)
                        .padding(Padding::all(kit::SPACE_MD))
                        .background(kit.palette.bg_dark)
                        .corner_radius(CornerRadius::all(8.0))
                        .clip(Clip::vertical()),
                );
                kit.scrollbar(ui, &text_id);
                if markdown {
                    // The room the text has: the box as last drawn, less its padding and the scrollbar.
                    let width = ui
                        .element_bounds(&text_id)
                        .map_or(480.0, |(_, size)| size.x - 2.0 * f32::from(kit::SPACE_MD) - 14.0);
                    let clicked = VIEWS.with(|views| {
                        views
                            .borrow_mut()
                            .entry(index)
                            .or_insert_with(|| Document::new(&notice.text))
                            .show(ui, kit, &format!("legal::notice::{index}::markdown-view"), width)
                    });
                    if let Some(url) = clicked
                        && let Err(err) = launcher_ui::desktop::open_url(&url)
                    {
                        tracing::warn!(target: "vertexlauncher/legal", "could not open {url}: {err}");
                    }
                } else {
                    kit::label(
                        ui,
                        notice.text.trim(),
                        &TextStyle {
                            wrap: TextWrap::Words,
                            ..kit.style(TextRole::Small)
                        },
                    );
                }
            }
        });
    }
}
