use std::io::{self, Cursor, Read};

use image::{DynamicImage, ImageFormat, RgbaImage, imageops};
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, warn};
use zeroize::Zeroizing;

use crate::constants::{
    MINECRAFT_ENTITLEMENTS_URL, MINECRAFT_LOGIN_LEGACY_URL, MINECRAFT_LOGIN_URL,
    MINECRAFT_PROFILE_CAPE_ACTIVE_URL, MINECRAFT_PROFILE_SKINS_URL, MINECRAFT_PROFILE_URL,
    XBOX_USER_AUTH_URL, XSTS_AUTH_URL,
};
use crate::error::{AuthError, map_http_error, prefix_auth_error};
use crate::types::{
    CachedAccount, MinecraftCapeState, MinecraftProfileState, MinecraftSkinState,
    MinecraftSkinVariant, RefreshTokenState,
};
use crate::util::{UreqResponseExt, encode_base64, unix_now_secs};

pub(crate) fn complete_minecraft_login(
    agent: &ureq::Agent,
    microsoft_access_token: &str,
    microsoft_refresh_token: Option<&str>,
) -> Result<CachedAccount, AuthError> {
    debug!(
        target: "vertexlauncher/auth/minecraft",
        "starting Minecraft account completion flow"
    );
    let xbox_user = authenticate_with_xbox_live(agent, microsoft_access_token)
        .map_err(|err| prefix_auth_error("XboxUserAuth", err))?;

    let xsts_token = Zeroizing::new(
        authorize_xsts(agent, &xbox_user.token)
            .map_err(|err| prefix_auth_error("XstsAuthorize", err))?,
    );

    let minecraft_token = Zeroizing::new(
        authenticate_with_minecraft(agent, &xbox_user.user_hash, &xsts_token)
            .map_err(|err| prefix_auth_error("MinecraftToken", err))?,
    );

    // Entitlements are not always reliable for all account types (for example Game Pass).
    // Keep this best-effort so we do not fail a valid login due to transient API behavior.
    let _ = fetch_minecraft_entitlements(agent, &minecraft_token)
        .map_err(|err| prefix_auth_error("MinecraftEntitlements", err));

    let profile = fetch_minecraft_profile(agent, &minecraft_token)
        .map_err(|err| prefix_auth_error("MinecraftProfile", err))?;

    Ok(build_cached_account(
        profile,
        minecraft_token.as_str(),
        microsoft_refresh_token,
    ))
}

fn authenticate_with_xbox_live(
    agent: &ureq::Agent,
    microsoft_access_token: &str,
) -> Result<XboxUserAuthResult, AuthError> {
    match authenticate_with_xbox_live_rps(agent, microsoft_access_token, "d=") {
        Ok(result) => Ok(result),
        Err(first_err) => {
            let first_is_401 = matches!(&first_err, AuthError::Http(message) if message.starts_with("HTTP status 401"));

            if !first_is_401 {
                return Err(first_err);
            }

            debug!(
                target: "vertexlauncher/auth/minecraft",
                "retrying Xbox auth with alternate RPS ticket format"
            );
            match authenticate_with_xbox_live_rps(agent, microsoft_access_token, "t=") {
                Ok(result) => Ok(result),
                Err(second_err) => Err(AuthError::Http(format!(
                    "Xbox user auth failed with both RPS ticket formats (d= then t=). First error: {first_err}; second error: {second_err}",
                ))),
            }
        }
    }
}

fn authenticate_with_xbox_live_rps(
    agent: &ureq::Agent,
    microsoft_access_token: &str,
    ticket_prefix: &str,
) -> Result<XboxUserAuthResult, AuthError> {
    let response = agent
        .post(XBOX_USER_AUTH_URL)
        .header("Accept", "application/json")
        .send_json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("{ticket_prefix}{microsoft_access_token}"),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }));

    match response {
        Ok(ok) => {
            let parsed = ok.into_json::<XboxUserAuthResponse>()?;
            let user_hash = parsed
                .display_claims
                .xui
                .first()
                .map(|entry| entry.user_hash.clone())
                .ok_or_else(|| {
                    AuthError::OAuth("Xbox response did not include user hash".to_owned())
                })?;

            Ok(XboxUserAuthResult {
                token: parsed.token,
                user_hash,
            })
        }
        Err(err) => Err(map_http_error(err)),
    }
}

fn authorize_xsts(agent: &ureq::Agent, xbox_token: &str) -> Result<String, AuthError> {
    let response = agent
        .post(XSTS_AUTH_URL)
        .header("Accept", "application/json")
        .send_json(json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbox_token],
            },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }));

    match response {
        Ok(ok) => {
            let parsed = ok.into_json::<XstsAuthResponse>()?;
            Ok(parsed.token)
        }
        Err(err) => Err(map_http_error(err)),
    }
}

