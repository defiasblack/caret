use std::{
    collections::HashMap,
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use crate::project::{GitStatus, ProjectTree};

const GIT_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const GIT_RETRY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug)]
enum ExplorerRequest {
    RefreshGit { generation: u64, root: PathBuf },
}

#[derive(Debug)]
enum ExplorerEvent {
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

struct ExplorerWorker {
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
    fn new() -> io::Result<Self> {
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

    fn refresh_git(&self, generation: u64, root: PathBuf) -> bool {
        self.requests
            .send(ExplorerRequest::RefreshGit { generation, root })
            .is_ok()
    }

    fn try_recv(&self) -> Option<ExplorerEvent> {
        self.events.try_recv().ok()
    }
}

/// Owns background explorer work independently of the editor's application
/// state. The service intentionally applies completed snapshots only on the UI
/// thread, so worker code never mutates `ProjectTree` directly.
#[derive(Debug)]
pub struct ExplorerService {
    worker: ExplorerWorker,
    root: PathBuf,
    statuses: HashMap<PathBuf, GitStatus>,
    generation: u64,
    request_pending: bool,
    refresh_queued: bool,
    last_request: Option<Instant>,
    last_error: Option<String>,
}

impl ExplorerService {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        Ok(Self {
            worker: ExplorerWorker::new()?,
            root,
            statuses: HashMap::new(),
            generation: 0,
            request_pending: false,
            refresh_queued: false,
            last_request: None,
            last_error: None,
        })
    }

    /// Applies finished background work to the current tree and schedules the
    /// next refresh. Returns true when visible rows changed.
    pub fn poll(&mut self, project: &mut ProjectTree) -> bool {
        let root_changed = project.root != self.root;
        if root_changed {
            self.reset_for_root(project.root.clone());
        }

        let refresh_requested = project.take_git_refresh_request();
        if root_changed || refresh_requested {
            if self.request_pending {
                self.refresh_queued = true;
            } else {
                self.schedule_refresh(true, true);
            }
        }

        let mut changed = false;
        while let Some(event) = self.worker.try_recv() {
            match event {
                ExplorerEvent::GitStatusReady {
                    generation,
                    root,
                    statuses,
                } if generation == self.generation && root == self.root => {
                    self.request_pending = false;
                    self.last_error = None;
                    if statuses != self.statuses {
                        self.statuses = statuses;
                    }
                    changed |= self.apply_statuses(project);
                }
                ExplorerEvent::GitStatusFailed {
                    generation,
                    root,
                    message,
                } if generation == self.generation && root == self.root => {
                    self.request_pending = false;
                    self.last_error = Some(message);
                }
                // The project changed, or a newer request superseded this one.
                // Stale results must never overwrite the active project.
                _ => {}
            }
        }

        if self.refresh_queued && !self.request_pending {
            self.refresh_queued = false;
            self.schedule_refresh(true, true);
        } else {
            self.schedule_refresh(false, project.visible);
        }

        // Tree refreshes rebuild rows with empty status fields. Reapply the
        // cached snapshot even if no new worker event arrived this tick.
        changed |= self.apply_statuses(project);
        changed |= self.sync_project_state(project);
        changed
    }

    fn reset_for_root(&mut self, root: PathBuf) {
        self.root = root;
        self.statuses.clear();
        self.last_error = None;
        self.last_request = None;
        self.request_pending = false;
        self.refresh_queued = false;
        self.generation = self.generation.wrapping_add(1);
    }

    fn apply_statuses(&self, project: &mut ProjectTree) -> bool {
        let mut changed = false;
        for entry in &mut project.entries {
            let status = self.statuses.get(&entry.path).copied();
            if entry.git_status != status {
                entry.git_status = status;
                changed = true;
            }
        }
        changed
    }

    fn sync_project_state(&self, project: &mut ProjectTree) -> bool {
        let mut changed = false;
        if project.git_refreshing != self.request_pending {
            project.git_refreshing = self.request_pending;
            changed = true;
        }
        if project.git_error != self.last_error {
            project.git_error.clone_from(&self.last_error);
            changed = true;
        }
        changed
    }

    fn schedule_refresh(&mut self, force: bool, visible: bool) {
        if self.request_pending || (!force && !visible) {
            return;
        }

        let interval = if self.last_error.is_some() {
            GIT_RETRY_INTERVAL
        } else {
            GIT_REFRESH_INTERVAL
        };
        if !force
            && self
                .last_request
                .is_some_and(|last_request| last_request.elapsed() < interval)
        {
            return;
        }

        self.generation = self.generation.wrapping_add(1);
        if self.worker.refresh_git(self.generation, self.root.clone()) {
            self.request_pending = true;
            self.last_request = Some(Instant::now());
        } else {
            self.last_error = Some("explorer background worker stopped".to_string());
        }
    }
}

fn scan_git_status(root: &Path) -> io::Result<HashMap<PathBuf, GitStatus>> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error),
    };

    if output.status.success() {
        return Ok(parse_git_status(root, &output.stdout));
    }

    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if message.contains("not a git repository") {
        return Ok(HashMap::new());
    }

    Err(io::Error::other(if message.is_empty() {
        "git status failed without an error message".to_string()
    } else {
        message
    }))
}

fn parse_git_status(root: &Path, output: &[u8]) -> HashMap<PathBuf, GitStatus> {
    let mut statuses = HashMap::new();
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());

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

        // NUL-delimited rename/copy records contain a second source path. The
        // first path is the destination visible in the current project tree.
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

    if code.contains(&b'U') || matches!(code, b"AA" | b"DD" | b"AU" | b"UA" | b"DU" | b"UD") {
        return Some(GitStatus::Modified);
    }
    if code.contains(&b'D') {
        return Some(GitStatus::Deleted);
    }
    if code.contains(&b'A') {
        return Some(GitStatus::Added);
    }
    if code.contains(&b'M') || code.contains(&b'T') || code.contains(&b'R') || code.contains(&b'C')
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

    // Bubble a summary marker through parent folders so collapsed directories
    // can communicate that they contain changes.
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
        assert_eq!(
            statuses.get(&root.join("docs")),
            Some(&GitStatus::Untracked)
        );
        assert_eq!(
            statuses.get(&root.join("Cargo.lock")),
            Some(&GitStatus::Added)
        );
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
    fn scans_a_real_repository_with_spaces_in_paths() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "caret-explorer-git-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("folder with spaces")).unwrap();

        let initialized = Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(&root)
            .status()
            .is_ok_and(|status| status.success());
        if !initialized {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }

        std::fs::write(root.join("folder with spaces/new file.txt"), "content").unwrap();
        let statuses = scan_git_status(&root).unwrap();
        assert_eq!(
            statuses.get(&root.join("folder with spaces/new file.txt")),
            Some(&GitStatus::Untracked)
        );
        assert_eq!(
            statuses.get(&root.join("folder with spaces")),
            Some(&GitStatus::Untracked)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stronger_status_wins_for_directory_summary() {
        let root = Path::new("/work");
        let statuses = parse_git_status(root, b"?? src/new.rs\0D  src/removed.rs\0");

        assert_eq!(statuses.get(&root.join("src")), Some(&GitStatus::Deleted));
    }
}
