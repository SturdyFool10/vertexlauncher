use super::*;

#[derive(Clone, Debug)]
pub enum TextMarkdownBlock {
    Heading {
        level: TextMarkdownHeadingLevel,
        text: String,
    },
    Paragraph(String),
    Code {
        language: Option<String>,
        text: String,
    },
}