fn authenticate_with_minecraft(
    agent: &ureq::Agent,
    user_hash: &str,
    xsts_token: &str,
) -> Result<String, AuthError> {
    let xtoken = format!("XBL3.0 x={user_hash};{xsts_token}");
    let launcher_response = agent
        .post(MINECRAFT_LOGIN_URL)
        .header("Accept", "application/json")
        .send_json(json!({
            "platform": "PC_LAUNCHER",
            "xtoken": xtoken,
        }));

    match launcher_response {
        Ok(ok) => {
            let parsed = ok.into_json::<MinecraftLoginResponse>()?;
            Ok(parsed.access_token)
        }
        Err(ureq::Error::StatusCode(code)) if matches!(code, 400 | 401 | 403 | 404) => {
            warn!(
                target: "vertexlauncher/auth/minecraft",
                status = code,
                "primary Minecraft token endpoint failed; trying legacy endpoint"
            );
            let launcher_error = map_http_error(ureq::Error::StatusCode(code)).to_string();

            let legacy_response = agent
                .post(MINECRAFT_LOGIN_LEGACY_URL)
                .header("Accept", "application/json")
                .send_json(json!({
                    "identityToken": format!("XBL3.0 x={user_hash};{xsts_token}"),
                }));

            match legacy_response {
                Ok(ok) => {
                    let parsed = ok.into_json::<MinecraftLoginResponse>()?;
                    Ok(parsed.access_token)
                }
                Err(err) => Err(AuthError::Http(format!(
                    "Minecraft token exchange failed on both endpoints. launcher/login error: {launcher_error}; legacy login_with_xbox error: {}",
                    map_http_error(err)
                ))),
            }
        }
        Err(err) => Err(map_http_error(err)),
    }
}

fn fetch_minecraft_entitlements(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
) -> Result<(), AuthError> {
    let response = agent
        .get(MINECRAFT_ENTITLEMENTS_URL)
        .header("Accept", "application/json")
        .header("Authorization", &format!("Bearer {minecraft_access_token}"))
        .call();

    match response {
        Ok(ok) => {
            let _ = ok.into_json::<serde_json::Value>()?;
            Ok(())
        }
        Err(err) => Err(map_http_error(err)),
    }
}

fn fetch_minecraft_profile(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
) -> Result<MinecraftProfileResponse, AuthError> {
    let response = agent
        .get(MINECRAFT_PROFILE_URL)
        .header("Accept", "application/json")
        .header("Authorization", &format!("Bearer {minecraft_access_token}"))
        .call();

    match response {
        Ok(ok) => Ok(ok.into_json::<MinecraftProfileResponse>()?),
        Err(ureq::Error::StatusCode(404)) => Err(AuthError::MinecraftProfileUnavailable),
        Err(err) => Err(map_http_error(err)),
    }
}

pub(crate) fn fetch_profile_state_with_textures(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
) -> Result<MinecraftProfileState, AuthError> {
    let profile = fetch_minecraft_profile(agent, minecraft_access_token)?;
    Ok(build_profile_state_from_response(agent, profile))
}

