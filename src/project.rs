use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

use ignore::gitignore::Gitignore;

use crate::config::ExplorerSort;

const MAX_EXPAND_ALL_DIRECTORIES: usize = 5_000;
const MAX_FILTER_RESULTS: usize = 500;
const MAX_TREE_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub struct ProjectEntry {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    pub ignored: bool,
    pub is_executable: Option<bool>,
    pub expanded: bool,
    pub git_status: Option<GitStatus>,
    pub size: Option<u64>,
    pub modified_unix_secs: Option<u64>,
    /// Whether each ancestor has a later sibling and therefore needs a guide.
    pub guides: Vec<bool>,
    pub is_last: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Conflicted,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeLoadState {
    Loading,
    Ready,
    Empty,
    PermissionDenied(String),
    Missing(String),
    Error(String),
}

impl TreeLoadState {
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::PermissionDenied(message) | Self::Missing(message) | Self::Error(message) => {
                Some(message)
            }
            Self::Loading | Self::Ready | Self::Empty => None,
        }
    }
}

#[derive(Debug)]
pub struct ProjectTree {
    pub root: PathBuf,
    pub entries: Vec<ProjectEntry>,
    pub selected: usize,
    pub scroll: usize,
    pub visible: bool,
    pub show_hidden: bool,
    /// Substring filter over project-relative paths; empty shows the tree.
    pub filter: String,
    pub width: usize,
    pub git_refreshing: bool,
    pub git_error: Option<String>,
    pub tree_loading: bool,
    pub tree_error: Option<String>,
    pub tree_state: TreeLoadState,
    pub sort: ExplorerSort,
    pub directories_first: bool,
    pub show_metadata: bool,
    expanded: HashSet<PathBuf>,
    git_refresh_requested: bool,
    tree_refresh_requested: bool,
    tree_invalidate_all: bool,
    tree_invalidated_directories: HashSet<PathBuf>,
    expand_all_requested: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TreeScanRequest {
    pub root: PathBuf,
    pub expanded: HashSet<PathBuf>,
    pub show_hidden: bool,
    pub filter: String,
    pub invalidate_all: bool,
    pub invalidated_directories: HashSet<PathBuf>,
    pub sort: ExplorerSort,
    pub directories_first: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryMetadata {
    pub size: u64,
    pub modified_unix_secs: u64,
    pub is_executable: bool,
}

impl ProjectTree {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        let root = normalize_root(root)?;
        let mut expanded = HashSet::new();
        expanded.insert(root.clone());

        let tree = Self {
            root,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            visible: true,
            show_hidden: false,
            filter: String::new(),
            width: 40,
            git_refreshing: false,
            git_error: None,
            tree_loading: true,
            tree_error: None,
            tree_state: TreeLoadState::Loading,
            sort: ExplorerSort::Name,
            directories_first: true,
            show_metadata: true,
            expanded,
            git_refresh_requested: true,
            tree_refresh_requested: true,
            tree_invalidate_all: true,
            tree_invalidated_directories: HashSet::new(),
            expand_all_requested: false,
        };
        Ok(tree)
    }

    pub fn root_name(&self) -> String {
        self.root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.root.display().to_string())
    }

    pub fn selected_entry(&self) -> Option<&ProjectEntry> {
        self.entries.get(self.selected)
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        self.tree_invalidate_all = true;
        self.tree_invalidated_directories.clear();
        self.queue_tree_projection();
        Ok(())
    }

