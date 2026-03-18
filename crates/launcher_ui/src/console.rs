use std::collections::{HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use installation::{is_instance_running, normalize_path_key, stop_running_instance};
use launcher_runtime as tokio_runtime;

const MAX_CONSOLE_LINES: usize = 4000;
const DEFAULT_TAB_ID: &str = "vertexlauncher";
const DEFAULT_TAB_LABEL: &str = "VertexLauncher";
const INSTANCE_TAB_PRUNE_GRACE: Duration = Duration::from_secs(10);
const LOG_TAIL_POLL_INTERVAL: Duration = Duration::from_millis(120);
const LOG_TAIL_EXIT_GRACE: Duration = Duration::from_secs(2);
const MAX_UNTERMINATED_LOG_CHARS: usize = 131_072;

#[derive(Clone, Debug)]
pub struct ConsoleTabSnapshot {
    pub id: String,
    pub label: String,
    pub can_close: bool,
}

#[derive(Clone, Debug)]
pub struct ConsoleSnapshot {
    pub tabs: Vec<ConsoleTabSnapshot>,
    pub active_tab_id: String,
    pub active_lines: Vec<String>,
}

#[derive(Debug)]
struct ConsoleTab {
    id: String,
    label: String,
    instance_root: Option<String>,
    user_identity: Option<String>,
    keep_alive_while_loading: bool,
    missing_since: Option<Instant>,
    lines: VecDeque<String>,
}

#[derive(Debug)]
struct ConsoleState {
    tabs: Vec<ConsoleTab>,
    active_tab_id: String,
}

static CONSOLE_STATE: OnceLock<Mutex<ConsoleState>> = OnceLock::new();

fn store() -> &'static Mutex<ConsoleState> {
    CONSOLE_STATE.get_or_init(|| {
        Mutex::new(ConsoleState {
            tabs: vec![ConsoleTab {
                id: DEFAULT_TAB_ID.to_owned(),
                label: DEFAULT_TAB_LABEL.to_owned(),
                instance_root: None,
                user_identity: None,
                keep_alive_while_loading: false,
                missing_since: None,
                lines: VecDeque::new(),
            }],
            active_tab_id: DEFAULT_TAB_ID.to_owned(),
        })
    })
}

pub fn push_line(line: impl Into<String>) {
    push_line_to_tab(DEFAULT_TAB_ID, line);
}

pub fn push_line_to_tab(tab_id: &str, line: impl Into<String>) {
    let sanitized = sanitize_console_line(line.into());
    let Ok(mut lines) = store().lock() else {
        return;
    };

    if let Some(tab) = lines.tabs.iter_mut().find(|tab| tab.id == tab_id) {
        tab.lines.push_back(sanitized);
        while tab.lines.len() > MAX_CONSOLE_LINES {
            let _ = tab.lines.pop_front();
        }
        return;
    }

    // Keep logs visible even if a tab disappears unexpectedly.
    if let Some(default_tab) = lines.tabs.iter_mut().find(|tab| tab.id == DEFAULT_TAB_ID) {
        default_tab.lines.push_back(sanitized);
        while default_tab.lines.len() > MAX_CONSOLE_LINES {
            let _ = default_tab.lines.pop_front();
        }
    }
}

