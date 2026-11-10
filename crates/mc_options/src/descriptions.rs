//! Plain-language explanations of every known option, shown in the settings tooltips.

use crate::schema;

/// What the option does, or `None` for keys this launcher doesn't know.
pub fn description(key: &str) -> Option<&'static str> {
    Some(match key {
        // ── Video ──
        "ao" => {
            vertex_i18n::src!("Softens the shadows where blocks meet. Higher settings look nicer but cost performance.")
        }
        "biomeBlendRadius" => {
            vertex_i18n::src!("How many blocks around you are averaged when tinting grass, leaves and water by biome. Higher blends transitions more smoothly.")
        }
        "chunkSectionFadeInTime" => {
            vertex_i18n::src!("Seconds that newly loaded chunks take to fade in. 0 shows them instantly.")
        }
        "cutoutLeaves" => {
            vertex_i18n::src!("Draws leaves with see-through gaps (fancy) instead of solid blocks (fast).")
        }
        "enableVsync" => {
            vertex_i18n::src!("Locks the frame rate to your monitor's refresh rate to prevent screen tearing.")
        }
        "entityDistanceScaling" => {
            vertex_i18n::src!("How far away mobs, items and other entities are still drawn. Higher values cost performance.")
        }
        "entityShadows" => vertex_i18n::src!("Draws a soft shadow under mobs and other entities."),
        "fov" => {
            vertex_i18n::src!("Field of view in degrees: how wide the game world appears. Higher shows more but distorts the edges.")
        }
        "fovEffectScale" => {
            vertex_i18n::src!("How strongly sprinting, speed effects and bows change your field of view.")
        }
        "screenEffectScale" => {
            vertex_i18n::src!("Strength of the wobble when you use a Nether portal or have nausea.")
        }
        "darknessEffectScale" => {
            vertex_i18n::src!("How strongly the screen pulses when a Warden or Darkness effect is nearby.")
        }
        "glintSpeed" => vertex_i18n::src!("How fast the shimmer on enchanted items moves."),
        "glintStrength" => vertex_i18n::src!("How bright the shimmer on enchanted items is."),
        "preferredGraphicsBackend" => {
            vertex_i18n::src!("Which graphics API the game renders with. Default lets Minecraft choose.")
        }
        "fancyGraphics" => {
            vertex_i18n::src!("Fancy draws more detail (leaves, water, clouds); fast is quicker on weak hardware.")
        }
        "graphicsMode" => {
            vertex_i18n::src!("Overall graphics quality: Fast, Fancy, or Fabulous! (adds transparency and weather effects).")
        }
        "graphicsPreset" => {
            vertex_i18n::src!("A bundle of graphics options. Changing individual options switches this to Custom.")
        }
        "prioritizeChunkUpdates" => {
            vertex_i18n::src!("How eagerly the game rebuilds chunks after a block changes. Higher options prevent visual lag when building but can lower frame rate.")
        }
        "fullscreen" => vertex_i18n::src!("Runs the game in fullscreen instead of a window."),
        "exclusiveFullscreen" => {
            vertex_i18n::src!("Takes full control of the display in fullscreen. Can lower latency but makes alt-tabbing slower.")
        }
        "macFullscreenMenuVisibility" => {
            vertex_i18n::src!("Whether the macOS menu bar can appear while the game is fullscreen.")
        }
        "gamma" => {
            vertex_i18n::src!("Brightness of dark areas. Moody is the default; Bright makes caves easier to see.")
        }
        "guiScale" => vertex_i18n::src!("Size of menus and the HUD. 0 picks the largest size that fits your window."),
        "debugGuiScale" => vertex_i18n::src!("Scale used for the debug screen text."),
        "maxAnisotropyBit" => {
            vertex_i18n::src!("Keeps textures sharp on surfaces seen at a steep angle. Higher values are sharper but cost performance.")
        }
        "textureFiltering" => {
            vertex_i18n::src!("How textures are smoothed: none, smooth, or anisotropic for crisper distant surfaces.")
        }
        "maxFps" | "fpsLimit" => {
            vertex_i18n::src!("Caps the frame rate. Lower it to save power and heat; the top value is unlimited.")
        }
        "improvedTransparency" => {
            vertex_i18n::src!("Sorts see-through things (water, stained glass, particles) correctly. Needs a capable GPU.")
        }
        "inactivityFpsLimit" => {
            vertex_i18n::src!("Lowers the frame rate when you are away from the keyboard or the window is minimized.")
        }
        "mipmapLevels" => vertex_i18n::src!("Blends textures at a distance to reduce shimmering. 0 turns it off."),
        "clouds" | "renderClouds" => vertex_i18n::src!("Cloud rendering: off, fast (flat), or fancy (3D)."),
        "cloudRange" => vertex_i18n::src!("How far away clouds are drawn, in blocks."),
        "viewDistance" | "renderDistance" => {
            vertex_i18n::src!("How many chunks around you are drawn. Higher lets you see farther but costs performance.")
        }
        "simulationDistance" => {
            vertex_i18n::src!("How many chunks around you the game keeps active (mobs move, crops grow). Lower saves performance.")
        }
        "vignette" => vertex_i18n::src!("Darkens the screen edges slightly."),
        "weatherRadius" => vertex_i18n::src!("How far from you rain and snow are drawn, in blocks."),
        "bobView" => vertex_i18n::src!("Makes the camera sway while you walk."),
        "particles" => vertex_i18n::src!("How many particles (smoke, sparks, bubbles) are shown."),
        "reducedDebugInfo" => vertex_i18n::src!("Hides coordinates and other details on the debug screen."),
        "attackIndicator" => vertex_i18n::src!("Where the attack cooldown meter is shown."),
        "anaglyph3d" => vertex_i18n::src!("Red/cyan 3D effect for anaglyph glasses."),
        "advancedOpengl" => vertex_i18n::src!("Uses occlusion culling to skip drawing hidden chunks."),
        "useVbo" => vertex_i18n::src!("Uses vertex buffer objects for faster rendering."),
        "fboEnable" => vertex_i18n::src!("Renders through framebuffer objects; needed for some effects."),
        "overrideWidth" => vertex_i18n::src!("Forces the game window width in pixels. 0 uses the default size."),
        "overrideHeight" => vertex_i18n::src!("Forces the game window height in pixels. 0 uses the default size."),
        "fullscreenResolution" => vertex_i18n::src!("Video mode used in exclusive fullscreen, as chosen in the game."),
        // ── Audio ──
        "music" | "soundCategory_music" => vertex_i18n::src!("Volume of background music."),
        "sound" => vertex_i18n::src!("Volume of sound effects."),
        "soundCategory_master" => vertex_i18n::src!("Overall volume for everything."),
        "soundCategory_record" => vertex_i18n::src!("Volume of jukeboxes and note blocks."),
        "soundCategory_weather" => vertex_i18n::src!("Volume of rain and thunder."),
        "soundCategory_block" => vertex_i18n::src!("Volume of sounds from blocks, like digging and placing."),
        "soundCategory_hostile" => vertex_i18n::src!("Volume of hostile mobs."),
        "soundCategory_neutral" => vertex_i18n::src!("Volume of passive and neutral mobs."),
        "soundCategory_player" => vertex_i18n::src!("Volume of sounds made by players."),
        "soundCategory_ambient" => vertex_i18n::src!("Volume of ambient and environmental sounds, like caves."),
        "soundCategory_voice" => vertex_i18n::src!("Volume of speech and voice sounds."),
        "soundCategory_ui" => vertex_i18n::src!("Volume of menu and interface sounds."),
        "soundDevice" => {
            vertex_i18n::src!("The audio output device the game plays through. System Default follows your operating system's choice.")
        }
        "showSubtitles" => vertex_i18n::src!("Shows captions for sounds, useful for accessibility or when muted."),
        "directionalAudio" => {
            vertex_i18n::src!("Uses a 3D audio effect so sounds seem to come from their direction. Best with headphones.")
        }
        "showNowPlayingToast" | "musicToast" => {
            vertex_i18n::src!("Shows a pop-up naming the music track that starts playing.")
        }
        "musicFrequency" => vertex_i18n::src!("How often background music plays."),
        "narrator" => vertex_i18n::src!("Reads chat and menus aloud. Choose what it reads, or turn it off."),
        // ── Controls ──
        "mouseSensitivity" => vertex_i18n::src!("How far the view turns for a given mouse movement."),
        "invertYMouse" => vertex_i18n::src!("Reverses vertical look, so moving the mouse up looks down."),
        "invertXMouse" => vertex_i18n::src!("Reverses horizontal look."),
        "mouseWheelSensitivity" => {
            vertex_i18n::src!("How much each scroll notch changes the selected hotbar slot or menu.")
        }
        "discrete_mouse_scroll" => {
            vertex_i18n::src!("Treats each scroll as exactly one step, ignoring high-resolution scrolling. Helps with free-spinning wheels.")
        }
        "touchscreen" => vertex_i18n::src!("Uses touchscreen controls instead of a mouse and keyboard."),
        "rawMouseInput" => {
            vertex_i18n::src!("Reads the mouse directly, ignoring operating-system acceleration and settings.")
        }
        "autoJump" => vertex_i18n::src!("Jumps automatically over single blocks while you move."),
        "toggleCrouch" => vertex_i18n::src!("Sneak stays on after one press instead of while held."),
        "toggleSprint" => vertex_i18n::src!("Sprint stays on after one press instead of while held."),
        "toggleAttack" => vertex_i18n::src!("Attack stays active after one press instead of while held."),
        "toggleUse" => vertex_i18n::src!("Use item stays active after one press instead of while held."),
        "sprintWindow" => vertex_i18n::src!("How many ticks you have to tap forward twice to start sprinting."),
        "rotateWithMinecart" => vertex_i18n::src!("The camera turns with the minecart on curves."),
        "allowCursorChanges" => vertex_i18n::src!("Lets the game change the mouse cursor's shape."),
        "quitShortcuts" => vertex_i18n::src!("Enables keyboard shortcuts for quitting the game."),
        "ctrlClickEmulatesRightClick" => vertex_i18n::src!("Treats Ctrl+click as a right click (macOS)."),
        "operatorItemsTab" => vertex_i18n::src!("Shows the operator-only items tab in creative mode."),
        // ── Chat ──
        "chatVisibility" => vertex_i18n::src!("Which messages appear in chat: everything, only commands, or nothing."),
        "chatColors" => vertex_i18n::src!("Shows colored text in chat."),
        "chatLinks" => vertex_i18n::src!("Makes web links in chat clickable."),
        "chatLinksPrompt" => vertex_i18n::src!("Asks for confirmation before opening a link from chat."),
        "chatOpacity" => vertex_i18n::src!("How opaque the chat text is."),
        "chatLineSpacing" => vertex_i18n::src!("Space between chat lines."),
        "textBackgroundOpacity" => vertex_i18n::src!("Opacity of the dark box behind chat text."),
        "backgroundForChatOnly" => vertex_i18n::src!("Only draws the text background behind chat, not other text."),
        "chatHeightFocused" => vertex_i18n::src!("Chat height while you have chat open."),
        "chatHeightUnfocused" => vertex_i18n::src!("Chat height while chat is closed."),
        "chatScale" => vertex_i18n::src!("Size of chat text."),
        "chatWidth" => vertex_i18n::src!("Width of the chat window."),
        "chatDelay" => vertex_i18n::src!("Seconds to delay incoming chat messages."),
        "notificationDisplayTime" => vertex_i18n::src!("How long pop-up notifications and toasts stay on screen."),
        "autoSuggestions" => vertex_i18n::src!("Suggests commands as you type in chat."),
        "hideMatchedNames" => vertex_i18n::src!("Hides player names that match a chat filter."),
        "onlyShowSecureChat" => vertex_i18n::src!("Only shows chat messages that carry a valid signature."),
        "saveChatDrafts" => vertex_i18n::src!("Remembers unsent chat text when you close chat."),
        "hideServerAddress" => vertex_i18n::src!("Hides the server address in the multiplayer list and pause screen."),
        "advancedItemTooltips" => vertex_i18n::src!("Adds item IDs and durability to item tooltips (same as F3+H)."),
        // ── Accessibility ──
        "highContrast" => vertex_i18n::src!("Uses higher-contrast menus and button outlines."),
        "highContrastBlockOutline" => {
            vertex_i18n::src!("Draws a stronger outline around the block you're looking at.")
        }
        "narratorHotkey" => vertex_i18n::src!("Lets you toggle the narrator with its keyboard shortcut."),
        "darkMojangStudiosBackground" => vertex_i18n::src!("Uses a dark background on the loading screen."),
        "hideLightningFlashes" => vertex_i18n::src!("Stops the sky flashing during thunderstorms."),
        "hideSplashTexts" => vertex_i18n::src!("Hides the yellow splash text on the title screen."),
        "damageTiltStrength" => vertex_i18n::src!("How much the view shakes when you take damage."),
        "panoramaScrollSpeed" => vertex_i18n::src!("Speed of the rotating title-screen panorama. 0 stops it."),
        "menuBackgroundBlurriness" => vertex_i18n::src!("How blurred the world behind menus looks."),
        "showAutosaveIndicator" => vertex_i18n::src!("Shows an icon while the world is saving."),
        "onboardAccessibility" => {
            vertex_i18n::src!("Whether the accessibility setup screen still needs to be shown at first launch.")
        }
        // ── Skin ──
        "modelPart_cape" => vertex_i18n::src!("Shows your cape."),
        "modelPart_jacket" => vertex_i18n::src!("Shows the jacket layer of your skin."),
        "modelPart_left_sleeve" => vertex_i18n::src!("Shows the left sleeve layer of your skin."),
        "modelPart_right_sleeve" => vertex_i18n::src!("Shows the right sleeve layer of your skin."),
        "modelPart_left_pants_leg" => vertex_i18n::src!("Shows the left pants layer of your skin."),
        "modelPart_right_pants_leg" => vertex_i18n::src!("Shows the right pants layer of your skin."),
        "modelPart_hat" => vertex_i18n::src!("Shows the hat layer of your skin."),
        "mainHand" => vertex_i18n::src!("Which hand is your main hand."),
        // ── Language ──
        "lang" => vertex_i18n::src!("The language the game's menus and text are shown in."),
        "forceUnicodeFont" => {
            vertex_i18n::src!("Uses the Unicode font for all text, which supports more characters but looks different.")
        }
        "japaneseGlyphVariants" => vertex_i18n::src!("Uses Japanese forms of shared CJK characters."),
        // ── Online & misc ──
        "lastServer" => vertex_i18n::src!("The last server address you connected to with Direct Connect."),
        "resourcePacks" => {
            vertex_i18n::src!("Resource packs the game has enabled, in load order. Manage them on the Resource Packs tab.")
        }
        "incompatibleResourcePacks" => {
            vertex_i18n::src!("Resource packs the game skipped because they target another version.")
        }
        "difficulty" => vertex_i18n::src!("Default difficulty for new worlds."),
        "pauseOnLostFocus" => vertex_i18n::src!("Pauses the game when the window loses focus."),
        "realmsNotifications" => vertex_i18n::src!("Shows Realms invites and news on the title screen."),
        "skipMultiplayerWarning" => vertex_i18n::src!("Skips the multiplayer safety warning."),
        "allowServerListing" => vertex_i18n::src!("Lets your name appear in the player list shown by servers."),
        "useNativeTransport" => vertex_i18n::src!("Uses the operating system's faster networking when available."),
        "telemetryOptInExtra" => vertex_i18n::src!("Sends extra optional diagnostic data to Mojang."),
        "sharePresence" => vertex_i18n::src!("Lets friends see what you're playing."),
        "inGameNotification" => vertex_i18n::src!("Shows in-game pop-up notifications."),
        // ── Advanced / internal ──
        "version" => vertex_i18n::src!("Data version of the game that last wrote this file. Managed by the game."),
        "tutorialStep" => vertex_i18n::src!("Progress through the in-game tutorial hints. Managed by the game."),
        "joinedFirstServer" => vertex_i18n::src!("Whether you have joined a server before. Managed by the game."),
        "syncChunkWrites" => {
            vertex_i18n::src!("Writes world data to disk synchronously. Safer against crashes, but slower.")
        }
        "startedCleanly" => vertex_i18n::src!("Set by the game to detect a crash on the previous run."),
        "glDebugVerbosity" => vertex_i18n::src!("How much OpenGL debug output the game prints to the log."),
        _ => return None,
    })
}

/// Tooltip text for an option: what it does, plus its raw `options.txt` key.
pub fn tooltip(def: &schema::OptionDef) -> String {
    let mut text = String::new();
    if let Some(desc) = description(def.key) {
        text.push_str(&vertex_i18n::t(desc));
    }
    if !def.note.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&vertex_i18n::t(def.note));
    }
    if text.is_empty() {
        vertex_i18n::tr!("Option: {key}", key = def.key).into_owned()
    } else {
        vertex_i18n::tr!("{text}\n\nOption: {key}", text = text, key = def.key).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Kind;

    #[test]
    fn every_editable_option_is_explained() {
        let missing: Vec<&str> = schema::OPTIONS
            .iter()
            .filter(|def| !matches!(def.kind, Kind::Keybind))
            .filter(|def| description(def.key).is_none())
            .map(|def| def.key)
            .collect();
        assert!(
            missing.is_empty(),
            "options without a description: {missing:?}"
        );
    }
}
