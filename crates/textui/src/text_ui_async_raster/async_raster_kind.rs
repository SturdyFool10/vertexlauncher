use super::*;

#[derive(Clone, Debug)]
pub(crate) enum AsyncRasterKind {
    Plain(String),
    Rich(Vec<RichSpan>),
}
