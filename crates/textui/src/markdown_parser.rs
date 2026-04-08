use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, Options as MdOptions, Parser, Tag, TagEnd,
};

use crate::{TextMarkdownBlock, TextMarkdownHeadingLevel};

pub(super) fn parse_markdown_blocks(markdown: &str) -> Vec<TextMarkdownBlock> {
    let parser = Parser::new_ext(markdown, MdOptions::all());

    let mut blocks = Vec::new();
    let mut text_buf = String::new();
    let mut current_heading: Option<HeadingLevel> = None;
    let mut in_code_block = false;
    let mut current_code_language: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                text_buf.clear();
                current_heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = current_heading.take() {
                    if !text_buf.trim().is_empty() {
                        blocks.push(TextMarkdownBlock::Heading {
                            level: text_markdown_heading_level(level),
                            text: text_buf.trim().to_owned(),
                        });
                    }
                    text_buf.clear();
                }
            }
            Event::Start(Tag::Paragraph) => {
                text_buf.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                if !text_buf.trim().is_empty() {
                    blocks.push(TextMarkdownBlock::Paragraph(text_buf.trim().to_owned()));
                }
                text_buf.clear();
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                text_buf.clear();
                current_code_language = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let token = lang.split_whitespace().next().unwrap_or_default();
                        if token.is_empty() {
                            None
                        } else {
                            Some(token.to_owned())
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(TextMarkdownBlock::Code {
                    language: current_code_language.take(),
                    text: text_buf.clone(),
                });
                text_buf.clear();
                in_code_block = false;
            }
            Event::Text(text) | Event::Code(text) => {
                text_buf.push_str(&text);
            }
            Event::SoftBreak | Event::HardBreak => {
                text_buf.push('\n');
            }
            Event::Start(Tag::Item) => {
                if !in_code_block {
                    if !text_buf.is_empty() {
                        text_buf.push('\n');
                    }
                    text_buf.push_str("- ");
                }
            }
            Event::Rule => {
                if !text_buf.trim().is_empty() {
                    blocks.push(TextMarkdownBlock::Paragraph(text_buf.trim().to_owned()));
                }
                text_buf.clear();
                blocks.push(TextMarkdownBlock::Paragraph("---".to_owned()));
            }
            _ => {}
        }
    }

    if !text_buf.trim().is_empty() {
        if in_code_block {
            blocks.push(TextMarkdownBlock::Code {
                language: current_code_language,
                text: text_buf,
            });
        } else if let Some(level) = current_heading {
            blocks.push(TextMarkdownBlock::Heading {
                level: text_markdown_heading_level(level),
                text: text_buf,
            });
        } else {
            blocks.push(TextMarkdownBlock::Paragraph(text_buf));
        }
    }

    blocks
}

fn text_markdown_heading_level(level: HeadingLevel) -> TextMarkdownHeadingLevel {
    match level {
        HeadingLevel::H1 => TextMarkdownHeadingLevel::H1,
        HeadingLevel::H2 => TextMarkdownHeadingLevel::H2,
        HeadingLevel::H3 => TextMarkdownHeadingLevel::H3,
        HeadingLevel::H4 => TextMarkdownHeadingLevel::H4,
        HeadingLevel::H5 => TextMarkdownHeadingLevel::H5,
        HeadingLevel::H6 => TextMarkdownHeadingLevel::H6,
    }
}
