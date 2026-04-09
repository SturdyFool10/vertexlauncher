use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VectorPathCommand {
    MoveTo(TextPoint),
    LineTo(TextPoint),
    QuadTo(TextPoint, TextPoint),
    CurveTo(TextPoint, TextPoint, TextPoint),
    Close,
}
