#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextPathError {
    EmptyPath,
    PathTooShort,
    EmptyText,
}
