use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedInstanceVersions {
    pub game_version: String,
    pub modloader: String,
    pub modloader_version: String,
    pub source_profile_id: String,
}

#[derive(Clone, Debug)]
struct VersionProfileCandidate {
    profile_id: String,
    game_version: String,
    modloader: String,
    modloader_version: String,
    score: u32,
}

pub fn detect_instance_versions(instance_root: &Path) -> Result<DetectedInstanceVersions, String> {
    let versions_dir = instance_root.join("versions");
    if !versions_dir.is_dir() {
        return Err(format!(
            "No versions directory was found at {}.",
            versions_dir.display()
        ));
    }

    let mut candidates = Vec::new();
    let entries = fs::read_dir(versions_dir.as_path())
        .map_err(|err| format!("failed to read {}: {err}", versions_dir.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("failed to read version directory entry: {err}"))?;
        let file_type = entry
            .file_type()
            .map_err(|err| format!("failed to inspect {}: {err}", entry.path().display()))?;
        if !file_type.is_dir() {
            continue;
        }

        let dir_name = entry.file_name().to_string_lossy().into_owned();
        let profile_path = entry.path().join(format!("{dir_name}.json"));
        if !profile_path.is_file() {
            continue;
        }

        let raw = fs::read_to_string(profile_path.as_path())
            .map_err(|err| format!("failed to read {}: {err}", profile_path.display()))?;
        let parsed: serde_json::Value = serde_json::from_str(raw.as_str())
            .map_err(|err| format!("failed to parse {}: {err}", profile_path.display()))?;
        if let Some(candidate) = candidate_from_profile(&dir_name, &parsed) {
            candidates.push(candidate);
        }
    }

    candidates
        .into_iter()
        .max_by_key(|candidate| candidate.score)
        .map(|candidate| DetectedInstanceVersions {
            game_version: candidate.game_version,
            modloader: candidate.modloader,
            modloader_version: candidate.modloader_version,
            source_profile_id: candidate.profile_id,
        })
        .ok_or_else(|| {
            "No usable Minecraft version profiles were found in this instance.".to_owned()
        })
}

fn candidate_from_profile(
    dir_name: &str,
    profile: &serde_json::Value,
) -> Option<VersionProfileCandidate> {
    let profile_id = profile
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(dir_name);
    let inherits_from = profile
        .get("inheritsFrom")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let release_time = profile
        .get("releaseTime")
        .and_then(serde_json::Value::as_str)
        .or_else(|| profile.get("time").and_then(serde_json::Value::as_str));
    let main_class = profile
        .get("mainClass")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if let Some((loader, loader_version, game_version)) = parse_fabric_or_quilt_profile(profile_id)
    {
        return Some(VersionProfileCandidate {
            profile_id: profile_id.to_owned(),
            game_version,
            modloader: loader,
            modloader_version: loader_version,
            score: 500,
        });
    }

    if let Some((loader, loader_version, game_version)) =
        parse_forge_like_profile(profile_id, inherits_from, main_class)
    {
        return Some(VersionProfileCandidate {
            profile_id: profile_id.to_owned(),
            game_version,
            modloader: loader,
            modloader_version: loader_version,
            score: 450,
        });
    }

    let game_version = inherits_from
        .or_else(|| detect_vanilla_game_version(profile_id, release_time))
        .map(str::to_owned)?;
    Some(VersionProfileCandidate {
        profile_id: profile_id.to_owned(),
        game_version,
        modloader: "Vanilla".to_owned(),
        modloader_version: String::new(),
        score: 100,
    })
}

fn parse_fabric_or_quilt_profile(profile_id: &str) -> Option<(String, String, String)> {
    for (prefix, label) in [("fabric-loader-", "Fabric"), ("quilt-loader-", "Quilt")] {
        let Some(rest) = profile_id.strip_prefix(prefix) else {
            continue;
        };
        let (loader_version, game_version) = split_loader_and_game_version(rest)?;
        return Some((label.to_owned(), loader_version, game_version));
    }
    None
}

fn parse_forge_like_profile(
    profile_id: &str,
    inherits_from: Option<&str>,
    main_class: &str,
) -> Option<(String, String, String)> {
    let lower_id = profile_id.to_ascii_lowercase();
    let lower_main = main_class.to_ascii_lowercase();
    let is_neoforge = lower_id.contains("neoforge") || lower_main.contains("neoforge");
    let is_forge = is_neoforge || lower_id.contains("forge") || lower_main.contains("forge");
    if !is_forge {
        return None;
    }

    let label = if is_neoforge { "NeoForge" } else { "Forge" };
    let game_version = inherits_from
        .map(str::to_owned)
        .or_else(|| infer_game_version_from_forge_id(profile_id))?;
    let modloader_version = if is_neoforge {
        infer_neoforge_loader_version(profile_id, game_version.as_str())
    } else {
        infer_forge_loader_version(profile_id, game_version.as_str())
    }?;

    Some((label.to_owned(), modloader_version, game_version))
}

fn split_loader_and_game_version(rest: &str) -> Option<(String, String)> {
    let parts = rest.split('-').collect::<Vec<_>>();
    for index in 1..parts.len() {
        let loader_version = parts[..index].join("-");
        let game_version = parts[index..].join("-");
        if looks_like_minecraft_version(game_version.as_str()) {
            return Some((loader_version, game_version));
        }
    }
    None
}

