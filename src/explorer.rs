use std::{
    collections::HashMap,
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
};

use crate::project::GitStatus;

#[derive(Debug)]
pub enum ExplorerRequest {
    RefreshGit {
        generation: u64,
        root: PathBuf,
    },
}

#[derive(Debug)]
pub enum ExplorerEvent {
    GitStatusReady {
        generation: u64,
        root: PathBuf,
        statuses: HashMap<PathBuf, GitStatus>,
    },
    GitStatusFailed {
        generation: u64,
        root: PathBuf,
        message: String,
    },
}

pub struct ExplorerWorker {
    requests: Sender<ExplorerRequest>,
    events: Receiver<ExplorerEvent>,
}

impl fmt::Debug for ExplorerWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExplorerWorker")
            .field("requests", &"Sender<ExplorerRequest>")
            .field("events", &"Receiver<ExplorerEvent>")
            .finish()
    }
}

impl ExplorerWorker {
    pub fn new() -> io::Result<Self> {
        let (request_sender, request_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();

        thread::Builder::new()
            .name("caret-explorer".to_string())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    match request {
                        ExplorerRequest::RefreshGit { generation, root } => {
                            let event = match scan_git_status(&root) {
                                Ok(statuses) => ExplorerEvent::GitStatusReady {
                                    generation,
                                    root,
                                    statuses,
                                },
                                Err(error) => ExplorerEvent::GitStatusFailed {
                                    generation,
                                    root,
                                    message: error.to_string(),
                                },
                            };

                            if event_sender.send(event).is_err() {
                                break;
                            }
                        }
                    }
                }
            })?;

        Ok(Self {
            requests: request_sender,
            events: event_receiver,
        })
    }

    pub fn refresh_git(&self, generation: u64, root: PathBuf) -> bool {
        self.requests
            .send(ExplorerRequest::RefreshGit { generation, root })
            .is_ok()
    }

    pub fn try_recv(&self) -> Option<ExplorerEvent> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

fn scan_git_status(root: &Path) -> io::Result<HashMap<PathBuf, GitStatus>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ])
        .output()?;

    if output.status.success() {
        return Ok(parse_git_status(root, &output.stdout));
    }

    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if message.contains("not a git repository") {
        return Ok(HashMap::new());
    }

    Err(io::Error::new(
        io::ErrorKind::Other,
        if message.is_empty() {
            "git status failed without an error message".to_string()
        } else {
            message
        },
    ))
}

fn parse_git_status(root: &Path, output: &[u8]) -> HashMap<PathBuf, GitStatus> {
    let mut statuses = HashMap::new();
    let mut records = output.split(|byte| *byte == 0).filter(|record| !record.is_empty());

    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            continue;
        }

        let code = &record[..2];
        let Some(status) = status_from_code(code) else {
            continue;
        };
        let relative = path_from_git_bytes(&record[3..]);
        insert_status(&mut statuses, root, &relative, status);

        // In porcelain v1's NUL-delimited format, rename and copy records have
        // a second path field. The first path is the destination, which is the
        // path present in the current project tree; consume the source field so
        // it is not mistaken for another status record.
        if code.contains(&b'R') || code.contains(&b'C') {
            let _ = records.next();
        }
    }

    statuses
}

fn status_from_code(code: &[u8]) -> Option<GitStatus> {
    if code == b"??" {
        return Some(GitStatus::Untracked);
    }
    if code == b"!!" {
        return None;
    }

    if code.contains(&b'U')
        || matches!(code, b"AA" | b"DD" | b"AU" | b"UA" | b"DU" | b"UD")
    {
        return Some(GitStatus::Modified);
    }
    if code.contains(&b'D') {
        return Some(GitStatus::Deleted);
    }
    if code.contains(&b'A') {
        return Some(GitStatus::Added);
    }
    if code.contains(&b'M')
        || code.contains(&b'T')
        || code.contains(&b'R')
        || code.contains(&b'C')
    {
        return Some(GitStatus::Modified);
    }

    None
}

fn insert_status(
    statuses: &mut HashMap<PathBuf, GitStatus>,
    root: &Path,
    relative: &Path,
    status: GitStatus,
) {
    let path = root.join(relative);
    merge_status(statuses, path.clone(), status);

    // Bubble a summary marker up through loaded directories. This lets a
    // collapsed folder communicate that it contains changes without scanning
    // or mutating the UI tree on the worker thread.
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == root || !directory.starts_with(root) {
            break;
        }
        merge_status(statuses, directory.to_path_buf(), status);
        parent = directory.parent();
    }
}

fn merge_status(statuses: &mut HashMap<PathBuf, GitStatus>, path: PathBuf, status: GitStatus) {
    match statuses.get_mut(&path) {
        Some(current) if status_priority(status) > status_priority(*current) => *current = status,
        Some(_) => {}
        None => {
            statuses.insert(path, status);
        }
    }
}

fn status_priority(status: GitStatus) -> u8 {
    match status {
        GitStatus::Untracked => 1,
        GitStatus::Modified => 2,
        GitStatus::Added => 3,
        GitStatus::Deleted => 4,
    }
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from(String::from_utf8_lossy(bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nul_delimited_paths_and_aggregates_directories() {
        let root = Path::new("/work");
        let statuses = parse_git_status(
            root,
            b" M src/main.rs\0?? docs/new file.md\0A  Cargo.lock\0",
        );

        assert_eq!(
            statuses.get(&root.join("src/main.rs")),
            Some(&GitStatus::Modified)
        );
        assert_eq!(statuses.get(&root.join("src")), Some(&GitStatus::Modified));
        assert_eq!(
            statuses.get(&root.join("docs/new file.md")),
            Some(&GitStatus::Untracked)
        );
        assert_eq!(statuses.get(&root.join("docs")), Some(&GitStatus::Untracked));
        assert_eq!(statuses.get(&root.join("Cargo.lock")), Some(&GitStatus::Added));
    }

    #[test]
    fn rename_source_is_not_parsed_as_an_independent_record() {
        let root = Path::new("/work");
        let statuses = parse_git_status(root, b"R  src/new.rs\0src/old.rs\0 M README.md\0");

        assert_eq!(
            statuses.get(&root.join("src/new.rs")),
            Some(&GitStatus::Modified)
        );
        assert!(!statuses.contains_key(&root.join("src/old.rs")));
        assert_eq!(
            statuses.get(&root.join("README.md")),
            Some(&GitStatus::Modified)
        );
    }

    #[test]
    fn stronger_status_wins_for_directory_summary() {
        let root = Path::new("/work");
        let statuses = parse_git_status(root, b"?? src/new.rs\0D  src/removed.rs\0");

        assert_eq!(statuses.get(&root.join("src")), Some(&GitStatus::Deleted));
    }
}
