use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;

use auth::{CachedAccount, CachedAccountsState};
use config::{Config, JavaRuntimeVersion, LoadConfigResult, load_config};
use flate2::read::GzDecoder;
use installation::{
    DownloadPolicy, LaunchRequest, display_user_path, ensure_game_files, ensure_openjdk_runtime,
    launch_instance,
};
use instances::{
    InstanceRecord, InstanceStore, instance_root_path, load_store, record_instance_launch_usage,
    save_store,
};

#[path = "cli/cli_command.rs"]
mod cli_command;
#[path = "cli/quick_launch_mode.rs"]
mod quick_launch_mode;
#[path = "cli/quick_launch_spec.rs"]
mod quick_launch_spec;
#[path = "cli/server_dat_entry.rs"]
mod server_dat_entry;

use self::cli_command::CliCommand;
use self::quick_launch_mode::QuickLaunchMode;
use self::quick_launch_spec::QuickLaunchSpec;
use self::server_dat_entry::ServerDatEntry;

pub fn maybe_run_from_args() -> Result<bool, String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = parse_cli_command(&args)? else {
        return Ok(false);
    };
    match command {
        CliCommand::Help => {
            print_help();
            Ok(true)
        }
        CliCommand::BuildArgs {
            mode,
            instance,
            user,
            world,
            server,
        } => {
            let spec = QuickLaunchSpec {
                mode,
                instance,
                user,
                world,
                server,
            };
            println!("{}", build_launch_args_string(&spec));
            Ok(true)
        }
        CliCommand::ListTargets { instance } => {
            let config = startup_config();
            let store = load_store().map_err(|err| format!("failed to load instances: {err}"))?;
            let instance = resolve_instance(&store, instance.as_str())?;
            let instance_root =
                instance_root_path(config.minecraft_installations_root_path(), instance);
            print_targets(instance, instance_root.as_path());
            Ok(true)
        }
        CliCommand::Launch {
            mode,
            instance,
            user,
            world,
            server,
        } => {
            let spec = QuickLaunchSpec {
                mode,
                instance,
                user,
                world,
                server,
            };
            run_quick_launch(spec)?;
            Ok(true)
        }
    }
}

fn run_quick_launch(spec: QuickLaunchSpec) -> Result<(), String> {
    let config = startup_config();
    let mut store = load_store().map_err(|err| format!("failed to load instances: {err}"))?;
    let instance = resolve_instance(&store, spec.instance.as_str())?.clone();
    let instance_root = instance_root_path(config.minecraft_installations_root_path(), &instance);
    let account = resolve_and_refresh_account(spec.user.as_str())?;
    let world_selector = spec
        .world
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let server_selector = spec
        .server
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let quick_play_singleplayer = match spec.mode {
        QuickLaunchMode::World => {
            let selector =
                world_selector.ok_or_else(|| "missing --world for world launch".to_owned())?;
            Some(resolve_world_name(instance_root.as_path(), selector)?)
        }
        _ => None,
    };
    let quick_play_multiplayer = match spec.mode {
        QuickLaunchMode::Server => {
            let selector =
                server_selector.ok_or_else(|| "missing --server for server launch".to_owned())?;
            Some(resolve_server_address(instance_root.as_path(), selector)?)
        }
        _ => None,
    };
    let download_policy = DownloadPolicy {
        max_concurrent_downloads: config.download_max_concurrent().max(1),
        max_download_bps: config.parsed_download_speed_limit_bps(),
    };
    let java_path = select_java_path(&config, instance.game_version.as_str())?;
    let modloader_version = normalize_optional(instance.modloader_version.as_str());
    let setup = ensure_game_files(
        instance_root.as_path(),
        instance.game_version.as_str(),
        instance.modloader.as_str(),
        modloader_version.as_deref(),
        Some(java_path.as_str()),
        &download_policy,
        None,
    )
    .map_err(|err| err.to_string())?;
    let (linux_set_opengl_driver, linux_use_zink_driver) =
        instances::effective_linux_graphics_settings(
            &instance,
            config.linux_set_opengl_driver(),
            config.linux_use_zink_driver(),
        );
    let launch_request = LaunchRequest {
        instance_root: instance_root.clone(),
        game_version: instance.game_version.clone(),
        modloader: instance.modloader.clone(),
        modloader_version,
        account_key: Some(account.minecraft_profile.id.clone()),
        java_executable: Some(java_path),
        max_memory_mib: instance
            .max_memory_mib
            .unwrap_or(config.default_instance_max_memory_mib()),
        extra_jvm_args: instance
            .cli_args
            .as_deref()
            .and_then(normalize_optional)
            .or_else(|| normalize_optional(config.default_instance_cli_args())),
        player_name: Some(account.minecraft_profile.name.clone()),
        player_uuid: Some(account.minecraft_profile.id.clone()),
        auth_access_token: account.minecraft_access_token.clone(),
        auth_xuid: account.xuid.clone(),
        auth_user_type: account.user_type.clone(),
        quick_play_singleplayer,
        quick_play_multiplayer,
        linux_set_opengl_driver,
        linux_use_zink_driver,
    };
    let launch = launch_instance(&launch_request).map_err(|err| err.to_string())?;
    let _ = record_instance_launch_usage(&mut store, instance.id.as_str());
    save_store(&store).map_err(|err| format!("failed to save instance usage: {err}"))?;
    println!(
        "Launched {} (pid {}, profile {}, files downloaded {}, loader {}). Log: {}",
        instance.name,
        launch.pid,
        launch.profile_id,
        setup.downloaded_files,
        setup.resolved_modloader_version.as_deref().unwrap_or("n/a"),
        display_user_path(launch.launch_log_path.as_path())
    );
    Ok(())
}