fn infer_game_version_from_forge_id(profile_id: &str) -> Option<String> {
    let lower = profile_id.to_ascii_lowercase();
    let trimmed = lower
        .strip_prefix("forge-")
        .or_else(|| lower.strip_prefix("neoforge-"))
        .unwrap_or(lower.as_str());
    let parts = trimmed.split('-').collect::<Vec<_>>();
    for end in (1..=parts.len()).rev() {
        let candidate = parts[..end].join("-");
        if looks_like_minecraft_version(candidate.as_str()) {
            return Some(candidate);
        }
    }
    None
}

fn infer_forge_loader_version(profile_id: &str, game_version: &str) -> Option<String> {
    let lower = profile_id.to_ascii_lowercase();
    let trimmed = lower.strip_prefix("forge-").unwrap_or(lower.as_str());
    let prefix = format!("{game_version}-").to_ascii_lowercase();
    trimmed
        .strip_prefix(prefix.as_str())
        .map(|value| value.strip_prefix("forge-").unwrap_or(value).to_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            trimmed
                .strip_suffix(format!("-{game_version}").as_str())
                .map(|value| value.strip_prefix("forge-").unwrap_or(value).to_owned())
                .filter(|value| !value.is_empty())
        })
}

fn infer_neoforge_loader_version(profile_id: &str, game_version: &str) -> Option<String> {
    let lower = profile_id.to_ascii_lowercase();
    let trimmed = lower.strip_prefix("neoforge-").unwrap_or(lower.as_str());
    let game_prefix = format!("{game_version}-neoforge-").to_ascii_lowercase();
    if let Some(version) = trimmed.strip_prefix(game_prefix.as_str()) {
        return (!version.is_empty()).then(|| version.to_owned());
    }
    if trimmed != game_version {
        return Some(trimmed.to_owned());
    }
    None
}

fn detect_vanilla_game_version<'a>(
    profile_id: &'a str,
    release_time: Option<&str>,
) -> Option<&'a str> {
    if release_time.is_some() && looks_like_minecraft_version(profile_id) {
        Some(profile_id)
    } else {
        None
    }
}

fn looks_like_minecraft_version(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "latest" | "latest-release" | "latest-snapshot"
    ) {
        return true;
    }
    if ["fabric", "forge", "neoforge", "quilt", "loader"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return false;
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_digit()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);

    fn temp_instance_root() -> PathBuf {
        let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "vertexlauncher-version-detect-test-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(root.as_path());
        root
    }

    fn write_profile(root: &Path, profile_id: &str, body: serde_json::Value) {
        let profile_dir = root.join("versions").join(profile_id);
        fs::create_dir_all(profile_dir.as_path()).unwrap();
        fs::write(
            profile_dir.join(format!("{profile_id}.json")),
            serde_json::to_string_pretty(&body).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn detects_fabric_profile() {
        let root = temp_instance_root();
        write_profile(
            root.as_path(),
            "fabric-loader-0.16.14-1.21.5",
            serde_json::json!({
                "id": "fabric-loader-0.16.14-1.21.5",
                "inheritsFrom": "1.21.5",
                "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient"
            }),
        );

        let detected = detect_instance_versions(root.as_path()).unwrap();
        assert_eq!(detected.game_version, "1.21.5");
        assert_eq!(detected.modloader, "Fabric");
        assert_eq!(detected.modloader_version, "0.16.14");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_quilt_profile() {
        let root = temp_instance_root();
        write_profile(
            root.as_path(),
            "quilt-loader-0.28.0-1.20.1",
            serde_json::json!({
                "id": "quilt-loader-0.28.0-1.20.1",
                "inheritsFrom": "1.20.1",
                "mainClass": "org.quiltmc.loader.impl.launch.knot.KnotClient"
            }),
        );

        let detected = detect_instance_versions(root.as_path()).unwrap();
        assert_eq!(detected.game_version, "1.20.1");
        assert_eq!(detected.modloader, "Quilt");
        assert_eq!(detected.modloader_version, "0.28.0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_forge_profile() {
        let root = temp_instance_root();
        write_profile(
            root.as_path(),
            "1.20.1-forge-47.2.0",
            serde_json::json!({
                "id": "1.20.1-forge-47.2.0",
                "inheritsFrom": "1.20.1",
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher"
            }),
        );

        let detected = detect_instance_versions(root.as_path()).unwrap();
        assert_eq!(detected.game_version, "1.20.1");
        assert_eq!(detected.modloader, "Forge");
        assert_eq!(detected.modloader_version, "47.2.0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_neoforge_profile() {
        let root = temp_instance_root();
        write_profile(
            root.as_path(),
            "1.20.4-neoforge-20.4.237",
            serde_json::json!({
                "id": "1.20.4-neoforge-20.4.237",
                "inheritsFrom": "1.20.4",
                "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher"
            }),
        );

        let detected = detect_instance_versions(root.as_path()).unwrap();
        assert_eq!(detected.game_version, "1.20.4");
        assert_eq!(detected.modloader, "NeoForge");
        assert_eq!(detected.modloader_version, "20.4.237");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_vanilla_profile() {
        let root = temp_instance_root();
        write_profile(
            root.as_path(),
            "1.21.1",
            serde_json::json!({
                "id": "1.21.1",
                "releaseTime": "2024-08-08T12:24:45+00:00",
                "mainClass": "net.minecraft.client.main.Main"
            }),
        );

        let detected = detect_instance_versions(root.as_path()).unwrap();
        assert_eq!(detected.game_version, "1.21.1");
        assert_eq!(detected.modloader, "Vanilla");
        assert_eq!(detected.modloader_version, "");
        let _ = fs::remove_dir_all(root);
    }
}
