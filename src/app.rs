use std::{
    collections::HashMap,
    fs,
    io,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use crossterm::{
    event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    terminal,
};
use serde_json::json;

use crate::{
    config::{self, KeymapProfile, Settings},
    editor::Cursor,
    lsp::{self, LspClient},
    project::ProjectTree,
    syntax::Language,
    tabs::{OpenDisposition, Tabs},
    theme::{Theme, ThemeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    Command,
    Help,
    QuitConfirm,
    ReloadConfirm,
    GitDiff,
    GitHistory,
    ThemeGallery,
    KeymapGallery,
    ContextMenu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    Files,
    Themes,
    Help,
    Quit,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView { Files, Outline }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacroPrefix {
    Record,
    Replay,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Search => "SEARCH",
            Self::Command => "COMMAND",
            Self::Help => "HELP",
            Self::QuitConfirm => "QUIT?",
            Self::ReloadConfirm => "RELOAD?",
            Self::GitDiff => "DIFF",
            Self::GitHistory => "HISTORY",
            Self::ThemeGallery => "THEMES",
            Self::KeymapGallery => "KEYMAPS",
            Self::ContextMenu => "MENU",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationLocation {
    path: Option<PathBuf>,
    tab_index: usize,
    cursor: Cursor,
    scroll_line: usize,
    scroll_column: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct EditorView {
    pub tab_index: usize,
    pub cursor: Cursor,
    pub scroll_line: usize,
    pub scroll_column: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct SplitViews {
    pub primary: EditorView,
    pub secondary: EditorView,
    pub secondary_active: bool,
    pub vertical: bool,
}

#[derive(Debug, Clone)]
pub struct GitHistoryEntry { pub hash: String, pub summary: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Open,
    Rename,
    Duplicate,
    NewFile,
    NewFolder,
    Stage,
    Unstage,
    SaveTab,
    CloseTab,
    Copy,
    Cut,
    Paste,
    SelectAll,
    ToggleComment,
    Format,
}

impl ContextAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Rename => "Rename…",
            Self::Duplicate => "Duplicate…",
            Self::NewFile => "New file…",
            Self::NewFolder => "New folder…",
            Self::Stage => "Stage changes",
            Self::Unstage => "Unstage changes",
            Self::SaveTab => "Save",
            Self::CloseTab => "Close tab",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Paste => "Paste",
            Self::SelectAll => "Select all",
            Self::ToggleComment => "Toggle comment",
            Self::Format => "Format document",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Rename => "F2",
            Self::SaveTab => "Ctrl-S",
            Self::CloseTab => "Ctrl-W",
            Self::Copy => "Ctrl-C",
            Self::Cut => "Ctrl-X",
            Self::Paste => "Ctrl-V",
            Self::SelectAll => "Ctrl-A",
            Self::ToggleComment => "Ctrl-/",
            _ => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub x: u16,
    pub y: u16,
    pub selected: usize,
    pub actions: Vec<ContextAction>,
}

pub struct App {
    pub editor: Tabs,
    pub project: ProjectTree,
    pub explorer_focused: bool,
    pub sidebar_view: SidebarView,
    pub outline_selected: usize,
    pub outline_scroll: usize,
    pub mode: Mode,
    pub should_quit: bool,
    pub command_input: String,
    pub command_suggestion: usize,
    command_selection: Option<(usize, usize)>,
    command_cursor: usize,
    command_anchor: Option<usize>,
    pub search_input: String,
    pub last_search: String,
    pub message: String,
    pub theme_kind: ThemeKind,
    pub theme: Theme,
    pub viewport_rows: usize,
    pub viewport_columns: usize,
    pub follow_cursor: bool,
    pub split_views: Option<SplitViews>,
    pub help_page: usize,
    pub hover_target: Option<HoverTarget>,
    settings: Settings,
    pending_key: Option<char>,
    macro_prefix: Option<MacroPrefix>,
    recording_macro: Option<char>,
    macros: HashMap<char, Vec<KeyEvent>>,
    replaying_macro: bool,
    yank: String,
    search_origin: Cursor,
    back_history: Vec<NavigationLocation>,
    forward_history: Vec<NavigationLocation>,
    last_editor_click: Option<(Instant, Cursor)>,
    last_project_click: Option<(Instant, PathBuf)>,
    lsp: Option<LspClient>,
    lsp_version: i64,
    lsp_requests: HashMap<u64, String>,
    lsp_initialized: bool,
    lsp_workspace_ready: bool,
    lsp_last_text: String,
    pending_save_after_disk_change: bool,
    pub git_diff_lines: Vec<String>,
    pub git_diff_scroll: usize,
    pub git_history: Vec<GitHistoryEntry>,
    pub git_history_selected: usize,
    pub theme_gallery_selected: usize,
    theme_gallery_original: ThemeKind,
    pub keymap_gallery_selected: usize,
    pub context_menu: Option<ContextMenu>,
}

impl App {
    pub fn new(path: Option<&Path>) -> io::Result<Self> {
        let (settings, config_message) = config::load();
        let current_dir = std::env::current_dir()?;

        let (editor_path, project_root, explorer_focused): (Option<PathBuf>, PathBuf, bool) =
            match path {
                Some(path) if path.is_dir() => (None, path.to_path_buf(), true),
                Some(path) => {
                    let file_path = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        current_dir.join(path)
                    };
                    let root = file_path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| current_dir.clone());
                    (Some(file_path), root, false)
                }
                None => (None, current_dir, false),
            };

        let mut editor = Tabs::new(editor_path.as_deref())?;
        editor.show_line_numbers = settings.show_line_numbers;
        editor.tab_width = settings.tab_width.clamp(1, 16);
        editor.checkpoint();

        let mut project = ProjectTree::new(project_root)?;
        project.width = settings.tree_width.clamp(22, 80);
        project.show_hidden = settings.show_hidden_files;
        let _ = project.refresh();
        project.refresh_git_status();
        let mode = if explorer_focused || settings.keymap == KeymapProfile::Vim {
            Mode::Normal
        } else {
            Mode::Insert
        };
        let initial_theme = settings.theme;

        Ok(Self {
            editor,
            project,
            explorer_focused,
            sidebar_view: SidebarView::Files,
            outline_selected: 0,
            outline_scroll: 0,
            mode,
            should_quit: false,
            command_input: String::new(),
            command_suggestion: 0,
            command_selection: None,
            command_cursor: 0,
            command_anchor: None,
            search_input: String::new(),
            last_search: String::new(),
            message: config_message.unwrap_or_else(|| {
                if explorer_focused {
                    "FILES · Enter opens · Ctrl-E returns to editor".to_string()
                } else {
                    "-- INSERT -- · Ctrl-E opens files · Ctrl-S saves".to_string()
                }
            }),
            theme_kind: settings.theme,
            theme: Theme::for_kind(settings.theme),
            viewport_rows: 1,
            viewport_columns: 1,
            follow_cursor: true,
            split_views: None,
            help_page: 0,
            hover_target: None,
            settings,
            pending_key: None,
            macro_prefix: None,
            recording_macro: None,
            macros: HashMap::new(),
            replaying_macro: false,
            yank: String::new(),
            search_origin: Cursor::default(),
            back_history: Vec::new(),
            forward_history: Vec::new(),
            last_editor_click: None,
            last_project_click: None,
            lsp: None,
            lsp_version: 1,
            lsp_requests: HashMap::new(),
            lsp_initialized: false,
            lsp_workspace_ready: false,
            lsp_last_text: String::new(),
            pending_save_after_disk_change: false,
            git_diff_lines: Vec::new(),
            git_diff_scroll: 0,
            git_history: Vec::new(),
            git_history_selected: 0,
            theme_gallery_selected: 0,
            theme_gallery_original: initial_theme,
            keymap_gallery_selected: 0,
            context_menu: None,
        })
    }

    pub fn handle_event(&mut self, event: Event) -> bool {
        self.poll_lsp();
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.activate_focused_view();
                self.follow_cursor = true;
                self.handle_key(key);
                self.sync_focused_view();
                self.sync_lsp_document();
                true
            }
            Event::Mouse(mouse)
                if matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp
                        | MouseEventKind::ScrollDown
                        | MouseEventKind::Down(MouseButton::Left)
                        | MouseEventKind::Down(MouseButton::Right)
                        | MouseEventKind::Drag(MouseButton::Left)
                ) =>
            {
                self.activate_focused_view();
                self.handle_mouse(mouse);
                self.sync_focused_view();
                true
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Moved) => {
                if self.mode == Mode::ThemeGallery {
                    if let Ok((width, height)) = terminal::size() {
                        if let Some(index) = crate::ui::theme_gallery_item_at(self, width, height, mouse.column, mouse.row) {
                            if index != self.theme_gallery_selected {
                                self.theme_gallery_selected = index;
                                self.preview_gallery_theme();
                                return true;
                            }
                        }
                    }
                }
                if self.mode == Mode::KeymapGallery {
                    if let Ok((width, height)) = terminal::size() {
                        if let Some(index) = crate::ui::keymap_gallery_item_at(self, width, height, mouse.column, mouse.row) {
                            if index != self.keymap_gallery_selected {
                                self.keymap_gallery_selected = index;
                                return true;
                            }
                        }
                    }
                }
                self.update_hover(mouse.column, mouse.row)
            }
            Event::Resize(_, _) => true,
            _ => false,
        }
    }

    pub fn poll_background(&mut self) -> bool {
        let message = self.message.clone();
        self.poll_lsp();
        if self.mode != Mode::ReloadConfirm && self.editor.changed_on_disk() {
            self.mode = Mode::ReloadConfirm;
            self.message = "File changed on disk — [R] Reload   [K] Keep buffer   [Esc] Later".to_string();
        }
        self.message != message
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.mode == Mode::QuitConfirm {
            return;
        }

        let Ok((width, height)) = terminal::size() else {
            return;
        };

        if width < 44 || height < 8 {
            return;
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if self.mode == Mode::ThemeGallery {
                if let Some(index) = crate::ui::theme_gallery_item_at(self, width, height, mouse.column, mouse.row) {
                    self.theme_gallery_selected = index;
                    self.preview_gallery_theme();
                    self.settings.theme = self.theme_kind;
                    self.persist_settings();
                    self.mode = self.preferred_editor_mode();
                    self.message = format!("Theme: {}", self.theme_kind.name());
                }
                return;
            }
            if self.mode == Mode::KeymapGallery {
                if let Some(index) = crate::ui::keymap_gallery_item_at(self, width, height, mouse.column, mouse.row) {
                    self.keymap_gallery_selected = index;
                    self.apply_selected_keymap();
                }
                return;
            }
            if let Some(index) = crate::ui::command_suggestion_at(self, width, height, mouse.column, mouse.row) {
                if let Some(command) = self.command_suggestions().get(index) {
                    self.command_input = (*command).to_string();
                    self.command_suggestion = index;
                    self.message = "Command selected; press Enter to run".to_string();
                }
                return;
            }
        }

        let layout = crate::ui::screen_layout(self, width, height);
        let over_sidebar = layout.sidebar_width > 0
            && (mouse.column as usize) < layout.sidebar_width
            && mouse.row >= layout.content_top
            && (mouse.row as usize) < layout.content_top as usize + layout.content_height;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if over_sidebar {
                    for _ in 0..3 {
                        self.project.move_up();
                    }
                    self.project
                        .ensure_selected_visible(layout.content_height.saturating_sub(1));
                } else {
                    self.follow_cursor = false;
                    self.editor.scroll_vertical(-3, layout.content_height);
                }
            }
            MouseEventKind::ScrollDown => {
                if over_sidebar {
                    for _ in 0..3 {
                        self.project.move_down();
                    }
                    self.project
                        .ensure_selected_visible(layout.content_height.saturating_sub(1));
                } else {
                    self.follow_cursor = false;
                    self.editor.scroll_vertical(3, layout.content_height);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_left_click(mouse.column, mouse.row, width, height);
            }
            MouseEventKind::Down(MouseButton::Right) => self.copy_selection_to_clipboard(),
            MouseEventKind::Drag(MouseButton::Left) => {
                self.handle_editor_drag(mouse.column, mouse.row, width, height);
            }
            _ => {}
        }
    }

    fn update_hover(&mut self, column: u16, row: u16) -> bool {
        let Ok((width, height)) = terminal::size() else {
            return false;
        };
        let layout = crate::ui::screen_layout(self, width, height);
        let target = if row == 0 && (9..=17).contains(&column) {
            Some(HoverTarget::Files)
        } else if row == 0 && column >= width.saturating_sub(8) {
            Some(HoverTarget::Quit)
        } else if row == 0
            && column >= width.saturating_sub(30)
            && column < width.saturating_sub(20)
        {
            Some(HoverTarget::Themes)
        } else if row == 0 && column >= width.saturating_sub(20) {
            Some(HoverTarget::Help)
        } else if row == layout.hotkey_row
            && crate::ui::hotkey_action_at(self, width, column) == Some("Command")
        {
            Some(HoverTarget::Command)
        } else {
            None
        };

        if self.hover_target == target {
            false
        } else {
            self.hover_target = target;
            true
        }
    }

    fn handle_left_click(&mut self, column: u16, row: u16, width: u16, height: u16) {
        self.follow_cursor = true;
        let layout = crate::ui::screen_layout(self, width, height);

        if row == layout.hotkey_row {
            if crate::ui::hotkey_action_at(self, width, column) == Some("Command") {
                self.explorer_focused = false;
                self.command_input.clear();
                self.command_cursor = 0;
                self.command_anchor = None;
                self.command_selection = None;
                self.mode = Mode::Command;
                self.message = "Command".to_string();
            }
            return;
        }

        // The FILES item in the title bar is a clickable explorer toggle.
        if row == 0 && (9..=17).contains(&column) {
            self.toggle_file_tree();
            return;
        }

        // Top-right controls are always available, including in Insert mode.
        if row == 0 && column >= width.saturating_sub(8) {
            self.request_quit(false);
            return;
        }

        if row == 0
            && column >= width.saturating_sub(30)
            && column < width.saturating_sub(20)
        {
            self.explorer_focused = false;
            self.open_theme_gallery();
            return;
        }

        if row == 0 && column >= width.saturating_sub(20) {
            self.explorer_focused = false;
            self.mode = Mode::Help;
            return;
        }

        // Tabs occupy the second terminal row.
        if row == 1 {
            if let Some(index) = crate::ui::tab_index_at(self, width, column) {
                self.select_tab_with_history(index);
                self.explorer_focused = false;
                self.mode = self.preferred_editor_mode();
            }
            return;
        }

        let content_end = layout.content_top as usize + layout.content_height;
        if row < layout.content_top || row as usize >= content_end {
            return;
        }

        // Project tree: the first content row is its root heading; subsequent
        // rows correspond directly to visible tree entries.
        if layout.sidebar_width > 0 && (column as usize) < layout.sidebar_width {
            self.explorer_focused = true;
            self.mode = Mode::Normal;

            let screen_row = (row - layout.content_top) as usize;
            if self.sidebar_view == SidebarView::Outline {
                if screen_row > 0 {
                    let symbols = self.outline_symbols();
                    let index = self.outline_scroll + screen_row - 1;
                    if let Some(symbol) = symbols.get(index) {
                        self.outline_selected = index;
                        self.editor.goto_line(symbol.start_line);
                        self.editor.move_line_start();
                        self.follow_cursor = true;
                        self.message = format!("{} · line {}", symbol.name, symbol.start_line + 1);
                    }
                }
                return;
            }
            if screen_row == 0 {
                self.message = format!("Project: {}", self.project.root.display());
                return;
            }

            let entry_index = self.project.scroll + screen_row - 1;
            if entry_index < self.project.entries.len() {
                self.project.selected = entry_index;
                let path = self.project.entries[entry_index].path.clone();
                let double_click = self.last_project_click.as_ref().is_some_and(|(time, previous)| {
                    previous == &path && time.elapsed() <= Duration::from_millis(500)
                });
                self.last_project_click = Some((Instant::now(), path));
                if double_click {
                    self.begin_rename_selected();
                } else {
                    self.activate_project_entry();
                }
            }
            return;
        }

        // Editor area: the fold column is clickable; other gutter clicks move
        // to column zero. Text clicks translate the visual screen column
        // (including tabs and wide Unicode) into the nearest character.
        let editor_end = layout.editor_x + layout.editor_width;
        let column = column as usize;

        if column < layout.editor_x || column >= editor_end {
            return;
        }

        if let Some(mut views) = self.split_views {
            views.secondary_active = if views.vertical {
                column > layout.editor_x + layout.editor_width.saturating_sub(1) / 2
            } else {
                row >= layout.content_top + layout.content_height.saturating_sub(1) as u16 / 2 + 1
            };
            self.split_views = Some(views);
            self.activate_focused_view();
        }

        let before = self.current_location();
        self.explorer_focused = false;
        self.follow_cursor = true;
        self.mode = self.preferred_editor_mode();
        self.pending_key = None;

        let screen_row = (row - layout.content_top) as usize;
        let line = self
            .editor
            .visible_line_at(self.editor.scroll_line, screen_row)
            .unwrap_or_else(|| self.editor.line_count().saturating_sub(1));
        let local_x = column.saturating_sub(layout.editor_x);
        let clicked_fold_control = layout.gutter_width >= 2
            && local_x >= layout.gutter_width.saturating_sub(2)
            && local_x < layout.gutter_width;

        if clicked_fold_control {
            self.editor.clear_selection();
            self.editor.finish_undo_group();
            self.editor.set_cursor_from_display_position(line, 0);
            self.mode = Mode::Normal;
            self.toggle_fold();
            self.search_origin = self.editor.cursor;
            return;
        }

        let display_column = if local_x < layout.gutter_width {
            0
        } else {
            self.editor.scroll_column + local_x - layout.gutter_width
        };

        self.editor.clear_selection();
        self.editor.finish_undo_group();
        self.editor
            .set_cursor_from_display_position(line, display_column);
        let cursor = self.editor.cursor;
        let double_click = self.last_editor_click.is_some_and(|(time, previous)| {
            previous == cursor && time.elapsed() <= Duration::from_millis(500)
        });
        self.last_editor_click = Some((Instant::now(), cursor));
        if double_click {
            self.editor.select_word_at_cursor();
        }
        self.search_origin = self.editor.cursor;
        self.commit_navigation(before);
        self.message = format!(
            "Cursor: line {}, column {}",
            self.editor.cursor.line + 1,
            self.editor.cursor.column + 1
        );
    }

    fn handle_editor_drag(&mut self, column: u16, row: u16, width: u16, height: u16) {
        let layout = crate::ui::screen_layout(self, width, height);
        let content_end = layout.content_top as usize + layout.content_height;
        let column = column as usize;
        if row < layout.content_top
            || row as usize >= content_end
            || column < layout.editor_x
            || column >= layout.editor_x + layout.editor_width
        {
            return;
        }

        self.explorer_focused = false;
        self.follow_cursor = true;
        self.mode = self.preferred_editor_mode();
        self.editor.finish_undo_group();
        self.editor.begin_selection();
        let screen_row = (row - layout.content_top) as usize;
        let line = self
            .editor
            .visible_line_at(self.editor.scroll_line, screen_row)
            .unwrap_or_else(|| self.editor.line_count().saturating_sub(1));
        let local_x = column - layout.editor_x;
        let display_column = if local_x < layout.gutter_width {
            0
        } else {
            self.editor.scroll_column + local_x - layout.gutter_width
        };
        self.editor
            .set_cursor_from_display_position(line, display_column);
    }

    fn toggle_file_tree(&mut self) {
        self.project.visible = !self.project.visible;
        if !self.project.visible {
            self.explorer_focused = false;
        }
        self.message = if self.project.visible {
            "File tree shown".to_string()
        } else {
            "File tree hidden".to_string()
        };
    }

    fn copy_selection_to_clipboard(&mut self) {
        let Some(text) = self.editor.selected_text() else {
            self.message = "Select text to copy".to_string();
            return;
        };

        self.yank = text;
        self.message = match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(self.yank.clone()))
        {
            Ok(()) => "Copied selection to clipboard".to_string(),
            Err(_) => "Copied selection (internal clipboard)".to_string(),
        };
    }

    pub fn active_search_query(&self) -> &str {
        if self.mode == Mode::Search {
            &self.search_input
        } else {
            &self.last_search
        }
    }

    pub fn active_panel_label(&self) -> &'static str {
        if self.explorer_focused {
            match self.sidebar_view {
                SidebarView::Files => "FILES",
                SidebarView::Outline => "SYMBOLS",
            }
        } else {
            self.mode.label()
        }
    }

    fn activate_focused_view(&mut self) {
        let Some(views) = self.split_views else { return; };
        let view = if views.secondary_active { views.secondary } else { views.primary };
        self.editor.select(view.tab_index);
        self.editor.cursor = view.cursor;
        self.editor.scroll_line = view.scroll_line;
        self.editor.scroll_column = view.scroll_column;
    }

    fn sync_focused_view(&mut self) {
        let Some(mut views) = self.split_views else { return; };
        let view = EditorView { tab_index: self.editor.active_index(), cursor: self.editor.cursor, scroll_line: self.editor.scroll_line, scroll_column: self.editor.scroll_column };
        if views.secondary_active { views.secondary = view; } else { views.primary = view; }
        self.split_views = Some(views);
    }

    fn open_split(&mut self, vertical: bool) {
        if let Some(mut views) = self.split_views {
            views.vertical = vertical;
            self.split_views = Some(views);
            self.message = if vertical { "Vertical split" } else { "Horizontal split" }.to_string();
        } else {
            let view = EditorView { tab_index: self.editor.active_index(), cursor: self.editor.cursor, scroll_line: self.editor.scroll_line, scroll_column: self.editor.scroll_column };
            self.split_views = Some(SplitViews { primary: view, secondary: view, secondary_active: false, vertical });
            self.message = if vertical { "Vertical split opened · Ctrl-\\ switches panes" } else { "Horizontal split opened · Ctrl-\\ switches panes" }.to_string();
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Quit confirmation gets first chance at every key so terminal/editor
        // shortcuts cannot accidentally dismiss or bypass the prompt.
        if self.mode == Mode::QuitConfirm {
            self.handle_quit_confirmation(key);
            return;
        }
        if self.mode == Mode::ReloadConfirm {
            self.handle_reload_confirmation(key);
            return;
        }
        if self.mode == Mode::GitDiff {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = self.preferred_editor_mode(),
                KeyCode::Up | KeyCode::Char('k') => self.git_diff_scroll = self.git_diff_scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.git_diff_scroll = (self.git_diff_scroll + 1).min(self.git_diff_lines.len().saturating_sub(1)),
                KeyCode::PageUp => self.git_diff_scroll = self.git_diff_scroll.saturating_sub(self.viewport_rows.saturating_sub(2)),
                KeyCode::PageDown => self.git_diff_scroll = (self.git_diff_scroll + self.viewport_rows.saturating_sub(2)).min(self.git_diff_lines.len().saturating_sub(1)),
                _ => {}
            }
            return;
        }
        if self.mode == Mode::GitHistory {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = self.preferred_editor_mode(),
                KeyCode::Up | KeyCode::Char('k') => self.git_history_selected = self.git_history_selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.git_history_selected = (self.git_history_selected + 1).min(self.git_history.len().saturating_sub(1)),
                KeyCode::Enter => self.show_selected_history_diff(),
                _ => {}
            }
            return;
        }
        if self.mode == Mode::ThemeGallery {
            match key.code {
                KeyCode::Esc => { self.theme_kind = self.theme_gallery_original; self.theme = Theme::for_kind(self.theme_kind); self.mode = self.preferred_editor_mode(); self.message = "Theme preview cancelled".to_string(); }
                KeyCode::Up | KeyCode::Char('k') => { self.theme_gallery_selected = self.theme_gallery_selected.saturating_sub(1); self.preview_gallery_theme(); }
                KeyCode::Down | KeyCode::Char('j') => { self.theme_gallery_selected = (self.theme_gallery_selected + 1).min(ThemeKind::ALL.len() - 1); self.preview_gallery_theme(); }
                KeyCode::Enter => { self.settings.theme = self.theme_kind; self.persist_settings(); self.mode = self.preferred_editor_mode(); self.message = format!("Theme: {}", self.theme_kind.name()); }
                _ => {}
            }
            return;
        }
        if self.mode == Mode::KeymapGallery {
            match key.code {
                KeyCode::Esc => {
                    self.mode = self.preferred_editor_mode();
                    self.message = "Keymap selection cancelled".to_string();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.keymap_gallery_selected = self.keymap_gallery_selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.keymap_gallery_selected = (self.keymap_gallery_selected + 1)
                        .min(KeymapProfile::ALL.len().saturating_sub(1));
                }
                KeyCode::Enter => self.apply_selected_keymap(),
                _ => {}
            }
            return;
        }

        if self.handle_macro_prefix(key) {
            return;
        }

        if self.is_macro_stop_key(key) {
            self.stop_macro_recording();
            return;
        }

        if let Some(register) = self.recording_macro.filter(|_| !self.replaying_macro) {
            self.macros.entry(register).or_default().push(key);
        }

        if self.handle_global_shortcut(key) {
            return;
        }

        if matches!(self.mode, Mode::Normal | Mode::Insert) && self.explorer_focused {
            self.handle_explorer(key);
            return;
        }

        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
            Mode::Search => self.handle_search(key),
            Mode::Command => self.handle_command_input(key),
            Mode::Help => match key.code {
                KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
                    self.mode = self.preferred_editor_mode();
                }
                KeyCode::Left | KeyCode::Up | KeyCode::PageUp => {
                    self.help_page = self.help_page.saturating_sub(1);
                }
                KeyCode::Right | KeyCode::Down | KeyCode::PageDown => {
                    self.help_page = (self.help_page + 1).min(4);
                }
                KeyCode::Char(page @ '1'..='5') => {
                    self.help_page = page.to_digit(10).unwrap_or(1) as usize - 1;
                }
                _ => {}
            },
            Mode::QuitConfirm | Mode::ReloadConfirm | Mode::GitDiff | Mode::GitHistory | Mode::ThemeGallery | Mode::KeymapGallery => {}
        }
    }

    fn is_plain_normal_key(&self, key: KeyEvent, character: char) -> bool {
        self.mode == Mode::Normal
            && !self.explorer_focused
            && key.code == KeyCode::Char(character)
            && key.modifiers.is_empty()
    }

    fn is_macro_stop_key(&self, key: KeyEvent) -> bool {
        !self.replaying_macro
            && self.recording_macro.is_some()
            && self.is_plain_normal_key(key, 'q')
    }

    fn handle_macro_prefix(&mut self, key: KeyEvent) -> bool {
        let Some(prefix) = self.macro_prefix.take() else {
            return false;
        };

        let KeyCode::Char(register) = key.code else {
            self.message = "Macro register must be a character".to_string();
            return true;
        };

        match prefix {
            MacroPrefix::Record => {
                self.recording_macro = Some(register);
                self.macros.insert(register, Vec::new());
                self.message =
                    format!("Recording macro @{register}; press q in Normal mode to stop");
            }
            MacroPrefix::Replay => {
                if let Some(recording) = self.recording_macro.filter(|_| !self.replaying_macro) {
                    self.macros.entry(recording).or_default().push(key);
                }
                self.play_macro(register);
            }
        }
        true
    }

    fn stop_macro_recording(&mut self) {
        let Some(register) = self.recording_macro.take() else {
            return;
        };
        let count = self.macros.get(&register).map_or(0, Vec::len);
        self.message = format!("Recorded macro @{register} ({count} key events)");
    }

    fn play_macro(&mut self, register: char) {
        let Some(keys) = self.macros.get(&register).cloned() else {
            self.message = format!("Macro @{register} is empty");
            return;
        };

        if self.replaying_macro {
            self.message = "Nested macro replay is not supported".to_string();
            return;
        }

        self.replaying_macro = true;
        for key in keys.into_iter().take(1_000) {
            self.handle_key(key);
        }
        self.replaying_macro = false;
        self.message = format!("Replayed macro @{register}");
    }

    fn handle_quit_confirmation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                self.mode = self.preferred_editor_mode();
                self.message = "Quit cancelled".to_string();
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.should_quit = true;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if self.save_all_internal() {
                    self.should_quit = true;
                } else {
                    // A common reason is an unnamed tab that needs a filename.
                    // Return to the editor and preserve the detailed save error.
                    self.mode = self.preferred_editor_mode();
                }
            }
            _ => {}
        }
    }

    fn handle_reload_confirmation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('r') | KeyCode::Char('R') => match self.editor.reload_from_disk() {
                Ok(()) => { self.pending_save_after_disk_change = false; self.mode = self.preferred_editor_mode(); self.message = "Reloaded changed file".to_string(); }
                Err(error) => self.message = format!("Reload failed: {error}"),
            },
            KeyCode::Char('k') | KeyCode::Char('K') => {
                if self.pending_save_after_disk_change {
                    self.pending_save_after_disk_change = false;
                    self.editor.clear_pending_external_change();
                    self.mode = self.preferred_editor_mode();
                    self.save_internal();
                } else {
                    self.editor.keep_disk_change();
                    self.mode = self.preferred_editor_mode();
                    self.message = "Kept current buffer; save will ask before overwriting disk changes".to_string();
                }
            }
            KeyCode::Esc => { self.pending_save_after_disk_change = false; self.editor.acknowledge_disk_change(); self.mode = self.preferred_editor_mode(); self.message = "Reload deferred".to_string(); }
            _ => {}
        }
    }

    fn handle_global_shortcut(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p' | 'P') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.command_input.clear();
                    self.command_cursor = 0;
                    self.command_anchor = None;
                    self.command_selection = None;
                    self.command_suggestion = 0;
                    self.mode = Mode::Command;
                    self.message = "Command palette".to_string();
                    return true;
                }
                KeyCode::Char('f') => {
                    self.search_origin = self.editor.cursor;
                    self.search_input.clear();
                    self.mode = Mode::Search;
                    return true;
                }
                KeyCode::Char('z') => {
                    self.message = if self.editor.undo() { "Undone" } else { "Nothing to undo" }.to_string();
                    return true;
                }
                KeyCode::Char('y') => {
                    self.message = if self.editor.redo() { "Redone" } else { "Nothing to redo" }.to_string();
                    return true;
                }
                KeyCode::Char('\\') => {
                    if let Some(mut views) = self.split_views { self.sync_focused_view(); views.secondary_active = !views.secondary_active; self.split_views = Some(views); self.activate_focused_view(); }
                    return true;
                }
                KeyCode::Char('a') => {
                    self.editor.clear_selection();
                    self.editor.move_file_start();
                    self.editor.begin_selection();
                    self.editor.move_file_end();
                    self.message = "Selected all".to_string();
                    return true;
                }
                KeyCode::Char('c') => {
                    self.copy_selection_to_clipboard();
                    return true;
                }
                KeyCode::Char('x') => {
                    if let Some(text) = self.editor.selected_text() {
                        self.editor.checkpoint();
                        self.yank = text;
                        self.editor.delete_selection();
                        self.message = match arboard::Clipboard::new()
                            .and_then(|mut clipboard| clipboard.set_text(self.yank.clone()))
                        {
                            Ok(()) => "Cut selection to clipboard".to_string(),
                            Err(_) => "Cut selection (internal clipboard)".to_string(),
                        };
                    } else {
                        self.message = "Select text to cut".to_string();
                    }
                    return true;
                }
                KeyCode::Char('v') => {
                    let clipboard_text = arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.get_text())
                        .ok();
                    let text = clipboard_text.as_deref().unwrap_or(&self.yank);
                    if text.is_empty() {
                        self.message = "Clipboard is empty".to_string();
                    } else {
                        self.editor.checkpoint();
                        self.editor.insert_text(text);
                        self.message = "Pasted".to_string();
                    }
                    return true;
                }
                KeyCode::Tab => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.previous_tab();
                    } else {
                        self.next_tab();
                    }
                    return true;
                }
                KeyCode::BackTab | KeyCode::PageUp => {
                    self.previous_tab();
                    return true;
                }
                KeyCode::PageDown => {
                    self.next_tab();
                    return true;
                }
                KeyCode::Char('t') => {
                    self.new_tab(None);
                    return true;
                }
                KeyCode::Char('w') => {
                    self.close_active_tab(false);
                    return true;
                }
                KeyCode::Char('b') => {
                    self.toggle_file_tree();
                    return true;
                }
                KeyCode::Char('o') => {
                    self.project.visible = true;
                    self.sidebar_view = if self.sidebar_view == SidebarView::Outline {
                        SidebarView::Files
                    } else {
                        SidebarView::Outline
                    };
                    self.explorer_focused = true;
                    self.mode = Mode::Normal;
                    self.message = match self.sidebar_view {
                        SidebarView::Outline => "SYMBOLS · Enter jumps · Ctrl-E returns to editor",
                        SidebarView::Files => "FILES · Enter opens · Ctrl-E returns to editor",
                    }.to_string();
                    return true;
                }
                KeyCode::Char('/') => {
                    self.toggle_comments();
                    return true;
                }
                KeyCode::Char('e') => {
                    if !self.project.visible {
                        self.project.visible = true;
                        self.explorer_focused = true;
                    } else {
                        self.explorer_focused = !self.explorer_focused;
                    }
                    if self.explorer_focused {
                        self.mode = Mode::Normal;
                        self.message = "FILES · Enter opens · Ctrl-E returns to editor".to_string();
                    } else {
                        self.mode = self.preferred_editor_mode();
                        self.message = "Editor focused".to_string();
                    }
                    return true;
                }
                _ => {}
            }
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Left => {
                    self.go_back();
                    return true;
                }
                KeyCode::Right => {
                    self.go_forward();
                    return true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.next_tab();
                    return true;
                }
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.previous_tab();
                    return true;
                }
                KeyCode::Char(character @ '1'..='9') => {
                    let index = character.to_digit(10).unwrap_or(1) as usize - 1;
                    self.select_tab_with_history(index);
                    return true;
                }
                _ => {}
            }
        }

        false
    }

    fn indent_selection(&mut self, outdent: bool) {
        self.editor.checkpoint();
        let count = if outdent {
            self.editor.outdent_selected_lines()
        } else {
            self.editor.indent_selected_lines()
        };
        self.message = format!(
            "{} {count} line(s)",
            if outdent { "Outdented" } else { "Indented" }
        );
    }

    fn toggle_comments(&mut self) {
        let language = Language::from_path(self.editor.path.as_deref());
        let Some((prefix, suffix)) = language.comment_delimiters() else {
            self.message = format!("{} has no configured line-comment syntax", language.name());
            return;
        };

        self.editor.checkpoint();
        self.message = match self.editor.toggle_line_comments(prefix, suffix) {
            Some(true) => "Commented line(s)".to_string(),
            Some(false) => "Uncommented line(s)".to_string(),
            None => "No nonblank lines to comment".to_string(),
        };
    }

    fn available_fold_ranges(&self) -> Vec<(usize, usize)> {
        let language = Language::from_path(self.editor.path.as_deref());
        crate::syntax::fold_ranges(&self.editor.text(), language)
    }

    pub fn current_breadcrumbs(&self) -> String {
        let language = Language::from_path(self.editor.path.as_deref());
        crate::syntax::breadcrumbs(&self.editor.text(), language, self.editor.cursor.line)
            .into_iter()
            .map(|symbol| symbol.name)
            .collect::<Vec<_>>()
            .join(" › ")
    }

    pub fn outline_symbols(&self) -> Vec<crate::syntax::Symbol> {
        let language = Language::from_path(self.editor.path.as_deref());
        crate::syntax::symbols(&self.editor.text(), language)
    }

    fn show_symbols(&mut self) {
        let language = Language::from_path(self.editor.path.as_deref());
        let symbols = crate::syntax::symbols(&self.editor.text(), language);
        self.message = if symbols.is_empty() {
            "No symbols in this file".to_string()
        } else {
            symbols.into_iter().take(8).map(|symbol| format!("{}:{}", symbol.name, symbol.start_line + 1)).collect::<Vec<_>>().join("  ·  ")
        };
    }

    fn selected_project_path(&self) -> Option<PathBuf> {
        self.project.selected_entry().map(|entry| entry.path.clone())
    }

    fn project_target_path(&self, value: &str) -> Option<PathBuf> {
        let path = self.resolve_project_path(value);
        path.starts_with(&self.project.root).then_some(path)
    }

    fn create_project_file(&mut self, value: &str) {
        let Some(path) = self.project_target_path(value) else { self.message = "Path must be inside the project".to_string(); return; };
        if path.exists() { self.message = "File already exists".to_string(); return; }
        match path.parent().map(fs::create_dir_all).transpose().and_then(|_| fs::File::create(&path).map(|_| ())) {
            Ok(()) => { let _ = self.project.refresh(); self.message = format!("Created {}", path.display()); }
            Err(error) => self.message = format!("Create failed: {error}"),
        }
    }

    fn create_project_directory(&mut self, value: &str) {
        let Some(path) = self.project_target_path(value) else { self.message = "Path must be inside the project".to_string(); return; };
        match fs::create_dir(&path) {
            Ok(()) => { let _ = self.project.refresh(); self.message = format!("Created {}", path.display()); }
            Err(error) => self.message = format!("Create failed: {error}"),
        }
    }

    fn rename_selected_project_entry(&mut self, value: &str) {
        let Some(source) = self.selected_project_path() else { self.message = "Select a file or folder first".to_string(); return; };
        let Some(destination) = source.parent().and_then(|parent| (!value.is_empty()).then(|| parent.join(value))) else { self.message = "Usage: :rename new-name".to_string(); return; };
        match fs::rename(&source, &destination) {
            Ok(()) => { let _ = self.project.refresh(); self.message = format!("Renamed to {}", destination.display()); }
            Err(error) => self.message = format!("Rename failed: {error}"),
        }
    }

    fn begin_rename_selected(&mut self) {
        let Some(path) = self.selected_project_path() else { self.message = "Select a file or folder first".to_string(); return; };
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let prefix = "rename ";
        self.command_input = format!("{prefix}{name}");
        let stem_end = name.rfind('.').filter(|index| *index > 0).unwrap_or(name.len());
        self.command_cursor = prefix.len() + stem_end;
        self.command_selection = None;
        self.command_anchor = None;
        self.command_suggestion = 0;
        self.command_suggestion = 0;
        self.mode = Mode::Command;
        self.message = "Type a new name, then press Enter".to_string();
    }

    fn replace_command_selection(&mut self) {
        if let Some((start, end)) = self.command_selection.take() {
            if start <= end && end <= self.command_input.len() { self.command_input.replace_range(start..end, ""); self.command_cursor = start; }
        }
        self.command_anchor = None;
    }

    fn duplicate_selected_project_file(&mut self, value: &str) {
        let Some(source) = self.selected_project_path() else { self.message = "Select a file first".to_string(); return; };
        if source.is_dir() { self.message = "Duplicate currently supports files only".to_string(); return; }
        let destination = if value.is_empty() { source.with_file_name(format!("{} copy", source.file_stem().and_then(|name| name.to_str()).unwrap_or("file"))).with_extension(source.extension().unwrap_or_default()) } else { source.parent().unwrap_or(&self.project.root).join(value) };
        match fs::copy(&source, &destination) {
            Ok(_) => { let _ = self.project.refresh(); self.message = format!("Duplicated to {}", destination.display()); }
            Err(error) => self.message = format!("Duplicate failed: {error}"),
        }
    }

    fn move_selected_project_entry(&mut self, value: &str) {
        let Some(source) = self.selected_project_path() else { self.message = "Select a file or folder first".to_string(); return; };
        let Some(destination) = self.project_target_path(value) else { self.message = "Path must be inside the project".to_string(); return; };
        match fs::rename(&source, &destination) {
            Ok(()) => { let _ = self.project.refresh(); self.message = format!("Moved to {}", destination.display()); }
            Err(error) => self.message = format!("Move failed: {error}"),
        }
    }

    fn delete_selected_project_entry(&mut self, force: bool) {
        if !force { self.message = "Deletion is permanent; use :delete!".to_string(); return; }
        let Some(path) = self.selected_project_path() else { self.message = "Select a file or folder first".to_string(); return; };
        let result = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
        match result { Ok(()) => { let _ = self.project.refresh(); self.message = format!("Deleted {}", path.display()); }, Err(error) => self.message = format!("Delete failed: {error}"), }
    }

    fn git_selected(&mut self, stage: bool) {
        let Some(path) = self.selected_project_path() else { self.message = "Select a file first".to_string(); return; };
        let relative = path.strip_prefix(&self.project.root).unwrap_or(&path);
        let relative = relative.to_string_lossy().to_string();
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.project.root);
        if stage { command.args(["add", "--", &relative]); } else { command.args(["restore", "--staged", "--", &relative]); }
        match command.output() {
            Ok(output) if output.status.success() => { self.project.refresh_git_status(); self.message = if stage { "Staged selected file" } else { "Unstaged selected file" }.to_string(); }
            Ok(output) => self.message = format!("Git failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            Err(error) => self.message = format!("Git failed: {error}"),
        }
    }

    fn show_git_diff(&mut self) {
        let path = self.selected_project_path().or_else(|| self.editor.path.clone());
        let Some(path) = path else { self.message = "Select or open a file first".to_string(); return; };
        let relative = path.strip_prefix(&self.project.root).unwrap_or(&path).to_string_lossy().to_string();
        let run = |staged| {
            let mut command = Command::new("git");
            command.arg("-C").arg(&self.project.root).arg("diff");
            if staged { command.arg("--cached"); }
            command.args(["--", &relative]).output()
        };
        let output = run(false).ok().filter(|output| output.status.success()).or_else(|| run(true).ok());
        let Some(output) = output else { self.message = "Git diff failed".to_string(); return; };
        let text = String::from_utf8_lossy(&output.stdout);
        self.git_diff_lines = if text.is_empty() { vec!["No unstaged or staged changes for this file.".to_string()] } else { text.lines().map(str::to_string).collect() };
        self.git_diff_scroll = 0;
        self.mode = Mode::GitDiff;
    }

    fn show_git_history(&mut self) {
        let path = self.selected_project_path().or_else(|| self.editor.path.clone());
        let Some(path) = path else { self.message = "Select or open a file first".to_string(); return; };
        let relative = path.strip_prefix(&self.project.root).unwrap_or(&path).to_string_lossy().to_string();
        let output = Command::new("git").arg("-C").arg(&self.project.root).args(["log", "--format=%h%x09%s", "-n", "30", "--", &relative]).output();
        let Ok(output) = output else { self.message = "Git history failed".to_string(); return; };
        self.git_history = String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| line.split_once('\t').map(|(hash, summary)| GitHistoryEntry { hash: hash.to_string(), summary: summary.to_string() })).collect();
        self.git_history_selected = 0;
        self.mode = Mode::GitHistory;
    }

    fn show_selected_history_diff(&mut self) {
        let Some(entry) = self.git_history.get(self.git_history_selected) else { return; };
        let output = Command::new("git").arg("-C").arg(&self.project.root).args(["show", "--format=fuller", "--stat", &entry.hash]).output();
        let Ok(output) = output else { self.message = "Git show failed".to_string(); return; };
        self.git_diff_lines = String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).collect();
        self.git_diff_scroll = 0;
        self.mode = Mode::GitDiff;
    }

    fn close_fold(&mut self) {
        let ranges = self.available_fold_ranges();
        self.message = if self.editor.close_fold(&ranges) {
            "Fold closed"
        } else {
            "No fold at cursor"
        }
        .to_string();
    }

    fn open_fold(&mut self) {
        self.message = if self.editor.open_fold() {
            "Fold opened"
        } else {
            "No closed fold at cursor"
        }
        .to_string();
    }

    fn toggle_fold(&mut self) {
        let ranges = self.available_fold_ranges();
        self.message = match self.editor.toggle_fold(&ranges) {
            Some(true) => "Fold closed",
            Some(false) => "Fold opened",
            None => "No fold at cursor",
        }
        .to_string();
    }

    fn close_all_folds(&mut self) {
        let ranges = self.available_fold_ranges();
        let count = self.editor.close_all_folds(&ranges);
        self.message = format!("Closed {count} fold(s)");
    }

    fn open_all_folds(&mut self) {
        let count = self.editor.open_all_folds();
        self.message = format!("Opened {count} fold(s)");
    }

    fn new_tab(&mut self, path: Option<PathBuf>) {
        let before = self.current_location();
        match path {
            Some(path) => self.editor.new_named_buffer(path),
            None => self.editor.new_buffer(),
        }
        self.explorer_focused = false;
        self.mode = self.preferred_editor_mode();
        self.pending_key = None;
        self.search_origin = self.editor.cursor;
        self.commit_navigation(before);
        self.message = format!("New tab: {}", self.editor.active_title());
    }

    fn next_tab(&mut self) {
        let before = self.current_location();
        self.editor.next();
        self.commit_navigation(before);
        self.after_tab_switch();
    }

    fn previous_tab(&mut self) {
        let before = self.current_location();
        self.editor.previous();
        self.commit_navigation(before);
        self.after_tab_switch();
    }

    fn select_tab_with_history(&mut self, index: usize) {
        let before = self.current_location();
        if self.editor.select(index) {
            self.commit_navigation(before);
            self.after_tab_switch();
        } else {
            self.message = format!("Tab {} is not open", index + 1);
        }
    }

    fn current_location(&self) -> NavigationLocation {
        NavigationLocation {
            path: self.editor.path.clone(),
            tab_index: self.editor.active_index(),
            cursor: self.editor.cursor,
            scroll_line: self.editor.scroll_line,
            scroll_column: self.editor.scroll_column,
        }
    }

    fn commit_navigation(&mut self, before: NavigationLocation) {
        let after = self.current_location();
        if before == after {
            return;
        }

        if self.back_history.last() != Some(&before) {
            self.back_history.push(before);
            const HISTORY_LIMIT: usize = 200;
            if self.back_history.len() > HISTORY_LIMIT {
                self.back_history.remove(0);
            }
        }
        self.forward_history.clear();
    }

    fn restore_location(&mut self, location: &NavigationLocation) -> io::Result<()> {
        if let Some(path) = &location.path {
            self.editor.open_or_switch(path)?;
        } else if !self.editor.select(location.tab_index) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the untitled tab is no longer open",
            ));
        }

        self.editor.goto_line(location.cursor.line);
        self.editor.cursor.column = location
            .cursor
            .column
            .min(self.editor.line_len_chars(self.editor.cursor.line));
        self.editor.scroll_line = location.scroll_line;
        self.editor.scroll_column = location.scroll_column;
        self.explorer_focused = false;
        self.mode = self.preferred_editor_mode();
        self.pending_key = None;
        self.search_origin = self.editor.cursor;
        Ok(())
    }

    fn go_back(&mut self) {
        let Some(location) = self.back_history.pop() else {
            self.message = "No earlier location".to_string();
            return;
        };

        let current = self.current_location();
        match self.restore_location(&location) {
            Ok(()) => {
                self.forward_history.push(current);
                self.message = format!(
                    "Back: {} · line {}",
                    self.editor.active_title(),
                    self.editor.cursor.line + 1
                );
            }
            Err(error) => {
                self.back_history.push(location);
                self.message = format!("Cannot go back: {error}");
            }
        }
    }

    fn go_forward(&mut self) {
        let Some(location) = self.forward_history.pop() else {
            self.message = "No later location".to_string();
            return;
        };

        let current = self.current_location();
        match self.restore_location(&location) {
            Ok(()) => {
                self.back_history.push(current);
                self.message = format!(
                    "Forward: {} · line {}",
                    self.editor.active_title(),
                    self.editor.cursor.line + 1
                );
            }
            Err(error) => {
                self.forward_history.push(location);
                self.message = format!("Cannot go forward: {error}");
            }
        }
    }

    fn after_tab_switch(&mut self) {
        self.pending_key = None;
        self.search_origin = self.editor.cursor;
        if matches!(
            self.mode,
            Mode::Search | Mode::Command | Mode::Help | Mode::QuitConfirm
        ) {
            self.mode = self.preferred_editor_mode();
        }
        self.message = format!(
            "Tab {}/{}: {}",
            self.editor.active_index() + 1,
            self.editor.len(),
            self.editor.active_title()
        );
    }

    fn close_active_tab(&mut self, force: bool) {
        match self.editor.close_active(force) {
            Ok(title) => {
                self.pending_key = None;
                self.search_origin = self.editor.cursor;
                self.message = format!("Closed {title}");
            }
            Err(message) => self.message = message,
        }
    }

    fn handle_explorer(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => self.save(),
                KeyCode::Char('q') => self.request_quit(false),
                _ => {}
            }
            return;
        }

        if self.sidebar_view == SidebarView::Outline {
            let symbols = self.outline_symbols();
            match key.code {
                KeyCode::Esc => { self.explorer_focused = false; self.mode = self.preferred_editor_mode(); }
                KeyCode::Up | KeyCode::Char('k') => self.outline_selected = self.outline_selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.outline_selected = (self.outline_selected + 1).min(symbols.len().saturating_sub(1)),
                KeyCode::PageUp => self.outline_selected = self.outline_selected.saturating_sub(self.viewport_rows.saturating_sub(2)),
                KeyCode::PageDown => self.outline_selected = (self.outline_selected + self.viewport_rows.saturating_sub(2)).min(symbols.len().saturating_sub(1)),
                KeyCode::Home => self.outline_selected = 0,
                KeyCode::End => self.outline_selected = symbols.len().saturating_sub(1),
                KeyCode::Enter => if let Some(symbol) = symbols.get(self.outline_selected) { self.editor.goto_line(symbol.start_line); self.editor.move_line_start(); self.follow_cursor = true; self.message = format!("{} · line {}", symbol.name, symbol.start_line + 1); },
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::F(1) | KeyCode::Char('?') => {
                self.explorer_focused = false;
                self.mode = Mode::Help;
            }
            KeyCode::Esc => {
                self.explorer_focused = false;
                self.mode = self.preferred_editor_mode();
                self.message = "Editor focused".to_string();
            }
            KeyCode::Up | KeyCode::Char('k') => self.project.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.project.move_down(),
            KeyCode::PageUp => self.project.page_up(self.viewport_rows.saturating_sub(2)),
            KeyCode::PageDown => self.project.page_down(self.viewport_rows.saturating_sub(2)),
            KeyCode::Home | KeyCode::Char('g') => self.project.jump_to_root(),
            KeyCode::End | KeyCode::Char('G') => {
                self.project.selected = self.project.entries.len().saturating_sub(1)
            }
            KeyCode::Backspace => {
                if let Err(error) = self.project.collapse_or_parent() {
                    self.message = format!("Folder error: {error}");
                }
            }
            KeyCode::Left | KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                match self.project.collapse_selected_recursive() {
                    Ok(count) => self.message = format!("Collapsed {count} folder(s)"),
                    Err(error) => self.message = format!("Folder error: {error}"),
                }
            }
            KeyCode::Right | KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                match self.project.expand_selected_one_level() {
                    Ok(count) => self.message = format!("Expanded {count} folder(s)"),
                    Err(error) => self.message = format!("Folder error: {error}"),
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Err(error) = self.project.collapse_or_parent() {
                    self.message = format!("Folder error: {error}");
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Err(error) = self.project.expand_selected() {
                    self.message = format!("Folder error: {error}");
                }
            }
            KeyCode::Char('*') => match self.project.expand_all() {
                Ok(count) => self.message = format!("Expanded {count} folder(s)"),
                Err(error) => self.message = format!("Expand all failed: {error}"),
            },
            KeyCode::Char('-') => match self.project.collapse_all() {
                Ok(count) => self.message = format!("Collapsed {count} folder(s)"),
                Err(error) => self.message = format!("Collapse all failed: {error}"),
            },
            KeyCode::Enter => self.activate_project_entry(),
            KeyCode::Char('r') => match self.project.refresh() {
                Ok(()) => self.message = "File tree refreshed".to_string(),
                Err(error) => self.message = format!("Refresh failed: {error}"),
            },
            KeyCode::Char('.') => match self.project.toggle_hidden() {
                Ok(()) => {
                    self.message = if self.project.show_hidden {
                        "Hidden files shown".to_string()
                    } else {
                        "Hidden files hidden".to_string()
                    }
                }
                Err(error) => self.message = format!("Refresh failed: {error}"),
            },
            KeyCode::F(2) => self.begin_rename_selected(),
            KeyCode::Char('n') => { self.command_input = "newfile ".to_string(); self.command_suggestion = 0; self.mode = Mode::Command; }
            KeyCode::Delete => self.delete_selected_project_entry(false),
            KeyCode::Char('s') => self.git_selected(true),
            KeyCode::Char('u') => self.git_selected(false),
            _ => {}
        }
    }

    fn activate_project_entry(&mut self) {
        match self.project.activate_selected() {
            Ok(Some(path)) => {
                let before = self.current_location();
                match self.editor.open_or_switch(&path) {
                    Ok(disposition) => {
                        self.commit_navigation(before);
                        self.explorer_focused = false;
                        self.mode = self.preferred_editor_mode();
                        self.pending_key = None;
                        self.search_origin = self.editor.cursor;
                        self.message = match disposition {
                            OpenDisposition::Opened => {
                                format!("Opened {} in a new tab", path.display())
                            }
                            OpenDisposition::Switched => format!("Switched to {}", path.display()),
                        };
                    }
                    Err(error) => self.message = format!("Open failed: {error}"),
                }
            }
            Ok(None) => {}
            Err(error) => self.message = format!("Folder error: {error}"),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && self.extend_selection(key.code)
        {
            return;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            let moved = match key.code {
                KeyCode::Up => {
                    self.editor.checkpoint();
                    self.editor.move_line(false)
                }
                KeyCode::Down => {
                    self.editor.checkpoint();
                    self.editor.move_line(true)
                }
                _ => return,
            };
            self.message = if moved {
                "Moved line".to_string()
            } else {
                "Line cannot move further".to_string()
            };
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => self.save(),
                KeyCode::Char('q') => self.request_quit(false),
                KeyCode::Char('r') => {
                    if self.editor.redo() {
                        self.message = "Redone".to_string();
                    } else {
                        self.message = "Nothing to redo".to_string();
                    }
                }
                KeyCode::Char('d') => {
                    if self.editor.select_next_occurrence() {
                        self.message = format!(
                            "Selected {} occurrences",
                            self.editor.selection_ranges().len()
                        );
                    } else {
                        self.message = "No next occurrence".to_string();
                    }
                }
                KeyCode::Char('j') => {
                    self.editor.checkpoint();
                    self.message = if self.editor.join_line_below() {
                        "Joined lines".to_string()
                    } else {
                        "No line below to join".to_string()
                    };
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.editor.begin_selection();
                        self.editor.extend_word_backward();
                    } else {
                        self.editor.clear_selection();
                        self.editor.move_word_backward();
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.editor.begin_selection();
                        self.editor.extend_word_forward();
                    } else {
                        self.editor.clear_selection();
                        self.editor.move_word_forward();
                    }
                }
                _ => {}
            }
            return;
        }

        // Conventional mode is intentionally non-modal. If an overlay or
        // explorer action left the editor in Normal mode, printable input
        // still behaves like a regular text field instead of running a Vim
        // command by surprise.
        if self.settings.keymap == KeymapProfile::Conventional
            && matches!(key.code, KeyCode::Char(_))
            && key.modifiers.is_empty()
        {
            self.mode = Mode::Insert;
            self.handle_insert(key);
            return;
        }

        if let Some(pending) = self.pending_key.take() {
            match (pending, key.code) {
                ('d', KeyCode::Char('d')) => {
                    self.editor.checkpoint();
                    if let Some(line) = self.editor.delete_line() {
                        self.yank = line;
                        self.message = "Deleted line".to_string();
                    }
                    return;
                }
                ('y', KeyCode::Char('y')) => {
                    self.yank = self.editor.line_with_ending(self.editor.cursor.line);
                    self.message = "Yanked line".to_string();
                    return;
                }
                ('g', KeyCode::Char('g')) => {
                    self.editor.move_file_start();
                    return;
                }
                ('z', KeyCode::Char('c')) => {
                    self.close_fold();
                    return;
                }
                ('z', KeyCode::Char('o')) => {
                    self.open_fold();
                    return;
                }
                ('z', KeyCode::Char('a')) => {
                    self.toggle_fold();
                    return;
                }
                ('z', KeyCode::Char('M')) => {
                    self.close_all_folds();
                    return;
                }
                ('z', KeyCode::Char('R')) => {
                    self.open_all_folds();
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::F(1) | KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('i') => self.enter_insert(false),
            KeyCode::Char('a') => self.enter_insert(true),
            KeyCode::Char('o') => {
                self.editor.checkpoint();
                self.editor.open_line_below();
                self.mode = Mode::Insert;
                self.message = "-- INSERT --".to_string();
            }
            KeyCode::Char('O') => {
                self.editor.checkpoint();
                self.editor.open_line_above();
                self.mode = Mode::Insert;
                self.message = "-- INSERT --".to_string();
            }
            KeyCode::Esc => {
                self.pending_key = None;
                self.message.clear();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.editor.clear_selection();
                self.editor.move_left();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.editor.clear_selection();
                self.editor.move_down();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.editor.clear_selection();
                self.editor.move_up();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.editor.clear_selection();
                self.editor.move_right();
            }
            KeyCode::Home | KeyCode::Char('0') => self.editor.move_line_start(),
            KeyCode::End | KeyCode::Char('$') => self.editor.move_line_end(),
            KeyCode::PageUp => self.editor.page_up(self.viewport_rows.saturating_sub(1)),
            KeyCode::PageDown => self.editor.page_down(self.viewport_rows.saturating_sub(1)),
            KeyCode::Char('w') => self.editor.move_word_forward(),
            KeyCode::Char('b') => self.editor.move_word_backward(),
            KeyCode::Char('G') => self.editor.move_file_end(),
            KeyCode::Char('g') => self.pending_key = Some('g'),
            KeyCode::Char('z') => self.pending_key = Some('z'),
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.indent_selection(true)
            }
            KeyCode::Tab => self.indent_selection(false),
            KeyCode::BackTab => self.indent_selection(true),
            KeyCode::Char('q') if key.modifiers.is_empty() => {
                self.macro_prefix = Some(MacroPrefix::Record);
                self.message = "Record macro: choose a register".to_string();
            }
            KeyCode::Char('@') if key.modifiers.is_empty() => {
                self.macro_prefix = Some(MacroPrefix::Replay);
                self.message = "Replay macro: choose a register".to_string();
            }
            KeyCode::Char('d') => self.pending_key = Some('d'),
            KeyCode::Char('D') => {
                self.editor.checkpoint();
                self.editor.duplicate_line();
                self.message = "Duplicated line".to_string();
            }
            KeyCode::Char('y') => self.pending_key = Some('y'),
            KeyCode::Char('x') | KeyCode::Delete => {
                self.editor.checkpoint();
                if !self.editor.delete_at_cursor() {
                    self.message = "Nothing to delete".to_string();
                }
            }
            KeyCode::Char('p') => {
                if self.yank.is_empty() {
                    self.message = "Yank buffer is empty".to_string();
                } else {
                    self.editor.checkpoint();
                    self.editor.paste_line_below(&self.yank);
                    self.message = "Pasted".to_string();
                }
            }
            KeyCode::Char('u') => {
                if self.editor.undo() {
                    self.message = "Undone".to_string();
                } else {
                    self.message = "Nothing to undo".to_string();
                }
            }
            KeyCode::Char('/') => {
                self.search_origin = self.editor.cursor;
                self.search_input.clear();
                self.mode = Mode::Search;
            }
            KeyCode::Char('n') => self.repeat_search(true),
            KeyCode::Char('N') => self.repeat_search(false),
            KeyCode::Char('%') => {
                if !self.editor.jump_to_matching_bracket() {
                    self.message = "No matching bracket".to_string();
                }
            }
            KeyCode::Char(':') | KeyCode::Char(';')
                if key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.command_input.clear();
                self.command_cursor = 0;
                self.command_anchor = None;
                self.command_selection = None;
                self.mode = Mode::Command;
            }
            _ => {}
        }
    }

    fn enter_insert(&mut self, after: bool) {
        self.editor.checkpoint();
        if after && self.editor.cursor.column < self.editor.line_len_chars(self.editor.cursor.line)
        {
            self.editor.move_right();
        }
        self.mode = Mode::Insert;
        self.message = "-- INSERT --".to_string();
    }

    fn handle_insert(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && self.extend_selection(key.code)
        {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('s') => self.save(),
                KeyCode::Char('q') => self.request_quit(false),
                KeyCode::Char('d') => {
                    if self.editor.select_next_occurrence() {
                        self.message = format!(
                            "Selected {} occurrences",
                            self.editor.selection_ranges().len()
                        );
                    } else {
                        self.message = "No next occurrence".to_string();
                    }
                }
                KeyCode::Left => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.editor.begin_selection();
                        self.editor.extend_word_backward();
                    } else {
                        self.editor.clear_selection();
                        self.editor.move_word_backward();
                    }
                }
                KeyCode::Right => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.editor.begin_selection();
                        self.editor.extend_word_forward();
                    } else {
                        self.editor.clear_selection();
                        self.editor.move_word_forward();
                    }
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::F(1) => self.mode = Mode::Help,
            KeyCode::F(7) => {
                self.editor.checkpoint();
                self.editor.duplicate_line();
                self.message = "Duplicated line".to_string();
            }
            KeyCode::Esc => {
                self.command_selection = None;
                self.command_anchor = None;
                self.editor.finish_undo_group();
                if self.settings.keymap == KeymapProfile::Conventional {
                    self.editor.clear_selection();
                    self.mode = Mode::Insert;
                    self.message = "Selection cleared".to_string();
                } else {
                    self.mode = Mode::Normal;
                    self.message = "-- NORMAL --".to_string();
                }
            }
            KeyCode::Enter => self.editor.insert_char('\n'),
            KeyCode::Backspace => {
                self.editor.backspace_pair();
            }
            KeyCode::Delete => {
                self.editor.delete_at_cursor();
            }
            KeyCode::Tab => self.editor.insert_tab(),
            KeyCode::Left => {
                self.editor.clear_selection();
                self.editor.move_left();
            }
            KeyCode::Right => {
                self.editor.clear_selection();
                self.editor.move_right();
            }
            KeyCode::Up => {
                self.editor.clear_selection();
                self.editor.move_up();
            }
            KeyCode::Down => {
                self.editor.clear_selection();
                self.editor.move_down();
            }
            KeyCode::Home => self.editor.move_line_start(),
            KeyCode::End => self.editor.move_line_end(),
            KeyCode::Char('(') => self.editor.insert_pair('(', ')'),
            KeyCode::Char('[') => self.editor.insert_pair('[', ']'),
            KeyCode::Char('{') => self.editor.insert_pair('{', '}'),
            KeyCode::Char(')') => {
                if !self.editor.skip_closing_character(')') {
                    self.editor.insert_char(')');
                }
            }
            KeyCode::Char(']') => {
                if !self.editor.skip_closing_character(']') {
                    self.editor.insert_char(']');
                }
            }
            KeyCode::Char('}') => {
                if !self.editor.skip_closing_character('}') {
                    self.editor.insert_char('}');
                }
            }
            KeyCode::Char(quote @ ('\'' | '"')) => {
                if !self.editor.skip_closing_character(quote) {
                    self.editor.insert_pair(quote, quote);
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.editor.insert_char(character);
            }
            _ => {}
        }
    }

    fn extend_selection(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Left => {
                self.editor.begin_selection();
                self.editor.move_left();
            }
            KeyCode::Right => {
                self.editor.begin_selection();
                self.editor.move_right();
            }
            KeyCode::Up => {
                self.editor.begin_selection();
                self.editor.move_up();
            }
            KeyCode::Down => {
                self.editor.begin_selection();
                self.editor.move_down();
            }
            KeyCode::Home => {
                self.editor.begin_selection();
                self.editor.move_line_start();
            }
            KeyCode::End => {
                self.editor.begin_selection();
                self.editor.move_line_end();
            }
            KeyCode::PageUp => {
                self.editor.begin_selection();
                self.editor.page_up(self.viewport_rows.saturating_sub(1));
            }
            KeyCode::PageDown => {
                self.editor.begin_selection();
                self.editor.page_down(self.viewport_rows.saturating_sub(1));
            }
            _ => return false,
        }
        true
    }

    fn handle_search(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.editor.cursor = self.search_origin;
                self.mode = self.preferred_editor_mode();
                self.message = "Search cancelled".to_string();
            }
            KeyCode::Enter => {
                if self.search_input.is_empty() {
                    self.message = "Empty search".to_string();
                } else {
                    self.last_search = self.search_input.clone();
                    self.message = format!("Search: {}", self.last_search);
                }
                self.mode = self.preferred_editor_mode();
            }
            KeyCode::Backspace => {
                self.search_input.pop();
                self.preview_search();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.search_input.push(character);
                self.preview_search();
            }
            _ => {}
        }
    }

    fn preview_search(&mut self) {
        self.editor.cursor = self.search_origin;

        if self.search_input.is_empty() {
            return;
        }

        if !self.editor.find_next(&self.search_input, true) {
            self.message = format!("No match: {}", self.search_input);
        }
    }

    fn repeat_search(&mut self, forward: bool) {
        if self.last_search.is_empty() {
            self.message = "No previous search".to_string();
        } else if !self.editor.find_next(&self.last_search, forward) {
            self.message = format!("No match: {}", self.last_search);
        }
    }

    fn handle_command_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = self.preferred_editor_mode();
                self.message = "Command cancelled".to_string();
            }
            KeyCode::Enter => {
                let command = self.selected_command().unwrap_or_else(|| self.command_input.trim().to_string());
                self.command_selection = None;
                self.command_anchor = None;
                self.mode = self.preferred_editor_mode();
                self.execute_command(&command);
            }
            KeyCode::Backspace => {
                if self.command_selection.is_some() { self.replace_command_selection(); } else if self.command_cursor > 0 {
                    let start = previous_char_boundary(&self.command_input, self.command_cursor);
                    self.command_input.replace_range(start..self.command_cursor, "");
                    self.command_cursor = start;
                }
                self.command_suggestion = 0;
            }
            KeyCode::Delete => {
                if self.command_selection.is_some() { self.replace_command_selection(); } else if self.command_cursor < self.command_input.len() {
                    let end = next_char_boundary(&self.command_input, self.command_cursor);
                    self.command_input.replace_range(self.command_cursor..end, "");
                }
            }
            KeyCode::Home => { self.command_cursor = 0; self.command_anchor = None; self.command_selection = None; }
            KeyCode::End => { self.command_cursor = self.command_input.len(); self.command_anchor = None; self.command_selection = None; }
            KeyCode::Left => self.move_command_cursor(false, key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::Right => self.move_command_cursor(true, key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::Up => self.command_suggestion = self.command_suggestion.saturating_sub(1),
            KeyCode::Down => self.command_suggestion = (self.command_suggestion + 1).min(self.command_suggestions().len().saturating_sub(1)),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.replace_command_selection();
                self.command_input.insert(self.command_cursor, character);
                self.command_cursor += character.len_utf8();
                self.command_suggestion = 0;
            }
            _ => {}
        }
    }

    fn move_command_cursor(&mut self, forward: bool, extend: bool) {
        if !extend && self.command_selection.is_some() {
            let (start, end) = self.command_selection.take().unwrap();
            self.command_cursor = if forward { end } else { start };
            self.command_anchor = None;
            return;
        }
        if extend && self.command_anchor.is_none() { self.command_anchor = Some(self.command_cursor); }
        self.command_cursor = if forward { next_char_boundary(&self.command_input, self.command_cursor) } else { previous_char_boundary(&self.command_input, self.command_cursor) };
        self.command_selection = self.command_anchor.and_then(|anchor| (anchor != self.command_cursor).then_some((anchor.min(self.command_cursor), anchor.max(self.command_cursor))));
        if !extend { self.command_anchor = None; }
    }

    pub fn command_suggestions(&self) -> Vec<&'static str> {
        const COMMANDS: &[&str] = &["w", "wa", "q", "q!", "wq", "e", "new", "tabnew", "tabnext", "tabprev", "tree", "newfile", "newdir", "rename", "copy", "move", "delete", "delete!", "refresh", "gitrefresh", "stage", "unstage", "diff", "history", "symbols", "outline", "split", "vsplit", "only", "unsplit", "fold", "foldopen", "foldall", "unfoldall", "format", "lsp", "lspstop", "set formatonsave", "set noformatonsave", "set number", "set nonumber", "set", "theme", "themes", "keymap", "keymaps", "help"];
        let query = self.command_input.trim().to_ascii_lowercase();
        COMMANDS.iter().copied().filter(|command| command.contains(&query)).collect()
    }

    pub fn command_cursor(&self) -> usize { self.command_cursor.min(self.command_input.len()) }

    pub fn command_selection(&self) -> Option<(usize, usize)> { self.command_selection }

    fn selected_command(&self) -> Option<String> {
        let suggestions = self.command_suggestions();
        if self.command_input.trim().is_empty() || suggestions.iter().any(|value| *value == self.command_input.trim()) { None }
        else { suggestions.get(self.command_suggestion).map(|value| (*value).to_string()) }
    }

    fn execute_command(&mut self, command: &str) {
        if command.is_empty() {
            return;
        }

        if let Ok(line_number) = command.parse::<usize>() {
            let before = self.current_location();
            self.editor.goto_line(line_number.saturating_sub(1));
            self.commit_navigation(before);
            self.message = format!("Line {line_number}");
            return;
        }

        let mut parts = command.split_whitespace();
        let action = parts.next().unwrap_or_default();
        let argument = parts.collect::<Vec<_>>().join(" ");

        match action {
            "w" => {
                if argument.is_empty() {
                    self.save();
                } else {
                    let path = self.resolve_project_path(&argument);
                    self.save_as(path);
                }
            }
            "wa" | "wall" => self.save_all(),
            "q" => self.request_quit(false),
            "q!" => self.request_quit(true),
            "qa" | "qall" => self.request_quit(false),
            "qa!" | "qall!" => self.request_quit(true),
            "wq" | "x" => {
                if self.save_internal() {
                    self.request_quit(false);
                }
            }
            "wqa" | "wqall" => {
                if self.save_all_internal() {
                    self.request_quit(false);
                }
            }
            "new" => {
                if argument.is_empty() {
                    self.new_tab(None);
                } else {
                    let path = self.resolve_project_path(&argument);
                    self.new_tab(Some(path));
                }
            }
            "e" | "e!" | "edit" => {
                if argument.is_empty() {
                    self.message = "Usage: :e path".to_string();
                } else {
                    let path = self.resolve_project_path(&argument);
                    if path.is_dir() {
                        self.change_project_root(path);
                    } else {
                        let before = self.current_location();
                        match self.editor.open_or_switch(&path) {
                            Ok(disposition) => {
                                self.commit_navigation(before);
                                self.explorer_focused = false;
                                self.mode = self.preferred_editor_mode();
                                self.search_origin = self.editor.cursor;
                                self.message = match disposition {
                                    OpenDisposition::Opened => {
                                        format!("Opened {} in a new tab", path.display())
                                    }
                                    OpenDisposition::Switched => {
                                        format!("Switched to {}", path.display())
                                    }
                                };
                            }
                            Err(error) => self.message = format!("Open failed: {error}"),
                        }
                    }
                }
            }
            "tabnew" | "tabe" => {
                if argument.is_empty() {
                    self.new_tab(None);
                } else {
                    let path = self.resolve_project_path(&argument);
                    let before = self.current_location();
                    match self.editor.open_or_switch(&path) {
                        Ok(_) => {
                            self.commit_navigation(before);
                            self.explorer_focused = false;
                            self.mode = self.preferred_editor_mode();
                            self.after_tab_switch();
                        }
                        Err(error) => self.message = format!("Open failed: {error}"),
                    }
                }
            }
            "tabnext" | "tabn" | "tn" => self.next_tab(),
            "tabprev" | "tabprevious" | "tabp" | "tp" => self.previous_tab(),
            "tabfirst" | "tabfir" => {
                let before = self.current_location();
                self.editor.first();
                self.commit_navigation(before);
                self.after_tab_switch();
            }
            "tablast" | "tabl" => {
                let before = self.current_location();
                self.editor.last();
                self.commit_navigation(before);
                self.after_tab_switch();
            }
            "tab" => {
                if let Ok(number) = argument.parse::<usize>() {
                    if number > 0 {
                        self.select_tab_with_history(number - 1);
                    } else {
                        self.message = format!("Tab {number} is not open");
                    }
                } else {
                    self.message = "Usage: :tab 2".to_string();
                }
            }
            "tabclose" | "tabc" | "bd" | "bdelete" => self.close_active_tab(false),
            "tabclose!" | "tabc!" | "bd!" | "bdelete!" => self.close_active_tab(true),
            "tabs" | "buffers" | "ls" => self.message = self.editor.titles_summary(),
            "cd" => {
                if argument.is_empty() {
                    self.message = "Usage: :cd folder".to_string();
                } else {
                    let path = self.resolve_project_path(&argument);
                    self.change_project_root(path);
                }
            }
            "tree" => {
                self.project.visible = !self.project.visible;
                if !self.project.visible {
                    self.explorer_focused = false;
                }
                self.message = if self.project.visible {
                    "File tree shown".to_string()
                } else {
                    "File tree hidden".to_string()
                };
            }
            "newfile" | "touch" => {
                if argument.is_empty() { self.message = "Usage: :newfile path".to_string(); } else { self.create_project_file(&argument); }
            }
            "newdir" | "mkdir" => {
                if argument.is_empty() { self.message = "Usage: :newdir path".to_string(); } else { self.create_project_directory(&argument); }
            }
            "rename" => self.rename_selected_project_entry(&argument),
            "copy" => self.duplicate_selected_project_file(&argument),
            "move" => {
                if argument.is_empty() { self.message = "Usage: :move path".to_string(); } else { self.move_selected_project_entry(&argument); }
            }
            "delete" | "rm" => self.delete_selected_project_entry(false),
            "delete!" | "rm!" => self.delete_selected_project_entry(true),
            "refresh" | "reloadtree" => match self.project.refresh() {
                Ok(()) => { self.project.refresh_git_status(); self.message = "File tree refreshed".to_string() },
                Err(error) => self.message = format!("Refresh failed: {error}"),
            },
            "gitrefresh" => { self.project.refresh_git_status(); self.message = "Git status refreshed".to_string(); },
            "stage" => self.git_selected(true),
            "unstage" => self.git_selected(false),
            "diff" => self.show_git_diff(),
            "history" | "log" => self.show_git_history(),
            "pwd" => self.message = self.project.root.display().to_string(),
            "back" | "previous" => self.go_back(),
            "forward" | "nextlocation" => self.go_forward(),
            "duplicate" | "dup" => {
                self.editor.checkpoint();
                self.editor.duplicate_line();
                self.message = "Duplicated line".to_string();
            }
            "moveup" => {
                self.editor.checkpoint();
                self.message = if self.editor.move_line(false) {
                    "Moved line up".to_string()
                } else {
                    "Line is already first".to_string()
                };
            }
            "movedown" => {
                self.editor.checkpoint();
                self.message = if self.editor.move_line(true) {
                    "Moved line down".to_string()
                } else {
                    "Line is already last".to_string()
                };
            }
            "join" => {
                self.editor.checkpoint();
                self.message = if self.editor.join_line_below() {
                    "Joined lines".to_string()
                } else {
                    "No line below to join".to_string()
                };
            }
            "sort" => {
                self.editor.checkpoint();
                let count = self.editor.sort_selected_lines();
                self.message = format!("Sorted {count} line(s)");
            }
            "indent" => self.indent_selection(false),
            "outdent" => self.indent_selection(true),
            "comment" | "togglecomment" => self.toggle_comments(),
            "symbols" | "outline" => self.show_symbols(),
            "split" => self.open_split(false),
            "vsplit" => self.open_split(true),
            "only" | "unsplit" => { self.split_views = None; self.message = "Split closed".to_string(); }
            "fold" | "foldclose" => self.close_fold(),
            "foldopen" => self.open_fold(),
            "foldtoggle" => self.toggle_fold(),
            "foldall" => self.close_all_folds(),
            "unfoldall" => self.open_all_folds(),
            "expandall" | "treeexpand" => match self.project.expand_all() {
                Ok(count) => self.message = format!("Expanded {count} folder(s)"),
                Err(error) => self.message = format!("Expand all failed: {error}"),
            },
            "collapseall" | "treecollapse" => match self.project.collapse_all() {
                Ok(count) => self.message = format!("Collapsed {count} folder(s)"),
                Err(error) => self.message = format!("Collapse all failed: {error}"),
            },
            "treewidth" => {
                if let Ok(number) = argument.parse::<usize>() {
                    if (22..=80).contains(&number) {
                        self.project.width = number;
                        self.message = format!("Tree width: {number}");
                    } else {
                        self.message = "Tree width must be between 22 and 80".to_string();
                    }
                } else {
                    self.message = "Usage: :treewidth 44".to_string();
                }
            }
            "reveal" => {
                if let Some(path) = self.editor.path.clone() {
                    match self.project.reveal_path(&path) {
                        Ok(true) => {
                            self.project.visible = true;
                            self.explorer_focused = true;
                            self.mode = Mode::Normal;
                            self.message = format!("Revealed {}", path.display());
                        }
                        Ok(false) => {
                            self.message = "Active file is outside the project root".to_string()
                        }
                        Err(error) => self.message = format!("Reveal failed: {error}"),
                    }
                } else {
                    self.message = "The active tab has no file path".to_string();
                }
            }
            "set" => self.execute_set(&argument),
            "theme" => self.execute_theme(&argument),
            "themes" | "themegallery" => self.open_theme_gallery(),
            "keymap" => self.execute_keymap(&argument),
            "keymaps" | "keymapgallery" => self.open_keymap_gallery(),
            "config" => self.message = config::config_path().display().to_string(),
            "lsp" => self.start_lsp(),
            "lspstop" => {
                self.lsp = None;
                self.message = "LSP stopped".to_string();
            }
            "hover" => self.request_hover(),
            "complete" => self.request_lsp_position("textDocument/completion"),
            "definition" | "def" => self.request_lsp_position("textDocument/definition"),
            "references" | "refs" => self.request_lsp_position("textDocument/references"),
            "actions" => self.request_lsp_position("textDocument/codeAction"),
            "format" => self.request_formatting(),
            "help" | "h" => self.mode = Mode::Help,
            _ => self.message = format!("Unknown command: {command}"),
        }
    }

    fn change_project_root(&mut self, path: PathBuf) {
        match self.project.set_root(path) {
            Ok(()) => {
                self.project.visible = true;
                self.explorer_focused = true;
                self.mode = Mode::Normal;
                self.message = format!("Project: {}", self.project.root.display());
            }
            Err(error) => self.message = format!("Cannot open folder: {error}"),
        }
    }

    fn resolve_project_path(&self, value: &str) -> PathBuf {
        if value == "~" {
            return std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.project.root.clone());
        }

        if let Some(rest) = value.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }

        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            self.project.root.join(path)
        }
    }

    fn execute_set(&mut self, argument: &str) {
        match argument {
            "number" | "nu" => {
                self.editor.show_line_numbers = true;
                self.settings.show_line_numbers = true;
                self.persist_settings();
                self.message = "Line numbers enabled".to_string();
            }
            "nonumber" | "nonu" => {
                self.editor.show_line_numbers = false;
                self.settings.show_line_numbers = false;
                self.persist_settings();
                self.message = "Line numbers disabled".to_string();
            }
            "formatonsave" | "fos" => {
                self.settings.format_on_save = true;
                self.persist_settings();
                self.message = "Format on save enabled".to_string();
            }
            "noformatonsave" | "nofos" => {
                self.settings.format_on_save = false;
                self.persist_settings();
                self.message = "Format on save disabled".to_string();
            }
            value if value.starts_with("tabstop=") || value.starts_with("ts=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());

                match number {
                    Some(number @ 1..=16) => {
                        self.editor.tab_width = number;
                        self.settings.tab_width = number;
                        self.persist_settings();
                        self.message = format!("Tab width: {number}");
                    }
                    _ => self.message = "Tab width must be between 1 and 16".to_string(),
                }
            }
            value if value.starts_with("treewidth=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());

                match number {
                    Some(number @ 22..=80) => {
                        self.project.width = number;
                        self.settings.tree_width = number;
                        self.persist_settings();
                        self.message = format!("Tree width: {number}");
                    }
                    _ => self.message = "Tree width must be between 22 and 80".to_string(),
                }
            }
            _ => self.message = "Try :set number, :set formatonsave, or :set tabstop=4".to_string(),
        }
    }

    fn start_lsp(&mut self) {
        let Some(path) = self.editor.path.clone() else {
            self.message = "Save the buffer before starting LSP".to_string();
            return;
        };
        let Some(server) = lsp::server_for_extension(&path) else {
            self.message = "No built-in LSP server for this file type".to_string();
            return;
        };
        let root = lsp_workspace_root(&path).unwrap_or_else(|| self.project.root.clone());
        match LspClient::start(server, &root) {
            Ok(client) => {
                self.lsp = Some(client);
                self.lsp_initialized = false;
                // csharp-ls may keep reporting workspace progress indefinitely
                // for a standalone file. Initialization is sufficient for
                // position and formatting requests, so never block on it.
                self.lsp_workspace_ready = true;
                self.lsp_last_text.clear();
                self.message = format!("Starting LSP: {server}");
            }
            Err(error) => self.message = format!("Could not start {server}: {error}"),
        }
    }

    fn request_hover(&mut self) {
        let Some(lsp) = &mut self.lsp else {
            self.message = "Start LSP first with :lsp".to_string();
            return;
        };
        if !self.lsp_initialized {
            self.message = "LSP is still initializing".to_string();
            return;
        }
        if !self.lsp_workspace_ready {
            self.message = "LSP is still loading the workspace".to_string();
            return;
        }
        let Some(path) = self.editor.path.as_ref() else {
            self.message = "Save the buffer before requesting hover".to_string();
            return;
        };
        match lsp.request("textDocument/hover", json!({
            "textDocument": { "uri": lsp::file_uri(path) },
            "position": { "line": self.editor.cursor.line, "character": self.editor.cursor.column }
        })) {
            Ok(id) => {
                self.lsp_requests.insert(id, "hover".to_string());
                self.message = "Hover requested".to_string();
            }
            Err(error) => self.message = format!("Hover request failed: {error}"),
        }
    }

    fn sync_lsp_document(&mut self) {
        let Some(lsp) = &self.lsp else { return };
        if !self.lsp_initialized {
            return;
        };
        let Some(path) = self.editor.path.as_ref() else {
            return;
        };
        let text = self.editor.text();
        if text == self.lsp_last_text {
            return;
        }
        self.lsp_version += 1;
        let _ = lsp.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": lsp::file_uri(path), "version": self.lsp_version },
                "contentChanges": [{ "text": text }]
            }),
        );
        self.lsp_last_text = self.editor.text();
    }

    fn request_lsp_position(&mut self, method: &str) {
        if !self.lsp_workspace_ready {
            self.message = "LSP is still loading the workspace".to_string();
            return;
        }
        let Some(path) = self.editor.path.as_ref() else {
            self.message = "Save the buffer before using LSP".to_string();
            return;
        };
        let mut params = json!({
            "textDocument": { "uri": lsp::file_uri(path) },
            "position": { "line": self.editor.cursor.line, "character": self.editor.cursor.column }
        });
        if method == "textDocument/references" {
            params["context"] = json!({ "includeDeclaration": true });
        } else if method == "textDocument/codeAction" {
            params["range"] = json!({
                "start": { "line": self.editor.cursor.line, "character": self.editor.cursor.column },
                "end": { "line": self.editor.cursor.line, "character": self.editor.cursor.column }
            });
            params["context"] = json!({ "diagnostics": [] });
        }
        match self
            .lsp
            .as_mut()
            .ok_or_else(|| "Start LSP first with :lsp".to_string())
            .and_then(|client| {
                client
                    .request(method, params)
                    .map_err(|error| error.to_string())
            }) {
            Ok(id) => {
                self.lsp_requests.insert(id, method.to_string());
                self.message = format!("LSP request: {method}");
            }
            Err(error) => self.message = error,
        }
    }

    fn request_formatting(&mut self) {
        let Some(path) = self.editor.path.as_ref() else {
            self.message = "Save the buffer before formatting".to_string();
            return;
        };
        let params = json!({ "textDocument": { "uri": lsp::file_uri(path) }, "options": { "tabSize": self.editor.tab_width, "insertSpaces": true } });
        match self
            .lsp
            .as_mut()
            .ok_or_else(|| "Start LSP first with :lsp".to_string())
            .and_then(|client| {
                client
                    .request("textDocument/formatting", params)
                    .map_err(|error| error.to_string())
            }) {
            Ok(id) => {
                self.lsp_requests
                    .insert(id, "textDocument/formatting".to_string());
                self.message = "LSP formatting requested".to_string();
            }
            Err(error) => self.message = error,
        }
    }

    fn request_formatting_after_save(&mut self) {
        if self.settings.format_on_save {
            self.request_formatting();
        }
    }

    fn poll_lsp(&mut self) {
        let Some(lsp) = &self.lsp else {
            return;
        };
        let Some(message) = lsp.try_recv() else {
            return;
        };
        if message.get("method").and_then(|method| method.as_str()) == Some("$/progress") {
            match message["params"]["value"]["kind"].as_str() {
                Some("begin") => self.message = "LSP loading workspace…".to_string(),
                Some("end") => {
                    self.lsp_workspace_ready = true;
                    self.message = "LSP ready".to_string();
                }
                _ => {}
            }
            return;
        }
        if let (Some(id), Some(method)) = (
            message.get("id"),
            message.get("method").and_then(|method| method.as_str()),
        ) {
            let result = match method {
                "workspace/configuration" => {
                    let count = message["params"]["items"].as_array().map_or(0, Vec::len);
                    json!(vec![json!({}); count])
                }
                "workspace/workspaceFolders" => {
                    let root = self
                        .editor
                        .path
                        .as_deref()
                        .and_then(lsp_workspace_root)
                        .unwrap_or_else(|| self.project.root.clone());
                    json!([{
                        "uri": lsp::file_uri(&root),
                        "name": root.file_name().and_then(|name| name.to_str()).unwrap_or("workspace")
                    }])
                }
                "client/registerCapability"
                | "client/unregisterCapability"
                | "window/workDoneProgress/create" => json!(null),
                _ => json!(null),
            };
            if let Some(lsp) = &self.lsp {
                let _ = lsp.respond(id, result);
            }
            return;
        }
        let request = message
            .get("id")
            .and_then(|id| id.as_u64())
            .and_then(|id| self.lsp_requests.remove(&id));
        if message.get("id").and_then(|id| id.as_u64()) == Some(1) {
            let Some(lsp) = &self.lsp else { return };
            if lsp.notify("initialized", json!({})).is_err() {
                self.message = "LSP initialization failed".to_string();
                return;
            }
            let Some(path) = self.editor.path.as_ref() else {
                return;
            };
            let language_id = lsp_language_id(crate::syntax::Language::from_path(Some(path)));
            if lsp.notify("textDocument/didOpen", json!({ "textDocument": { "uri": lsp::file_uri(path), "languageId": language_id, "version": 1, "text": self.editor.text() } })).is_ok() {
                self.lsp_initialized = true;
                self.lsp_last_text = self.editor.text();
                self.message = "LSP ready".to_string();
            }
            return;
        }
        if request.as_deref() == Some("textDocument/definition") {
            let location = message["result"]
                .as_array()
                .and_then(|locations| locations.first())
                .unwrap_or(&message["result"]);
            let uri = location["uri"]
                .as_str()
                .or_else(|| location["targetUri"].as_str());
            let range = if location["range"].is_object() {
                &location["range"]
            } else {
                &location["targetSelectionRange"]
            };
            if let (Some(uri), Some(line), Some(character)) = (
                uri,
                range["start"]["line"].as_u64(),
                range["start"]["character"].as_u64(),
            ) {
                let path = PathBuf::from(uri.trim_start_matches("file:///").replace("%20", " "));
                match self.editor.open_or_switch(&path) {
                    Ok(_) => {
                        self.editor.goto_line(line as usize);
                        self.editor.cursor.column = character as usize;
                        self.message = format!("Definition: {}", path.display());
                    }
                    Err(error) => self.message = format!("Cannot open definition: {error}"),
                }
                return;
            }
            self.message = "No definition found at cursor".to_string();
            return;
        }
        if request.as_deref() == Some("textDocument/formatting") {
            if self.apply_formatting_edits(&message["result"]) {
                match self.editor.save() {
                    Ok(()) => self.message = "Formatted and saved".to_string(),
                    Err(error) => self.message = format!("Formatted but save failed: {error}"),
                }
            } else {
                self.message = "No formatting changes".to_string();
            }
            return;
        }
        if message.get("method").and_then(|value| value.as_str())
            == Some("textDocument/publishDiagnostics")
        {
            let count = message["params"]["diagnostics"]
                .as_array()
                .map_or(0, Vec::len);
            self.message = format!("LSP diagnostics: {count}");
        } else if let Some(result) = message.get("result") {
            let text = result["contents"]
                .as_str()
                .map(str::to_string)
                .or_else(|| result["contents"]["value"].as_str().map(str::to_string))
                .unwrap_or_else(|| result.to_string());
            self.message = format!(
                "LSP: {}",
                text.replace('\n', " ")
                    .chars()
                    .take(120)
                    .collect::<String>()
            );
        }
    }

    fn apply_formatting_edits(&mut self, result: &serde_json::Value) -> bool {
        let Some(edits) = result.as_array() else { return false; };
        let mut text = self.editor.text();
        let mut changes = Vec::new();
        for edit in edits {
            let Some(replacement) = edit["newText"].as_str() else { continue; };
            let range = &edit["range"];
            let Some(start_line) = range["start"]["line"].as_u64() else { continue; };
            let Some(start_character) = range["start"]["character"].as_u64() else { continue; };
            let Some(end_line) = range["end"]["line"].as_u64() else { continue; };
            let Some(end_character) = range["end"]["character"].as_u64() else { continue; };
            let Some(start) = lsp_text_offset(&text, start_line as usize, start_character as usize) else { continue; };
            let Some(end) = lsp_text_offset(&text, end_line as usize, end_character as usize) else { continue; };
            changes.push((start, end, replacement.to_string()));
        }
        changes.sort_by_key(|(start, _, _)| *start);
        if changes.is_empty() { return false; }
        for (start, end, replacement) in changes.into_iter().rev() {
            if start <= end && end <= text.len() { text.replace_range(start..end, &replacement); }
        }
        if text == self.editor.text() { return false; }
        self.editor.replace_text(&text);
        true
    }

    fn execute_theme(&mut self, argument: &str) {
        let kind = match argument {
            "oxide" | "" => ThemeKind::Oxide,
            "mono" | "monochrome" => ThemeKind::Mono,
            "nord" => ThemeKind::Nord,
            "dracula" => ThemeKind::Dracula,
            "solarized" | "solarized-dark" => ThemeKind::Solarized,
            "gallery" => { self.open_theme_gallery(); return; }
            _ => {
                self.message = "Themes: oxide, nord, dracula, solarized, mono, gallery".to_string();
                return;
            }
        };

        self.theme_kind = kind;
        self.theme = Theme::for_kind(kind);
        self.settings.theme = kind;
        self.persist_settings();
        self.message = format!("Theme: {argument}");
    }

    fn open_theme_gallery(&mut self) {
        self.theme_gallery_original = self.theme_kind;
        self.theme_gallery_selected = ThemeKind::ALL.iter().position(|kind| *kind == self.theme_kind).unwrap_or(0);
        self.mode = Mode::ThemeGallery;
        self.preview_gallery_theme();
    }

    fn preview_gallery_theme(&mut self) {
        self.theme_kind = ThemeKind::ALL[self.theme_gallery_selected];
        self.theme = Theme::for_kind(self.theme_kind);
    }

    pub fn keymap_profile(&self) -> KeymapProfile { self.settings.keymap }

    fn preferred_editor_mode(&self) -> Mode {
        match self.settings.keymap {
            KeymapProfile::Vim => Mode::Normal,
            KeymapProfile::Caret | KeymapProfile::Conventional => Mode::Insert,
        }
    }

    fn open_keymap_gallery(&mut self) {
        self.keymap_gallery_selected = KeymapProfile::ALL
            .iter()
            .position(|profile| *profile == self.settings.keymap)
            .unwrap_or(0);
        self.mode = Mode::KeymapGallery;
        self.message = "Choose a keymap · click or press Enter to apply".to_string();
    }

    fn apply_selected_keymap(&mut self) {
        self.settings.keymap = KeymapProfile::ALL[self.keymap_gallery_selected];
        self.persist_settings();
        self.pending_key = None;
        self.macro_prefix = None;
        self.mode = self.preferred_editor_mode();
        self.message = format!("Keymap: {}", self.settings.keymap.name());
    }

    fn execute_keymap(&mut self, argument: &str) {
        if argument.is_empty() || argument == "gallery" {
            self.open_keymap_gallery();
            return;
        }
        let profile = match argument.to_ascii_lowercase().as_str() {
            "caret" | "default" => KeymapProfile::Caret,
            "vim" => KeymapProfile::Vim,
            "conventional" | "standard" | "classic" => KeymapProfile::Conventional,
            _ => {
                self.message = "Keymaps: caret, vim, conventional".to_string();
                return;
            }
        };
        self.settings.keymap = profile;
        self.keymap_gallery_selected = KeymapProfile::ALL
            .iter()
            .position(|candidate| *candidate == profile)
            .unwrap_or(0);
        self.persist_settings();
        self.mode = self.preferred_editor_mode();
        self.message = format!("Keymap: {}", profile.name());
    }

    fn persist_settings(&mut self) {
        if let Err(error) = config::save(&self.settings) {
            self.message = format!("Could not save settings: {error}");
        }
    }

    fn save_all(&mut self) {
        self.save_all_internal();
    }

    fn save_all_internal(&mut self) -> bool {
        let (saved, errors) = self.editor.save_all();
        let _ = self.project.refresh();

        if errors.is_empty() {
            self.message = if saved == 0 {
                "All named tabs are already saved".to_string()
            } else {
                format!("Saved {saved} tab(s)")
            };
            true
        } else {
            self.message = format!(
                "Saved {saved}; {} tab(s) failed: {}",
                errors.len(),
                errors.join("; ")
            );
            false
        }
    }

    fn save(&mut self) {
        self.save_internal();
    }

    fn save_as(&mut self, path: PathBuf) {
        match self.editor.save_as(&path) {
            Ok(()) => {
                let _ = self.project.refresh();
                self.message = format!("Saved {}", path.display());
                self.request_formatting_after_save();
            }
            Err(error) => self.message = format!("Save failed: {error}"),
        }
    }

    fn save_internal(&mut self) -> bool {
        if self.editor.has_pending_external_change() {
            self.pending_save_after_disk_change = true;
            self.mode = Mode::ReloadConfirm;
            self.message = "Disk changes were kept — [R] Reload   [K] Overwrite with current buffer   [Esc] Cancel".to_string();
            return false;
        }
        match self.editor.save() {
            Ok(()) => {
                let _ = self.project.refresh();
                let name = self
                    .editor
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                self.message = format!("Saved {name}");
                self.request_formatting_after_save();
                true
            }
            Err(error) => {
                self.message = format!("Save failed: {error}");
                false
            }
        }
    }

    fn request_quit(&mut self, force: bool) {
        if force || !self.editor.any_dirty() {
            self.should_quit = true;
            return;
        }

        let dirty = self.editor.dirty_titles();
        self.explorer_focused = false;
        self.mode = Mode::QuitConfirm;
        self.message = format!("{} unsaved tab(s): {}", dirty.len(), dirty.join(", "));
    }
}

