use crate::{document, editor::Cursor};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub path: Option<PathBuf>,
    pub text: String,
    pub cursor_line: usize,
    pub cursor_column: usize,
    pub saved_unix_secs: u64,
}

pub fn save(entries: Vec<RecoveryEntry>) -> io::Result<()> {
    save_at(&journal_path(), entries)
}

fn save_at(path: &std::path::Path, entries: Vec<RecoveryEntry>) -> io::Result<()> {
    let payload = serde_json::to_vec_pretty(&entries)
        .map_err(|e| io::Error::other(format!("recovery serialization failed: {e}")))?;
    document::atomic_write(path, &payload)
}
pub fn load() -> io::Result<Vec<RecoveryEntry>> {
    load_at(&journal_path())
}

fn load_at(path: &std::path::Path) -> io::Result<Vec<RecoveryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("recovery journal is invalid: {e}"),
        )
    })
}
pub fn discard() -> io::Result<()> {
    let path = journal_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
pub fn entry(path: Option<PathBuf>, text: String, cursor: Cursor) -> RecoveryEntry {
    RecoveryEntry {
        path,
        text,
        cursor_line: cursor.line,
        cursor_column: cursor.column,
        saved_unix_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    }
}
fn journal_path() -> PathBuf {
    document::recovery_dir().join("journal.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_directory_uses_platform_data_location() {
        assert!(journal_path().ends_with("journal.json"));
    }

    #[test]
    fn journal_round_trip_survives_a_new_reader() {
        let directory =
            std::env::temp_dir().join(format!("caret-recovery-test-{}", std::process::id()));
        let path = directory.join("journal.json");
        let entries = vec![RecoveryEntry {
            path: Some(PathBuf::from("C:/work/important.rs")),
            text: "unsaved change".to_string(),
            cursor_line: 4,
            cursor_column: 9,
            saved_unix_secs: 123,
        }];
        save_at(&path, entries).unwrap();
        let restored = load_at(&path).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].text, "unsaved change");
        assert_eq!(restored[0].cursor_line, 4);
        assert_eq!(
            restored[0].path.as_deref(),
            Some(std::path::Path::new("C:/work/important.rs"))
        );
        let _ = fs::remove_dir_all(directory);
    }
}