pub fn ensure_instance_tab(
    instance_name: &str,
    username: &str,
    instance_root: &str,
    user_key: Option<&str>,
) -> String {
    let trimmed_instance = instance_name.trim();
    let trimmed_user = username.trim();
    let instance = if trimmed_instance.is_empty() {
        "Instance"
    } else {
        trimmed_instance
    };
    let user = if trimmed_user.is_empty() {
        "Player"
    } else {
        trimmed_user
    };
    let label = format!("{instance} for {user}");
    let normalized_instance_root = normalize_instance_root_key(instance_root);
    let normalized_user_identity =
        normalize_tab_user_identity(user_key).or_else(|| normalize_tab_user_identity(Some(user)));
    let fallback_user_identity = user.to_ascii_lowercase();
    let user_identity_id = normalized_user_identity
        .as_deref()
        .unwrap_or(fallback_user_identity.as_str());
    let root_id = normalized_instance_root
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| instance_root.trim().to_ascii_lowercase());
    let id = format!(
        "instance:{}:{}:{}",
        instance.to_ascii_lowercase(),
        user_identity_id,
        root_id
    );

    let Ok(mut state) = store().lock() else {
        return id;
    };
    if let Some(existing_index) = state.tabs.iter().position(|tab| tab.id == id) {
        {
            let existing = &mut state.tabs[existing_index];
            existing.label = label.clone();
            existing.instance_root = normalized_instance_root.clone();
            existing.user_identity = normalized_user_identity.clone();
            existing.keep_alive_while_loading = false;
            existing.missing_since = None;
        }
        if let (Some(root), Some(user_identity)) = (
            normalized_instance_root.as_deref(),
            normalized_user_identity.as_deref(),
        ) {
            collapse_tabs_for_instance_user(&mut state, root, user_identity, id.as_str());
        }
        state.active_tab_id = id.clone();
        return id;
    }

    if let (Some(root), Some(user_identity)) = (
        normalized_instance_root.as_deref(),
        normalized_user_identity.as_deref(),
    ) && let Some(existing_index) = state.tabs.iter().position(|tab| {
        tab.instance_root.as_deref() == Some(root)
            && tab.user_identity.as_deref() == Some(user_identity)
    }) {
        {
            let existing = &mut state.tabs[existing_index];
            existing.id = id.clone();
            existing.label = label;
            existing.instance_root = Some(root.to_owned());
            existing.user_identity = Some(user_identity.to_owned());
            existing.keep_alive_while_loading = false;
            existing.missing_since = None;
        }
        collapse_tabs_for_instance_user(&mut state, root, user_identity, id.as_str());
        state.active_tab_id = id.clone();
        return id;
    }

    state.tabs.push(ConsoleTab {
        id: id.clone(),
        label,
        instance_root: normalized_instance_root,
        user_identity: normalized_user_identity,
        keep_alive_while_loading: false,
        missing_since: None,
        lines: VecDeque::new(),
    });
    state.active_tab_id = id.clone();
    id
}

fn collapse_tabs_for_instance_user(
    state: &mut ConsoleState,
    instance_root: &str,
    user_identity: &str,
    keep_id: &str,
) {
    let mut kept = false;
    state.tabs.retain(|tab| {
        if tab.instance_root.as_deref() != Some(instance_root)
            || tab.user_identity.as_deref() != Some(user_identity)
        {
            return true;
        }
        if tab.id == keep_id && !kept {
            kept = true;
            return true;
        }
        false
    });
}

pub fn set_instance_tab_loading(instance_root: &str, user_key: Option<&str>, loading: bool) {
    let Some(normalized) = normalize_instance_root_key(instance_root) else {
        return;
    };
    let normalized_user_identity = normalize_tab_user_identity(user_key);
    let Ok(mut state) = store().lock() else {
        return;
    };
    for tab in &mut state.tabs {
        if tab.instance_root.as_deref() != Some(normalized.as_str()) {
            continue;
        }
        if let Some(user_identity) = normalized_user_identity.as_deref()
            && tab.user_identity.as_deref() != Some(user_identity)
        {
            continue;
        }
        tab.keep_alive_while_loading = loading;
        if loading {
            tab.missing_since = None;
        }
    }
}

pub fn attach_launch_log(tab_id: &str, instance_root: &str, log_path: &Path) {
    let trimmed_tab_id = tab_id.trim();
    if trimmed_tab_id.is_empty() {
        return;
    }
    let Some(instance_root_key) = normalize_instance_root_key(instance_root) else {
        return;
    };
    let tab_id = trimmed_tab_id.to_owned();
    let log_path = log_path.to_path_buf();
    let _ = tokio_runtime::spawn_blocking(move || {
        tail_launch_log_to_tab(
            tab_id.as_str(),
            instance_root_key.as_str(),
            log_path.as_path(),
        );
    });
}