    pub fn refresh_paths(&mut self, paths: &[PathBuf]) -> io::Result<()> {
        if paths.is_empty() {
            return self.refresh();
        }
        for path in paths {
            if !path.starts_with(&self.root) {
                return self.refresh();
            }
            let directory = if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(&self.root)
            };
            self.tree_invalidated_directories
                .insert(directory.to_path_buf());
        }
        self.queue_tree_projection();
        Ok(())
    }

    fn queue_tree_projection(&mut self) {
        self.tree_refresh_requested = true;
        self.tree_loading = true;
        self.tree_error = None;
        self.tree_state = TreeLoadState::Loading;
    }

    pub(crate) fn take_tree_refresh_request(&mut self) -> Option<TreeScanRequest> {
        if !std::mem::take(&mut self.tree_refresh_requested) {
            return None;
        }
        Some(TreeScanRequest {
            root: self.root.clone(),
            expanded: self.expanded.clone(),
            show_hidden: self.show_hidden,
            filter: self.filter.clone(),
            invalidate_all: std::mem::take(&mut self.tree_invalidate_all),
            invalidated_directories: std::mem::take(&mut self.tree_invalidated_directories),
            sort: self.sort,
            directories_first: self.directories_first,
        })
    }

    pub(crate) fn apply_tree_snapshot(&mut self, entries: Vec<ProjectEntry>) -> bool {
        let selected_path = self.selected_entry().map(|entry| entry.path.clone());
        let changed =
            self.entries.len() != entries.len()
                || self.entries.iter().zip(&entries).any(|(left, right)| {
                    left.path != right.path || left.expanded != right.expanded
                });
        self.entries = entries;
        if let Some(path) = selected_path {
            self.selected = self
                .entries
                .iter()
                .position(|entry| entry.path == path)
                .unwrap_or_else(|| self.selected.min(self.entries.len().saturating_sub(1)));
        } else {
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        }
        self.clamp_scroll(1);
        self.tree_loading = false;
        self.tree_error = None;
        self.tree_state = if self.entries.is_empty() {
            TreeLoadState::Empty
        } else {
            TreeLoadState::Ready
        };

        if self.expand_all_requested {
            let before = self.expanded.len();
            self.expanded.extend(
                self.entries
                    .iter()
                    .filter(|entry| entry.is_dir && !entry.is_symlink)
                    .map(|entry| entry.path.clone()),
            );
            if self.expanded.len() > before && self.expanded.len() < MAX_EXPAND_ALL_DIRECTORIES {
                self.tree_refresh_requested = true;
                self.tree_loading = true;
                self.tree_state = TreeLoadState::Loading;
            } else {
                self.expand_all_requested = false;
            }
        }
        changed
    }

    pub(crate) fn apply_tree_metadata(
        &mut self,
        metadata: &std::collections::HashMap<PathBuf, EntryMetadata>,
    ) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            let next = metadata.get(&entry.path).copied();
            let size = next.map(|value| value.size);
            let modified = next.map(|value| value.modified_unix_secs);
            let executable = next.map(|value| value.is_executable);
            if entry.size != size
                || entry.modified_unix_secs != modified
                || entry.is_executable != executable
            {
                entry.size = size;
                entry.modified_unix_secs = modified;
                entry.is_executable = executable;
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn fail_tree_refresh(&mut self, kind: io::ErrorKind, message: String) -> bool {
        let changed = self.tree_loading || self.tree_error.as_deref() != Some(&message);
        self.tree_loading = false;
        self.tree_error = Some(message.clone());
        self.tree_state = match kind {
            io::ErrorKind::PermissionDenied => TreeLoadState::PermissionDenied(message),
            io::ErrorKind::NotFound => TreeLoadState::Missing(message),
            _ => TreeLoadState::Error(message),
        };
        changed
    }

    #[cfg(test)]
    pub(crate) fn complete_pending_refresh_for_test(&mut self) -> io::Result<()> {
        let Some(request) = self.take_tree_refresh_request() else {
            return Ok(());
        };
        let entries = TreeScanner::default().scan(&request)?;
        self.apply_tree_snapshot(entries);
        Ok(())
    }

    pub fn request_git_refresh(&mut self) {
        self.git_refresh_requested = true;
    }

    pub fn set_sort(&mut self, sort: ExplorerSort, directories_first: bool) {
        if self.sort != sort || self.directories_first != directories_first {
            self.sort = sort;
            self.directories_first = directories_first;
            self.queue_tree_projection();
        }
    }

    pub(crate) fn resort_after_metadata(&mut self) {
        if self.sort != ExplorerSort::Name {
            self.queue_tree_projection();
        }
    }

    pub(crate) fn take_git_refresh_request(&mut self) -> bool {
        std::mem::take(&mut self.git_refresh_requested)
    }

    pub fn set_root(&mut self, root: PathBuf) -> io::Result<()> {
        let root = normalize_root(root)?;
        self.root = root.clone();
        self.expanded.clear();
        self.expanded.insert(root);
        self.selected = 0;
        self.scroll = 0;
        self.filter.clear();
        self.git_refreshing = false;
        self.git_error = None;
        self.tree_error = None;
        self.tree_state = TreeLoadState::Loading;
        self.git_refresh_requested = true;
        self.tree_refresh_requested = true;
        self.tree_invalidate_all = true;
        self.tree_loading = true;
        self.expand_all_requested = false;
        Ok(())
    }

    /// Filters the tree to files whose project-relative path contains
    /// `filter` (case-insensitive).  An empty filter restores the tree.
    pub fn set_filter(&mut self, filter: String) -> io::Result<()> {
        self.filter = filter;
        self.selected = 0;
        self.scroll = 0;
        self.queue_tree_projection();
        Ok(())
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }

    pub fn page_up(&mut self, amount: usize) {
        self.selected = self.selected.saturating_sub(amount.max(1));
    }

    pub fn page_down(&mut self, amount: usize) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + amount.max(1)).min(self.entries.len() - 1);
        }
    }

    pub fn jump_to_root(&mut self) {
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn activate_selected(&mut self) -> io::Result<Option<PathBuf>> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(None);
        };

        if entry.is_dir {
            if self.expanded.contains(&entry.path) {
                self.expanded.remove(&entry.path);
            } else {
                self.expanded.insert(entry.path);
            }
            self.queue_tree_projection();
            Ok(None)
        } else {
            Ok(Some(entry.path))
        }
    }

    pub fn expand_selected(&mut self) -> io::Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };

        if entry.is_dir && !self.expanded.contains(&entry.path) {
            self.expanded.insert(entry.path);
            self.queue_tree_projection();
        }

        Ok(())
    }

    pub fn expand_selected_one_level(&mut self) -> io::Result<usize> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(0);
        };

        if !entry.is_dir {
            return Ok(0);
        }

        let mut added = usize::from(self.expanded.insert(entry.path.clone()));
        let child_depth = entry.depth.saturating_add(1);
        for child in self
            .entries
            .iter()
            .skip(self.selected.saturating_add(1))
            .take_while(|child| child.depth > entry.depth)
            .filter(|child| child.depth == child_depth && child.is_dir && !child.is_symlink)
        {
            added += usize::from(self.expanded.insert(child.path.clone()));
        }

        self.queue_tree_projection();
        Ok(added)
    }

    pub fn collapse_selected_recursive(&mut self) -> io::Result<usize> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(0);
        };

        if !entry.is_dir {
            return Ok(0);
        }

        let before = self.expanded.len();
        self.expanded
            .retain(|path| !path.starts_with(&entry.path) || path == &self.root);
        let removed = before.saturating_sub(self.expanded.len());
        self.queue_tree_projection();
        Ok(removed)
    }

    pub fn collapse_or_parent(&mut self) -> io::Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };

        if entry.is_dir && self.expanded.remove(&entry.path) {
            self.queue_tree_projection();
            return Ok(());
        }

        if entry.depth == 0 {
            return Ok(());
        }

        for index in (0..self.selected).rev() {
            if self.entries[index].depth < entry.depth {
                self.selected = index;
                break;
            }
        }

        Ok(())
    }

    pub fn expand_all(&mut self) -> io::Result<usize> {
        let before = self.expanded.len();
        self.expanded.insert(self.root.clone());
        self.expanded.extend(
            self.entries
                .iter()
                .filter(|entry| entry.is_dir && !entry.is_symlink)
                .map(|entry| entry.path.clone()),
        );
        self.expand_all_requested = true;
        let added = self.expanded.len().saturating_sub(before);
        self.queue_tree_projection();
        Ok(added)
    }

    pub fn collapse_all(&mut self) -> io::Result<usize> {
        let removed = self.expanded.len().saturating_sub(1);
        self.expanded.clear();
        self.expanded.insert(self.root.clone());
        self.expand_all_requested = false;
        self.selected = 0;
        self.scroll = 0;
        self.queue_tree_projection();
        Ok(removed)
    }

    pub fn reveal_path(&mut self, path: &Path) -> io::Result<bool> {
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !path.starts_with(&self.root) {
            return Ok(false);
        }

        let mut current = path.parent();
        while let Some(directory) = current {
            if !directory.starts_with(&self.root) {
                break;
            }
            self.expanded.insert(directory.to_path_buf());
            if directory == self.root {
                break;
            }
            current = directory.parent();
        }

        self.queue_tree_projection();
        if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
            self.selected = index;
            return Ok(true);
        }

        Ok(false)
    }

    pub fn toggle_hidden(&mut self) -> io::Result<()> {
        self.show_hidden = !self.show_hidden;
        self.queue_tree_projection();
        Ok(())
    }

    pub fn ensure_selected_visible(&mut self, rows: usize) {
        self.clamp_scroll(rows);

        if self.entries.is_empty() || rows == 0 {
            self.scroll = 0;
            return;
        }

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
    }

    fn clamp_scroll(&mut self, rows: usize) {
        let maximum = self.entries.len().saturating_sub(rows.max(1));
        self.scroll = self.scroll.min(maximum);
    }
}

