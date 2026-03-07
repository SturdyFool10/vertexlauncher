use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

const MAX_CONSOLE_LINES: usize = 4000;
const DEFAULT_TAB_ID: &str = "vertexlauncher";
const DEFAULT_TAB_LABEL: &str = "VertexLauncher";

#[derive(Clone, Debug)]
pub struct ConsoleTabSnapshot {
    pub id: String,
    pub label: String,
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
    let Ok(mut lines) = store().lock() else {
        return;
    };
    let Some(tab) = lines.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
        return;
    };
    tab.lines.push_back(line.into());
    while tab.lines.len() > MAX_CONSOLE_LINES {
        let _ = tab.lines.pop_front();
    }
}

pub fn ensure_instance_tab(instance_name: &str, username: &str) -> String {
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
    let id = format!(
        "instance:{}:{}",
        instance.to_ascii_lowercase(),
        user.to_ascii_lowercase()
    );

    let Ok(mut state) = store().lock() else {
        return id;
    };
    if !state.tabs.iter().any(|tab| tab.id == id) {
        state.tabs.push(ConsoleTab {
            id: id.clone(),
            label,
            lines: VecDeque::new(),
        });
    }
    state.active_tab_id = id.clone();
    id
}

pub fn set_active_tab(tab_id: &str) {
    let Ok(mut state) = store().lock() else {
        return;
    };
    if state.tabs.iter().any(|tab| tab.id == tab_id) {
        state.active_tab_id = tab_id.to_owned();
    }
}

pub fn snapshot() -> ConsoleSnapshot {
    let Ok(state) = store().lock() else {
        return ConsoleSnapshot {
            tabs: vec![ConsoleTabSnapshot {
                id: DEFAULT_TAB_ID.to_owned(),
                label: DEFAULT_TAB_LABEL.to_owned(),
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
        })
        .collect();

    ConsoleSnapshot {
        tabs,
        active_tab_id: state.active_tab_id.clone(),
        active_lines,
    }
}
