#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextGlyphRasterMode {
    Auto,
    AlphaMask,
    Sdf,
    Msdf,
}