fn resolve_and_refresh_account(selector: &str) -> Result<CachedAccount, String> {
    let state = auth::load_cached_accounts().map_err(|err| err.to_string())?;
    let selected_profile_id = select_profile_id(&state, selector)?;
    let mut selected = state
        .accounts
        .iter()
        .find(|account| account.minecraft_profile.id == selected_profile_id)
        .cloned()
        .ok_or_else(|| format!("cached account '{selected_profile_id}' disappeared"))?;

    let refreshed_state = if selected
        .microsoft_refresh_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|token| !token.is_empty())
    {
        let client_id = microsoft_client_id()?;
        match auth::renew_cached_account_token(client_id.as_str(), selected_profile_id.as_str()) {
            Ok(state) => state,
            Err(err) => {
                let err_text = err.to_string();
                if is_http_auth_error(err_text.as_str()) {
                    eprintln!(
                        "warning: failed to renew account over HTTP ({err_text}); continuing in offline mode with cached account."
                    );
                    selected.minecraft_access_token = None;
                    return Ok(selected);
                }
                return Err(err_text);
            }
        }
    } else {
        state
    };

    let mut refreshed = refreshed_state
        .accounts
        .into_iter()
        .find(|account| account.minecraft_profile.id == selected_profile_id)
        .ok_or_else(|| format!("no cached account found for '{selected_profile_id}'"))?;
    if refreshed
        .minecraft_access_token
        .as_deref()
        .map(str::trim)
        .is_none_or(|value| value.is_empty())
    {
        refreshed.minecraft_access_token = None;
        eprintln!(
            "warning: account '{}' has no online token; continuing in offline mode.",
            refreshed.minecraft_profile.name
        );
    }
    Ok(refreshed)
}

fn is_http_auth_error(error_text: &str) -> bool {
    let lowered = error_text.to_ascii_lowercase();
    lowered.contains("http status")
        || lowered.contains("http request failed")
        || lowered.contains("transport")
        || lowered.contains("connection")
}

fn select_profile_id(state: &CachedAccountsState, selector: &str) -> Result<String, String> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return state
            .active_account()
            .map(|account| account.minecraft_profile.id.clone())
            .ok_or_else(|| "no cached accounts available".to_owned());
    }
    if let Some(account) = state
        .accounts
        .iter()
        .find(|account| account.minecraft_profile.id == trimmed)
    {
        return Ok(account.minecraft_profile.id.clone());
    }
    let lowered = trimmed.to_ascii_lowercase();
    if let Some(account) = state.accounts.iter().find(|account| {
        account.minecraft_profile.id.to_ascii_lowercase() == lowered
            || account.minecraft_profile.name.to_ascii_lowercase() == lowered
    }) {
        return Ok(account.minecraft_profile.id.clone());
    }
    Err(format!(
        "no cached account matched user selector '{selector}' (try profile id or username)."
    ))
}

