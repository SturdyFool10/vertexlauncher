/// Result of a text truncation operation, providing both the display string
/// (with an ellipsis appended) and the raw, untruncated original text.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TruncatedText {
    /// The text as it should be rendered, with an ellipsis appended when
    /// the original text was too wide.
    pub display: String,
    /// The original, unmodified text before truncation.
    pub raw: String,
    /// `true` when the text was shortened and an ellipsis was appended.
    pub was_truncated: bool,
}