pub fn prune_instance_tabs(active_instance_roots: &[String]) {
    let Ok(mut state) = store().lock() else {
        return;
    };
    let now = Instant::now();
    let active_roots: HashSet<String> = active_instance_roots
        .iter()
        .filter_map(|root| normalize_instance_root_key(root.as_str()))
        .collect();

    for tab in &mut state.tabs {
        if let Some(root) = tab.instance_root.as_deref()
            && let Some(normalized_root) = normalize_instance_root_key(root)
        {
            tab.instance_root = Some(normalized_root);
        }
        let Some(root) = tab.instance_root.as_deref() else {
            continue;
        };
        if active_roots.contains(root) {
            tab.missing_since = None;
        } else if tab.missing_since.is_none() {
            tab.missing_since = Some(now);
        }
    }

    state.tabs.retain(|tab| {
        let Some(_) = tab.instance_root.as_deref() else {
            return true;
        };
        if tab.keep_alive_while_loading {
            return true;
        }
        tab.missing_since.is_none_or(|missing_since| {
            now.duration_since(missing_since) < INSTANCE_TAB_PRUNE_GRACE
        })
    });

    if state.tabs.is_empty() {
        state.tabs.push(ConsoleTab {
            id: DEFAULT_TAB_ID.to_owned(),
            label: DEFAULT_TAB_LABEL.to_owned(),
            instance_root: None,
            user_identity: None,
            keep_alive_while_loading: false,
            missing_since: None,
            lines: VecDeque::new(),
        });
    }

    if !state.tabs.iter().any(|tab| tab.id == state.active_tab_id) {
        state.active_tab_id = DEFAULT_TAB_ID.to_owned();
    }
}

pub fn set_active_tab(tab_id: &str) {
    let Ok(mut state) = store().lock() else {
        return;
    };
    if state.tabs.iter().any(|tab| tab.id == tab_id) {
        state.active_tab_id = tab_id.to_owned();
    }
}

pub fn activate_tab_for_user(user_key: Option<&str>, username: Option<&str>) -> bool {
    let normalized_user_key = normalize_tab_user_identity(user_key);
    let normalized_username = normalize_tab_user_identity(username);
    let Ok(mut state) = store().lock() else {
        return false;
    };

    let tab_for_identity = |identity: &str, state: &ConsoleState| -> Option<String> {
        state
            .tabs
            .iter()
            .rev()
            .find(|tab| tab.user_identity.as_deref() == Some(identity))
            .map(|tab| tab.id.clone())
    };

    let selected_tab_id = normalized_user_key
        .as_deref()
        .and_then(|identity| tab_for_identity(identity, &state))
        .or_else(|| {
            normalized_username
                .as_deref()
                .and_then(|identity| tab_for_identity(identity, &state))
        });

    if let Some(tab_id) = selected_tab_id {
        state.active_tab_id = tab_id;
        return true;
    }

    false
}

pub fn is_default_tab(tab_id: &str) -> bool {
    tab_id == DEFAULT_TAB_ID
}

pub fn close_tab(tab_id: &str) -> bool {
    if is_default_tab(tab_id) {
        return false;
    }

    let Ok(mut state) = store().lock() else {
        return false;
    };

    let Some(index) = state.tabs.iter().position(|tab| tab.id == tab_id) else {
        return false;
    };

    let removed = state.tabs.remove(index);
    let instance_root_to_stop = removed.instance_root;

    if state.tabs.is_empty() {
        state.tabs.push(ConsoleTab {
            id: DEFAULT_TAB_ID.to_owned(),
            label: DEFAULT_TAB_LABEL.to_owned(),
            instance_root: None,
            user_identity: None,
            keep_alive_while_loading: false,
            missing_since: None,
            lines: VecDeque::new(),
        });
    }

    if state.active_tab_id == tab_id {
        state.active_tab_id = if state.tabs.iter().any(|tab| tab.id == DEFAULT_TAB_ID) {
            DEFAULT_TAB_ID.to_owned()
        } else {
            state.tabs[0].id.clone()
        };
    }
    drop(state);

    if let Some(instance_root) = instance_root_to_stop.as_deref() {
        let _ = stop_running_instance(Path::new(instance_root));
    }

    true
}

