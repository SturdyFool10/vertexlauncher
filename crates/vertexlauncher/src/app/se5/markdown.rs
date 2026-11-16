//! Markdown as one document: parsed with pulldown-cmark (CommonMark plus tables, strikethrough and task lists) and
//! laid out as a column of blocks inside whatever scrolls around it, so a whole text scrolls together and every
//! paragraph word-wraps to the width it is given.
//!
//! Prose is wrapped by hand: a run of mixed styles (bold, italic, code, a link) cannot be one wrapping text element,
//! so each word is measured in its own face and the words are dealt into lines, each line a row of styled runs. The
//! lines of a block are worked out once per width and font and kept until either changes.
//!
//! Raw HTML is left out and an image is shown as its alt text.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use vertex_shell::sturdy::glam::Vec4;
use vertex_shell::sturdy::ui::{
    AlignX, AlignY, AttachPoint, Border, ChildAlignment, CornerRadius, Direction, ElementDesc, FloatTarget, Floating,
    Padding, Sizing, TextStyle, TextWrap, Ui,
};

use super::fonts;
use super::kit::{self, Kit, TextRole};

/// Size multipliers of heading levels 1 to 6.
const HEADING_SCALE: [f32; 6] = [1.7, 1.45, 1.25, 1.12, 1.05, 1.0];
/// Indent of each list level and of each quote level, in pixels.
const LIST_INDENT: f32 = 20.0;
const QUOTE_INDENT: u16 = 12;
/// Width of the marker column of a list item.
const MARKER_WIDTH: f32 = 22.0;

/// A piece of inline text and how it is styled.
#[derive(Debug, Clone, PartialEq, Default)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    /// Index into [`Document::links`].
    link: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Heading { level: u8, runs: Vec<Run> },
    Paragraph { runs: Vec<Run>, quote: u8 },
    Item { depth: u8, marker: String, runs: Vec<Run>, quote: u8 },
    Code { text: String },
    Rule,
    Table { header: Vec<Vec<Run>>, rows: Vec<Vec<Vec<Run>>> },
}

/// One line of a wrapped block: runs ready to draw.
type Line = Vec<Run>;

/// What the wrapped lines of a block depend on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LayoutKey {
    width: i32,
    font_id: u16,
    font_size: u16,
}

/// A parsed document and the wrapped lines of its blocks.
pub struct Document {
    blocks: Vec<Block>,
    links: Vec<String>,
    key: Option<LayoutKey>,
    /// Per block: its lines (a table's lines are its cells', row after row, header first).
    lines: Vec<Option<Vec<Line>>>,
}

impl Document {
    pub fn new(source: &str) -> Self {
        let (blocks, links) = parse(source);
        let lines = vec![None; blocks.len()];
        Self { blocks, links, key: None, lines }
    }

