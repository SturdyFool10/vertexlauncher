/// Declares the rendering expectation for a text run.
///
/// This enum is the formal rendering policy described in the typography design
/// document.  It allows call sites to express whether a text run **must** stay
/// on the GPU path, **prefers** the GPU path, or **may** fall back to software
/// rendering when required.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextRenderingPolicy {
    /// Do not use software rendering.  If the requested effect cannot be
    /// represented on the active GPU path, degrade the effect rather than
    /// leaving the GPU path.
    GpuRequired,

    /// Use the GPU path if at all possible.  Allow software fallback only for
    /// hard capability failures.  This is the default for all normal UI text.
    #[default]
    GpuPreferred,

    /// Permissive mode for edge cases, exports, debugging, or rare glyph/effect
    /// paths.
    FallbackAllowed,
}
