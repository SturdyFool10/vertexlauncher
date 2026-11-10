use vertex_i18n::tr;
use super::*;



pub(super) fn server_player_list_tip(ping: Option<&ServerPingSnapshot>) -> Option<String> {
    let snapshot = ping?;
    let online = snapshot.players_online?;
    if snapshot.players_sample.is_empty() {
        return Some(if online == 0 {
            tr!("No players online").into_owned()
        } else {
            tr!("The server did not share which players are online").into_owned()
        });
    }
    let mut tip = tr!("Players online ({online}):", online = online).into_owned();
    for name in &snapshot.players_sample {
        tip.push_str("\n");
        tip.push_str(name);
    }
    let hidden = (online as usize).saturating_sub(snapshot.players_sample.len());
    if hidden > 0 {
        tip.push_str(&format!("\n…and {hidden} more"));
    }
    Some(tip)
}

pub(super) fn server_player_count_text(ping: Option<&ServerPingSnapshot>) -> Option<String> {
    let snapshot = ping?;
    Some(format!(
        "{}/{}",
        snapshot.players_online?, snapshot.players_max?
    ))
}