    /// Draws the document in a column filling the parent's width. `width` is the room the column has (last frame's
    /// measurement is fine); returns the link clicked this frame, if any.
    pub fn show(&mut self, ui: &mut Ui<'_>, kit: &mut Kit, id: &str, width: f32) -> Option<String> {
        let base = kit.style(TextRole::Small);
        let key = LayoutKey { width: width.round() as i32, font_id: base.font_id, font_size: base.font_size };
        if self.key != Some(key) {
            self.key = Some(key);
            self.lines.iter_mut().for_each(|lines| *lines = None);
        }
        let mut clicked = None;
        let _column = ui.element(
            &ElementDesc::new()
                .id(id.to_owned())
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::TopToBottom)
                .child_gap(10),
        );
        for index in 0..self.blocks.len() {
            let block_id = format!("{id}::{index}");
            if let Some(link) = self.draw_block(ui, kit, &block_id, index, &base, width) {
                clicked = Some(link);
            }
        }
        clicked
    }

    fn draw_block(&mut self, ui: &mut Ui<'_>, kit: &mut Kit, id: &str, index: usize, base: &TextStyle, width: f32) -> Option<String> {
        let block = self.blocks[index].clone();
        let mut clicked = None;
        match block {
            Block::Heading { level, runs } => {
                let scale = HEADING_SCALE[usize::from(level.saturating_sub(1)).min(5)];
                let style = TextStyle { font_size: (f32::from(base.font_size) * scale).round() as u16, ..*base };
                let lines = self.wrapped(ui, kit, index, &runs, &style, width, true);
                clicked = draw_lines(ui, kit, id, lines, &style, true, &self.links);
            }
            Block::Paragraph { runs, quote } => {
                let inner = width - f32::from(QUOTE_INDENT) * f32::from(quote);
                let lines = self.wrapped(ui, kit, index, &runs, base, inner, false);
                let _quote = quoted(ui, kit, quote);
                clicked = draw_lines(ui, kit, id, lines, base, false, &self.links);
            }
            Block::Item { depth, marker, runs, quote } => {
                let indent = LIST_INDENT * f32::from(depth);
                let inner = width - f32::from(QUOTE_INDENT) * f32::from(quote) - indent - MARKER_WIDTH;
                let lines = self.wrapped(ui, kit, index, &runs, base, inner, false);
                let _quote = quoted(ui, kit, quote);
                let _row = ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Grow)
                        .height(Sizing::Fit)
                        .direction(Direction::LeftToRight)
                        .padding(Padding { left: indent as u16, ..Padding::default() }),
                );
                {
                    let _marker = ui.element(&ElementDesc::new().width(Sizing::Fixed(MARKER_WIDTH)).height(Sizing::Fit));
                    kit::label(ui, &marker, &TextStyle { color: kit.palette.text_muted, ..*base });
                }
                clicked = draw_lines(ui, kit, id, lines, base, false, &self.links);
            }
            Block::Code { text } => {
                let _code = ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Grow)
                        .height(Sizing::Fit)
                        .padding(Padding::all(10))
                        .background(kit.palette.bg_light)
                        .corner_radius(CornerRadius::all(6.0)),
                );
                kit::wrapped_label(
                    ui,
                    text.trim_end(),
                    &TextStyle { font_id: kit.style(TextRole::Code).font_id, wrap: TextWrap::Words, ..*base },
                    (width - 20.0).max(40.0),
                );
            }
            Block::Rule => {
                drop(ui.element(
                    &ElementDesc::new().width(Sizing::Grow).height(Sizing::Fixed(1.0)).background(kit.palette.border_muted),
                ));
            }
            Block::Table { header, rows } => {
                let columns = header.len().max(1);
                let cell_width = (width / columns as f32 - 16.0).max(40.0);
                let _table = ui.element(
                    &ElementDesc::new()
                        .width(Sizing::Grow)
                        .height(Sizing::Fit)
                        .direction(Direction::TopToBottom)
                        .border(Border { color: kit.palette.border_muted, left: 1, right: 1, top: 1, bottom: 1, between_children: 1 }),
                );
                let mut cell = 0usize;
                // Wrapped per cell: laid out here rather than cached, tables are small.
                for (row_index, row) in std::iter::once(&header).chain(rows.iter()).enumerate() {
                    let _row = ui.element(
                        &ElementDesc::new()
                            .width(Sizing::Grow)
                            .height(Sizing::Fit)
                            .direction(Direction::LeftToRight)
                            .background(if row_index == 0 { kit.palette.bg_light } else { Vec4::ZERO })
                            .border(Border { color: kit.palette.border_muted, between_children: 1, ..Border::default() }),
                    );
                    for runs in row {
                        let _cell = ui.element(
                            &ElementDesc::new().width(Sizing::Grow).height(Sizing::Fit).padding(Padding::symmetric(8, 4)),
                        );
                        let lines = flow(ui, kit, runs, base, cell_width, row_index == 0);
                        if let Some(link) = draw_lines(ui, kit, &format!("{id}::c{cell}"), lines, base, row_index == 0, &self.links) {
                            clicked = Some(link);
                        }
                        cell += 1;
                    }
                }
            }
        }
        clicked
    }

    /// The lines of block `index`, from the cache or worked out now.
    #[allow(clippy::too_many_arguments)]
    fn wrapped(&mut self, ui: &Ui<'_>, kit: &Kit, index: usize, runs: &[Run], style: &TextStyle, width: f32, bold: bool) -> Vec<Line> {
        if let Some(lines) = &self.lines[index] {
            return lines.clone();
        }
        let lines = flow(ui, kit, runs, style, width, bold);
        self.lines[index] = Some(lines.clone());
        lines
    }
}

