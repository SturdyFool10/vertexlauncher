use vertex_i18n::tr;
use super::*;



pub(super) fn world_meta_line(world: &WorldEntry, now_ms: u64) -> String {
    let mut parts = vec![
        tr!("instance {instance_name}", instance_name = world.instance_name).into_owned(),
        tr!("folder {world_id}", world_id = world.world_id).into_owned(),
        tr!("last used {last_used}", last_used = format_time_ago(world.last_used_at_ms, now_ms)).into_owned(),
    ];
    if let Some(game_mode) = world.game_mode.as_deref() {
        parts.push(game_mode.to_owned());
    }
    if let Some(difficulty) = world.difficulty.as_deref() {
        parts.push(tr!("difficulty {difficulty}", difficulty = difficulty).into_owned());
    }
    if let Some(hardcore) = world.hardcore {
        parts.push(if hardcore {
            "hardcore".to_owned()
        } else {
            "non-hardcore".to_owned()
        });
    }
    if let Some(cheats_enabled) = world.cheats_enabled {
        parts.push(if cheats_enabled {
            tr!("cheats on").into_owned()
        } else {
            tr!("cheats off").into_owned()
        });
    }
    if let Some(version_name) = world.version_name.as_deref() {
        parts.push(tr!("version {version_name}", version_name = version_name).into_owned());
    }
    parts.join(" | ")
}
