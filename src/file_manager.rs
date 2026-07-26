use std::{
    collections::{BTreeSet, HashMap},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::file_ops::{
    self, ConflictPolicy, OperationKind, OperationRequest, OperationSummary, UndoRecord,
};
pub use crate::preview::Preview;
use crate::preview::{self, PreviewRequest};
use serde::{Deserialize, Serialize};

const MAX_DIRECTORY_ENTRIES: usize = 50_000;
const MAX_CACHED_DIRECTORIES: usize = 128;
const PREVIEW_TIMEOUT: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortMode {
    #[default]
    Name,
    Size,
    Modified,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Modified,
            Self::Modified => Self::Name,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "modified",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    pub size: u64,
    pub modified_unix_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationProgress {
    pub id: u64,
    pub kind: OperationKind,
    pub completed: usize,
    pub total: usize,
    pub current: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Parent,
    Current,
}

enum Request {
    Scan {
        generation: u64,
        pane: Pane,
        directory: PathBuf,
        show_hidden: bool,
        directories_first: bool,
        sort: SortMode,
    },
    Preview {
        generation: u64,
        request: PreviewRequest,
    },
    Operation {
        request: OperationRequest,
        cancelled: Arc<AtomicBool>,
    },
    Undo {
        id: u64,
        record: UndoRecord,
        cancelled: Arc<AtomicBool>,
    },
    BulkRename {
        id: u64,
        pairs: Vec<(PathBuf, PathBuf)>,
        cancelled: Arc<AtomicBool>,
    },
    Stop,
}

enum WorkerEvent {
    Scanned {
        generation: u64,
        pane: Pane,
        directory: PathBuf,
        result: io::Result<Vec<FileEntry>>,
    },
    Previewed {
        generation: u64,
        path: PathBuf,
        preview: Preview,
    },
    Progress(OperationProgress),
    OperationFinished(OperationSummary),
    DirectoryChanged {
        generation: u64,
        directory: PathBuf,
    },
}

struct Worker {
    tx: Sender<Request>,
    rx: Receiver<WorkerEvent>,
}

impl Worker {
    fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<Request>();
        let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>();
        thread::Builder::new()
            .name("caret-file-manager".to_string())
            .spawn(move || {
                let mut watched: Option<(u64, PathBuf, u64)> = None;
                loop {
                    match request_rx.recv_timeout(Duration::from_millis(250)) {
                        Ok(request) => match request {
                            Request::Scan {
                                generation,
                                pane,
                                directory,
                                show_hidden,
                                directories_first,
                                sort,
                            } => {
                                let result = scan_directory(
                                    &directory,
                                    show_hidden,
                                    directories_first,
                                    sort,
                                );
                                if pane == Pane::Current {
                                    watched = Some((
                                        generation,
                                        directory.clone(),
                                        directory_signature(&directory),
                                    ));
                                }
                                let _ = event_tx.send(WorkerEvent::Scanned {
                                    generation,
                                    pane,
                                    directory,
                                    result,
                                });
                            }
                            Request::Preview {
                                generation,
                                request,
                            } => {
                                let (preview_tx, preview_rx) = mpsc::sync_channel(1);
                                let path = request.path.clone();
                                let thread_request = request.clone();
                                let spawn = thread::Builder::new()
                                    .name("caret-file-preview".to_string())
                                    .spawn(move || {
                                        let _ = preview_tx.send(preview::generate(&thread_request));
                                    });
                                let preview = match spawn {
                                    Ok(_) => match preview_rx.recv_timeout(PREVIEW_TIMEOUT) {
                                        Ok(preview) => preview,
                                        Err(_) => {
                                            request.cancel();
                                            Preview::Error(
                                                "preview timed out after 1.5 seconds".to_string(),
                                            )
                                        }
                                    },
                                    Err(error) => Preview::Error(format!(
                                        "could not start preview worker: {error}"
                                    )),
                                };
                                let _ = event_tx.send(WorkerEvent::Previewed {
                                    generation,
                                    path,
                                    preview,
                                });
                            }
                            Request::Operation { request, cancelled } => {
                                let id = request.id;
                                let kind = request.kind;
                                let summary = file_ops::execute(
                                    &request,
                                    &cancelled,
                                    |completed, total, path| {
                                        let _ = event_tx.send(WorkerEvent::Progress(
                                            OperationProgress {
                                                id,
                                                kind,
                                                completed,
                                                total,
                                                current: path.to_path_buf(),
                                            },
                                        ));
                                    },
                                );
                                let _ = event_tx.send(WorkerEvent::OperationFinished(summary));
                            }
                            Request::Undo {
                                id,
                                record,
                                cancelled,
                            } => {
                                let summary = file_ops::execute_undo(
                                    id,
                                    &record,
                                    &cancelled,
                                    |completed, total, path| {
                                        let _ = event_tx.send(WorkerEvent::Progress(
                                            OperationProgress {
                                                id,
                                                kind: OperationKind::Undo,
                                                completed,
                                                total,
                                                current: path.to_path_buf(),
                                            },
                                        ));
                                    },
                                );
                                let _ = event_tx.send(WorkerEvent::OperationFinished(summary));
                            }
                            Request::BulkRename {
                                id,
                                pairs,
                                cancelled,
                            } => {
                                let summary = file_ops::execute_bulk_rename(
                                    id,
                                    &pairs,
                                    &cancelled,
                                    |completed, total, path| {
                                        let _ = event_tx.send(WorkerEvent::Progress(
                                            OperationProgress {
                                                id,
                                                kind: OperationKind::BulkRename,
                                                completed,
                                                total,
                                                current: path.to_path_buf(),
                                            },
                                        ));
                                    },
                                );
                                let _ = event_tx.send(WorkerEvent::OperationFinished(summary));
                            }
                            Request::Stop => break,
                        },
                        Err(RecvTimeoutError::Timeout) => {
                            let Some((generation, directory, signature)) = watched.as_mut() else {
                                continue;
                            };
                            let current = directory_signature(directory);
                            if current != *signature {
                                *signature = current;
                                let _ = event_tx.send(WorkerEvent::DirectoryChanged {
                                    generation: *generation,
                                    directory: directory.clone(),
                                });
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("spawn file-manager worker");
        Self {
            tx: request_tx,
            rx: event_rx,
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Stop);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivateResult {
    None,
    OpenFile(PathBuf),
    EnteredDirectory(PathBuf),
}

pub struct FileManager {
    pub current_dir: PathBuf,
    pub parent_entries: Vec<FileEntry>,
    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub selected_paths: BTreeSet<PathBuf>,
    pub anchor: Option<usize>,
    pub filter: String,
    pub show_hidden: bool,
    pub directories_first: bool,
    pub sort: SortMode,
    pub preview_enabled: bool,
    pub preview: Preview,
    pub loading: bool,
    pub error: Option<String>,
    pub progress: Option<OperationProgress>,
    pub last_operation: Option<OperationSummary>,
    pub clipboard: Vec<PathBuf>,
    pub clipboard_cut: bool,
    history_back: Vec<PathBuf>,
    history_forward: Vec<PathBuf>,
    generation: u64,
    preview_generation: u64,
    preview_cancel: Option<Arc<AtomicBool>>,
    operation_id: u64,
    cancel_operation: Option<Arc<AtomicBool>>,
    last_operation_request: Option<OperationRequest>,
    max_preview_bytes: usize,
    max_preview_lines: usize,
    worker: Worker,
    snapshots: HashMap<PathBuf, Vec<FileEntry>>,
    pending_selection: Option<PathBuf>,
    undo_stack: Vec<UndoRecord>,
}

impl FileManager {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        let current_dir = normalize_directory(root)?;
        let mut manager = Self {
            current_dir,
            parent_entries: Vec::new(),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            selected_paths: BTreeSet::new(),
            anchor: None,
            filter: String::new(),
            show_hidden: false,
            directories_first: true,
            sort: SortMode::Name,
            preview_enabled: true,
            preview: Preview::Loading,
            loading: false,
            error: None,
            progress: None,
            last_operation: None,
            clipboard: Vec::new(),
            clipboard_cut: false,
            history_back: Vec::new(),
            history_forward: Vec::new(),
            generation: 0,
            preview_generation: 0,
            preview_cancel: None,
            operation_id: 0,
            cancel_operation: None,
            last_operation_request: None,
            max_preview_bytes: 256 * 1024,
            max_preview_lines: 200,
            worker: Worker::new(),
            snapshots: HashMap::new(),
            pending_selection: None,
            undo_stack: Vec::new(),
        };
        manager.refresh();
        Ok(manager)
    }

    pub fn configure(
        &mut self,
        show_hidden: bool,
        directories_first: bool,
        preview_enabled: bool,
        max_preview_bytes: usize,
    ) {
        let needs_refresh =
            self.show_hidden != show_hidden || self.directories_first != directories_first;
        self.show_hidden = show_hidden;
        self.directories_first = directories_first;
        self.preview_enabled = preview_enabled;
        self.max_preview_bytes = max_preview_bytes.clamp(4 * 1024, 8 * 1024 * 1024);
        if needs_refresh {
            self.refresh();
        } else {
            self.request_preview();
        }
    }

    pub fn session_state(&self, active: bool) -> crate::session::FileManagerState {
        crate::session::FileManagerState {
            active,
            current_dir: self.current_dir.clone(),
            selected_path: self.selected_path().map(Path::to_path_buf),
            history_back: self.history_back.clone(),
            history_forward: self.history_forward.clone(),
            filter: self.filter.clone(),
            sort: self.sort,
        }
    }

    pub fn restore_session_state(&mut self, state: &crate::session::FileManagerState) {
        let Ok(current_dir) = normalize_directory(state.current_dir.clone()) else {
            return;
        };
        self.current_dir = current_dir;
        self.history_back = valid_history(&state.history_back);
        self.history_forward = valid_history(&state.history_forward);
        self.filter = state
            .filter
            .chars()
            .filter(|character| !character.is_control())
            .take(256)
            .collect();
        self.sort = state.sort;
        self.selected = 0;
        self.scroll = 0;
        self.selected_paths.clear();
        self.anchor = None;
        self.pending_selection = state
            .selected_path
            .as_ref()
            .and_then(|path| fs::canonicalize(path).ok())
            .filter(|path| {
                path.parent()
                    .is_some_and(|parent| parent == self.current_dir)
            });
        self.entries.clear();
        self.parent_entries.clear();
        self.snapshots.clear();
        self.refresh();
    }

    pub fn refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.loading = true;
        self.error = None;
        let generation = self.generation;
        self.request_scan(Pane::Current, self.current_dir.clone(), generation);
        if let Some(parent) = self.current_dir.parent() {
            self.request_scan(Pane::Parent, parent.to_path_buf(), generation);
        } else {
            self.parent_entries.clear();
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.worker.rx.try_recv() {
                Ok(WorkerEvent::Scanned {
                    generation,
                    pane,
                    directory,
                    result,
                }) if generation == self.generation
                    && (pane != Pane::Current || directory == self.current_dir) =>
                {
                    match (pane, result) {
                        (Pane::Current, Ok(entries)) => {
                            let selected_path = self
                                .pending_selection
                                .take()
                                .or_else(|| self.selected_path().map(Path::to_path_buf));
                            self.cache_snapshot(self.current_dir.clone(), entries.clone());
                            self.entries = entries;
                            self.loading = false;
                            self.error = None;
                            self.restore_selection(selected_path.as_deref());
                            self.request_preview();
                        }
                        (Pane::Parent, Ok(entries)) => {
                            self.cache_snapshot(directory, entries.clone());
                            self.parent_entries = entries;
                        }
                        (Pane::Current, Err(error)) => {
                            self.loading = false;
                            self.error = Some(error.to_string());
                            self.entries.clear();
                            self.preview = Preview::Error(error.to_string());
                        }
                        (Pane::Parent, Err(_)) => self.parent_entries.clear(),
                    }
                    changed = true;
                }
                Ok(WorkerEvent::Previewed {
                    generation,
                    path,
                    preview,
                }) if generation == self.preview_generation
                    && self.selected_path() == Some(path.as_path()) =>
                {
                    self.preview = preview;
                    changed = true;
                }
                Ok(WorkerEvent::Progress(progress)) => {
                    self.progress = Some(progress);
                    changed = true;
                }
                Ok(WorkerEvent::OperationFinished(summary)) => {
                    self.progress = None;
                    self.cancel_operation = None;
                    if let Some(undo) = summary.undo.clone() {
                        if self.undo_stack.len() >= 20 {
                            self.undo_stack.remove(0);
                        }
                        self.undo_stack.push(undo);
                    }
                    self.last_operation = Some(summary);
                    self.refresh();
                    changed = true;
                }
                Ok(WorkerEvent::DirectoryChanged {
                    generation,
                    directory,
                }) if generation == self.generation && directory == self.current_dir => {
                    self.refresh();
                    changed = true;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        changed
    }

    pub fn visible_entries(&self) -> Vec<&FileEntry> {
        let filter = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| filter.is_empty() || entry.name.to_lowercase().contains(&filter))
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&FileEntry> {
        let filter = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| filter.is_empty() || entry.name.to_lowercase().contains(&filter))
            .nth(self.selected)
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_entry().map(|entry| entry.path.as_path())
    }

    pub fn move_selection(&mut self, amount: isize) {
        let count = self.visible_entries().len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(count.saturating_sub(1));
        self.request_preview();
    }

    pub fn select_index(&mut self, index: usize) -> bool {
        let count = self.visible_entries().len();
        if index >= count {
            return false;
        }
        let changed = self.selected != index;
        self.selected = index;
        self.request_preview();
        changed
    }

    pub fn begin_mouse_range(&mut self, index: usize) -> bool {
        let changed = self.select_index(index);
        if index < self.visible_entries().len() {
            self.anchor = Some(index);
        }
        changed
    }

    pub fn page(&mut self, amount: isize) {
        self.move_selection(amount);
    }

    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
        self.scroll = 0;
        self.request_preview();
    }

    pub fn clear_filter(&mut self) {
        self.set_filter(String::new());
    }

    pub fn activate(&mut self) -> ActivateResult {
        let Some(entry) = self.selected_entry().cloned() else {
            return ActivateResult::None;
        };
        if entry.is_dir {
            match self.change_directory(entry.path.clone(), true) {
                Ok(()) => ActivateResult::EnteredDirectory(entry.path),
                Err(error) => {
                    self.error = Some(error.to_string());
                    ActivateResult::None
                }
            }
        } else {
            ActivateResult::OpenFile(entry.path)
        }
    }

    pub fn parent(&mut self) -> io::Result<()> {
        let Some(parent) = self.current_dir.parent().map(Path::to_path_buf) else {
            return Ok(());
        };
        self.change_directory(parent, true)
    }

    pub fn back(&mut self) -> io::Result<()> {
        let Some(path) = self.history_back.pop() else {
            return Ok(());
        };
        self.history_forward.push(self.current_dir.clone());
        self.change_directory(path, false)
    }

    pub fn forward(&mut self) -> io::Result<()> {
        let Some(path) = self.history_forward.pop() else {
            return Ok(());
        };
        self.history_back.push(self.current_dir.clone());
        self.change_directory(path, false)
    }

    pub fn go_to(&mut self, path: PathBuf) -> io::Result<()> {
        self.change_directory(path, true)
    }

    pub fn toggle_selection(&mut self) {
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            return;
        };
        if !self.selected_paths.remove(&path) {
            self.selected_paths.insert(path);
        }
        self.anchor = Some(self.selected);
    }

    pub fn select_range(&mut self) {
        let Some(anchor) = self.anchor else {
            self.toggle_selection();
            return;
        };
        let start = anchor.min(self.selected);
        let end = anchor.max(self.selected);
        let paths = self
            .visible_entries()
            .into_iter()
            .skip(start)
            .take(end - start + 1)
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        self.selected_paths.extend(paths);
    }

    pub fn select_all(&mut self) {
        let paths = self
            .visible_entries()
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        self.selected_paths.extend(paths);
    }

    pub fn invert_selection(&mut self) {
        let visible = self
            .visible_entries()
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        for path in visible {
            if !self.selected_paths.remove(&path) {
                self.selected_paths.insert(path);
            }
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected_paths.clear();
        self.anchor = None;
    }

    pub fn selected_or_cursor_paths(&self) -> Vec<PathBuf> {
        if self.selected_paths.is_empty() {
            self.selected_path()
                .map(Path::to_path_buf)
                .into_iter()
                .collect()
        } else {
            self.selected_paths.iter().cloned().collect()
        }
    }

    pub fn copy_to_clipboard(&mut self, cut: bool) {
        self.clipboard = self.selected_or_cursor_paths();
        self.clipboard_cut = cut;
    }

    pub fn paste(&mut self, conflict: ConflictPolicy) -> bool {
        if self.clipboard.is_empty() {
            return false;
        }
        let kind = if self.clipboard_cut {
            OperationKind::Move
        } else {
            OperationKind::Copy
        };
        self.start_operation(
            kind,
            self.clipboard.clone(),
            Some(self.current_dir.clone()),
            conflict,
        )
    }

    pub fn clipboard_conflicts(&self) -> usize {
        self.clipboard
            .iter()
            .filter_map(|source| source.file_name())
            .filter(|name| self.current_dir.join(name).exists())
            .count()
    }

    pub fn duplicate(&mut self, conflict: ConflictPolicy) -> bool {
        self.start_operation(
            OperationKind::Duplicate,
            self.selected_or_cursor_paths(),
            None,
            conflict,
        )
    }

    pub fn trash(&mut self) -> bool {
        self.start_operation(
            OperationKind::Trash,
            self.selected_or_cursor_paths(),
            None,
            ConflictPolicy::Ask,
        )
    }

    pub fn delete_permanently(&mut self) -> bool {
        self.start_operation(
            OperationKind::Delete,
            self.selected_or_cursor_paths(),
            None,
            ConflictPolicy::Ask,
        )
    }

    pub fn cancel(&mut self) -> bool {
        let Some(cancelled) = self.cancel_operation.as_ref() else {
            return false;
        };
        cancelled.store(true, Ordering::Relaxed);
        true
    }

    pub fn retry_failures(&mut self) -> bool {
        let Some(previous) = self.last_operation_request.clone() else {
            return false;
        };
        let Some(summary) = self.last_operation.as_ref() else {
            return false;
        };
        let failed = summary
            .failures
            .iter()
            .map(|failure| failure.path.clone())
            .collect::<Vec<_>>();
        if failed.is_empty() {
            return false;
        }
        self.start_operation(
            previous.kind,
            failed,
            previous.destination,
            previous.conflict,
        )
    }

    pub fn undo_last_operation(&mut self) -> bool {
        if self.cancel_operation.is_some() {
            return false;
        }
        let Some(record) = self.undo_stack.pop() else {
            return false;
        };
        self.operation_id = self.operation_id.wrapping_add(1);
        let id = self.operation_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        if self
            .worker
            .tx
            .send(Request::Undo {
                id,
                record: record.clone(),
                cancelled: cancelled.clone(),
            })
            .is_err()
        {
            self.undo_stack.push(record);
            return false;
        }
        self.cancel_operation = Some(cancelled);
        self.last_operation = None;
        true
    }

    pub fn bulk_rename(&mut self, pairs: Vec<(PathBuf, PathBuf)>) -> bool {
        if pairs.is_empty() || self.cancel_operation.is_some() {
            return false;
        }
        self.operation_id = self.operation_id.wrapping_add(1);
        let id = self.operation_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        if self
            .worker
            .tx
            .send(Request::BulkRename {
                id,
                pairs,
                cancelled: cancelled.clone(),
            })
            .is_err()
        {
            return false;
        }
        self.cancel_operation = Some(cancelled);
        self.last_operation = None;
        true
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.refresh();
    }

    pub fn ensure_selected_visible(&mut self, rows: usize) {
        let rows = rows.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
    }

    fn start_operation(
        &mut self,
        kind: OperationKind,
        sources: Vec<PathBuf>,
        destination: Option<PathBuf>,
        conflict: ConflictPolicy,
    ) -> bool {
        if sources.is_empty() || self.cancel_operation.is_some() {
            return false;
        }
        self.operation_id = self.operation_id.wrapping_add(1);
        let request = OperationRequest {
            id: self.operation_id,
            kind,
            sources,
            destination,
            conflict,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        if self
            .worker
            .tx
            .send(Request::Operation {
                request: request.clone(),
                cancelled: cancelled.clone(),
            })
            .is_err()
        {
            return false;
        }
        self.cancel_operation = Some(cancelled);
        self.last_operation_request = Some(request);
        self.last_operation = None;
        true
    }

    fn change_directory(&mut self, path: PathBuf, record_history: bool) -> io::Result<()> {
        let path = normalize_directory(path)?;
        if path == self.current_dir {
            return Ok(());
        }
        if record_history {
            self.history_back.push(self.current_dir.clone());
            self.history_forward.clear();
        }
        self.current_dir = path;
        self.selected = 0;
        self.scroll = 0;
        self.selected_paths.clear();
        self.anchor = None;
        self.filter.clear();
        self.entries = self
            .snapshots
            .get(&self.current_dir)
            .cloned()
            .unwrap_or_default();
        self.preview = Preview::Loading;
        self.refresh();
        Ok(())
    }

    fn request_scan(&self, pane: Pane, directory: PathBuf, generation: u64) {
        let _ = self.worker.tx.send(Request::Scan {
            generation,
            pane,
            directory,
            show_hidden: self.show_hidden,
            directories_first: self.directories_first,
            sort: self.sort,
        });
    }

    fn request_preview(&mut self) {
        if let Some(cancelled) = self.preview_cancel.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.preview_generation = self.preview_generation.wrapping_add(1);
        if !self.preview_enabled {
            self.preview = Preview::Empty;
            return;
        }
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            self.preview = Preview::Empty;
            return;
        };
        self.preview = Preview::Loading;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.preview_cancel = Some(cancelled.clone());
        let _ = self.worker.tx.send(Request::Preview {
            generation: self.preview_generation,
            request: PreviewRequest::new(
                path,
                self.max_preview_bytes,
                self.max_preview_lines,
                PREVIEW_TIMEOUT,
                cancelled,
            ),
        });
    }

    fn restore_selection(&mut self, selected_path: Option<&Path>) {
        let filter = self.filter.trim().to_lowercase();
        if let Some(path) = selected_path {
            if let Some(index) = self
                .entries
                .iter()
                .filter(|entry| filter.is_empty() || entry.name.to_lowercase().contains(&filter))
                .position(|entry| entry.path == path)
            {
                self.selected = index;
                return;
            }
        }
        self.selected = self
            .selected
            .min(self.visible_entries().len().saturating_sub(1));
    }

    fn cache_snapshot(&mut self, directory: PathBuf, entries: Vec<FileEntry>) {
        if self.snapshots.len() >= MAX_CACHED_DIRECTORIES
            && !self.snapshots.contains_key(&directory)
        {
            if let Some(stale) = self
                .snapshots
                .keys()
                .find(|path| **path != self.current_dir && **path != directory)
                .cloned()
            {
                self.snapshots.remove(&stale);
            }
        }
        self.snapshots.insert(directory, entries);
    }
}

fn normalize_directory(path: PathBuf) -> io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let path = fs::canonicalize(path)?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a directory", path.display()),
        ))
    }
}

fn valid_history(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .rev()
        .take(64)
        .filter_map(|path| normalize_directory(path.clone()).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn scan_directory(
    directory: &Path,
    show_hidden: bool,
    directories_first: bool,
    sort: SortMode,
) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();
    for result in fs::read_dir(directory)?.take(MAX_DIRECTORY_ENTRIES) {
        let entry = result?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let hidden = is_hidden(&entry.path(), &name);
        if hidden && !show_hidden {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        entries.push(FileEntry {
            path: entry.path(),
            name,
            is_dir: metadata.is_dir(),
            is_symlink: metadata.file_type().is_symlink(),
            hidden,
            size: metadata.len(),
            modified_unix_secs: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
        });
    }
    entries.sort_by(|left, right| {
        let directory_order = if directories_first {
            right.is_dir.cmp(&left.is_dir)
        } else {
            std::cmp::Ordering::Equal
        };
        directory_order.then_with(|| match sort {
            SortMode::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            SortMode::Size => left
                .size
                .cmp(&right.size)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())),
            SortMode::Modified => right
                .modified_unix_secs
                .cmp(&left.modified_unix_secs)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())),
        })
    });
    Ok(entries)
}

fn directory_signature(directory: &Path) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut rows = match fs::read_dir(directory) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| {
                let metadata = fs::symlink_metadata(entry.path()).ok();
                (
                    entry.file_name(),
                    metadata.as_ref().map(|value| value.len()),
                    metadata
                        .and_then(|value| value.modified().ok())
                        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                        .map(|value| value.as_nanos()),
                )
            })
            .collect::<Vec<_>>(),
        Err(error) => return error.raw_os_error().unwrap_or(-1) as u64,
    };
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rows.hash(&mut hasher);
    hasher.finish()
}