#[derive(Debug, Default)]
pub(crate) struct TreeScanner {
    root: Option<PathBuf>,
    directories: HashMap<PathBuf, Vec<ChildEntry>>,
    metadata: HashMap<PathBuf, EntryMetadata>,
}

impl TreeScanner {
    pub(crate) fn scan(&mut self, request: &TreeScanRequest) -> io::Result<Vec<ProjectEntry>> {
        if self.root.as_ref() != Some(&request.root) || request.invalidate_all {
            self.root = Some(request.root.clone());
            self.directories.clear();
            self.metadata.clear();
        } else {
            for directory in &request.invalidated_directories {
                self.directories.remove(directory);
                self.metadata.retain(|path, _| !path.starts_with(directory));
            }
        }

        let ignore_rules = load_ignore_rules(&request.root);
        let view = TreeView {
            show_hidden: request.show_hidden,
            ignore_rules: ignore_rules.as_ref(),
        };
        let context = ProjectionContext {
            expanded: &request.expanded,
            view,
            sort: request.sort,
            directories_first: request.directories_first,
            metadata: &self.metadata,
        };
        let mut entries = Vec::new();
        if request.filter.trim().is_empty() {
            let mut guides = Vec::new();
            collect_entries(
                &request.root,
                0,
                &context,
                &mut self.directories,
                &mut guides,
                &mut entries,
            )?;
        } else {
            collect_filtered_entries(
                &request.root,
                &request.root,
                &request.filter.to_lowercase(),
                0,
                &context,
                &mut self.directories,
                &mut entries,
            );
            let length = entries.len();
            for (index, entry) in entries.iter_mut().enumerate() {
                entry.is_last = index + 1 == length;
            }
        }
        Ok(entries)
    }

