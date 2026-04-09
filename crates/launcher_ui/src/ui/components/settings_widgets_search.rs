use egui;

pub(super) const DROPDOWN_POPUP_FOCUS_PENDING_ID: &str =
    "settings_widgets_dropdown_popup_focus_pending";
pub(super) const DROPDOWN_OWNER_FOCUS_PENDING_ID: &str =
    "settings_widgets_dropdown_owner_focus_pending";
pub(super) const DROPDOWN_POPUP_HAD_FOCUS_ID: &str = "settings_widgets_dropdown_popup_had_focus";

#[derive(Clone, Debug, Default)]
pub(super) struct SearchableDropdownState {
    pub query: String,
}

pub(super) fn set_popup_focus_pending(ctx: &egui::Context, open_id: egui::Id, pending: bool) {
    ctx.data_mut(|data| {
        let key = egui::Id::new((DROPDOWN_POPUP_FOCUS_PENDING_ID, open_id));
        if pending {
            data.insert_temp(key, true);
        } else {
            data.remove::<bool>(key);
        }
    });
}

pub(super) fn take_popup_focus_pending(ctx: &egui::Context, open_id: egui::Id) -> bool {
    ctx.data_mut(|data| {
        let key = egui::Id::new((DROPDOWN_POPUP_FOCUS_PENDING_ID, open_id));
        let pending = data.get_temp::<bool>(key).unwrap_or(false);
        if pending {
            data.remove::<bool>(key);
        }
        pending
    })
}

pub(super) fn set_owner_focus_pending(ctx: &egui::Context, open_id: egui::Id, pending: bool) {
    ctx.data_mut(|data| {
        let key = egui::Id::new((DROPDOWN_OWNER_FOCUS_PENDING_ID, open_id));
        if pending {
            data.insert_temp(key, true);
        } else {
            data.remove::<bool>(key);
        }
    });
}

pub(super) fn take_owner_focus_pending(ctx: &egui::Context, open_id: egui::Id) -> bool {
    ctx.data_mut(|data| {
        let key = egui::Id::new((DROPDOWN_OWNER_FOCUS_PENDING_ID, open_id));
        let pending = data.get_temp::<bool>(key).unwrap_or(false);
        if pending {
            data.remove::<bool>(key);
        }
        pending
    })
}

pub(super) fn set_popup_had_focus(ctx: &egui::Context, open_id: egui::Id, had_focus: bool) {
    ctx.data_mut(|data| {
        let key = egui::Id::new((DROPDOWN_POPUP_HAD_FOCUS_ID, open_id));
        if had_focus {
            data.insert_temp(key, true);
        } else {
            data.remove::<bool>(key);
        }
    });
}

pub(super) fn take_popup_had_focus(ctx: &egui::Context, open_id: egui::Id) -> bool {
    ctx.data_mut(|data| {
        let key = egui::Id::new((DROPDOWN_POPUP_HAD_FOCUS_ID, open_id));
        let had_focus = data.get_temp::<bool>(key).unwrap_or(false);
        if had_focus {
            data.remove::<bool>(key);
        }
        had_focus
    })
}

pub(super) fn searchable_dropdown_matches(options: &[&str], query: &str) -> Vec<usize> {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return (0..options.len()).collect();
    }
    let query_chars = normalized_query.chars().collect::<Vec<_>>();

    let mut matches = options
        .iter()
        .enumerate()
        .filter_map(|(index, option)| {
            fuzzy_match_score(normalized_query.as_str(), query_chars.as_slice(), option)
                .map(|score| (index, score))
        })
        .collect::<Vec<_>>();

    matches.sort_by(|(left_index, left_score), (right_index, right_score)| {
        right_score
            .category
            .cmp(&left_score.category)
            .then_with(|| right_score.score.cmp(&left_score.score))
            .then_with(|| left_score.start.cmp(&right_score.start))
            .then_with(|| left_index.cmp(right_index))
    });

    matches.into_iter().map(|(index, _score)| index).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FuzzyMatchScore {
    category: i32,
    score: i32,
    start: usize,
}

fn fuzzy_match_score(
    normalized_query: &str,
    query_chars: &[char],
    candidate: &str,
) -> Option<FuzzyMatchScore> {
    if normalized_query.is_empty() {
        return Some(FuzzyMatchScore {
            category: 0,
            score: 0,
            start: 0,
        });
    }

    let normalized_candidate = candidate.trim().to_lowercase();
    if normalized_candidate.is_empty() {
        return None;
    }

    if normalized_candidate == normalized_query {
        return Some(FuzzyMatchScore {
            category: 3,
            score: i32::MAX,
            start: 0,
        });
    }

    if let Some(start) = normalized_candidate.find(&normalized_query) {
        return Some(FuzzyMatchScore {
            category: 2,
            score: 10_000 - start as i32,
            start,
        });
    }

    let candidate_chars = normalized_candidate.chars().collect::<Vec<_>>();
    let mut query_index = 0;
    let mut first_match_start = None;
    let mut previous_match_index = None;
    let mut score = 0_i32;

    for (candidate_index, candidate_char) in candidate_chars.iter().enumerate() {
        if query_index >= query_chars.len() {
            break;
        }

        if *candidate_char != query_chars[query_index] {
            continue;
        }

        if first_match_start.is_none() {
            first_match_start = Some(candidate_index);
        }

        score += 10;
        if let Some(previous_match_index) = previous_match_index {
            if candidate_index == previous_match_index + 1 {
                score += 6;
            } else {
                score -= (candidate_index - previous_match_index - 1) as i32;
            }
        }

        previous_match_index = Some(candidate_index);
        query_index += 1;
    }

    if query_index != query_chars.len() {
        return None;
    }

    Some(FuzzyMatchScore {
        category: 1,
        score,
        start: first_match_start.unwrap_or(0),
    })
}