fn lsp_workspace_root(path: &Path) -> Option<PathBuf> {
    let mut directory = path.parent()?.to_path_buf();
    loop {
        let has_workspace_file = std::fs::read_dir(&directory)
            .ok()?
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(extension.to_ascii_lowercase().as_str(), "sln" | "csproj")
                    })
            });
        if has_workspace_file {
            return Some(directory);
        }
        let parent = directory.parent()?.to_path_buf();
        if parent == directory {
            return None;
        }
        directory = parent;
    }
}

fn lsp_text_offset(text: &str, line: usize, character: usize) -> Option<usize> {
    let line_start = if line == 0 { 0 } else { text.match_indices('\n').nth(line - 1).map(|(index, _)| index + 1)? };
    let line_end = text[line_start..].find('\n').map_or(text.len(), |offset| line_start + offset);
    let prefix = &text[line_start..line_end];
    let byte_offset = prefix.char_indices().nth(character).map_or(prefix.len(), |(offset, _)| offset);
    Some(line_start + byte_offset)
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].char_indices().last().map_or(0, |(offset, _)| offset)
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    text[index.min(text.len())..].chars().next().map_or(text.len(), |character| index + character.len_utf8())
}

fn lsp_language_id(language: crate::syntax::Language) -> &'static str {
    match language {
        crate::syntax::Language::CSharp => "csharp",
        crate::syntax::Language::Go => "go",
        crate::syntax::Language::Rust => "rust",
        crate::syntax::Language::Python => "python",
        crate::syntax::Language::Json => "json",
        crate::syntax::Language::Yaml => "yaml",
        crate::syntax::Language::Toml => "toml",
        crate::syntax::Language::Shell => "shellscript",
        crate::syntax::Language::Markdown => "markdown",
        crate::syntax::Language::Plain => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
    }

    #[test]
    fn records_and_replays_a_macro() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Normal;

        for key in [
            key('q'),
            key('a'),
            key('i'),
            key('x'),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            key('q'),
        ] {
            app.handle_key(key);
        }
        assert_eq!(app.editor.line_text(0), "x");

        app.handle_key(key('@'));
        app.handle_key(key('a'));
        assert_eq!(app.editor.line_text(0), "xx");
        assert_eq!(app.recording_macro, None);
    }

    #[test]
    fn keyboard_input_resumes_cursor_following_after_viewport_scroll() {
        let mut app = App::new(None).expect("create app");
        app.follow_cursor = false;

        app.handle_event(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));

        assert!(app.follow_cursor);
    }

    #[test]
    fn conventional_profile_never_turns_printable_input_into_normal_commands() {
        let mut app = App::new(None).expect("create app");
        app.settings.keymap = KeymapProfile::Conventional;
        app.mode = Mode::Normal;

        app.handle_key(key('i'));

        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.editor.line_text(0), "i");
    }

    #[test]
    fn vim_profile_prefers_normal_mode() {
        let mut app = App::new(None).expect("create app");
        app.settings.keymap = KeymapProfile::Vim;

        assert_eq!(app.preferred_editor_mode(), Mode::Normal);
    }

    #[test]
    fn ctrl_shift_p_opens_command_palette() {
        let mut app = App::new(None).expect("create app");
        app.handle_key(KeyEvent::new(
            KeyCode::Char('P'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));

        assert_eq!(app.mode, Mode::Command);
    }
}