fn resolve_instance<'a>(
    store: &'a InstanceStore,
    selector: &str,
) -> Result<&'a InstanceRecord, String> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return Err("missing --instance".to_owned());
    }
    if let Some(instance) = store
        .instances
        .iter()
        .find(|instance| instance.id == trimmed)
    {
        return Ok(instance);
    }
    let lowered = trimmed.to_ascii_lowercase();
    store
        .instances
        .iter()
        .find(|instance| {
            instance.id.to_ascii_lowercase() == lowered
                || instance.name.to_ascii_lowercase() == lowered
        })
        .ok_or_else(|| format!("instance '{selector}' not found"))
}

fn resolve_world_name(instance_root: &Path, selector: &str) -> Result<String, String> {
    let saves = instance_root.join("saves");
    let entries = fs::read_dir(saves.as_path())
        .map_err(|err| format!("failed to read worlds under '{}': {err}", saves.display()))?;
    let lowered = selector.to_ascii_lowercase();
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.trim().is_empty() {
            continue;
        }
        if name == selector || name.to_ascii_lowercase() == lowered {
            return Ok(name);
        }
        candidates.push(name);
    }
    Err(format!(
        "world '{selector}' not found. Available worlds: {}",
        if candidates.is_empty() {
            "(none)".to_owned()
        } else {
            candidates.join(", ")
        }
    ))
}

fn resolve_server_address(instance_root: &Path, selector: &str) -> Result<String, String> {
    let servers =
        parse_servers_dat(instance_root.join("servers.dat").as_path()).unwrap_or_default();
    let lowered = selector.to_ascii_lowercase();
    if let Some(entry) = servers.iter().find(|entry| {
        entry.ip.eq_ignore_ascii_case(selector) || entry.name.to_ascii_lowercase() == lowered
    }) {
        return Ok(entry.ip.clone());
    }
    Ok(selector.to_owned())
}

fn print_targets(instance: &InstanceRecord, instance_root: &Path) {
    println!("Instance: {} ({})", instance.name, instance.id);
    println!("Worlds:");
    let saves = instance_root.join("saves");
    match fs::read_dir(saves.as_path()) {
        Ok(entries) => {
            let mut found = false;
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.trim().is_empty() {
                        println!("  - {}", name);
                        found = true;
                    }
                }
            }
            if !found {
                println!("  (none)");
            }
        }
        Err(_) => println!("  (none)"),
    }
    println!("Servers:");
    let servers =
        parse_servers_dat(instance_root.join("servers.dat").as_path()).unwrap_or_default();
    if servers.is_empty() {
        println!("  (none)");
    } else {
        for server in servers {
            println!("  - {} ({})", server.name, server.ip);
        }
    }
    println!("Sample:");
    println!(
        "  {}",
        build_launch_args_string(&QuickLaunchSpec {
            mode: QuickLaunchMode::Pack,
            instance: instance.id.clone(),
            user: "<profile-id>".to_owned(),
            world: None,
            server: None,
        })
    );
}

fn parse_cli_command(args: &[String]) -> Result<Option<CliCommand>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    if has_flag(args, "--quick-launch-help") || has_flag(args, "--help-quick-launch") {
        return Ok(Some(CliCommand::Help));
    }
    if has_flag(args, "--list-quick-launch-targets") {
        let instance = required_flag_value(args, "--instance")?;
        return Ok(Some(CliCommand::ListTargets { instance }));
    }
    if has_flag(args, "--build-quick-launch-args") {
        let mode = parse_mode(required_flag_value(args, "--mode")?.as_str())?;
        return Ok(Some(CliCommand::BuildArgs {
            mode,
            instance: required_flag_value(args, "--instance")?,
            user: required_flag_value(args, "--user")?,
            world: optional_flag_value(args, "--world"),
            server: optional_flag_value(args, "--server"),
        }));
    }
    let mode = if has_flag(args, "--quick-launch-pack") {
        Some(QuickLaunchMode::Pack)
    } else if has_flag(args, "--quick-launch-world") {
        Some(QuickLaunchMode::World)
    } else if has_flag(args, "--quick-launch-server") {
        Some(QuickLaunchMode::Server)
    } else {
        None
    };
    let Some(mode) = mode else {
        return Ok(None);
    };
    Ok(Some(CliCommand::Launch {
        mode,
        instance: required_flag_value(args, "--instance")?,
        user: required_flag_value(args, "--user")?,
        world: optional_flag_value(args, "--world"),
        server: optional_flag_value(args, "--server"),
    }))
}