/// The left bar and indent of a quote, as a scope; `None` outside quotes.
fn quoted(ui: &mut Ui<'_>, kit: &Kit, depth: u8) -> Option<vertex_shell::sturdy::ui::ElementScope> {
    (depth > 0).then(|| {
        ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .padding(Padding { left: QUOTE_INDENT * u16::from(depth), ..Padding::default() })
                .border(Border { color: kit.palette.primary, left: 3, ..Border::default() }),
        )
    })
}

/// The font a run is drawn in: the bold, italic or bold italic face of the text role when the font has one, the
/// code role's font for code, else the base font.
fn font_of(kit: &Kit, base: &TextStyle, run: &Run, force_bold: bool) -> u16 {
    if run.code {
        return kit.style(TextRole::Code).font_id;
    }
    let bold = run.bold || force_bold;
    let variant = match (bold, run.italic) {
        (false, false) => return base.font_id,
        (true, false) => 0,
        (false, true) => 1,
        (true, true) => 2,
    };
    match fonts::styled_slot(base.font_id) {
        Some(slot) if kit.styled_faces[slot][variant] => fonts::styled_face_id(slot, variant),
        _ => base.font_id,
    }
}

/// Deals the words of `runs` into lines no wider than `width`, measuring each word in the face it is drawn with.
fn flow(ui: &Ui<'_>, kit: &Kit, runs: &[Run], base: &TextStyle, width: f32, bold: bool) -> Vec<Line> {
    let width = width.max(40.0);
    let mut lines: Vec<Line> = vec![Vec::new()];
    let mut used = 0.0_f32;
    for run in runs {
        let style = TextStyle { font_id: font_of(kit, base, run, bold), ..*base };
        for (n, part) in run.text.split('\n').enumerate() {
            if n > 0 {
                trim_end(lines.last_mut().expect("a line is always open"));
                lines.push(Vec::new());
                used = 0.0;
            }
            for word in part.split_inclusive(' ') {
                let size = ui.measure_text(&style, word).x;
                let line = lines.last_mut().expect("a line is always open");
                if used + size > width && !line.is_empty() {
                    trim_end(line);
                    lines.push(Vec::new());
                    used = 0.0;
                }
                let line = lines.last_mut().expect("a line is always open");
                // A word that continues the run before it on this line joins it.
                match line.last_mut() {
                    Some(last) if same_style(last, run) => last.text.push_str(word),
                    _ => line.push(Run { text: word.to_owned(), ..run.clone() }),
                }
                used += size;
            }
        }
    }
    trim_end(lines.last_mut().expect("a line is always open"));
    lines
}

fn same_style(a: &Run, b: &Run) -> bool {
    (a.bold, a.italic, a.code, a.strike, a.link) == (b.bold, b.italic, b.code, b.strike, b.link)
}

/// Drops the space a line ends in, which is not drawn and must not count towards its width.
fn trim_end(line: &mut Line) {
    if let Some(last) = line.last_mut() {
        let trimmed = last.text.trim_end_matches(' ').len();
        last.text.truncate(trimmed);
        if last.text.is_empty() {
            line.pop();
        }
    }
}