fn is_hidden(path: &Path, name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        fs::metadata(path)
            .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

#[cfg(test)]
fn build_preview(path: &Path, max_bytes: usize, max_lines: usize) -> Preview {
    preview::generate(&PreviewRequest::new(
        path.to_path_buf(),
        max_bytes,
        max_lines,
        PREVIEW_TIMEOUT,
        Arc::new(AtomicBool::new(false)),
    ))
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn unix_time_label(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "unknown".to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(seconds);
    let age = now.saturating_sub(seconds);
    match age {
        0..=59 => format!("{age}s ago"),
        60..=3_599 => format!("{}m ago", age / 60),
        3_600..=86_399 => format!("{}h ago", age / 3_600),
        _ => format!("{}d ago", age / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "caret-manager-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn poll_until_loaded(manager: &mut FileManager) {
        for _ in 0..100 {
            manager.poll();
            if !manager.loading {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("manager did not finish scanning");
    }

    #[test]
    fn background_scan_sorts_directories_and_rejects_stale_results() {
        let root = temp_dir("scan");
        fs::write(root.join("z.txt"), "z").unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        fs::write(root.join(".hidden"), "hidden").unwrap();
        let mut manager = FileManager::new(root.clone()).unwrap();
        manager.refresh();
        poll_until_loaded(&mut manager);
        assert_eq!(manager.entries.len(), 2);
        assert_eq!(manager.entries[0].name, "folder");
        assert!(manager.entries[0].is_dir);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn session_restore_keeps_safe_manager_navigation_state() {
        let root = temp_dir("session-state");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let selected = nested.join("target.txt");
        fs::write(&selected, "target").unwrap();
        let missing = root.join("missing");
        let mut manager = FileManager::new(root.clone()).unwrap();
        manager.restore_session_state(&crate::session::FileManagerState {
            active: true,
            current_dir: nested.clone(),
            selected_path: Some(selected.clone()),
            history_back: vec![root.clone(), missing],
            history_forward: Vec::new(),
            filter: "target\n".to_string(),
            sort: SortMode::Size,
        });

        for _ in 0..100 {
            manager.poll();
            if !manager.loading {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(manager.current_dir, nested.canonicalize().unwrap());
        let expected_selected = selected.canonicalize().unwrap();
        assert_eq!(manager.selected_path(), Some(expected_selected.as_path()));
        assert_eq!(manager.filter, "target");
        assert_eq!(manager.sort, SortMode::Size);
        assert_eq!(manager.history_back, vec![root.canonicalize().unwrap()]);
        assert!(manager.selected_paths.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn safe_operation_undo_runs_through_the_background_manager() {
        let root = temp_dir("operation-undo");
        fs::write(root.join("note.txt"), "content").unwrap();
        let mut manager = FileManager::new(root.clone()).unwrap();
        poll_until_loaded(&mut manager);
        manager.selected = manager
            .visible_entries()
            .iter()
            .position(|entry| entry.name == "note.txt")
            .unwrap();
        assert!(manager.duplicate(ConflictPolicy::Rename));
        for _ in 0..100 {
            manager.poll();
            if manager.last_operation.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let copy = root.join("note copy.txt");
        assert!(copy.exists());

        assert!(manager.undo_last_operation());
        for _ in 0..100 {
            manager.poll();
            if manager
                .last_operation
                .as_ref()
                .is_some_and(|summary| summary.kind == OperationKind::Undo)
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!copy.exists());
        assert_eq!(
            manager.last_operation.as_ref().map(|summary| summary.kind),
            Some(OperationKind::Undo)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selection_range_and_inversion_are_path_based() {
        let root = temp_dir("selection");
        for name in ["a", "b", "c"] {
            fs::write(root.join(name), name).unwrap();
        }
        let mut manager = FileManager::new(root.clone()).unwrap();
        poll_until_loaded(&mut manager);
        manager.toggle_selection();
        manager.move_selection(2);
        manager.select_range();
        assert_eq!(manager.selected_paths.len(), 3);
        manager.invert_selection();
        assert!(manager.selected_paths.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_distinguishes_text_binary_structured_and_directories() {
        let root = temp_dir("preview");
        let json = root.join("data.json");
        fs::write(&json, "{\"ok\":true}\n").unwrap();
        let preview = build_preview(&json, 1024, 20);
        assert!(matches!(
            preview,
            Preview::Text {
                structured: Some("JSON"),
                ..
            }
        ));

        let binary = root.join("image.png");
        let mut png = b"\x89PNG\r\n\x1A\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        png.push(0);
        fs::write(&binary, png).unwrap();
        assert!(matches!(
            build_preview(&binary, 1024, 20),
            Preview::Binary {
                kind: "PNG image",
                dimensions: Some((640, 480)),
                ..
            }
        ));
        assert!(matches!(
            build_preview(&root, 1024, 20),
            Preview::Directory { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filtering_and_navigation_preserve_safe_bounds() {
        let root = temp_dir("filter");
        fs::write(root.join("alpha.txt"), "a").unwrap();
        fs::write(root.join("beta.txt"), "b").unwrap();
        let mut manager = FileManager::new(root.clone()).unwrap();
        poll_until_loaded(&mut manager);
        manager.set_filter("beta".to_string());
        assert_eq!(manager.visible_entries().len(), 1);
        manager.move_selection(999);
        assert_eq!(manager.selected, 0);
        manager.clear_filter();
        assert_eq!(manager.visible_entries().len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn watcher_invalidates_only_the_active_directory_snapshot() {
        let root = temp_dir("watch");
        fs::write(root.join("before.txt"), "before").unwrap();
        let mut manager = FileManager::new(root.clone()).unwrap();
        poll_until_loaded(&mut manager);
        fs::write(root.join("after.txt"), "after").unwrap();

        for _ in 0..200 {
            manager.poll();
            if manager
                .entries
                .iter()
                .any(|entry| entry.name == "after.txt")
            {
                let _ = fs::remove_dir_all(root);
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(root);
        panic!("watcher did not invalidate the active directory");
    }
}
