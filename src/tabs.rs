use std::{
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use crate::editor::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenDisposition {
    Opened,
    Switched,
}

pub struct BufferTab {
    pub editor: Editor,
    untitled_id: usize,
}

pub struct Tabs {
    tabs: Vec<BufferTab>,
    active: usize,
    next_untitled_id: usize,
}

impl Tabs {
    pub fn new(path: Option<&Path>) -> io::Result<Self> {
        let mut next_untitled_id = 1;
        let editor = Editor::new(path)?;
        let untitled_id = if path.is_none() {
            let id = next_untitled_id;
            next_untitled_id += 1;
            id
        } else {
            0
        };

        Ok(Self {
            tabs: vec![BufferTab {
                editor,
                untitled_id,
            }],
            active: 0,
            next_untitled_id,
        })
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn editor_for_path_mut(&mut self, path: &Path) -> Option<&mut Editor> {
        self.tabs.iter_mut().find_map(|tab| {
            tab.editor
                .path
                .as_deref()
                .filter(|candidate| *candidate == path)?;
            Some(&mut tab.editor)
        })
    }

    pub fn active_title(&self) -> String {
        self.tab_title(self.active)
    }

    pub fn tab_title(&self, index: usize) -> String {
        let Some(tab) = self.tabs.get(index) else {
            return "[Missing]".to_string();
        };

        tab.editor
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Untitled {}", tab.untitled_id.max(1)))
    }

    pub fn tab_dirty(&self, index: usize) -> bool {
        self.tabs
            .get(index)
            .map(|tab| tab.editor.dirty)
            .unwrap_or(false)
    }

    pub fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.editor.dirty)
    }

    pub fn dirty_titles(&self) -> Vec<String> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.editor.dirty)
            .map(|(index, _)| self.tab_title(index))
            .collect()
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }

        self.active = index;
        true
    }

    pub fn next(&mut self) {
        if self.tabs.len() > 1 {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    pub fn previous(&mut self) {
        if self.tabs.len() > 1 {
            self.active = if self.active == 0 {
                self.tabs.len() - 1
            } else {
                self.active - 1
            };
        }
    }

    pub fn first(&mut self) {
        self.active = 0;
    }

    pub fn last(&mut self) {
        self.active = self.tabs.len().saturating_sub(1);
    }

    pub fn new_buffer(&mut self) {
        let (show_line_numbers, tab_width) = self.active_settings();
        let mut editor = Editor::blank();
        editor.show_line_numbers = show_line_numbers;
        editor.tab_width = tab_width;
        editor.checkpoint();

        let untitled_id = self.next_untitled_id;
        self.next_untitled_id += 1;
        self.tabs.push(BufferTab {
            editor,
            untitled_id,
        });
        self.active = self.tabs.len() - 1;
    }

    pub fn new_named_buffer(&mut self, path: PathBuf) {
        let (show_line_numbers, tab_width) = self.active_settings();
        let mut editor = Editor::blank();
        editor.path = Some(absolute_path(path));
        editor.show_line_numbers = show_line_numbers;
        editor.tab_width = tab_width;
        editor.checkpoint();

        self.tabs.push(BufferTab {
            editor,
            untitled_id: 0,
        });
        self.active = self.tabs.len() - 1;
    }

    pub fn open_or_switch(&mut self, path: &Path) -> io::Result<OpenDisposition> {
        let path = normalized_path(path);

        if let Some(index) = self.tabs.iter().position(|tab| {
            tab.editor
                .path
                .as_ref()
                .map(|existing| normalized_path(existing))
                == Some(path.clone())
        }) {
            self.active = index;
            return Ok(OpenDisposition::Switched);
        }

        let (show_line_numbers, tab_width) = self.active_settings();
        let mut editor = Editor::new(Some(&path))?;
        editor.show_line_numbers = show_line_numbers;
        editor.tab_width = tab_width;
        editor.checkpoint();

        let replace_pristine_untitled = self.tabs.len() == 1
            && self.tabs[0].editor.path.is_none()
            && !self.tabs[0].editor.dirty
            && self.tabs[0].editor.len_chars() == 0;

        if replace_pristine_untitled {
            self.tabs[0] = BufferTab {
                editor,
                untitled_id: 0,
            };
            self.active = 0;
        } else {
            self.tabs.push(BufferTab {
                editor,
                untitled_id: 0,
            });
            self.active = self.tabs.len() - 1;
        }

        Ok(OpenDisposition::Opened)
    }

    pub fn close_active(&mut self, force: bool) -> Result<String, String> {
        if self.tabs.is_empty() {
            self.new_buffer();
        }

        let title = self.active_title();
        if self.tabs[self.active].editor.dirty && !force {
            return Err(format!(
                "{title} has unsaved changes — save it or use :tabclose!"
            ));
        }

        self.tabs.remove(self.active);

        if self.tabs.is_empty() {
            self.active = 0;
            self.new_buffer();
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }

        Ok(title)
    }

    pub fn save_all(&mut self) -> (usize, Vec<String>) {
        let mut saved = 0usize;
        let mut errors = Vec::new();

        for index in 0..self.tabs.len() {
            if !self.tabs[index].editor.dirty {
                continue;
            }

            let title = self.tab_title(index);
            if self.tabs[index].editor.path.is_none() {
                errors.push(format!("{title}: no filename"));
                continue;
            }

            match self.tabs[index].editor.save() {
                Ok(()) => saved += 1,
                Err(error) => errors.push(format!("{title}: {error}")),
            }
        }

        (saved, errors)
    }

    fn active_settings(&self) -> (bool, usize) {
        self.tabs
            .get(self.active)
            .map(|tab| (tab.editor.show_line_numbers, tab.editor.tab_width))
            .unwrap_or((true, 4))
    }

    pub fn titles_summary(&self) -> String {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let active = if index == self.active { ">" } else { " " };
                let dirty = if self.tab_dirty(index) { "*" } else { "" };
                format!("{active}{}:{}{dirty}", index + 1, self.tab_title(index))
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

impl Deref for Tabs {
    type Target = Editor;

    fn deref(&self) -> &Self::Target {
        &self.tabs[self.active].editor
    }
}

impl DerefMut for Tabs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tabs[self.active].editor
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    let absolute = absolute_path(path.to_path_buf());
    std::fs::canonicalize(&absolute).unwrap_or(absolute)
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(&path))
            .unwrap_or(path)
    }
}