/// Draws wrapped lines, one row each; returns the link clicked, if any.
fn draw_lines(ui: &mut Ui<'_>, kit: &mut Kit, id: &str, lines: Vec<Line>, base: &TextStyle, bold: bool, links: &[String]) -> Option<String> {
    let mut clicked = None;
    let _lines = ui.element(&ElementDesc::new().width(Sizing::Grow).height(Sizing::Fit).direction(Direction::TopToBottom));
    for (line_index, line) in lines.iter().enumerate() {
        let _row = ui.element(
            &ElementDesc::new()
                .width(Sizing::Grow)
                .height(Sizing::Fit)
                .direction(Direction::LeftToRight)
                .align(ChildAlignment::new(AlignX::Left, AlignY::Center)),
        );
        if line.is_empty() {
            // A blank line (a hard break twice) keeps its height.
            kit::label(ui, " ", base);
        }
        for (run_index, run) in line.iter().enumerate() {
            let run_id = format!("{id}::{line_index}::{run_index}");
            let style = TextStyle {
                color: if run.link.is_some() { kit.palette.primary } else { base.color },
                font_id: font_of(kit, base, run, bold),
                ..*base
            };
            let wrapper = ElementDesc::new()
                .id(run_id.clone())
                .width(Sizing::Fit)
                .height(Sizing::Fit)
                .background(if run.code { kit.palette.bg_light } else { Vec4::ZERO })
                .corner_radius(CornerRadius::all(3.0));
            let _run = ui.element(&wrapper);
            kit::label(ui, &run.text, &style);
            if run.link.is_some() || run.strike {
                let at = if run.strike { AttachPoint::LeftCenter } else { AttachPoint::LeftBottom };
                let _bar = ui.element(
                    &ElementDesc::new()
                        .id(format!("{run_id}::bar"))
                        .width(Sizing::Percent(1.0))
                        .height(Sizing::Fixed(1.0))
                        .background(style.color)
                        .floating(Some(Floating {
                            element_attach_point: at,
                            parent_attach_point: at,
                            capture_pointer: false,
                            ..Floating::new(FloatTarget::Parent)
                        })),
                );
            }
            if let Some(link) = run.link
                && kit.clicked_plain(ui, &run_id)
            {
                clicked = links.get(link).cloned();
            }
        }
    }
    clicked
}

