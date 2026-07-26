use crate::{document, editor::Cursor};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::PathBuf,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

static JOURNAL_ID: OnceLock<String> = OnceLock::new();

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
    let loaded = load_from_directory(&document::recovery_dir())?;
    for warning in &loaded.warnings {
        let _ = crate::diagnostics::append("recovery", warning);
    }
    if loaded.entries.is_empty() {
        if let Some(warning) = loaded.warnings.first() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, warning.clone()));
        }
    }
    Ok(loaded.entries)
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
pub fn discard_current() -> io::Result<()> {
    let path = journal_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn discard_all() -> io::Result<()> {
    for path in journal_paths(&document::recovery_dir())? {
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
    let name = JOURNAL_ID
        .get_or_init(|| {
            let started = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("journal-{}-{started}.json", std::process::id())
        })
        .clone();
    document::recovery_dir().join(name)
}

#[derive(Debug, Default)]
struct LoadedRecovery {
    entries: Vec<RecoveryEntry>,
    warnings: Vec<String>,
}

fn load_from_directory(directory: &std::path::Path) -> io::Result<LoadedRecovery> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for path in journal_paths(directory)? {
        match load_at(&path) {
            Ok(journal_entries) => entries.extend(journal_entries),
            Err(error) => warnings.push(format!("{}: {error}", path.display())),
        }
    }
    entries.sort_by_key(|entry| entry.saved_unix_secs);
    Ok(LoadedRecovery { entries, warnings })
}

fn journal_paths(directory: &std::path::Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let reader = match fs::read_dir(directory) {
        Ok(reader) => reader,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(paths),
        Err(error) => return Err(error),
    };
    for entry in reader {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "journal.json" || (name.starts_with("journal-") && name.ends_with(".json")) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_directory_uses_platform_data_location() {
        assert_eq!(
            journal_path().parent(),
            Some(document::recovery_dir().as_path())
        );
        assert!(journal_path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("journal-") && name.ends_with(".json")));
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

    #[test]
    fn corrupt_journal_is_reported_instead_of_being_silently_discarded() {
        let directory =
            std::env::temp_dir().join(format!("caret-recovery-invalid-{}", std::process::id()));
        let path = directory.join("journal.json");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"not json").unwrap();
        let error = load_at(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn separate_process_journals_are_combined_without_overwriting() {
        let directory =
            std::env::temp_dir().join(format!("caret-recovery-multi-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        for (pid, text, timestamp) in [(101, "first", 20), (202, "second", 10)] {
            save_at(
                &directory.join(format!("journal-{pid}.json")),
                vec![RecoveryEntry {
                    path: None,
                    text: text.to_string(),
                    cursor_line: 0,
                    cursor_column: 0,
                    saved_unix_secs: timestamp,
                }],
            )
            .unwrap();
        }

        let loaded = load_from_directory(&directory).unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].text, "second");
        assert_eq!(loaded.entries[1].text, "first");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn legacy_single_journal_is_still_discovered() {
        let directory =
            std::env::temp_dir().join(format!("caret-recovery-legacy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        save_at(
            &directory.join("journal.json"),
            vec![RecoveryEntry {
                path: None,
                text: "legacy".to_string(),
                cursor_line: 0,
                cursor_column: 0,
                saved_unix_secs: 1,
            }],
        )
        .unwrap();

        assert_eq!(
            load_from_directory(&directory).unwrap().entries[0].text,
            "legacy"
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn one_corrupt_journal_does_not_hide_valid_recovery_entries() {
        let directory =
            std::env::temp_dir().join(format!("caret-recovery-partial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("journal-1-corrupt.json"), b"not json").unwrap();
        save_at(
            &directory.join("journal-2-valid.json"),
            vec![RecoveryEntry {
                path: None,
                text: "still recoverable".to_string(),
                cursor_line: 0,
                cursor_column: 0,
                saved_unix_secs: 1,
            }],
        )
        .unwrap();

        let loaded = load_from_directory(&directory).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].text, "still recoverable");
        assert_eq!(loaded.warnings.len(), 1);
        let _ = fs::remove_dir_all(directory);
    }
}