    pub(crate) fn apply_metadata(&mut self, metadata: &HashMap<PathBuf, EntryMetadata>) {
        self.metadata.extend(
            metadata
                .iter()
                .map(|(path, metadata)| (path.clone(), *metadata)),
        );
    }
}

pub(crate) fn load_entry_metadata(paths: &[PathBuf]) -> HashMap<PathBuf, EntryMetadata> {
    use std::time::UNIX_EPOCH;
    paths
        .iter()
        .filter_map(|path| {
            let metadata = fs::symlink_metadata(path).ok()?;
            let modified_unix_secs = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |value| value.as_secs());
            Some((
                path.clone(),
                EntryMetadata {
                    size: metadata.len(),
                    modified_unix_secs,
                    is_executable: is_executable(path, &metadata),
                },
            ))
        })
        .collect()
}

#[cfg(unix)]
fn is_executable(_path: &Path, metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(windows)]
fn is_executable(path: &Path, metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "exe" | "com" | "bat" | "cmd" | "ps1"
                )
            })
}

#[cfg(not(any(unix, windows)))]
fn is_executable(_path: &Path, _metadata: &fs::Metadata) -> bool {
    false
}

#[derive(Debug, Clone)]
struct ChildEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    is_symlink: bool,
}

/// Visibility rules shared by every tree walk.
#[derive(Clone, Copy)]
struct TreeView<'rules> {
    show_hidden: bool,
    ignore_rules: Option<&'rules Gitignore>,
}

