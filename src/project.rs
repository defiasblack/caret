use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use ignore::gitignore::Gitignore;

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
    pub expanded: bool,
    pub git_status: Option<GitStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
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
    expanded: HashSet<PathBuf>,
    ignore_rules: Option<Gitignore>,
}

impl ProjectTree {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        let root = normalize_root(root)?;
        let mut expanded = HashSet::new();
        expanded.insert(root.clone());

        let mut tree = Self {
            ignore_rules: load_ignore_rules(&root),
            root,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            visible: true,
            show_hidden: false,
            filter: String::new(),
            width: 40,
            expanded,
        };
        tree.refresh()?;
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
        self.ignore_rules = load_ignore_rules(&self.root);
        let selected_path = self.selected_entry().map(|entry| entry.path.clone());
        let mut entries = Vec::new();

        let view = TreeView {
            show_hidden: self.show_hidden,
            ignore_rules: self.ignore_rules.as_ref(),
        };
        if self.filter.trim().is_empty() {
            collect_entries(&self.root, 0, &self.expanded, view, &mut entries)?;
        } else {
            collect_filtered_entries(
                &self.root,
                &self.root,
                &self.filter.to_lowercase(),
                view,
                0,
                &mut entries,
            );
        }

        self.entries = entries;

        if let Some(path) = selected_path {
            if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
                self.selected = index;
            } else {
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            }
        } else {
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        }

        self.clamp_scroll(1);
        Ok(())
    }

    pub fn refresh_git_status(&mut self) {
        let Ok(output) = Command::new("git")
            .args(["-C"])
            .arg(&self.root)
            .args(["status", "--porcelain"])
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        for entry in &mut self.entries {
            entry.git_status = None;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.len() < 4 {
                continue;
            }
            let code = &line[..2];
            let path = self.root.join(&line[3..]);
            let status = if code == "??" {
                Some(GitStatus::Untracked)
            } else if code.contains('D') {
                Some(GitStatus::Deleted)
            } else if code.contains('A') {
                Some(GitStatus::Added)
            } else if code.contains('M') {
                Some(GitStatus::Modified)
            } else {
                None
            };
            if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) {
                entry.git_status = status;
            }
        }
    }

    pub fn set_root(&mut self, root: PathBuf) -> io::Result<()> {
        let root = normalize_root(root)?;
        self.root = root.clone();
        self.expanded.clear();
        self.expanded.insert(root);
        self.selected = 0;
        self.scroll = 0;
        self.filter.clear();
        self.refresh()
    }

    /// Filters the tree to files whose project-relative path contains
    /// `filter` (case-insensitive).  An empty filter restores the tree.
    pub fn set_filter(&mut self, filter: String) -> io::Result<()> {
        self.filter = filter;
        self.selected = 0;
        self.scroll = 0;
        self.refresh()
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
            self.refresh()?;
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
            self.refresh()?;
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

        let view = TreeView {
            show_hidden: self.show_hidden,
            ignore_rules: self.ignore_rules.as_ref(),
        };
        for child in readable_children(&entry.path, view)? {
            if child.is_dir && self.expanded.insert(child.path) {
                added += 1;
            }
        }

        self.refresh()?;
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
        self.refresh()?;
        Ok(removed)
    }

    pub fn collapse_or_parent(&mut self) -> io::Result<()> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(());
        };

        if entry.is_dir && self.expanded.remove(&entry.path) {
            self.refresh()?;
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
        let mut directories = Vec::new();
        let view = TreeView {
            show_hidden: self.show_hidden,
            ignore_rules: self.ignore_rules.as_ref(),
        };
        collect_directory_paths(
            &self.root,
            view,
            &mut directories,
            MAX_EXPAND_ALL_DIRECTORIES,
        )?;

        let before = self.expanded.len();
        self.expanded.insert(self.root.clone());
        self.expanded.extend(directories);
        let added = self.expanded.len().saturating_sub(before);
        self.refresh()?;
        Ok(added)
    }

    pub fn collapse_all(&mut self) -> io::Result<usize> {
        let removed = self.expanded.len().saturating_sub(1);
        self.expanded.clear();
        self.expanded.insert(self.root.clone());
        self.selected = 0;
        self.scroll = 0;
        self.refresh()?;
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

        self.refresh()?;
        if let Some(index) = self.entries.iter().position(|entry| entry.path == path) {
            self.selected = index;
            return Ok(true);
        }

        Ok(false)
    }

    pub fn toggle_hidden(&mut self) -> io::Result<()> {
        self.show_hidden = !self.show_hidden;
        self.refresh()
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

#[derive(Debug)]
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

impl TreeView<'_> {
    fn shows(&self, path: &Path, name: &str, is_dir: bool) -> bool {
        if self.show_hidden {
            return true;
        }
        if name.starts_with('.') {
            return false;
        }
        !self
            .ignore_rules
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

fn readable_children(directory: &Path, view: TreeView) -> io::Result<Vec<ChildEntry>> {
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

        if !view.shows(&child.path(), &name, is_dir) {
            continue;
        }

        children.push(ChildEntry {
            path: child.path(),
            name,
            is_dir,
            is_symlink,
        });
    }

    children.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(children)
}

fn collect_entries(
    directory: &Path,
    depth: usize,
    expanded: &HashSet<PathBuf>,
    view: TreeView,
    output: &mut Vec<ProjectEntry>,
) -> io::Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Ok(());
    }
    let children = match readable_children(directory, view) {
        Ok(children) => children,
        Err(error) if depth > 0 => {
            let _ = error;
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    for child in children {
        let is_expanded = child.is_dir && expanded.contains(&child.path);
        output.push(ProjectEntry {
            path: child.path.clone(),
            name: child.name,
            depth,
            is_dir: child.is_dir,
            is_symlink: child.is_symlink,
            expanded: is_expanded,
            git_status: None,
        });

        if is_expanded {
            collect_entries(&child.path, depth + 1, expanded, view, output)?;
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
    view: TreeView,
    depth: usize,
    output: &mut Vec<ProjectEntry>,
) {
    if depth > MAX_TREE_DEPTH || output.len() >= MAX_FILTER_RESULTS {
        return;
    }
    let Ok(children) = readable_children(directory, view) else {
        return;
    };

    for child in children {
        if output.len() >= MAX_FILTER_RESULTS {
            return;
        }
        if child.is_dir {
            if !child.is_symlink {
                collect_filtered_entries(root, &child.path, needle, view, depth + 1, output);
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
            output.push(ProjectEntry {
                path: child.path.clone(),
                name: relative,
                depth: 0,
                is_dir: false,
                is_symlink: child.is_symlink,
                expanded: false,
                git_status: None,
            });
        }
    }
}

fn collect_directory_paths(
    directory: &Path,
    view: TreeView,
    output: &mut Vec<PathBuf>,
    limit: usize,
) -> io::Result<()> {
    if output.len() >= limit {
        return Ok(());
    }

    let children = match readable_children(directory, view) {
        Ok(children) => children,
        Err(_) => return Ok(()),
    };

    for child in children {
        // Never auto-expand through symlinks; a link cycle would recurse
        // forever.
        if !child.is_dir || child.is_symlink {
            continue;
        }

        output.push(child.path.clone());
        if output.len() >= limit {
            return Ok(());
        }

        collect_directory_paths(&child.path, view, output, limit)?;
        if output.len() >= limit {
            return Ok(());
        }
    }

    Ok(())
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
        assert!(!tree.entries.iter().any(|entry| entry.name == "build.log"));

        tree.toggle_hidden().unwrap();
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
        assert_eq!(tree.entries.len(), 1);
        assert!(tree.entries[0].name.contains("main.rs"));

        tree.set_filter(String::new()).unwrap();
        assert!(tree.entries.len() > 1);
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
        assert!(tree
            .entries
            .iter()
            .any(|entry| entry.name == "loop" && entry.is_dir && entry.is_symlink));
        tree.expand_all().unwrap();
        let _ = fs::remove_dir_all(root);
    }
}