pub fn snapshot() -> ConsoleSnapshot {
    let Ok(state) = store().lock() else {
        return ConsoleSnapshot {
            tabs: vec![ConsoleTabSnapshot {
                id: DEFAULT_TAB_ID.to_owned(),
                label: DEFAULT_TAB_LABEL.to_owned(),
                can_close: false,
            }],
            active_tab_id: DEFAULT_TAB_ID.to_owned(),
            active_lines: Vec::new(),
        };
    };

    let active_lines = state
        .tabs
        .iter()
        .find(|tab| tab.id == state.active_tab_id)
        .map(|tab| tab.lines.iter().cloned().collect())
        .unwrap_or_default();
    let tabs = state
        .tabs
        .iter()
        .map(|tab| ConsoleTabSnapshot {
            id: tab.id.clone(),
            label: tab.label.clone(),
            can_close: !is_default_tab(tab.id.as_str()),
        })
        .collect();

    ConsoleSnapshot {
        tabs,
        active_tab_id: state.active_tab_id.clone(),
        active_lines,
    }
}

fn normalize_instance_root_key(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(normalize_path_key(Path::new(trimmed)))
}

fn normalize_tab_user_identity(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn tail_launch_log_to_tab(tab_id: &str, instance_root: &str, log_path: &Path) {
    let mut offset = 0u64;
    let mut pending_line = String::new();
    let mut stopped_since: Option<Instant> = None;
    loop {
        let mut advanced = false;
        if let Ok(metadata) = std::fs::metadata(log_path) {
            let file_len = metadata.len();
            if file_len < offset {
                offset = 0;
                pending_line.clear();
            }
            if file_len > offset
                && let Ok((chunk, new_offset)) = read_log_chunk(log_path, offset)
            {
                offset = new_offset;
                if !chunk.is_empty() {
                    pending_line.push_str(chunk.as_str());
                    flush_complete_log_lines(tab_id, &mut pending_line);
                    advanced = true;
                }
            }
        }

        if is_instance_running(Path::new(instance_root)) {
            stopped_since = None;
        } else {
            let now = Instant::now();
            if stopped_since.is_none() {
                stopped_since = Some(now);
            }
            let file_complete = std::fs::metadata(log_path)
                .map(|meta| meta.len() <= offset)
                .unwrap_or(true);
            if file_complete
                && !advanced
                && stopped_since
                    .is_some_and(|since| now.duration_since(since) >= LOG_TAIL_EXIT_GRACE)
            {
                break;
            }
        }

        std::thread::sleep(LOG_TAIL_POLL_INTERVAL);
    }
    if !pending_line.trim().is_empty() {
        push_line_to_tab(tab_id, pending_line.trim_end_matches('\r').to_owned());
    }
}

fn read_log_chunk(path: &Path, offset: u64) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let chunk = String::from_utf8_lossy(bytes.as_slice()).into_owned();
    Ok((chunk, offset.saturating_add(bytes.len() as u64)))
}

fn flush_complete_log_lines(tab_id: &str, pending_line: &mut String) {
    let mut consumed = 0usize;
    while let Some(relative_end) = pending_line[consumed..].find('\n') {
        let end = consumed + relative_end;
        let line = pending_line[consumed..end]
            .trim_end_matches('\r')
            .to_owned();
        push_line_to_tab(tab_id, line);
        consumed = end + 1;
    }
    if consumed > 0 {
        pending_line.drain(..consumed);
    }
    while pending_line.chars().count() > MAX_UNTERMINATED_LOG_CHARS {
        let split_at = byte_index_at_char_limit(pending_line.as_str(), MAX_UNTERMINATED_LOG_CHARS);
        if split_at == 0 {
            break;
        }
        let chunk = pending_line[..split_at].to_owned();
        push_line_to_tab(tab_id, chunk);
        pending_line.drain(..split_at);
    }
}

fn sanitize_console_line(mut line: String) -> String {
    line.retain(|ch| ch == '\t' || !ch.is_control());
    line
}

fn byte_index_at_char_limit(value: &str, max_chars: usize) -> usize {
    value
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}
