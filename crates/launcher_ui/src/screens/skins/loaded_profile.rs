use super::*;

#[derive(Clone, Debug)]
pub(super) struct LoadedProfile {
    pub(super) player_name: String,
    pub(super) active_skin_png: Option<Vec<u8>>,
    pub(super) skin_variant: MinecraftSkinVariant,
    pub(super) capes: Vec<CapeChoice>,
    pub(super) active_cape_id: Option<String>,
}

impl LoadedProfile {
    pub(super) fn from_profile(profile: MinecraftProfileState) -> Self {
        let active_skin = profile
            .skins
            .iter()
            .find(|skin| skin.state.eq_ignore_ascii_case("active"))
            .or_else(|| profile.skins.first());

        let active_skin_png = active_skin.and_then(|skin| skin.texture_png_bytes());
        let skin_variant = active_skin
            .and_then(|skin| skin.variant.as_deref())
            .map(parse_variant)
            .unwrap_or(MinecraftSkinVariant::Classic);

        let mut active_cape_id = None;
        let mut capes = Vec::with_capacity(profile.capes.len());
        for cape in profile.capes {
            let texture_bytes = cape.texture_png_bytes();
            let texture_size = texture_bytes.as_deref().and_then(decode_image_dimensions);
            if cape.state.eq_ignore_ascii_case("active") {
                active_cape_id = Some(cape.id.clone());
            }
            capes.push(CapeChoice {
                label: cape
                    .alias
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(cape.id.as_str())
                    .to_owned(),
                id: cape.id,
                texture_bytes,
                texture_size,
            });
        }

        Self {
            player_name: profile.name,
            active_skin_png,
            skin_variant,
            capes,
            active_cape_id,
        }
    }
}
