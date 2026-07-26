use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::project::{
    load_entry_metadata, EntryMetadata, GitStatus, ProjectEntry, ProjectTree, TreeScanRequest,
    TreeScanner,
};

const GIT_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const GIT_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(150);
const WATCH_WAIT: Duration = Duration::from_millis(200);

#[derive(Debug)]
enum WatchSignal {
    Changed { paths: Vec<PathBuf>, rescan: bool },
    Failed(String),
}

struct NativeWatcher {
    events: Receiver<WatchSignal>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for NativeWatcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeWatcher")
            .field("running", &self.thread.is_some())
            .finish()
    }
}

impl NativeWatcher {
    fn new(root: PathBuf) -> io::Result<Self> {
        let (event_sender, events) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let watcher_thread = thread::Builder::new()
            .name("caret-fs-watch".to_string())
            .spawn(move || watch_directory(root, event_sender, ready_sender, thread_stop))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                events,
                stop,
                thread: Some(watcher_thread),
            }),
            Ok(Err(message)) => {
                let _ = watcher_thread.join();
                Err(io::Error::other(message))
            }
            Err(_) => {
                let _ = watcher_thread.join();
                Err(io::Error::other(
                    "filesystem watcher stopped during startup",
                ))
            }
        }
    }

    fn try_recv(&self) -> Option<WatchSignal> {
        self.events.try_recv().ok()
    }
}