pub(crate) fn upload_profile_skin(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
    skin_png_bytes: &[u8],
    variant: MinecraftSkinVariant,
) -> Result<MinecraftProfileState, AuthError> {
    let boundary = format!("----vertexlauncher-{}", unix_now_secs());
    let mut body = Vec::with_capacity(skin_png_bytes.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"variant\"\r\n\r\n");
    body.extend_from_slice(variant.as_api_str().as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"skin.png\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    body.extend_from_slice(skin_png_bytes);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let response = agent
        .post(MINECRAFT_PROFILE_SKINS_URL)
        .header("Accept", "application/json")
        .header("Authorization", &format!("Bearer {minecraft_access_token}"))
        .header(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send(body.as_slice());

    match response {
        Ok(ok) => parse_mutation_profile_response(agent, minecraft_access_token, ok, "upload skin"),
        Err(err) => Err(map_http_error(err)),
    }
}

pub(crate) fn set_active_profile_cape(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
    cape_id: &str,
) -> Result<MinecraftProfileState, AuthError> {
    let put_response = agent
        .put(MINECRAFT_PROFILE_CAPE_ACTIVE_URL)
        .header("Accept", "application/json")
        .header("Authorization", &format!("Bearer {minecraft_access_token}"))
        .send_json(json!({ "capeId": cape_id }));

    match put_response {
        Ok(ok) => {
            parse_mutation_profile_response(agent, minecraft_access_token, ok, "set active cape")
        }
        Err(ureq::Error::StatusCode(code)) if matches!(code, 401 | 404 | 405) => {
            warn!(
                target: "vertexlauncher/auth/minecraft",
                status = code,
                "PUT cape activation failed; retrying with POST compatibility fallback"
            );
            let first_error = map_http_error(ureq::Error::StatusCode(code)).to_string();
            let post_response = agent
                .post(MINECRAFT_PROFILE_CAPE_ACTIVE_URL)
                .header("Accept", "application/json")
                .header("Authorization", &format!("Bearer {minecraft_access_token}"))
                .send_json(json!({ "capeId": cape_id }));

            match post_response {
                Ok(ok) => parse_mutation_profile_response(
                    agent,
                    minecraft_access_token,
                    ok,
                    "set active cape",
                ),
                Err(err) => Err(AuthError::Http(format!(
                    "cape activation failed on both PUT and POST. put error: {first_error}; post error: {}",
                    map_http_error(err)
                ))),
            }
        }
        Err(err) => Err(map_http_error(err)),
    }
}

pub(crate) fn clear_active_profile_cape(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
) -> Result<MinecraftProfileState, AuthError> {
    let response = agent
        .delete(MINECRAFT_PROFILE_CAPE_ACTIVE_URL)
        .header("Accept", "application/json")
        .header("Authorization", &format!("Bearer {minecraft_access_token}"))
        .call();

    match response {
        Ok(ok) => {
            parse_mutation_profile_response(agent, minecraft_access_token, ok, "clear active cape")
        }
        Err(err) => Err(map_http_error(err)),
    }
}

fn parse_mutation_profile_response(
    agent: &ureq::Agent,
    minecraft_access_token: &str,
    response: ureq::http::Response<ureq::Body>,
    op: &str,
) -> Result<MinecraftProfileState, AuthError> {
    let raw = response.into_string()?;
    if raw.trim().is_empty() {
        debug!(
            target: "vertexlauncher/auth/minecraft",
            operation = op,
            "profile mutation response body was empty; falling back to profile fetch"
        );
        let profile = fetch_minecraft_profile(agent, minecraft_access_token)?;
        return Ok(build_profile_state_from_response(agent, profile));
    }

    match serde_json::from_str::<MinecraftProfileResponse>(&raw) {
        Ok(profile) => Ok(build_profile_state_from_response(agent, profile)),
        Err(err) => {
            warn!(
                target: "vertexlauncher/auth/minecraft",
                operation = op,
                parse_error = %err,
                "failed to parse profile mutation response; falling back to profile fetch"
            );
            let profile = fetch_minecraft_profile(agent, minecraft_access_token)?;
            Ok(build_profile_state_from_response(agent, profile))
        }
    }
}

fn build_cached_account(
    profile: MinecraftProfileResponse,
    minecraft_access_token: &str,
    microsoft_refresh_token: Option<&str>,
) -> CachedAccount {
    let avatar_source_skin_url = active_skin_url_from_profile(&profile);
    let minecraft_profile = build_profile_state_without_textures(profile);

    CachedAccount {
        minecraft_profile,
        minecraft_access_token: Some(minecraft_access_token.to_owned()),
        microsoft_refresh_token: microsoft_refresh_token.map(str::to_owned),
        microsoft_client_id: None,
        microsoft_token_uri: None,
        microsoft_scope: None,
        refresh_token_state: if microsoft_refresh_token.is_some() {
            RefreshTokenState::Present
        } else {
            RefreshTokenState::Missing
        },
        xuid: None,
        user_type: Some("msa".to_owned()),
        avatar_png_base64: None,
        avatar_source_skin_url,
        cached_at_unix_secs: unix_now_secs(),
    }
}

fn build_profile_state_without_textures(
    profile: MinecraftProfileResponse,
) -> MinecraftProfileState {
    let mut out = MinecraftProfileState {
        id: profile.id,
        name: profile.name,
        skins: Vec::new(),
        capes: Vec::new(),
    };

    for raw_skin in profile.skins {
        out.skins.push(MinecraftSkinState {
            id: raw_skin.id,
            state: raw_skin.state,
            url: raw_skin.url,
            variant: raw_skin.variant,
            alias: raw_skin.alias,
            texture_png_base64: None,
        });
    }

    for raw_cape in profile.capes {
        out.capes.push(MinecraftCapeState {
            id: raw_cape.id,
            state: raw_cape.state,
            url: raw_cape.url,
            alias: raw_cape.alias,
            texture_png_base64: None,
        });
    }

    out
}

fn build_profile_state_from_response(
    agent: &ureq::Agent,
    profile: MinecraftProfileResponse,
) -> MinecraftProfileState {
    let mut out = MinecraftProfileState {
        id: profile.id,
        name: profile.name,
        skins: Vec::new(),
        capes: Vec::new(),
    };

    for raw_skin in profile.skins {
        let texture_png_base64 = fetch_texture_base64(agent, &raw_skin.url);
        out.skins.push(MinecraftSkinState {
            id: raw_skin.id,
            state: raw_skin.state,
            url: raw_skin.url,
            variant: raw_skin.variant,
            alias: raw_skin.alias,
            texture_png_base64,
        });
    }

    for raw_cape in profile.capes {
        let texture_png_base64 = fetch_texture_base64(agent, &raw_cape.url);
        out.capes.push(MinecraftCapeState {
            id: raw_cape.id,
            state: raw_cape.state,
            url: raw_cape.url,
            alias: raw_cape.alias,
            texture_png_base64,
        });
    }

    out
}

fn fetch_texture_base64(agent: &ureq::Agent, url: &str) -> Option<String> {
    let response = agent
        .get(url)
        .header("Accept", "image/png,image/*")
        .call()
        .ok()?;

    let mut bytes = Vec::new();
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
    if reader.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
        return None;
    }

    Some(encode_base64(&bytes))
}

pub(crate) fn resolve_cached_account_avatar(
    account: &CachedAccount,
) -> Result<Option<Vec<u8>>, AuthError> {
    if let Some(bytes) = account.avatar_png_bytes() {
        return Ok(Some(bytes));
    }

    let Some(url) = account
        .avatar_source_skin_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return Ok(None);
    };

    let agent = crate::util::build_http_agent();
    let response = agent
        .get(url)
        .header("Accept", "image/png,image/*")
        .call()
        .map_err(map_http_error)?;
    let mut bytes = Vec::new();
    let (_, body) = response.into_parts();
    let mut reader = body.into_reader();
    reader.read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }

    Ok(Some(generate_avatar_png_from_skin(&bytes)?))
}

