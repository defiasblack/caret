//! Durable workspace state. Session data deliberately excludes terminal panes:
//! restarting a user's shell or process is unsafe and surprising.
use std::{fs, io, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{document, editor::Cursor};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub path: PathBuf,
    pub cursor: CursorState,
    pub scroll_line: usize,
    pub scroll_column: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CursorState {
    pub line: usize,
    pub column: usize,
}

impl From<Cursor> for CursorState {
    fn from(value: Cursor) -> Self {
        Self {
            line: value.line,
            column: value.column,
        }
    }
}
impl From<CursorState> for Cursor {
    fn from(value: CursorState) -> Self {
        Self {
            line: value.line,
            column: value.column,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub project_root: PathBuf,
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub sidebar_visible: bool,
    pub sidebar_outline: bool,
    #[serde(default)]
    pub split: Option<SplitState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewState {
    pub tab_index: usize,
    pub cursor: CursorState,
    pub scroll_line: usize,
    pub scroll_column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitState {
    pub primary: ViewState,
    pub secondary: ViewState,
    pub secondary_active: bool,
    pub vertical: bool,
}

pub fn load() -> io::Result<Option<SessionState>> {
    let path = session_path();
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("session is invalid: {error}"),
        )
    })
}

pub fn save(session: &SessionState) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(session)
        .map_err(|error| io::Error::other(format!("session serialization failed: {error}")))?;
    document::atomic_write(&session_path(), &bytes)
}

pub fn session_path() -> PathBuf {
    document::recovery_dir()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("session.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_uses_application_data_directory() {
        assert!(session_path().ends_with("session.json"));
    }

    #[test]
    fn session_round_trip_preserves_workspace_and_view_state() {
        let root = std::env::temp_dir().join(format!("caret-session-test-{}", std::process::id()));
        let path = root.join("session.json");
        let state = SessionState {
            project_root: PathBuf::from("C:/work/project"),
            tabs: vec![TabState {
                path: PathBuf::from("C:/work/project/src/main.rs"),
                cursor: CursorState {
                    line: 7,
                    column: 11,
                },
                scroll_line: 4,
                scroll_column: 2,
            }],
            active_tab: 0,
            sidebar_visible: false,
            sidebar_outline: true,
            split: Some(SplitState {
                primary: ViewState {
                    tab_index: 0,
                    cursor: CursorState {
                        line: 7,
                        column: 11,
                    },
                    scroll_line: 4,
                    scroll_column: 2,
                },
                secondary: ViewState {
                    tab_index: 0,
                    cursor: CursorState { line: 2, column: 3 },
                    scroll_line: 1,
                    scroll_column: 0,
                },
                secondary_active: true,
                vertical: true,
            }),
        };
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        save_at(&path, &state).unwrap();
        let bytes = fs::read(&path).unwrap();
        let restored: SessionState = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.project_root, state.project_root);
        assert_eq!(restored.tabs[0].cursor.line, 7);
        assert!(restored.split.as_ref().unwrap().vertical);
        let _ = fs::remove_dir_all(root);
    }

    fn save_at(path: &std::path::Path, session: &SessionState) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(session)
            .map_err(|error| io::Error::other(error.to_string()))?;
        document::atomic_write(path, &bytes)
    }
}