/// Parses `source` into blocks and the link targets they refer to.
fn parse(source: &str) -> (Vec<Block>, Vec<String>) {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut blocks = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    let (mut bold, mut italic, mut strike) = (0u32, 0u32, 0u32);
    let mut link: Option<usize> = None;
    let mut heading: Option<u8> = None;
    let mut quote = 0u8;
    // One entry per open list: the number the next item gets, or None for a bullet list.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut marker: Option<String> = None;
    let mut code: Option<String> = None;
    let mut table: Option<(Vec<Vec<Run>>, Vec<Vec<Vec<Run>>>)> = None;
    let mut row: Vec<Vec<Run>> = Vec::new();
    let mut in_head = false;

    // Closes the inline content gathered so far as the block it belongs to.
    fn flush(
        blocks: &mut Vec<Block>,
        runs: &mut Vec<Run>,
        heading: Option<u8>,
        marker: &mut Option<String>,
        depth: usize,
        quote: u8,
    ) {
        if runs.iter().all(|run| run.text.trim().is_empty()) {
            runs.clear();
            return;
        }
        let runs = std::mem::take(runs);
        match (heading, marker.take()) {
            (Some(level), _) => blocks.push(Block::Heading { level, runs }),
            (None, Some(marker)) => blocks.push(Block::Item { depth: depth.saturating_sub(1) as u8, marker, runs, quote }),
            (None, None) => blocks.push(Block::Paragraph { runs, quote }),
        }
    }

    let push = |runs: &mut Vec<Run>, text: &str, code: bool, bold: u32, italic: u32, strike: u32, link: Option<usize>| {
        runs.push(Run { text: text.to_owned(), bold: bold > 0, italic: italic > 0, code, strike: strike > 0, link });
    };

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut runs, heading.take(), &mut marker, lists.len(), quote);
            }
            Event::End(TagEnd::Paragraph) => flush(&mut blocks, &mut runs, None, &mut marker, lists.len(), quote),
            Event::Start(Tag::BlockQuote(_)) => quote = quote.saturating_add(1),
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::List(first)) => {
                flush(&mut blocks, &mut runs, None, &mut marker, lists.len(), quote);
                lists.push(first);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut runs, None, &mut marker, lists.len(), quote);
                marker = Some(match lists.last_mut() {
                    Some(Some(next)) => {
                        let text = format!("{next}.");
                        *next += 1;
                        text
                    }
                    _ => "\u{2022}".to_owned(),
                });
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut runs, None, &mut marker, lists.len(), quote),
            Event::TaskListMarker(checked) => marker = Some(if checked { "[x]" } else { "[ ]" }.to_owned()),
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut blocks, &mut runs, None, &mut marker, lists.len(), quote);
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(text) = code.take() {
                    blocks.push(Block::Code { text });
                }
            }
            Event::Rule => blocks.push(Block::Rule),
            Event::Start(Tag::Table(_)) => table = Some((Vec::new(), Vec::new())),
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                if let Some((header, _)) = table.as_mut() {
                    *header = std::mem::take(&mut row);
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some((_, rows)) = table.as_mut() {
                    rows.push(std::mem::take(&mut row));
                }
            }
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut runs)),
            Event::End(TagEnd::Table) => {
                if let Some((header, rows)) = table.take() {
                    blocks.push(Block::Table { header, rows });
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => {
                links.push(dest_url.to_string());
                link = Some(links.len() - 1);
            }
            Event::End(TagEnd::Link) => link = None,
            Event::Text(text) => match code.as_mut() {
                Some(code) => code.push_str(&text),
                None => push(&mut runs, &text, false, bold, italic, strike, link),
            },
            Event::Code(text) => push(&mut runs, &text, true, bold, italic, strike, link),
            Event::SoftBreak => push(&mut runs, " ", false, bold, italic, strike, link),
            Event::HardBreak => push(&mut runs, "\n", false, bold, italic, strike, link),
            _ => {}
        }
    }
    let _ = in_head;
    flush(&mut blocks, &mut runs, None, &mut marker, lists.len(), quote);
    (blocks, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_and_inline_styles() {
        let (blocks, links) = parse("# Title\n\nSome **bold** and *italic* and `code` and [a link](https://example.com).");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }));
        let Block::Paragraph { runs, .. } = &blocks[1] else { panic!("{blocks:?}") };
        assert!(runs.iter().any(|run| run.bold && run.text == "bold"));
        assert!(runs.iter().any(|run| run.italic && run.text == "italic"));
        assert!(runs.iter().any(|run| run.code && run.text == "code"));
        assert_eq!(links, ["https://example.com"]);
        assert!(runs.iter().any(|run| run.link == Some(0) && run.text == "a link"));
    }

    #[test]
    fn lists_nest_and_number() {
        let (blocks, _) = parse("- one\n- two\n  1. nested\n  2. again\n\n3. third\n");
        let items: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Item { depth, marker, runs, .. } => Some((*depth, marker.clone(), runs.iter().map(|r| r.text.as_str()).collect::<String>())),
                _ => None,
            })
            .collect();
        assert_eq!(items[0], (0, "\u{2022}".to_owned(), "one".to_owned()));
        assert_eq!(items[1], (0, "\u{2022}".to_owned(), "two".to_owned()));
        assert_eq!(items[2], (1, "1.".to_owned(), "nested".to_owned()));
        assert_eq!(items[3], (1, "2.".to_owned(), "again".to_owned()));
        assert_eq!(items[4].1, "3.");
    }

    #[test]
    fn code_rules_quotes_and_tables() {
        let (blocks, _) = parse("> quoted\n\n---\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(matches!(&blocks[0], Block::Paragraph { quote: 1, .. }));
        assert!(matches!(&blocks[1], Block::Rule));
        assert!(matches!(&blocks[2], Block::Code { text } if text.contains("fn main")));
        let Block::Table { header, rows } = &blocks[3] else { panic!("{blocks:?}") };
        assert_eq!((header.len(), rows.len(), rows[0].len()), (2, 1, 2));
    }

    #[test]
    fn task_items_and_strikethrough() {
        let (blocks, _) = parse("- [x] done\n- [ ] todo ~~old~~\n");
        let markers: Vec<_> = blocks
            .iter()
            .filter_map(|block| if let Block::Item { marker, .. } = block { Some(marker.as_str()) } else { None })
            .collect();
        assert_eq!(markers, ["[x]", "[ ]"]);
    }
}