fn active_skin_url_from_profile(profile: &MinecraftProfileResponse) -> Option<String> {
    profile
        .skins
        .iter()
        .find(|skin| skin.state.eq_ignore_ascii_case("active"))
        .or_else(|| profile.skins.first())
        .map(|skin| skin.url.clone())
}

fn generate_avatar_png_from_skin(skin_png_bytes: &[u8]) -> Result<Vec<u8>, AuthError> {
    let skin = image::load_from_memory(skin_png_bytes)?.to_rgba8();
    let (width, height) = skin.dimensions();

    if width < 64 || height < 16 {
        return Err(AuthError::Image(image::ImageError::IoError(
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Skin texture is smaller than expected",
            ),
        )));
    }

    let mut head = RgbaImage::new(8, 8);

    for y in 0..8 {
        for x in 0..8 {
            let pixel = skin.get_pixel(8 + x, 8 + y);
            head.put_pixel(x, y, *pixel);
        }
    }

    if width >= 48 && height >= 16 {
        for y in 0..8 {
            for x in 0..8 {
                let overlay = skin.get_pixel(40 + x, 8 + y);
                if overlay[3] > 0 {
                    head.put_pixel(x, y, *overlay);
                }
            }
        }
    }

    let upscaled = imageops::resize(&head, 64, 64, imageops::FilterType::Nearest);
    let mut png_out = Vec::new();
    DynamicImage::ImageRgba8(upscaled)
        .write_to(&mut Cursor::new(&mut png_out), ImageFormat::Png)?;
    Ok(png_out)
}

#[derive(Debug, Deserialize)]
struct XboxUserAuthResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XboxDisplayClaims,
}

#[derive(Debug, Deserialize)]
struct XboxDisplayClaims {
    xui: Vec<XboxUserHashEntry>,
}

#[derive(Debug, Deserialize)]
struct XboxUserHashEntry {
    #[serde(rename = "uhs")]
    user_hash: String,
}

#[derive(Debug)]
struct XboxUserAuthResult {
    token: String,
    user_hash: String,
}

#[derive(Debug, Deserialize)]
struct XstsAuthResponse {
    #[serde(rename = "Token")]
    token: String,
}

#[derive(Debug, Deserialize)]
struct MinecraftLoginResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct MinecraftProfileResponse {
    id: String,
    name: String,
    #[serde(default)]
    skins: Vec<MinecraftSkinResponse>,
    #[serde(default)]
    capes: Vec<MinecraftCapeResponse>,
}

#[derive(Debug, Deserialize)]
struct MinecraftSkinResponse {
    id: String,
    state: String,
    url: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MinecraftCapeResponse {
    id: String,
    state: String,
    url: String,
    #[serde(default)]
    alias: Option<String>,
}
