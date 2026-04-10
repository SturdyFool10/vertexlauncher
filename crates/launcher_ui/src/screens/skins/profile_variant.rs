use super::*;

/// Parses a Mojang skin-variant string into the launcher's preview variant.
///
/// Any value other than ASCII-case-insensitive `"slim"` is treated as `Classic`.
/// This function does not panic.
pub(super) fn parse_variant(raw: &str) -> MinecraftSkinVariant {
    if raw.eq_ignore_ascii_case("slim") {
        MinecraftSkinVariant::Slim
    } else {
        MinecraftSkinVariant::Classic
    }
}
