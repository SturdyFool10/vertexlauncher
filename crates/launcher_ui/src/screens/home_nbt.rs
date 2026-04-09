use super::*;

#[path = "home_nbt/nbt_cursor.rs"]
mod nbt_cursor;
#[path = "home_nbt/server_dat_entry.rs"]
mod server_dat_entry;
#[path = "home_nbt/world_metadata.rs"]
mod world_metadata;

use self::nbt_cursor::NbtCursor;
pub(super) use self::server_dat_entry::ServerDatEntry;
pub(super) use self::world_metadata::WorldMetadata;

pub(super) fn parse_world_metadata(path: &Path) -> Option<WorldMetadata> {
    let data = read_nbt_file(path)?;
    parse_world_metadata_from_nbt(data.as_slice()).ok()
}

fn parse_world_metadata_from_nbt(bytes: &[u8]) -> Result<WorldMetadata, ()> {
    let mut cursor = NbtCursor::new(bytes);
    let root_tag = cursor.read_u8()?;
    if root_tag != 10 {
        return Err(());
    }
    let _ = cursor.read_string()?;
    let mut metadata = WorldMetadata::default();
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            break;
        }
        let key = cursor.read_string()?;
        if tag == 10 && key == "Data" {
            parse_world_data_compound(&mut cursor, &mut metadata)?;
        } else {
            skip_nbt_payload(&mut cursor, tag)?;
        }
    }
    Ok(metadata)
}

fn parse_world_data_compound(
    cursor: &mut NbtCursor<'_>,
    metadata: &mut WorldMetadata,
) -> Result<(), ()> {
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            return Ok(());
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "LevelName") => metadata.level_name = Some(cursor.read_string()?),
            (3, "GameType") => metadata.game_mode = Some(game_mode_label(cursor.read_i32()?)),
            (1, "hardcore") => metadata.hardcore = Some(cursor.read_u8()? != 0),
            (1, "allowCommands") => metadata.cheats_enabled = Some(cursor.read_u8()? != 0),
            (1, "Difficulty") => metadata.difficulty = Some(difficulty_label(cursor.read_u8()?)),
            (4, "LastPlayed") => {
                let last_played = cursor.read_i64()?;
                if last_played > 0 {
                    metadata.last_played_ms = Some(last_played as u64);
                }
            }
            (10, "Version") => parse_world_version_compound(cursor, metadata)?,
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
}

fn parse_world_version_compound(
    cursor: &mut NbtCursor<'_>,
    metadata: &mut WorldMetadata,
) -> Result<(), ()> {
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            return Ok(());
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "Name") => metadata.version_name = Some(cursor.read_string()?),
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
}

fn game_mode_label(game_type: i32) -> String {
    match game_type {
        0 => "survival".to_owned(),
        1 => "creative".to_owned(),
        2 => "adventure".to_owned(),
        3 => "spectator".to_owned(),
        other => format!("mode {other}"),
    }
}

fn difficulty_label(value: u8) -> String {
    match value {
        0 => "peaceful".to_owned(),
        1 => "easy".to_owned(),
        2 => "normal".to_owned(),
        3 => "hard".to_owned(),
        other => format!("difficulty {other}"),
    }
}

fn read_nbt_file(path: &Path) -> Option<Vec<u8>> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return Some(Vec::new());
    }
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).ok()?;
        return Some(out);
    }
    Some(bytes)
}

pub(super) fn parse_servers_dat(path: &Path) -> Option<Vec<ServerDatEntry>> {
    let data = read_nbt_file(path)?;
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
    let mut icon = None;
    loop {
        let tag = cursor.read_u8()?;
        if tag == 0 {
            break;
        }
        let key = cursor.read_string()?;
        match (tag, key.as_str()) {
            (8, "name") => name = cursor.read_string()?,
            (8, "ip") => ip = cursor.read_string()?,
            (8, "icon") => icon = Some(cursor.read_string()?),
            _ => skip_nbt_payload(cursor, tag)?,
        }
    }
    if ip.trim().is_empty() {
        return Ok(None);
    }
    if name.trim().is_empty() {
        name = ip.clone();
    }
    Ok(Some(ServerDatEntry { name, ip, icon }))
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