struct ProjectionContext<'a, 'rules> {
    expanded: &'a HashSet<PathBuf>,
    view: TreeView<'rules>,
    sort: ExplorerSort,
    directories_first: bool,
    metadata: &'a HashMap<PathBuf, EntryMetadata>,
}

impl TreeView<'_> {
    fn shows(&self, path: &Path, name: &str, is_dir: bool) -> bool {
        if self.show_hidden {
            return true;
        }
        if self.is_hidden(name) {
            return false;
        }
        !self.is_ignored(path, is_dir)
    }

    fn is_hidden(&self, name: &str) -> bool {
        name.starts_with('.')
    }

    fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        self.ignore_rules
            .is_some_and(|rules| rules.matched(path, is_dir).is_ignore())
    }
}

fn load_ignore_rules(root: &Path) -> Option<Gitignore> {
    let file = root.join(".gitignore");
    if !file.is_file() {
        return None;
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    builder.add(&file);
    builder.build().ok()
}

fn normalize_root(root: PathBuf) -> io::Result<PathBuf> {
    let absolute = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()?.join(root)
    };

    if !absolute.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("folder does not exist: {}", absolute.display()),
        ));
    }

    if !absolute.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not a folder: {}", absolute.display()),
        ));
    }

    Ok(fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn read_children(directory: &Path) -> io::Result<Vec<ChildEntry>> {
    let mut children = Vec::new();

    for child in fs::read_dir(directory)? {
        let Ok(child) = child else {
            continue;
        };

        let name = child.file_name().to_string_lossy().to_string();
        let Ok(file_type) = child.file_type() else {
            continue;
        };
        let is_symlink = file_type.is_symlink();
        // A symlink's own file type never says "directory"; follow it once so
        // linked folders still expand like folders.
        let is_dir = file_type.is_dir()
            || (is_symlink
                && fs::metadata(child.path())
                    .map(|metadata| metadata.is_dir())
                    .unwrap_or(false));

        children.push(ChildEntry {
            path: child.path(),
            name,
            is_dir,
            is_symlink,
        });
    }

    Ok(children)
}

fn cached_visible_children(
    directory: &Path,
    view: TreeView,
    sort: ExplorerSort,
    directories_first: bool,
    metadata: &HashMap<PathBuf, EntryMetadata>,
    cache: &mut HashMap<PathBuf, Vec<ChildEntry>>,
) -> io::Result<Vec<ChildEntry>> {
    if !cache.contains_key(directory) {
        cache.insert(directory.to_path_buf(), read_children(directory)?);
    }
    let mut children = cache
        .get(directory)
        .into_iter()
        .flatten()
        .filter(|child| view.shows(&child.path, &child.name, child.is_dir))
        .cloned()
        .collect::<Vec<_>>();
    children.sort_by(|left, right| {
        let directory_order = if directories_first {
            right.is_dir.cmp(&left.is_dir)
        } else {
            std::cmp::Ordering::Equal
        };
        directory_order
            .then_with(|| match sort {
                ExplorerSort::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
                ExplorerSort::Size => metadata
                    .get(&left.path)
                    .map_or(0, |value| value.size)
                    .cmp(&metadata.get(&right.path).map_or(0, |value| value.size)),
                ExplorerSort::Modified => metadata
                    .get(&right.path)
                    .map_or(0, |value| value.modified_unix_secs)
                    .cmp(
                        &metadata
                            .get(&left.path)
                            .map_or(0, |value| value.modified_unix_secs),
                    ),
            })
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(children)
}

fn collect_entries(
    directory: &Path,
    depth: usize,
    context: &ProjectionContext<'_, '_>,
    cache: &mut HashMap<PathBuf, Vec<ChildEntry>>,
    guides: &mut Vec<bool>,
    output: &mut Vec<ProjectEntry>,
) -> io::Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Ok(());
    }
    let children = match cached_visible_children(
        directory,
        context.view,
        context.sort,
        context.directories_first,
        context.metadata,
        cache,
    ) {
        Ok(children) => children,
        Err(error) if depth > 0 => {
            let _ = error;
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    let child_count = children.len();
    for (index, child) in children.into_iter().enumerate() {
        let is_last = index + 1 == child_count;
        let is_expanded = child.is_dir && context.expanded.contains(&child.path);
        let hidden = context.view.is_hidden(&child.name);
        let ignored = context.view.is_ignored(&child.path, child.is_dir);
        let child_metadata = context.metadata.get(&child.path);
        output.push(ProjectEntry {
            path: child.path.clone(),
            name: child.name,
            depth,
            is_dir: child.is_dir,
            is_symlink: child.is_symlink,
            hidden,
            ignored,
            is_executable: child_metadata.map(|value| value.is_executable),
            expanded: is_expanded,
            git_status: None,
            size: child_metadata.map(|value| value.size),
            modified_unix_secs: child_metadata.map(|value| value.modified_unix_secs),
            guides: guides.clone(),
            is_last,
        });

        if is_expanded {
            guides.push(!is_last);
            collect_entries(&child.path, depth + 1, context, cache, guides, output)?;
            guides.pop();
        }
    }

    Ok(())
}

/// Flat list of files whose project-relative path contains `needle`.
/// Symlinked folders are not followed, so link cycles cannot loop.
fn collect_filtered_entries(
    root: &Path,
    directory: &Path,
    needle: &str,
    depth: usize,
    context: &ProjectionContext<'_, '_>,
    cache: &mut HashMap<PathBuf, Vec<ChildEntry>>,
    output: &mut Vec<ProjectEntry>,
) {
    if depth > MAX_TREE_DEPTH || output.len() >= MAX_FILTER_RESULTS {
        return;
    }
    let Ok(children) = cached_visible_children(
        directory,
        context.view,
        context.sort,
        context.directories_first,
        context.metadata,
        cache,
    ) else {
        return;
    };

    for child in children {
        if output.len() >= MAX_FILTER_RESULTS {
            return;
        }
        if child.is_dir {
            if !child.is_symlink {
                collect_filtered_entries(
                    root,
                    &child.path,
                    needle,
                    depth + 1,
                    context,
                    cache,
                    output,
                );
            }
            continue;
        }
        let relative = child
            .path
            .strip_prefix(root)
            .unwrap_or(&child.path)
            .display()
            .to_string();
        if relative.to_lowercase().contains(needle) {
            let hidden = context.view.is_hidden(&child.name);
            let ignored = context.view.is_ignored(&child.path, false);
            let child_metadata = context.metadata.get(&child.path);
            output.push(ProjectEntry {
                path: child.path.clone(),
                name: relative,
                depth: 0,
                is_dir: false,
                is_symlink: child.is_symlink,
                hidden,
                ignored,
                is_executable: child_metadata.map(|value| value.is_executable),
                expanded: false,
                git_status: None,
                size: child_metadata.map(|value| value.size),
                modified_unix_secs: child_metadata.map(|value| value.modified_unix_secs),
                guides: Vec::new(),
                is_last: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("caret-project-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        root
    }

    #[test]
    fn gitignored_files_are_hidden_until_toggled() {
        let root = temp_root("ignore");
        fs::write(root.join("src/app.rs"), "code").unwrap();
        fs::write(root.join("build.log"), "log").unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();

        let mut tree = ProjectTree::new(root.clone()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        assert!(!tree.entries.iter().any(|entry| entry.name == "build.log"));

        tree.toggle_hidden().unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        assert!(tree.entries.iter().any(|entry| entry.name == "build.log"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filter_lists_matching_files_with_relative_paths() {
        let root = temp_root("filter");
        fs::write(root.join("src/main.rs"), "code").unwrap();
        fs::write(root.join("src/helper.rs"), "code").unwrap();
        fs::write(root.join("readme.md"), "text").unwrap();

        let mut tree = ProjectTree::new(root.clone()).unwrap();
        tree.set_filter("main".to_string()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        assert_eq!(tree.entries.len(), 1);
        assert!(tree.entries[0].name.contains("main.rs"));
        assert!(tree.entries[0].is_last);

        tree.set_filter(String::new()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        assert!(tree.entries.len() > 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_refresh_requests_are_edge_triggered() {
        let root = temp_root("git-request");
        let mut tree = ProjectTree::new(root.clone()).unwrap();

        assert!(tree.take_git_refresh_request());
        assert!(!tree.take_git_refresh_request());
        tree.request_git_refresh();
        assert!(tree.take_git_refresh_request());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cached_directory_snapshots_change_only_after_invalidation() {
        let root = temp_root("cached-snapshot");
        fs::write(root.join("first.txt"), "first").unwrap();
        let mut scanner = TreeScanner::default();
        let mut request = TreeScanRequest {
            root: root.clone(),
            expanded: HashSet::from([root.clone()]),
            show_hidden: false,
            filter: String::new(),
            invalidate_all: true,
            invalidated_directories: HashSet::new(),
            sort: ExplorerSort::Name,
            directories_first: true,
        };

        let initial = scanner.scan(&request).unwrap();
        assert!(initial.iter().any(|entry| entry.name == "first.txt"));
        fs::write(root.join("second.txt"), "second").unwrap();

        request.invalidate_all = false;
        let cached = scanner.scan(&request).unwrap();
        assert!(!cached.iter().any(|entry| entry.name == "second.txt"));

        request.invalidate_all = true;
        let refreshed = scanner.scan(&request).unwrap();
        assert!(refreshed.iter().any(|entry| entry.name == "second.txt"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn targeted_invalidation_rescans_only_the_changed_directory() {
        let root = temp_root("targeted-snapshot");
        let first_dir = root.join("src");
        let second_dir = root.join("other");
        fs::create_dir_all(&second_dir).unwrap();
        fs::write(first_dir.join("first.rs"), "first").unwrap();
        fs::write(second_dir.join("stable.txt"), "stable").unwrap();
        let mut scanner = TreeScanner::default();
        let mut request = TreeScanRequest {
            root: root.clone(),
            expanded: HashSet::from([root.clone(), first_dir.clone(), second_dir.clone()]),
            show_hidden: false,
            filter: String::new(),
            invalidate_all: true,
            invalidated_directories: HashSet::new(),
            sort: ExplorerSort::Name,
            directories_first: true,
        };
        scanner.scan(&request).unwrap();

        fs::write(first_dir.join("new.rs"), "new").unwrap();
        fs::write(second_dir.join("should-stay-cached.txt"), "cached").unwrap();
        request.invalidate_all = false;
        request.invalidated_directories = HashSet::from([first_dir]);
        let refreshed = scanner.scan(&request).unwrap();

        assert!(refreshed.iter().any(|entry| entry.name == "new.rs"));
        assert!(!refreshed
            .iter()
            .any(|entry| entry.name == "should-stay-cached.txt"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cached_metadata_drives_size_and_modified_sorting() {
        let root = temp_root("metadata-sort");
        let small = root.join("small.txt");
        let large = root.join("large.txt");
        fs::write(&small, "x").unwrap();
        fs::write(&large, "0123456789").unwrap();
        let mut scanner = TreeScanner::default();
        let mut request = TreeScanRequest {
            root: root.clone(),
            expanded: HashSet::from([root.clone()]),
            show_hidden: false,
            filter: String::new(),
            invalidate_all: true,
            invalidated_directories: HashSet::new(),
            sort: ExplorerSort::Name,
            directories_first: false,
        };
        scanner.scan(&request).unwrap();
        scanner.apply_metadata(&load_entry_metadata(&[small.clone(), large.clone()]));

        request.invalidate_all = false;
        request.sort = ExplorerSort::Size;
        let by_size = scanner.scan(&request).unwrap();
        let small_index = by_size
            .iter()
            .position(|entry| entry.path == small)
            .unwrap();
        let large_index = by_size
            .iter()
            .position(|entry| entry.path == large)
            .unwrap();
        assert!(small_index < large_index);
        assert_eq!(by_size[large_index].size, Some(10));

        request.sort = ExplorerSort::Modified;
        let by_modified = scanner.scan(&request).unwrap();
        assert!(by_modified
            .iter()
            .filter(|entry| entry.path == small || entry.path == large)
            .all(|entry| entry.modified_unix_secs.is_some()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn connector_metadata_tracks_last_children() {
        let root = temp_root("guides");
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("src/a.rs"), "a").unwrap();
        fs::write(root.join("src/z.rs"), "z").unwrap();

        let mut tree = ProjectTree::new(root.clone()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        let src_index = tree
            .entries
            .iter()
            .position(|entry| entry.name == "src")
            .unwrap();
        tree.selected = src_index;
        tree.expand_selected().unwrap();
        tree.complete_pending_refresh_for_test().unwrap();

        let children = tree
            .entries
            .iter()
            .filter(|entry| entry.depth == 1)
            .collect::<Vec<_>>();
        assert!(!children.is_empty());
        assert!(children.last().unwrap().is_last);
        assert!(children
            .iter()
            .all(|entry| entry.guides.len() == 1 && !entry.guides[0]));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_are_marked_and_not_auto_expanded() {
        let root = temp_root("symlink");
        fs::write(root.join("src/real.rs"), "code").unwrap();
        // A link cycle back to the root must not hang expand_all.
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();

        let mut tree = ProjectTree::new(root.clone()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        assert!(tree
            .entries
            .iter()
            .any(|entry| entry.name == "loop" && entry.is_dir && entry.is_symlink));
        tree.expand_all().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlinks_are_visible_without_being_followed() {
        let root = temp_root("broken-symlink");
        let link = root.join("missing-link");
        std::os::unix::fs::symlink(root.join("does-not-exist"), &link).unwrap();

        let mut tree = ProjectTree::new(root.clone()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        let entry = tree
            .entries
            .iter()
            .find(|entry| entry.name == "missing-link")
            .expect("broken symlink should remain visible");
        assert!(entry.is_symlink);
        assert!(!entry.is_dir);
        tree.expand_all().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_child_does_not_break_the_project_tree() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("permission-denied");
        let denied = root.join("denied");
        fs::create_dir_all(&denied).unwrap();
        fs::write(denied.join("private.txt"), "private").unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();

        let mut tree = ProjectTree::new(root.clone()).unwrap();
        tree.complete_pending_refresh_for_test().unwrap();
        let index = tree
            .entries
            .iter()
            .position(|entry| entry.name == "denied")
            .expect("denied directory should still be listed");
        tree.selected = index;
        assert!(tree.expand_selected().is_ok());

        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn disappearing_project_root_returns_a_recoverable_error() {
        let root = temp_root("disappearing-root");
        let mut tree = ProjectTree::new(root.clone()).unwrap();
        let normalized_root = tree.root.clone();
        fs::remove_dir_all(&root).unwrap();

        tree.refresh().unwrap();
        let error = tree.complete_pending_refresh_for_test().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(tree.root, normalized_root);
    }
}