impl Drop for NativeWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn watch_directory(
    root: PathBuf,
    events: Sender<WatchSignal>,
    ready: mpsc::SyncSender<Result<(), String>>,
    stop: Arc<AtomicBool>,
) {
    use notify::{EventKind, RecursiveMode, Watcher};

    let (native_sender, native_events) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(native_sender) {
        Ok(watcher) => watcher,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
        let _ = ready.send(Err(error.to_string()));
        return;
    }
    let _ = ready.send(Ok(()));

    while !stop.load(Ordering::Acquire) {
        match native_events.recv_timeout(WATCH_WAIT) {
            Ok(Ok(event)) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    continue;
                }
                if events
                    .send(WatchSignal::Changed {
                        rescan: event.need_rescan(),
                        paths: event.paths,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(Err(error)) => {
                // Backend errors can represent queue overflow or a lost native
                // watch. Force a full reconciliation before restarting.
                let _ = events.send(WatchSignal::Changed {
                    paths: Vec::new(),
                    rescan: true,
                });
                let _ = events.send(WatchSignal::Failed(error.to_string()));
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = events.send(WatchSignal::Changed {
                    paths: Vec::new(),
                    rescan: true,
                });
                let _ = events.send(WatchSignal::Failed(
                    "native filesystem event channel disconnected".to_string(),
                ));
                break;
            }
        }
    }
}

#[derive(Debug)]
enum ExplorerRequest {
    RefreshGit {
        generation: u64,
        root: PathBuf,
    },
    RefreshTree {
        generation: u64,
        request: TreeScanRequest,
    },
    LoadMetadata {
        generation: u64,
        root: PathBuf,
        paths: Vec<PathBuf>,
    },
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
    TreeReady {
        generation: u64,
        root: PathBuf,
        entries: Vec<ProjectEntry>,
    },
    TreeFailed {
        generation: u64,
        root: PathBuf,
        kind: io::ErrorKind,
        message: String,
    },
    MetadataReady {
        generation: u64,
        root: PathBuf,
        metadata: HashMap<PathBuf, EntryMetadata>,
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
                let mut tree_scanner = TreeScanner::default();
                while let Ok(request) = request_receiver.recv() {
                    match request {
                        ExplorerRequest::RefreshGit { generation, root } => {
                            let event = match scan_git_status(&root) {
                                Ok(statuses) => ExplorerEvent::GitStatusReady {
                                    generation,
                                    root,
                                    statuses,
                                },
                                Err(error) => {
                                    let message = error.to_string();
                                    let _ = crate::diagnostics::append(
                                        "filesystem",
                                        &format!(
                                            "background Git status failed for {}: {message}",
                                            root.display()
                                        ),
                                    );
                                    ExplorerEvent::GitStatusFailed {
                                        generation,
                                        root,
                                        message,
                                    }
                                }
                            };

                            if event_sender.send(event).is_err() {
                                break;
                            }
                        }
                        ExplorerRequest::RefreshTree {
                            generation,
                            request,
                        } => {
                            let root = request.root.clone();
                            let event = match tree_scanner.scan(&request) {
                                Ok(entries) => ExplorerEvent::TreeReady {
                                    generation,
                                    root,
                                    entries,
                                },
                                Err(error) => {
                                    let kind = error.kind();
                                    let message = error.to_string();
                                    let _ = crate::diagnostics::append(
                                        "filesystem",
                                        &format!(
                                            "background project scan failed for {}: {message}",
                                            root.display()
                                        ),
                                    );
                                    ExplorerEvent::TreeFailed {
                                        generation,
                                        root,
                                        kind,
                                        message,
                                    }
                                }
                            };
                            if event_sender.send(event).is_err() {
                                break;
                            }
                        }
                        ExplorerRequest::LoadMetadata {
                            generation,
                            root,
                            paths,
                        } => {
                            let metadata = load_entry_metadata(&paths);
                            tree_scanner.apply_metadata(&metadata);
                            if event_sender
                                .send(ExplorerEvent::MetadataReady {
                                    generation,
                                    root,
                                    metadata,
                                })
                                .is_err()
                            {
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

    fn refresh_tree(&self, generation: u64, request: TreeScanRequest) -> bool {
        self.requests
            .send(ExplorerRequest::RefreshTree {
                generation,
                request,
            })
            .is_ok()
    }

    fn load_metadata(&self, generation: u64, root: PathBuf, paths: Vec<PathBuf>) -> bool {
        self.requests
            .send(ExplorerRequest::LoadMetadata {
                generation,
                root,
                paths,
            })
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
    tree_generation: u64,
    tree_request_pending: bool,
    queued_tree_request: Option<TreeScanRequest>,
    watcher: Option<NativeWatcher>,
    watcher_error: Option<String>,
    watch_dirty_since: Option<Instant>,
    watch_last_event: Option<Instant>,
    watch_paths: HashSet<PathBuf>,
    watch_rescan: bool,
    last_watcher_retry: Option<Instant>,
}

impl ExplorerService {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        let (watcher, watcher_error) = match NativeWatcher::new(root.clone()) {
            Ok(watcher) => (Some(watcher), None),
            Err(error) => {
                let message = error.to_string();
                let _ = crate::diagnostics::append(
                    "filesystem",
                    &format!("native watcher failed for {}: {message}", root.display()),
                );
                (None, Some(message))
            }
        };
        Ok(Self {
            worker: ExplorerWorker::new()?,
            root,
            statuses: HashMap::new(),
            generation: 0,
            request_pending: false,
            refresh_queued: false,
            last_request: None,
            last_error: None,
            tree_generation: 0,
            tree_request_pending: false,
            queued_tree_request: None,
            watcher,
            watcher_error,
            watch_dirty_since: None,
            watch_last_event: None,
            watch_paths: HashSet::new(),
            watch_rescan: false,
            last_watcher_retry: None,
        })
    }

    /// Applies finished background work to the current tree and schedules the
    /// next refresh. Returns true when visible rows changed.
    pub fn poll(&mut self, project: &mut ProjectTree) -> bool {
        let root_changed = project.root != self.root;
        if root_changed {
            self.reset_for_root(project.root.clone());
        }
        self.poll_watcher(project);

        let refresh_requested = project.take_git_refresh_request();
        if root_changed || refresh_requested {
            if self.request_pending {
                self.refresh_queued = true;
            } else {
                self.schedule_refresh(true, true);
            }
        }

        if let Some(request) = project.take_tree_refresh_request() {
            if self.tree_request_pending {
                self.queued_tree_request = Some(request);
            } else {
                self.schedule_tree_refresh(request);
            }
        }

        let mut changed = false;
        let mut metadata_request = None;
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
                ExplorerEvent::TreeReady {
                    generation,
                    root,
                    entries,
                } if generation == self.tree_generation && root == self.root => {
                    self.tree_request_pending = false;
                    let paths = entries.iter().map(|entry| entry.path.clone()).collect();
                    changed |= project.apply_tree_snapshot(entries);
                    metadata_request = Some((generation, root, paths));
                }
                ExplorerEvent::TreeFailed {
                    generation,
                    root,
                    kind,
                    message,
                } if generation == self.tree_generation && root == self.root => {
                    self.tree_request_pending = false;
                    changed |= project.fail_tree_refresh(kind, message);
                }
                ExplorerEvent::MetadataReady {
                    generation,
                    root,
                    metadata,
                } if generation == self.tree_generation && root == self.root => {
                    let metadata_changed = project.apply_tree_metadata(&metadata);
                    changed |= metadata_changed;
                    if metadata_changed {
                        project.resort_after_metadata();
                    }
                }
                // The project changed, or a newer request superseded this one.
                // Stale results must never overwrite the active project.
                _ => {}
            }
        }
        if let Some((generation, root, paths)) = metadata_request {
            let _ = self.worker.load_metadata(generation, root, paths);
        }

        if !self.tree_request_pending {
            if let Some(request) = self.queued_tree_request.take() {
                self.schedule_tree_refresh(request);
            } else if let Some(request) = project.take_tree_refresh_request() {
                self.schedule_tree_refresh(request);
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
        self.root = root.clone();
        self.statuses.clear();
        self.last_error = None;
        self.last_request = None;
        self.request_pending = false;
        self.refresh_queued = false;
        self.generation = self.generation.wrapping_add(1);
        self.tree_generation = self.tree_generation.wrapping_add(1);
        self.tree_request_pending = false;
        self.queued_tree_request = None;
        self.watch_dirty_since = None;
        self.watch_last_event = None;
        self.watch_paths.clear();
        self.watch_rescan = false;
        self.last_watcher_retry = None;
        match NativeWatcher::new(root) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.watcher_error = None;
            }
            Err(error) => {
                self.watcher = None;
                self.watcher_error = Some(error.to_string());
            }
        }
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

    fn schedule_tree_refresh(&mut self, request: TreeScanRequest) {
        self.tree_generation = self.tree_generation.wrapping_add(1);
        if self.worker.refresh_tree(self.tree_generation, request) {
            self.tree_request_pending = true;
        }
    }

    fn poll_watcher(&mut self, project: &mut ProjectTree) {
        let mut watcher_failed = None;
        if let Some(watcher) = &self.watcher {
            while let Some(event) = watcher.try_recv() {
                match event {
                    WatchSignal::Changed { paths, rescan } => {
                        let now = Instant::now();
                        self.watch_dirty_since.get_or_insert(now);
                        self.watch_last_event = Some(now);
                        if rescan {
                            self.watch_rescan = true;
                            self.watch_paths.clear();
                        } else if !self.watch_rescan {
                            self.watch_paths.extend(paths);
                        }
                    }
                    WatchSignal::Failed(message) => watcher_failed = Some(message),
                }
            }
        }

        if let Some(message) = watcher_failed {
            let _ = crate::diagnostics::append(
                "filesystem",
                &format!(
                    "native watcher failed for {}: {message}",
                    self.root.display()
                ),
            );
            self.watcher = None;
            self.watcher_error = Some(message);
            self.last_watcher_retry = Some(Instant::now());
        }

        let settled = self
            .watch_last_event
            .is_some_and(|event| event.elapsed() >= WATCH_DEBOUNCE);
        let maximum_delay_reached = self
            .watch_dirty_since
            .is_some_and(|first| first.elapsed() >= Duration::from_secs(1));
        if self.watch_dirty_since.is_some() && (settled || maximum_delay_reached) {
            if self.watch_rescan || self.watch_paths.is_empty() {
                let _ = project.refresh();
            } else {
                let paths = self.watch_paths.drain().collect::<Vec<_>>();
                let _ = project.refresh_paths(&paths);
            }
            project.request_git_refresh();
            self.watch_dirty_since = None;
            self.watch_last_event = None;
            self.watch_rescan = false;
        }

        let should_retry = self.watcher.is_none()
            && self
                .last_watcher_retry
                .is_none_or(|attempt| attempt.elapsed() >= GIT_RETRY_INTERVAL);
        if should_retry {
            self.last_watcher_retry = Some(Instant::now());
            match NativeWatcher::new(self.root.clone()) {
                Ok(watcher) => {
                    self.watcher = Some(watcher);
                    self.watcher_error = None;
                }
                Err(error) => self.watcher_error = Some(error.to_string()),
            }
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
        return Some(GitStatus::Conflicted);
    }
    if code.contains(&b'R') {
        return Some(GitStatus::Renamed);
    }
    if code.contains(&b'D') {
        return Some(GitStatus::Deleted);
    }
    if code.contains(&b'A') {
        return Some(GitStatus::Added);
    }
    if code.contains(&b'M') || code.contains(&b'T') || code.contains(&b'C') {
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
        GitStatus::Renamed => 5,
        GitStatus::Conflicted => 6,
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

    fn temp_root(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "caret-explorer-{name}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn background_tree_scan_then_loads_metadata_progressively() {
        let root = temp_root("background-tree");
        std::fs::write(root.join("note.txt"), "hello").unwrap();
        let mut project = ProjectTree::new(root.clone()).unwrap();
        let mut service = ExplorerService::new(root.clone()).unwrap();
        assert!(project.entries.is_empty());
        assert!(project.tree_loading);

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_snapshot_without_metadata = false;
        let mut loaded_metadata = false;
        while Instant::now() < deadline {
            service.poll(&mut project);
            if let Some(entry) = project
                .entries
                .iter()
                .find(|entry| entry.name == "note.txt")
            {
                saw_snapshot_without_metadata |= entry.size.is_none();
                if entry.size == Some(5) {
                    loaded_metadata = true;
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(saw_snapshot_without_metadata);
        assert!(loaded_metadata);
        assert!(!project.tree_loading);
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_watcher_reports_filesystem_changes() {
        let root = temp_root("native-watch");
        let watcher = NativeWatcher::new(root.clone()).unwrap();
        std::fs::write(root.join("created.txt"), "created").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut changed = false;
        while Instant::now() < deadline {
            if matches!(watcher.try_recv(), Some(WatchSignal::Changed { .. })) {
                changed = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(changed);
        drop(watcher);
        let _ = std::fs::remove_dir_all(root);
    }

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
            Some(&GitStatus::Renamed)
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

    #[test]
    fn conflicts_and_renames_keep_distinct_git_states() {
        let root = Path::new("/work");
        let statuses = parse_git_status(root, b"UU src/conflict.rs\0R  src/new.rs\0src/old.rs\0");

        assert_eq!(
            statuses.get(&root.join("src/conflict.rs")),
            Some(&GitStatus::Conflicted)
        );
        assert_eq!(
            statuses.get(&root.join("src/new.rs")),
            Some(&GitStatus::Renamed)
        );
        assert_eq!(
            statuses.get(&root.join("src")),
            Some(&GitStatus::Conflicted)
        );
    }
}