fn build_launch_args_string(spec: &QuickLaunchSpec) -> String {
    let mut args = Vec::new();
    args.push(match spec.mode {
        QuickLaunchMode::Pack => "--quick-launch-pack".to_owned(),
        QuickLaunchMode::World => "--quick-launch-world".to_owned(),
        QuickLaunchMode::Server => "--quick-launch-server".to_owned(),
    });
    args.push("--instance".to_owned());
    args.push(shell_escape(spec.instance.as_str()));
    args.push("--user".to_owned());
    args.push(shell_escape(spec.user.as_str()));
    if let Some(world) = spec.world.as_deref() {
        args.push("--world".to_owned());
        args.push(shell_escape(world));
    }
    if let Some(server) = spec.server.as_deref() {
        args.push("--server".to_owned());
        args.push(shell_escape(server));
    }
    args.join(" ")
}

fn required_flag_value(args: &[String], flag: &str) -> Result<String, String> {
    optional_flag_value(args, flag).ok_or_else(|| format!("missing required argument {flag}"))
}

fn optional_flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut index = 0usize;
    while index < args.len() {
        if args[index] == flag {
            return args.get(index + 1).cloned();
        }
        index += 1;
    }
    None
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn parse_mode(value: &str) -> Result<QuickLaunchMode, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "pack" | "modpack" | "instance" => Ok(QuickLaunchMode::Pack),
        "world" => Ok(QuickLaunchMode::World),
        "server" => Ok(QuickLaunchMode::Server),
        _ => Err("mode must be one of: pack, world, server".to_owned()),
    }
}

fn startup_config() -> Config {
    match load_config() {
        LoadConfigResult::Loaded(mut config) => {
            config.normalize();
            config
        }
        LoadConfigResult::Missing { .. } => Config::default(),
    }
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn select_java_path(config: &Config, game_version: &str) -> Result<String, String> {
    let runtime = recommended_java_runtime_for_game(game_version);
    let configured = runtime.and_then(|runtime| config.java_runtime_path_ref(runtime));
    if let Some(path) = configured.filter(|path| path.exists()) {
        let normalized = path.as_os_str().to_string_lossy().trim().to_owned();
        if !normalized.is_empty() {
            return Ok(normalized);
        }
    }
    if let Some(runtime) = runtime {
        return ensure_openjdk_runtime(runtime.major())
            .map(|path| path.display().to_string())
            .map_err(|err| format!("failed to auto-install OpenJDK {}: {err}", runtime.major()));
    }
    Ok("java".to_owned())
}

fn recommended_java_runtime_for_game(game_version: &str) -> Option<JavaRuntimeVersion> {
    let mut parts = game_version
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or(0);

    if major != 1 {
        // New versioning scheme (e.g. 26.x): Java version is major - 1
        return Some(JavaRuntimeVersion::Java25);
    }
    if minor <= 16 {
        return Some(JavaRuntimeVersion::Java8);
    }
    if minor == 17 {
        return Some(JavaRuntimeVersion::Java16);
    }
    if minor > 20 || (minor == 20 && patch >= 5) {
        return Some(JavaRuntimeVersion::Java21);
    }
    Some(JavaRuntimeVersion::Java17)
}

fn microsoft_client_id() -> Result<String, String> {
    let client_id = env::var("VERTEX_MSA_CLIENT_ID")
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
        .or_else(|| auth::builtin_client_id().map(str::to_owned))
        .ok_or_else(|| {
            "Microsoft OAuth client ID is not configured. Set VERTEX_MSA_CLIENT_ID or configure auth::BUILTIN_MICROSOFT_CLIENT_ID."
                .to_owned()
        })?;

    if is_valid_microsoft_client_id(&client_id) {
        Ok(client_id)
    } else {
        Err(format!("Invalid Microsoft client id '{client_id}'."))
    }
}

fn is_valid_microsoft_client_id(value: &str) -> bool {
    is_hex_client_id(value) || is_guid_client_id(value)
}

fn is_hex_client_id(value: &str) -> bool {
    value.len() == 16 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_guid_client_id(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }

    for (index, ch) in value.chars().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if ch != '-' {
                return false;
            }
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '/' | '\\'))
    {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn print_help() {
    println!("Vertex quick-launch commands (no UI):");
    println!("  --quick-launch-pack --instance <id|name> --user <profile-id|username>");
    println!(
        "  --quick-launch-world --instance <id|name> --world <world-folder> --user <profile-id|username>"
    );
    println!(
        "  --quick-launch-server --instance <id|name> --server <server-name-or-address> --user <profile-id|username>"
    );
    println!("Tools:");
    println!("  --list-quick-launch-targets --instance <id|name>");
    println!(
        "  --build-quick-launch-args --mode <pack|world|server> --instance <id|name> --user <profile-id|username> [--world <name>] [--server <name>]"
    );
}

fn parse_servers_dat(path: &Path) -> Option<Vec<ServerDatEntry>> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    let data = if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).ok()?;
        out
    } else {
        bytes
    };
    parse_servers_from_nbt(data.as_slice()).ok()
}

fn parse_servers_from_nbt(bytes: &[u8]) -> Result<Vec<ServerDatEntry>, ()> {
    let mut cursor = NbtCursor::new(bytes);
    let root_tag = cursor.read_u8()?;
    if root_tag != 10 {
        return Err(());
    }
    let _ = cursor.read_string()?;
    let mut servers = Vec::new();
    parse_compound_for_servers(&mut cursor, &mut servers)?;
    Ok(servers)
}

fn parse_compound_for_servers(
    cursor: &mut NbtCursor<'_>,
    servers: &mut Vec<ServerDatEntry>,
) -> Result<(), ()> {
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            return Ok(());
        }
        let name = cursor.read_string()?;
        if tag == 9 && name == "servers" {
            parse_servers_list(cursor, servers)?;
        } else {
            skip_nbt_payload(cursor, tag)?;
        }
    }
}

fn parse_servers_list(cursor: &mut NbtCursor<'_>, out: &mut Vec<ServerDatEntry>) -> Result<(), ()> {
    let item_tag = cursor.read_u8()?;
    let len = cursor.read_i32()?;
    if len <= 0 {
        return Ok(());
    }
    let len = len as usize;
    for _ in 0..len {
        if item_tag == 10 {
            if let Some(entry) = parse_server_compound(cursor)? {
                out.push(entry);
            }
        } else {
            skip_nbt_payload(cursor, item_tag)?;
        }
    }
    Ok(())
}

fn parse_server_compound(cursor: &mut NbtCursor<'_>) -> Result<Option<ServerDatEntry>, ()> {
    let mut name = String::new();
    let mut ip = String::new();
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            break;
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "name") => name = cursor.read_string()?,
            (8, "ip") => ip = cursor.read_string()?,
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
    if ip.trim().is_empty() {
        return Ok(None);
    }
    if name.trim().is_empty() {
        name = ip.clone();
    }
    Ok(Some(ServerDatEntry { name, ip }))
}

fn skip_nbt_payload(cursor: &mut NbtCursor<'_>, tag: u8) -> Result<(), ()> {
    match tag {
        0 => Ok(()),
        1 => cursor.skip(1),
        2 => cursor.skip(2),
        3 => cursor.skip(4),
        4 => cursor.skip(8),
        5 => cursor.skip(4),
        6 => cursor.skip(8),
        7 => {
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            cursor.skip(len as usize)
        }
        8 => {
            let len = cursor.read_u16()? as usize;
            cursor.skip(len)
        }
        9 => {
            let nested_tag = cursor.read_u8()?;
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            for _ in 0..(len as usize) {
                skip_nbt_payload(cursor, nested_tag)?;
            }
            Ok(())
        }
        10 => loop {
            let nested = cursor.read_u8()?;
            if nested == 0 {
                break Ok(());
            }
            let _ = cursor.read_string()?;
            skip_nbt_payload(cursor, nested)?;
        },
        11 => {
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            cursor.skip((len as usize) * 4)
        }
        12 => {
            let len = cursor.read_i32()?;
            if len < 0 {
                return Err(());
            }
            cursor.skip((len as usize) * 8)
        }
        _ => Err(()),
    }
}

#[derive(Debug)]
struct NbtCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> NbtCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn skip(&mut self, len: usize) -> Result<(), ()> {
        if self.pos.saturating_add(len) > self.bytes.len() {
            return Err(());
        }
        self.pos += len;
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8, ()> {
        if self.pos >= self.bytes.len() {
            return Err(());
        }
        let value = self.bytes[self.pos];
        self.pos += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16, ()> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_i32(&mut self) -> Result<i32, ()> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_string(&mut self) -> Result<String, ()> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        Ok(String::from_utf8_lossy(bytes).to_string())
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], ()> {
        if self.pos.saturating_add(len) > self.bytes.len() {
            return Err(());
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.bytes[start..start + len])
    }
}
