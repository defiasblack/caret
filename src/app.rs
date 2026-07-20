use std::{
    collections::HashMap,
    fs, io,
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
use serde_json::{json, Value};

use crate::{
    config::{self, KeymapProfile, Settings, StartupView},
    document::FinalNewline,
    editor::{Cursor, EditorOptions, ReplaceOutcome},
    keys::{Action as KeyAction, KeyBindings},
    lsp::{self, LspClient},
    plugin::{PluginContext, PluginRegistry, PluginResponse},
    project::ProjectTree,
    search::{CompiledSearch, SearchOptions},
    syntax::Language,
    tabs::{OpenDisposition, Tabs},
    terminal::TerminalPane,
    theme::{Theme, ThemeKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    ProjectSearch,
    FilePicker,
    KeyBrowser,
    Command,
    Help,
    QuitConfirm,
    TabCloseConfirm,
    ReloadConfirm,
    GitDiff,
    GitHistory,
    ThemeGallery,
    KeymapGallery,
    ContextMenu,
    Dashboard,
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
pub enum SidebarView {
    Files,
    Outline,
}

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
            Self::ProjectSearch => "PROJECT FIND",
            Self::FilePicker => "OPEN FILE",
            Self::KeyBrowser => "KEYS",
            Self::Command => "COMMAND",
            Self::Help => "HELP",
            Self::QuitConfirm => "QUIT?",
            Self::TabCloseConfirm => "CLOSE TAB?",
            Self::ReloadConfirm => "RELOAD?",
            Self::GitDiff => "DIFF",
            Self::GitHistory => "HISTORY",
            Self::ThemeGallery => "THEMES",
            Self::KeymapGallery => "KEYMAPS",
            Self::ContextMenu => "MENU",
            Self::Dashboard => "WELCOME",
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
pub struct GitHistoryEntry {
    pub hash: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitLineChange {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundState {
    Working,
    Ready,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LspStatus {
    Off,
    Starting,
    Loading,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LspPanelKind {
    Completion,
    Hover,
    Locations,
    CodeActions,
    Diagnostics,
}

#[derive(Debug, Clone)]
pub struct LspPanelItem {
    pub label: String,
    pub detail: String,
    payload: Value,
}

#[derive(Debug, Clone)]
pub struct LspPanel {
    pub title: String,
    pub items: Vec<LspPanelItem>,
    pub selected: usize,
    kind: LspPanelKind,
}

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

/// One row in the fuzzy file picker.
#[derive(Debug, Clone)]
pub struct PickerMatch {
    pub file_index: usize,
    /// Char positions inside the path that matched the query.
    pub positions: Vec<usize>,
    pub recent: bool,
}

/// State for the Ctrl-P fuzzy file picker.
#[derive(Debug, Clone, Default)]
pub struct FilePickerState {
    pub input: String,
    /// Project-relative paths of every candidate file.
    pub files: Vec<String>,
    pub matches: Vec<PickerMatch>,
    pub selected: usize,
    pub scroll: usize,
    pub truncated: bool,
}

/// State for the project-wide search and replace panel.
#[derive(Debug, Clone, Default)]
pub struct ProjectSearchState {
    pub query: String,
    pub replacement: String,
    pub focus_replace: bool,
    pub results: Vec<crate::project_search::ProjectMatch>,
    /// Indices into `results` the user excluded from replacement.
    pub excluded: std::collections::HashSet<usize>,
    pub selected: usize,
    pub scroll: usize,
    pub truncated: bool,
    pub files_with_matches: usize,
    pub error: Option<String>,
    /// The query text of the most recent completed search run.
    pub ran_query: String,
    /// Set after the first Alt-A; the second Alt-A applies the replacement.
    pub confirm_replace: bool,
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
    pub command_suggestion_scroll: usize,
    command_suggestion_hover_lock_until: Option<Instant>,
    theme_gallery_hover_lock_until: Option<Instant>,
    command_selection: Option<(usize, usize)>,
    command_cursor: usize,
    command_anchor: Option<usize>,
    pub search_input: String,
    pub last_search: String,
    pub search_replace_input: String,
    pub search_focus_replace: bool,
    pub search_options: SearchOptions,
    search_scope: Option<(usize, usize)>,
    search_scoped: bool,
    pub search_error: Option<String>,
    active_search: Option<CompiledSearch>,
    search_history: Vec<String>,
    search_history_index: Option<usize>,
    pub project_search: ProjectSearchState,
    pub file_picker: FilePickerState,
    tree_filter_active: bool,
    keys: KeyBindings,
    pub key_browser_input: String,
    pub key_browser_scroll: usize,
    pub message: String,
    pub theme_kind: ThemeKind,
    pub theme: Theme,
    pub viewport_rows: usize,
    pub viewport_columns: usize,
    pub follow_cursor: bool,
    pub split_views: Option<SplitViews>,
    pub help_page: usize,
    help_return_mode: Mode,
    pub hover_target: Option<HoverTarget>,
    settings: Settings,
    plugins: PluginRegistry,
    active_custom_theme: Option<String>,
    pending_key: Option<char>,
    macro_prefix: Option<MacroPrefix>,
    recording_macro: Option<char>,
    macros: HashMap<char, Vec<KeyEvent>>,
    replaying_macro: bool,
    yank: String,
    search_origin: Cursor,
    column_select_origin: Option<Cursor>,
    back_history: Vec<NavigationLocation>,
    forward_history: Vec<NavigationLocation>,
    last_editor_click: Option<(Instant, Cursor)>,
    last_project_click: Option<(Instant, PathBuf)>,
    lsp: Option<LspClient>,
    lsp_version: i64,
    lsp_requests: HashMap<u64, String>,
    lsp_initialized: bool,
    lsp_workspace_ready: bool,
    lsp_status: LspStatus,
    lsp_started_at: Option<Instant>,
    diagnostic_count: usize,
    diagnostics: Vec<Value>,
    pub lsp_panel: Option<LspPanel>,
    background_tick: usize,
    last_background_animation: Instant,
    observed_message: String,
    message_changed_at: Instant,
    lsp_last_text: String,
    lsp_open_path: Option<PathBuf>,
    pending_save_after_disk_change: bool,
    pub git_diff_lines: Vec<String>,
    pub git_diff_scroll: usize,
    pub git_diff_title: String,
    git_diff_return_mode: Mode,
    pub git_history: Vec<GitHistoryEntry>,
    pub git_history_selected: usize,
    git_line_changes: HashMap<usize, GitLineChange>,
    pub theme_gallery_selected: usize,
    theme_gallery_original: ThemeKind,
    theme_gallery_original_theme: Theme,
    theme_gallery_original_custom: Option<String>,
    theme_gallery_return_mode: Mode,
    pub keymap_gallery_selected: usize,
    keymap_gallery_return_mode: Mode,
    pub context_menu: Option<ContextMenu>,
    context_menu_previous_mode: Mode,
    pub dashboard_selected: usize,
    pub dashboard_hover: Option<usize>,
    terminal: Option<TerminalPane>,
    pub terminal_focused: bool,
    last_recovery_checkpoint: Instant,
    last_session_checkpoint: Instant,
}

impl App {
    fn editor_options(settings: &Settings) -> EditorOptions {
        EditorOptions {
            show_line_numbers: settings.show_line_numbers,
            tab_width: settings.tab_width,
            auto_indent: settings.auto_indent,
            trim_on_save: settings.trim_trailing_whitespace_on_save,
            final_newline: settings.final_newline,
            history_limit: settings.undo_history_limit,
        }
    }

    /// Reapplies editor-level settings to every open tab after `:set`.
    fn apply_editor_settings(&mut self) {
        let options = Self::editor_options(&self.settings);
        self.editor
            .for_each_editor(|editor| editor.apply_options(options));
    }

    pub fn new(path: Option<&Path>) -> io::Result<Self> {
        let (settings, mut config_message) = config::load();
        let keys = KeyBindings::from_custom(&settings.custom_keys);
        if !keys.warnings.is_empty() {
            config_message = Some(keys.warnings.join(" · "));
        }
        let restored_session = if !cfg!(test)
            && settings.restore_session
            && settings.startup == StartupView::Session
            && path.is_none()
        {
            match crate::session::load() {
                Ok(session) => session,
                Err(error) => {
                    config_message = Some(format!("Session ignored: {error}"));
                    None
                }
            }
        } else {
            None
        };
        let plugins = PluginRegistry::load(&config::plugins_dir());
        let current_dir = std::env::current_dir()?;

        let (editor_path, project_root, explorer_focused, sidebar_visible): (
            Option<PathBuf>,
            PathBuf,
            bool,
            bool,
        ) = match path {
            // Opened on a folder: show and focus the file tree.
            Some(path) if path.is_dir() => (None, path.to_path_buf(), true, true),
            // Opened on a file: focus the document with the tree hidden.
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
                (Some(file_path), root, false, false)
            }
            None => match restored_session.as_ref() {
                // Sidebar visibility is restored from the saved session below.
                Some(session) => (None, session.project_root.clone(), false, true),
                None => {
                    // No path: the startup setting decides. Folder/Session open the
                    // tree focused; Dashboard keeps it available once dismissed;
                    // Empty starts on a bare buffer with the tree hidden.
                    let focus_tree =
                        matches!(settings.startup, StartupView::Folder | StartupView::Session);
                    let show_tree = settings.startup != StartupView::Empty;
                    (None, current_dir, focus_tree, show_tree)
                }
            },
        };

        let mut editor = match restored_session.as_ref() {
            Some(session) => Tabs::from_session(&session.tabs, session.active_tab)?
                .unwrap_or(Tabs::new(editor_path.as_deref())?),
            None => Tabs::new(editor_path.as_deref())?,
        };
        let editor_options = Self::editor_options(&settings);
        editor.for_each_editor(|editor| editor.apply_options(editor_options));
        editor.checkpoint();

        let mut project = ProjectTree::new(project_root)?;
        project.visible = sidebar_visible;
        project.width = settings.tree_width.clamp(22, 80);
        project.show_hidden = settings.show_hidden_files;
        let _ = project.refresh();
        project.refresh_git_status();
        let show_dashboard = path.is_none() && settings.startup == StartupView::Dashboard;
        let mode = if show_dashboard {
            Mode::Dashboard
        } else if explorer_focused || settings.keymap == KeymapProfile::Vim {
            Mode::Normal
        } else {
            Mode::Insert
        };
        let initial_theme = if std::env::var_os("NO_COLOR").is_some() {
            ThemeKind::Mono
        } else {
            settings.theme
        };
        let custom_theme = if std::env::var_os("NO_COLOR").is_none() {
            settings
                .custom_theme
                .as_ref()
                .and_then(|name| plugins.theme(name).map(|theme| (name.clone(), theme)))
        } else {
            None
        };
        let initial_rendered_theme = custom_theme
            .as_ref()
            .map_or_else(|| Theme::for_kind(initial_theme), |(_, theme)| *theme);
        let initial_custom_name = custom_theme.as_ref().map(|(name, _)| name.clone());

        let mut app = Self {
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
            command_suggestion_scroll: 0,
            command_suggestion_hover_lock_until: None,
            theme_gallery_hover_lock_until: None,
            command_selection: None,
            command_cursor: 0,
            command_anchor: None,
            search_input: String::new(),
            last_search: String::new(),
            search_replace_input: String::new(),
            search_focus_replace: false,
            search_options: SearchOptions::default(),
            search_scope: None,
            search_scoped: false,
            search_error: None,
            active_search: None,
            search_history: Vec::new(),
            search_history_index: None,
            project_search: ProjectSearchState::default(),
            file_picker: FilePickerState::default(),
            tree_filter_active: false,
            keys,
            key_browser_input: String::new(),
            key_browser_scroll: 0,
            message: config_message.unwrap_or_else(|| {
                if show_dashboard {
                    return "Welcome to Caret · choose a recent project or open the current folder"
                        .to_string();
                }
                if explorer_focused {
                    "FILES · Enter opens · Ctrl-E returns to editor".to_string()
                } else {
                    "-- INSERT -- · Ctrl-E opens files · Ctrl-S saves".to_string()
                }
            }),
            theme_kind: initial_theme,
            theme: initial_rendered_theme,
            viewport_rows: 1,
            viewport_columns: 1,
            follow_cursor: true,
            split_views: None,
            help_page: 0,
            help_return_mode: mode,
            hover_target: None,
            settings,
            plugins,
            active_custom_theme: initial_custom_name.clone(),
            pending_key: None,
            macro_prefix: None,
            recording_macro: None,
            macros: HashMap::new(),
            replaying_macro: false,
            yank: String::new(),
            search_origin: Cursor::default(),
            column_select_origin: None,
            back_history: Vec::new(),
            forward_history: Vec::new(),
            last_editor_click: None,
            last_project_click: None,
            lsp: None,
            lsp_version: 1,
            lsp_requests: HashMap::new(),
            lsp_initialized: false,
            lsp_workspace_ready: false,
            lsp_status: LspStatus::Off,
            lsp_started_at: None,
            diagnostic_count: 0,
            diagnostics: Vec::new(),
            lsp_panel: None,
            background_tick: 0,
            last_background_animation: Instant::now(),
            observed_message: String::new(),
            message_changed_at: Instant::now(),
            lsp_last_text: String::new(),
            lsp_open_path: None,
            pending_save_after_disk_change: false,
            git_diff_lines: Vec::new(),
            git_diff_scroll: 0,
            git_diff_title: "GIT DIFF".to_string(),
            git_diff_return_mode: mode,
            git_history: Vec::new(),
            git_history_selected: 0,
            git_line_changes: HashMap::new(),
            theme_gallery_selected: 0,
            theme_gallery_original: initial_theme,
            theme_gallery_original_theme: initial_rendered_theme,
            theme_gallery_original_custom: initial_custom_name,
            theme_gallery_return_mode: mode,
            keymap_gallery_selected: 0,
            keymap_gallery_return_mode: mode,
            context_menu: None,
            context_menu_previous_mode: mode,
            dashboard_selected: 0,
            dashboard_hover: None,
            terminal: None,
            terminal_focused: false,
            last_recovery_checkpoint: Instant::now(),
            last_session_checkpoint: Instant::now(),
        };
        if let Some(session) = restored_session {
            app.project.visible = session.sidebar_visible;
            app.sidebar_view = if session.sidebar_outline {
                SidebarView::Outline
            } else {
                SidebarView::Files
            };
            if !session.tabs.is_empty() {
                app.message = format!(
                    "Restored {} tab(s) from the previous session",
                    app.editor.len()
                );
            }
            app.split_views = session.split.and_then(|split| {
                let view = |state: crate::session::ViewState| EditorView {
                    tab_index: state.tab_index,
                    cursor: state.cursor.into(),
                    scroll_line: state.scroll_line,
                    scroll_column: state.scroll_column,
                };
                (split.primary.tab_index < app.editor.len()
                    && split.secondary.tab_index < app.editor.len())
                .then(|| SplitViews {
                    primary: view(split.primary),
                    secondary: view(split.secondary),
                    secondary_active: split.secondary_active,
                    vertical: split.vertical,
                })
            });
        }
        if let Ok(entries) = crate::recovery::load() {
            if !entries.is_empty() {
                let summary = entries
                    .iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        format!(
                            "{}:{}@{}",
                            index + 1,
                            entry
                                .path
                                .as_ref()
                                .and_then(|path| path.file_name())
                                .and_then(|name| name.to_str())
                                .unwrap_or("untitled"),
                            entry.saved_unix_secs
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                app.message = format!(
                    "Recovery: {summary} — :recover N, :recovercompare N, or :discardrecovery"
                );
            }
        }
        if path.is_some() {
            app.remember_current_project();
        }
        // Opening an explicit file is an editor action. Keep the project tree
        // visible, but never leave it focused after startup.
        if editor_path.is_some() {
            app.explorer_focused = false;
            app.mode = app.preferred_editor_mode();
        }
        app.refresh_git_line_changes();
        Ok(app)
    }

    pub fn handle_event(&mut self, event: Event) -> bool {
        self.poll_lsp();
        if self.last_recovery_checkpoint.elapsed() >= Duration::from_secs(2) {
            let entries = self.editor.dirty_recovery_entries();
            if entries.is_empty() {
                let _ = crate::recovery::discard();
            } else if let Err(error) = crate::recovery::save(entries) {
                self.message = format!("Recovery checkpoint failed: {error}");
            }
            self.last_recovery_checkpoint = Instant::now();
        }
        if self.settings.restore_session
            && self.last_session_checkpoint.elapsed() >= Duration::from_secs(2)
        {
            let session = crate::session::SessionState {
                project_root: self.project.root.clone(),
                tabs: self.editor.session_tabs(),
                active_tab: self.editor.active_index(),
                sidebar_visible: self.project.visible,
                sidebar_outline: self.sidebar_view == SidebarView::Outline,
                split: self.split_views.map(|split| crate::session::SplitState {
                    primary: crate::session::ViewState {
                        tab_index: split.primary.tab_index,
                        cursor: split.primary.cursor.into(),
                        scroll_line: split.primary.scroll_line,
                        scroll_column: split.primary.scroll_column,
                    },
                    secondary: crate::session::ViewState {
                        tab_index: split.secondary.tab_index,
                        cursor: split.secondary.cursor.into(),
                        scroll_line: split.secondary.scroll_line,
                        scroll_column: split.secondary.scroll_column,
                    },
                    secondary_active: split.secondary_active,
                    vertical: split.vertical,
                }),
            };
            if let Err(error) = crate::session::save(&session) {
                self.message = format!("Session checkpoint failed: {error}");
            }
            self.last_session_checkpoint = Instant::now();
        }
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
                if self.mode == Mode::Command {
                    if let Ok((width, height)) = terminal::size() {
                        if let Some(index) = crate::ui::command_suggestion_at(
                            self,
                            width,
                            height,
                            mouse.column,
                            mouse.row,
                        ) {
                            if self.command_suggestion_hover_is_locked() {
                                return false;
                            }
                            if self.command_suggestion != index {
                                self.command_suggestion = index;
                                return true;
                            }
                        }
                    }
                }
                if self.lsp_panel.is_some() {
                    if let Ok((width, height)) = terminal::size() {
                        if let Some(index) = crate::ui::lsp_panel_item_at(
                            self,
                            width,
                            height,
                            mouse.column,
                            mouse.row,
                        ) {
                            if let Some(panel) = self.lsp_panel.as_mut() {
                                if panel.selected != index {
                                    panel.selected = index;
                                    return true;
                                }
                            }
                        }
                    }
                }
                if self.mode == Mode::Dashboard {
                    if let Ok((width, height)) = terminal::size() {
                        let hit = crate::ui::dashboard_hit_at(
                            self,
                            width,
                            height,
                            mouse.column,
                            mouse.row,
                        );
                        if hit != self.dashboard_hover {
                            self.dashboard_hover = hit;
                            if let Some(index) =
                                hit.filter(|index| *index < self.settings.recent_projects.len())
                            {
                                self.dashboard_selected = index;
                            }
                            return true;
                        }
                    }
                }
                if self.mode == Mode::ContextMenu {
                    if let Ok((width, height)) = terminal::size() {
                        if let Some(index) = crate::ui::context_menu_action_at(
                            self,
                            width,
                            height,
                            mouse.column,
                            mouse.row,
                        ) {
                            if let Some(menu) = self.context_menu.as_mut() {
                                if menu.selected != index {
                                    menu.selected = index;
                                    return true;
                                }
                            }
                        }
                    }
                }
                if self.mode == Mode::ThemeGallery {
                    if self.theme_gallery_hover_is_locked() {
                        return false;
                    }
                    if let Ok((width, height)) = terminal::size() {
                        if let Some(index) = crate::ui::theme_gallery_item_at(
                            self,
                            width,
                            height,
                            mouse.column,
                            mouse.row,
                        ) {
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
                        if let Some(index) = crate::ui::keymap_gallery_item_at(
                            self,
                            width,
                            height,
                            mouse.column,
                            mouse.row,
                        ) {
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
        let mut changed = false;
        if let Some(terminal) = self.terminal.as_mut() {
            changed |= terminal.poll();
        }
        self.poll_lsp();
        if self.mode != Mode::ReloadConfirm && self.editor.changed_on_disk() {
            self.mode = Mode::ReloadConfirm;
            self.message =
                "File changed on disk — [R] Reload   [K] Keep buffer   [C] Compare   [Esc] Later"
                    .to_string();
        }
        if self.message != self.observed_message {
            self.observed_message = self.message.clone();
            self.message_changed_at = Instant::now();
            changed = true;
        } else if matches!(self.mode, Mode::Normal | Mode::Insert)
            && !self.message.is_empty()
            && self.message_changed_at.elapsed() >= Duration::from_secs(7)
        {
            self.message.clear();
            self.observed_message.clear();
            changed = true;
        }
        if matches!(self.lsp_status, LspStatus::Starting | LspStatus::Loading)
            && !self.reduced_motion()
            && self.last_background_animation.elapsed() >= Duration::from_millis(200)
        {
            self.background_tick = self.background_tick.wrapping_add(1);
            self.last_background_animation = Instant::now();
            changed = true;
        }
        changed || self.message != message
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        if matches!(self.mode, Mode::QuitConfirm | Mode::TabCloseConfirm) {
            return;
        }

        let Ok((width, height)) = terminal::size() else {
            return;
        };

        if width < 44 || height < 8 {
            return;
        }

        if self.mode == Mode::Command
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
            && crate::ui::command_suggestion_at(self, width, height, mouse.column, mouse.row)
                .is_some()
        {
            self.move_command_suggestion(if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                -1
            } else {
                1
            });
            self.command_suggestion_hover_lock_until =
                Some(Instant::now() + Duration::from_millis(500));
            return;
        }

        if self.mode == Mode::ThemeGallery
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
            && crate::ui::theme_gallery_contains(self, width, height, mouse.column, mouse.row)
        {
            self.theme_gallery_selected = if mouse.kind == MouseEventKind::ScrollUp {
                self.theme_gallery_selected.saturating_sub(1)
            } else {
                (self.theme_gallery_selected + 1).min(ThemeKind::ALL.len() - 1)
            };
            self.preview_gallery_theme();
            self.theme_gallery_hover_lock_until = Some(Instant::now() + Duration::from_millis(500));
            return;
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if self.lsp_panel.is_some() {
                if let Some(index) =
                    crate::ui::lsp_panel_item_at(self, width, height, mouse.column, mouse.row)
                {
                    if let Some(panel) = self.lsp_panel.as_mut() {
                        panel.selected = index;
                    }
                    self.activate_lsp_panel_item();
                } else {
                    self.lsp_panel = None;
                }
                return;
            }
            if self.mode == Mode::Dashboard {
                if let Some(hit) =
                    crate::ui::dashboard_hit_at(self, width, height, mouse.column, mouse.row)
                {
                    if hit < self.settings.recent_projects.len() {
                        self.dashboard_selected = hit;
                        self.open_selected_recent_project();
                    } else if hit == self.settings.recent_projects.len() {
                        self.open_current_folder_from_dashboard();
                    } else {
                        self.command_input = "e ".to_string();
                        self.command_cursor = self.command_input.len();
                        self.command_selection = None;
                        self.command_anchor = None;
                        self.mode = Mode::Command;
                        self.message = "Enter a file or folder path".to_string();
                    }
                    return;
                }
                if mouse.row != 0 {
                    return;
                }
            }
            if self.mode == Mode::ContextMenu {
                if let Some(index) =
                    crate::ui::context_menu_action_at(self, width, height, mouse.column, mouse.row)
                {
                    if let Some(menu) = self.context_menu.as_mut() {
                        menu.selected = index;
                    }
                    self.execute_context_action();
                } else {
                    self.close_context_menu();
                }
                return;
            }
            if self.mode == Mode::ThemeGallery {
                if let Some(index) =
                    crate::ui::theme_gallery_item_at(self, width, height, mouse.column, mouse.row)
                {
                    self.theme_gallery_selected = index;
                    self.preview_gallery_theme();
                    self.settings.theme = self.theme_kind;
                    self.settings.custom_theme = None;
                    self.active_custom_theme = None;
                    self.persist_settings();
                    self.mode = self.theme_gallery_return_mode;
                    self.message = format!("Theme: {}", self.theme_kind.name());
                }
                return;
            }
            if self.mode == Mode::KeymapGallery {
                if let Some(index) =
                    crate::ui::keymap_gallery_item_at(self, width, height, mouse.column, mouse.row)
                {
                    self.keymap_gallery_selected = index;
                    self.apply_selected_keymap();
                }
                return;
            }
            if self.mode == Mode::ProjectSearch {
                if let Some(index) = crate::ui::project_search_result_at(
                    self,
                    width,
                    height,
                    mouse.column,
                    mouse.row,
                ) {
                    if self.project_search.selected == index {
                        self.open_selected_project_match();
                    } else {
                        self.project_search.selected = index;
                    }
                }
                return;
            }
            if let Some(index) =
                crate::ui::command_suggestion_at(self, width, height, mouse.column, mouse.row)
            {
                if let Some(command) = self.command_suggestions().get(index) {
                    self.command_input = command.clone();
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
        let over_terminal = layout.terminal_height > 0
            && mouse.row >= layout.terminal_top.saturating_sub(1)
            && mouse.row
                < layout
                    .terminal_top
                    .saturating_add(layout.terminal_height as u16);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if over_terminal {
                    if let Some(terminal) = self.terminal.as_mut() {
                        terminal.scroll_up(3);
                    }
                } else if over_sidebar {
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
                if over_terminal {
                    if let Some(terminal) = self.terminal.as_mut() {
                        terminal.scroll_down(3);
                    }
                } else if over_sidebar {
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
            MouseEventKind::Down(MouseButton::Right) => {
                self.open_context_menu(mouse.column, mouse.row, width, height)
            }
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

    fn open_context_menu(&mut self, column: u16, row: u16, width: u16, height: u16) {
        if !matches!(self.mode, Mode::Normal | Mode::Insert | Mode::ContextMenu) {
            return;
        }
        let layout = crate::ui::screen_layout(self, width, height);
        let mut actions = Vec::new();
        let return_mode;

        if row == 1 {
            let Some(index) = crate::ui::tab_index_at(self, width, column) else {
                return;
            };
            self.select_tab_with_history(index);
            self.explorer_focused = false;
            return_mode = self.preferred_editor_mode();
            actions.extend([ContextAction::SaveTab, ContextAction::CloseTab]);
        } else if layout.sidebar_width > 0
            && (column as usize) < layout.sidebar_width
            && row >= layout.content_top
            && (row as usize) < layout.content_top as usize + layout.content_height
        {
            let screen_row = (row - layout.content_top) as usize;
            if screen_row == 0 {
                return;
            }
            let index = self.project.scroll + screen_row - 1;
            let Some(entry) = self.project.entries.get(index) else {
                return;
            };
            let is_dir = entry.is_dir;
            self.project.selected = index;
            self.explorer_focused = true;
            return_mode = Mode::Normal;
            actions.push(ContextAction::Open);
            if is_dir {
                actions.extend([ContextAction::NewFile, ContextAction::NewFolder]);
            } else {
                actions.extend([
                    ContextAction::Duplicate,
                    ContextAction::Stage,
                    ContextAction::Unstage,
                ]);
            }
            actions.push(ContextAction::Rename);
        } else {
            let editor_end = layout.editor_x + layout.editor_width;
            if row < layout.content_top
                || (row as usize) >= layout.content_top as usize + layout.content_height
                || (column as usize) < layout.editor_x
                || (column as usize) >= editor_end
            {
                return;
            }
            self.explorer_focused = false;
            return_mode = self.preferred_editor_mode();
            if self.editor.selected_text().is_some() {
                actions.extend([ContextAction::Copy, ContextAction::Cut]);
            }
            actions.extend([
                ContextAction::Paste,
                ContextAction::SelectAll,
                ContextAction::ToggleComment,
                ContextAction::Format,
            ]);
        }

        if actions.is_empty() {
            return;
        }
        self.context_menu_previous_mode = return_mode;
        self.context_menu = Some(ContextMenu {
            x: column.saturating_add(1),
            y: row,
            selected: 0,
            actions,
        });
        self.mode = Mode::ContextMenu;
        self.message = "Context menu · ↑↓ select · Enter apply · Esc close".to_string();
    }

    fn close_context_menu(&mut self) {
        self.context_menu = None;
        self.mode = self.context_menu_previous_mode;
        self.message.clear();
    }

    fn execute_context_action(&mut self) {
        let Some(action) = self
            .context_menu
            .as_ref()
            .and_then(|menu| menu.actions.get(menu.selected))
            .copied()
        else {
            self.close_context_menu();
            return;
        };
        let return_mode = self.context_menu_previous_mode;
        self.context_menu = None;
        self.mode = return_mode;
        match action {
            ContextAction::Open => self.activate_project_entry(),
            ContextAction::Rename => self.begin_rename_selected(),
            ContextAction::Duplicate => {
                self.begin_context_command("copy ", "Enter the duplicate file name")
            }
            ContextAction::NewFile => {
                self.begin_context_command("newfile ", "Enter the new file path")
            }
            ContextAction::NewFolder => {
                self.begin_context_command("newdir ", "Enter the new folder path")
            }
            ContextAction::Stage => self.git_selected(true),
            ContextAction::Unstage => self.git_selected(false),
            ContextAction::SaveTab => self.save(),
            ContextAction::CloseTab => self.close_active_tab(false),
            ContextAction::Copy => {
                self.handle_global_shortcut(KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL,
                ));
            }
            ContextAction::Cut => {
                self.handle_global_shortcut(KeyEvent::new(
                    KeyCode::Char('x'),
                    KeyModifiers::CONTROL,
                ));
            }
            ContextAction::Paste => {
                self.handle_global_shortcut(KeyEvent::new(
                    KeyCode::Char('v'),
                    KeyModifiers::CONTROL,
                ));
            }
            ContextAction::SelectAll => {
                self.handle_global_shortcut(KeyEvent::new(
                    KeyCode::Char('a'),
                    KeyModifiers::CONTROL,
                ));
            }
            ContextAction::ToggleComment => self.toggle_comments(),
            ContextAction::Format => self.request_formatting(),
        }
    }

    fn begin_context_command(&mut self, prefix: &str, message: &str) {
        self.command_input = prefix.to_string();
        self.command_cursor = self.command_input.len();
        self.command_anchor = None;
        self.command_selection = None;
        self.command_suggestion = 0;
        self.command_suggestion_scroll = 0;
        self.mode = Mode::Command;
        self.message = message.to_string();
    }

    fn handle_left_click(&mut self, column: u16, row: u16, width: u16, height: u16) {
        self.follow_cursor = true;
        let layout = crate::ui::screen_layout(self, width, height);

        if layout.terminal_height > 0
            && row >= layout.terminal_top.saturating_sub(1)
            && row
                < layout
                    .terminal_top
                    .saturating_add(layout.terminal_height as u16)
        {
            self.terminal_focused = true;
            self.explorer_focused = false;
            self.message = "Terminal focused · Ctrl-` returns to editor".to_string();
            return;
        }
        self.terminal_focused = false;

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

        if row == 0 && column >= width.saturating_sub(30) && column < width.saturating_sub(20) {
            self.explorer_focused = false;
            self.open_theme_gallery();
            return;
        }

        if row == 0 && column >= width.saturating_sub(20) {
            self.explorer_focused = false;
            self.open_help();
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
                let double_click =
                    self.last_project_click
                        .as_ref()
                        .is_some_and(|(time, previous)| {
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
                row > layout.content_top + layout.content_height.saturating_sub(1) as u16 / 2
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
        self.message = copy_message(crate::clipboard::copy(&self.yank), "Copied selection");
    }

    fn show_diagnostic_report(&mut self) {
        self.git_diff_lines = crate::diagnostics::report(env!("CARGO_PKG_VERSION"))
            .lines()
            .map(str::to_string)
            .collect();
        self.git_diff_scroll = 0;
        self.git_diff_title = "DIAGNOSTIC REPORT".to_string();
        self.git_diff_return_mode = self.preferred_editor_mode();
        self.mode = Mode::GitDiff;
    }

    fn copy_diagnostic_report(&mut self) {
        self.yank = crate::diagnostics::report(env!("CARGO_PKG_VERSION"));
        self.message = copy_message(
            crate::clipboard::copy(&self.yank),
            "Diagnostic report copied",
        );
    }

    /// Per-char match flags for one line, used to paint search highlights.
    pub fn search_line_hits(&self, line: &str) -> Vec<bool> {
        let char_count = line.chars().count();
        let mut hits = vec![false; char_count];
        let Some(search) = self.active_search.as_ref() else {
            return hits;
        };
        for (byte_start, byte_end) in search.find_byte_ranges(line) {
            let start = line[..byte_start].chars().count();
            let length = line[byte_start..byte_end].chars().count();
            let end = (start + length).min(char_count);
            for hit in &mut hits[start..end] {
                *hit = true;
            }
        }
        hits
    }

    /// The single-line find/replace panel shown in place of the prompt bar.
    pub fn search_panel_text(&self) -> String {
        let mut flags = String::new();
        if self.search_options.case_sensitive {
            flags.push_str("  Aa");
        }
        if self.search_options.whole_word {
            flags.push_str("  Word");
        }
        if self.search_options.use_regex {
            flags.push_str("  .*");
        }
        if self.search_scoped && self.search_scope.is_some() {
            flags.push_str("  InSel");
        }

        let status = if let Some(error) = &self.search_error {
            error.clone()
        } else if self.search_input.is_empty() {
            "type to search".to_string()
        } else {
            let matches = self.active_search.as_ref().map_or(0, |search| {
                self.editor
                    .search_match_ranges(search, self.effective_search_scope())
                    .len()
            });
            let current = self
                .active_search
                .as_ref()
                .zip(self.editor.selection_range())
                .and_then(|(search, range)| {
                    self.editor
                        .search_match_ranges(search, self.effective_search_scope())
                        .iter()
                        .position(|candidate| *candidate == range)
                })
                .map(|index| index + 1);
            match (matches, current) {
                (0, _) => "no matches".to_string(),
                (total, Some(index)) => format!("{index}/{total}"),
                (total, None) => format!("{total} match(es)"),
            }
        };

        format!(
            " FIND {}  REPLACE {}  · {status}{flags}",
            self.search_input, self.search_replace_input
        )
    }

    /// Where the terminal cursor sits inside the search panel row.
    pub fn search_cursor_offset(&self) -> (String, String) {
        let before_query = " FIND ".to_string();
        let between = "  REPLACE ".to_string();
        if self.search_focus_replace {
            (
                format!("{before_query}{}{between}", self.search_input),
                self.search_replace_input.clone(),
            )
        } else {
            (before_query, self.search_input.clone())
        }
    }

    pub fn active_panel_label(&self) -> &'static str {
        if self.terminal_focused {
            "TERMINAL"
        } else if self.explorer_focused {
            match self.sidebar_view {
                SidebarView::Files => "FILES",
                SidebarView::Outline => "SYMBOLS",
            }
        } else {
            self.mode.label()
        }
    }

    pub fn background_status(&self) -> Option<(String, BackgroundState)> {
        const SPINNER: [char; 4] = ['|', '/', '-', '\\'];
        match self.lsp_status {
            LspStatus::Off => None,
            LspStatus::Starting => {
                let slow = self
                    .lsp_started_at
                    .is_some_and(|started| started.elapsed() > Duration::from_secs(30));
                Some((
                    if slow {
                        "LSP starting slowly".to_string()
                    } else if self.reduced_motion() {
                        "LSP starting".to_string()
                    } else {
                        format!(
                            "{} LSP starting",
                            SPINNER[self.background_tick % SPINNER.len()]
                        )
                    },
                    if slow {
                        BackgroundState::Warning
                    } else {
                        BackgroundState::Working
                    },
                ))
            }
            LspStatus::Loading => {
                let slow = self
                    .lsp_started_at
                    .is_some_and(|started| started.elapsed() > Duration::from_secs(30));
                Some((
                    if slow {
                        "LSP workspace still loading".to_string()
                    } else if self.reduced_motion() {
                        "LSP loading".to_string()
                    } else {
                        format!(
                            "{} LSP loading",
                            SPINNER[self.background_tick % SPINNER.len()]
                        )
                    },
                    if slow {
                        BackgroundState::Warning
                    } else {
                        BackgroundState::Working
                    },
                ))
            }
            LspStatus::Ready => Some((
                if self.diagnostic_count == 0 {
                    "LSP ready".to_string()
                } else {
                    format!("LSP ready · {} issues", self.diagnostic_count)
                },
                if self.diagnostic_count == 0 {
                    BackgroundState::Ready
                } else {
                    BackgroundState::Warning
                },
            )),
            LspStatus::Error => Some(("LSP error".to_string(), BackgroundState::Error)),
        }
    }

    pub fn reduced_motion(&self) -> bool {
        self.settings.reduced_motion || std::env::var_os("CARET_REDUCED_MOTION").is_some()
    }

    fn open_help(&mut self) {
        if self.mode != Mode::Help {
            self.help_return_mode = self.mode;
        }
        self.mode = Mode::Help;
    }

    fn activate_focused_view(&mut self) {
        let Some(views) = self.split_views else {
            return;
        };
        let view = if views.secondary_active {
            views.secondary
        } else {
            views.primary
        };
        self.editor.select(view.tab_index);
        self.editor.cursor = view.cursor;
        self.editor.scroll_line = view.scroll_line;
        self.editor.scroll_column = view.scroll_column;
    }

    fn sync_focused_view(&mut self) {
        let Some(mut views) = self.split_views else {
            return;
        };
        let view = EditorView {
            tab_index: self.editor.active_index(),
            cursor: self.editor.cursor,
            scroll_line: self.editor.scroll_line,
            scroll_column: self.editor.scroll_column,
        };
        if views.secondary_active {
            views.secondary = view;
        } else {
            views.primary = view;
        }
        self.split_views = Some(views);
    }

    fn open_split(&mut self, vertical: bool) {
        if let Some(mut views) = self.split_views {
            views.vertical = vertical;
            self.split_views = Some(views);
            self.message = if vertical {
                "Vertical split"
            } else {
                "Horizontal split"
            }
            .to_string();
        } else {
            let view = EditorView {
                tab_index: self.editor.active_index(),
                cursor: self.editor.cursor,
                scroll_line: self.editor.scroll_line,
                scroll_column: self.editor.scroll_column,
            };
            self.split_views = Some(SplitViews {
                primary: view,
                secondary: view,
                secondary_active: false,
                vertical,
            });
            self.message = if vertical {
                "Vertical split opened · Ctrl-\\ switches panes"
            } else {
                "Horizontal split opened · Ctrl-\\ switches panes"
            }
            .to_string();
        }
    }

    pub fn terminal_visible(&self) -> bool {
        self.terminal.is_some()
    }
    pub fn terminal_shell_name(&self) -> &str {
        self.terminal
            .as_ref()
            .map_or("Terminal", |terminal| terminal.shell_name.as_str())
    }
    pub fn terminal_cwd(&self) -> Option<&Path> {
        self.terminal
            .as_ref()
            .map(|terminal| terminal.cwd.as_path())
    }
    pub fn terminal_lines(&self, rows: usize) -> Vec<String> {
        self.terminal
            .as_ref()
            .map_or_else(Vec::new, |terminal| terminal.visible_lines(rows))
    }
    pub fn resize_terminal(&mut self, rows: usize, columns: usize) {
        if let Some(terminal) = self.terminal.as_mut() {
            terminal.resize(rows, columns);
        }
    }
    pub fn terminal_cursor_position(&self) -> (usize, usize) {
        self.terminal
            .as_ref()
            .map_or((0, 0), TerminalPane::cursor_position)
    }
    pub fn terminal_exited(&self) -> bool {
        self.terminal.as_ref().is_some_and(TerminalPane::is_exited)
    }

    fn open_terminal(&mut self) {
        if self.terminal.is_none() {
            match TerminalPane::start(&self.project.root) {
                Ok(terminal) => self.terminal = Some(terminal),
                Err(error) => {
                    self.message = format!("Could not start terminal: {error}");
                    return;
                }
            }
        }
        if self.mode == Mode::Dashboard {
            self.mode = self.preferred_editor_mode();
        }
        self.explorer_focused = false;
        self.terminal_focused = true;
        self.message = "Terminal focused · Ctrl-` returns to editor".to_string();
    }

    fn toggle_terminal_focus(&mut self) {
        if self.terminal.is_none() {
            self.open_terminal();
        } else {
            self.terminal_focused = !self.terminal_focused;
            self.message = if self.terminal_focused {
                "Terminal focused · Ctrl-` returns to editor"
            } else {
                "Editor focused · Ctrl-` returns to terminal"
            }
            .to_string();
        }
    }

    fn close_terminal(&mut self) {
        self.terminal = None;
        self.terminal_focused = false;
        self.message = "Terminal closed".to_string();
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Quit confirmation gets first chance at every key so terminal/editor
        // shortcuts cannot accidentally dismiss or bypass the prompt.
        if self.mode == Mode::QuitConfirm {
            self.handle_quit_confirmation(key);
            return;
        }
        if self.mode == Mode::TabCloseConfirm {
            self.handle_tab_close_confirmation(key);
            return;
        }
        if self.mode == Mode::ReloadConfirm {
            self.handle_reload_confirmation(key);
            return;
        }
        if self.lsp_panel.is_some() {
            self.handle_lsp_panel_key(key);
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('`' | '~'))
        {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.close_terminal();
            } else {
                self.toggle_terminal_focus();
            }
            return;
        }
        if self.terminal_focused {
            if key.code == KeyCode::F(1) {
                self.terminal_focused = false;
                self.open_help();
            } else if let Some(terminal) = self.terminal.as_mut() {
                if let Err(error) = terminal.handle_key(key) {
                    self.message = format!("Terminal input failed: {error}");
                }
            }
            return;
        }
        if key.code == KeyCode::F(2)
            && !self.explorer_focused
            && matches!(self.mode, Mode::Normal | Mode::Insert)
        {
            self.command_input = "symbolrename ".to_string();
            self.command_cursor = self.command_input.len();
            self.command_selection = None;
            self.command_anchor = None;
            self.mode = Mode::Command;
            self.message = "Rename symbol to…".to_string();
            return;
        }
        if self.mode == Mode::Dashboard {
            if key
                .modifiers
                .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
                && matches!(key.code, KeyCode::Char('p' | 'P'))
                && self.handle_global_shortcut(key)
            {
                return;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.dashboard_selected = self.dashboard_selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.dashboard_selected = (self.dashboard_selected + 1)
                        .min(self.settings.recent_projects.len().saturating_sub(1));
                }
                KeyCode::Enter => self.open_selected_recent_project(),
                KeyCode::Char('c') | KeyCode::Esc => self.open_current_folder_from_dashboard(),
                KeyCode::Char('e') | KeyCode::Char(':') => {
                    self.command_input = "e ".to_string();
                    self.command_cursor = self.command_input.len();
                    self.command_selection = None;
                    self.command_anchor = None;
                    self.mode = Mode::Command;
                    self.message = "Enter a file or folder path".to_string();
                }
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::F(1) | KeyCode::Char('?') => self.open_help(),
                _ => {}
            }
            return;
        }
        if self.mode == Mode::GitDiff {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = self.git_diff_return_mode,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.git_diff_scroll = self.git_diff_scroll.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.git_diff_scroll =
                        (self.git_diff_scroll + 1).min(self.git_diff_lines.len().saturating_sub(1))
                }
                KeyCode::PageUp => {
                    self.git_diff_scroll = self
                        .git_diff_scroll
                        .saturating_sub(self.viewport_rows.saturating_sub(2))
                }
                KeyCode::PageDown => {
                    self.git_diff_scroll = (self.git_diff_scroll
                        + self.viewport_rows.saturating_sub(2))
                    .min(self.git_diff_lines.len().saturating_sub(1))
                }
                _ => {}
            }
            return;
        }
        if self.mode == Mode::GitHistory {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.mode = self.preferred_editor_mode(),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.git_history_selected = self.git_history_selected.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.git_history_selected = (self.git_history_selected + 1)
                        .min(self.git_history.len().saturating_sub(1))
                }
                KeyCode::Enter => self.show_selected_history_diff(),
                _ => {}
            }
            return;
        }
        if self.mode == Mode::ThemeGallery {
            match key.code {
                KeyCode::Esc => {
                    self.theme_kind = self.theme_gallery_original;
                    self.theme = self.theme_gallery_original_theme;
                    self.active_custom_theme = self.theme_gallery_original_custom.clone();
                    self.mode = self.theme_gallery_return_mode;
                    self.message = "Theme preview cancelled".to_string();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.theme_gallery_selected = self.theme_gallery_selected.saturating_sub(1);
                    self.preview_gallery_theme();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.theme_gallery_selected =
                        (self.theme_gallery_selected + 1).min(ThemeKind::ALL.len() - 1);
                    self.preview_gallery_theme();
                }
                KeyCode::Enter => {
                    self.settings.theme = self.theme_kind;
                    self.settings.custom_theme = None;
                    self.active_custom_theme = None;
                    self.persist_settings();
                    self.mode = self.theme_gallery_return_mode;
                    self.message = format!("Theme: {}", self.theme_kind.name());
                }
                _ => {}
            }
            return;
        }
        if self.mode == Mode::KeymapGallery {
            match key.code {
                KeyCode::Esc => {
                    self.mode = self.keymap_gallery_return_mode;
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
        if self.mode == Mode::ProjectSearch {
            self.handle_project_search(key);
            return;
        }
        if self.mode == Mode::FilePicker {
            self.handle_file_picker(key);
            return;
        }
        if self.mode == Mode::KeyBrowser {
            self.handle_key_browser(key);
            return;
        }
        if self.mode == Mode::ContextMenu {
            match key.code {
                KeyCode::Esc => self.close_context_menu(),
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(menu) = self.context_menu.as_mut() {
                        menu.selected = menu.selected.saturating_sub(1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(menu) = self.context_menu.as_mut() {
                        menu.selected =
                            (menu.selected + 1).min(menu.actions.len().saturating_sub(1));
                    }
                }
                KeyCode::Enter => self.execute_context_action(),
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

        if self.explorer_focused
            && matches!(self.mode, Mode::Normal | Mode::Insert)
            && key.code == KeyCode::Char(':')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.command_input.clear();
            self.command_cursor = 0;
            self.command_anchor = None;
            self.command_selection = None;
            self.command_suggestion = 0;
            self.command_suggestion_scroll = 0;
            self.mode = Mode::Command;
            self.message = "Command palette".to_string();
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
                    self.mode = self.help_return_mode;
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
            Mode::QuitConfirm
            | Mode::TabCloseConfirm
            | Mode::ReloadConfirm
            | Mode::ProjectSearch
            | Mode::FilePicker
            | Mode::KeyBrowser
            | Mode::GitDiff
            | Mode::GitHistory
            | Mode::ThemeGallery
            | Mode::KeymapGallery
            | Mode::ContextMenu
            | Mode::Dashboard => {}
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

    fn handle_tab_close_confirmation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                self.mode = self.preferred_editor_mode();
                self.message = "Tab close cancelled".to_string();
            }
            KeyCode::Char('d') | KeyCode::Char('D') => self.close_active_tab(true),
            _ => {}
        }
    }

    fn handle_reload_confirmation(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('r') | KeyCode::Char('R') => match self.editor.reload_from_disk() {
                Ok(()) => {
                    self.pending_save_after_disk_change = false;
                    self.mode = self.preferred_editor_mode();
                    self.message = "Reloaded changed file".to_string();
                }
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
                    self.message =
                        "Kept current buffer; save will ask before overwriting disk changes"
                            .to_string();
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => self.compare_external_change(),
            KeyCode::Esc => {
                self.pending_save_after_disk_change = false;
                self.editor.acknowledge_disk_change();
                self.mode = self.preferred_editor_mode();
                self.message = "Reload deferred".to_string();
            }
            _ => {}
        }
    }

    /// Global shortcuts resolve through the user-configurable binding
    /// registry first, so every action here can be rebound with :bind.
    fn handle_global_shortcut(&mut self, key: KeyEvent) -> bool {
        if let Some(action) = self.keys.action_for(key) {
            self.run_action(action);
            return true;
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            if let KeyCode::Char(character @ '1'..='9') = key.code {
                let index = character.to_digit(10).unwrap_or(1) as usize - 1;
                self.select_tab_with_history(index);
                return true;
            }
        }

        false
    }

    fn run_action(&mut self, action: KeyAction) {
        match action {
            KeyAction::Save => self.save(),
            KeyAction::Quit => self.request_quit(false),
            KeyAction::Find => self.open_search_panel(false),
            KeyAction::Replace => self.open_search_panel(true),
            KeyAction::ProjectSearch => self.open_project_search(),
            KeyAction::OpenFile => self.open_file_picker(),
            KeyAction::Palette => self.open_command_palette(),
            KeyAction::Complete => self.request_lsp_position("textDocument/completion"),
            KeyAction::CodeActions => self.request_lsp_position("textDocument/codeAction"),
            KeyAction::Undo => {
                self.message = if self.editor.undo() {
                    "Undone"
                } else {
                    "Nothing to undo"
                }
                .to_string();
            }
            KeyAction::Redo => {
                self.message = if self.editor.redo() {
                    "Redone"
                } else {
                    "Nothing to redo"
                }
                .to_string();
            }
            KeyAction::SelectAll => {
                self.editor.clear_selection();
                self.editor.move_file_start();
                self.editor.begin_selection();
                self.editor.move_file_end();
                self.message = "Selected all".to_string();
            }
            KeyAction::Copy => self.copy_selection_to_clipboard(),
            KeyAction::Cut => {
                if let Some(text) = self.editor.selected_text() {
                    self.editor.checkpoint();
                    self.yank = text;
                    self.editor.delete_selection();
                    self.message =
                        copy_message(crate::clipboard::copy(&self.yank), "Cut selection");
                } else {
                    self.message = "Select text to cut".to_string();
                }
            }
            KeyAction::Paste => {
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
            }
            KeyAction::NextTab => self.next_tab(),
            KeyAction::PrevTab => self.previous_tab(),
            KeyAction::NewTab => self.new_tab(None),
            KeyAction::CloseTab => self.close_active_tab(false),
            KeyAction::ToggleTree => self.toggle_file_tree(),
            KeyAction::ToggleOutline => {
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
                }
                .to_string();
            }
            KeyAction::FocusTree => {
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
            }
            KeyAction::ToggleComment => self.toggle_comments(),
            KeyAction::AddCursorAbove => self.add_cursor_line(false),
            KeyAction::AddCursorBelow => self.add_cursor_line(true),
            KeyAction::SelectOccurrences => {
                let count = self.editor.select_all_occurrences();
                self.message = if count > 1 {
                    format!("Selected {count} occurrences · type to replace them all")
                } else {
                    "Select or place the cursor on a word first".to_string()
                };
            }
            KeyAction::Back => self.go_back(),
            KeyAction::Forward => self.go_forward(),
            KeyAction::Definition => self.request_lsp_position("textDocument/definition"),
            KeyAction::References => self.request_lsp_position("textDocument/references"),
            KeyAction::SwitchSplit => {
                if let Some(mut views) = self.split_views {
                    self.sync_focused_view();
                    views.secondary_active = !views.secondary_active;
                    self.split_views = Some(views);
                    self.activate_focused_view();
                }
            }
        }
    }

    fn open_key_browser(&mut self) {
        self.key_browser_input.clear();
        self.key_browser_scroll = 0;
        self.mode = Mode::KeyBrowser;
        self.message = "Key bindings · type to search · :bind <action> <keys> rebinds".to_string();
    }

    fn handle_key_browser(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.mode = self.preferred_editor_mode();
                self.message.clear();
            }
            KeyCode::Up => self.key_browser_scroll = self.key_browser_scroll.saturating_sub(1),
            KeyCode::Down => {
                self.key_browser_scroll = (self.key_browser_scroll + 1)
                    .min(self.keybinding_rows().len().saturating_sub(1))
            }
            KeyCode::PageUp => self.key_browser_scroll = self.key_browser_scroll.saturating_sub(10),
            KeyCode::PageDown => {
                self.key_browser_scroll = (self.key_browser_scroll + 10)
                    .min(self.keybinding_rows().len().saturating_sub(1))
            }
            KeyCode::Home => self.key_browser_scroll = 0,
            KeyCode::Backspace => {
                self.key_browser_input.pop();
                self.key_browser_scroll = 0;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.key_browser_input.push(character);
                self.key_browser_scroll = 0;
            }
            _ => {}
        }
    }

    /// Rows for the searchable keybinding browser: every rebindable action
    /// with its current chord, then the fixed keys of the active profile.
    pub fn keybinding_rows(&self) -> Vec<(String, String, String)> {
        let query = self.key_browser_input.to_lowercase();
        let mut rows = Vec::new();
        for action in KeyAction::ALL {
            let chord = self.keys.chord_for(action);
            let chord_text = chord
                .map(|chord| chord.display())
                .unwrap_or_else(|| "unbound".to_string());
            let mut note = String::new();
            if self.keys.is_custom(action) {
                note.push_str("custom · ");
            }
            if let Some(warning) = chord.as_ref().and_then(crate::keys::chord_warning) {
                note.push_str("⚠ ");
                note.push_str(warning);
            } else {
                note.push_str(action.id());
            }
            rows.push((chord_text, action.description().to_string(), note));
        }
        for (chord, description) in crate::keys::fixed_keys(self.keymap_profile()) {
            rows.push((
                chord.to_string(),
                description.to_string(),
                format!("{} profile", self.keymap_profile().name()),
            ));
        }
        if query.is_empty() {
            return rows;
        }
        rows.into_iter()
            .filter(|(chord, description, note)| {
                chord.to_lowercase().contains(&query)
                    || description.to_lowercase().contains(&query)
                    || note.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn rebuild_keys(&mut self) {
        self.keys = KeyBindings::from_custom(&self.settings.custom_keys);
    }

    fn execute_bind(&mut self, action_id: &str, chord_text: &str) {
        let Some(action) = KeyAction::from_id(action_id) else {
            self.message = format!("Unknown action: {action_id} — see :keybindings for the list");
            return;
        };
        if chord_text.eq_ignore_ascii_case("default") {
            self.settings.custom_keys.remove(action.id());
            self.rebuild_keys();
            self.persist_settings();
            self.message = format!(
                "{} reset to {}",
                action.id(),
                self.keys
                    .chord_for(action)
                    .map(|chord| chord.display())
                    .unwrap_or_else(|| "unbound".to_string())
            );
            return;
        }
        match self.keys.validate(action, chord_text) {
            Ok(chord) => {
                self.settings
                    .custom_keys
                    .insert(action.id().to_string(), chord_text.to_string());
                self.rebuild_keys();
                self.persist_settings();
                let warning = crate::keys::chord_warning(&chord)
                    .map(|warning| format!(" · warning: {warning}"))
                    .unwrap_or_default();
                self.message = format!("{} → {}{}", action.id(), chord.display(), warning);
            }
            Err(error) => self.message = error,
        }
    }

    fn open_command_palette(&mut self) {
        self.command_input.clear();
        self.command_cursor = 0;
        self.command_anchor = None;
        self.command_selection = None;
        self.command_suggestion = 0;
        self.command_suggestion_scroll = 0;
        self.mode = Mode::Command;
        self.message = "Command palette".to_string();
    }

    fn add_cursor_line(&mut self, below: bool) {
        self.message = if self.editor.add_cursor_line(below) {
            format!(
                "{} cursor(s) · Esc returns to one",
                self.editor.secondary_cursors.len() + 1
            )
        } else if below {
            "No line below to add a cursor on".to_string()
        } else {
            "No line above to add a cursor on".to_string()
        };
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
        if let Some(language) = self.plugins.language_for_path(self.editor.path.as_deref()) {
            let Some(prefix) = language.line_comment.as_deref() else {
                self.message = format!("{} has no configured line-comment syntax", language.name);
                return;
            };
            self.editor.checkpoint();
            self.message = match self.editor.toggle_line_comments(prefix, None) {
                Some(true) => "Commented line(s)".to_string(),
                Some(false) => "Uncommented line(s)".to_string(),
                None => "No nonblank lines to comment".to_string(),
            };
            return;
        }
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

    pub fn language_name(&self) -> String {
        self.plugins
            .language_for_path(self.editor.path.as_deref())
            .map(|language| language.name.clone())
            .unwrap_or_else(|| {
                Language::from_path(self.editor.path.as_deref())
                    .name()
                    .to_string()
            })
    }

    fn lsp_cursor_character(&self) -> usize {
        self.editor
            .line_text(self.editor.cursor.line)
            .chars()
            .take(self.editor.cursor.column)
            .map(char::len_utf16)
            .sum()
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
            symbols
                .into_iter()
                .take(8)
                .map(|symbol| format!("{}:{}", symbol.name, symbol.start_line + 1))
                .collect::<Vec<_>>()
                .join("  ·  ")
        };
    }

    fn selected_project_path(&self) -> Option<PathBuf> {
        self.project
            .selected_entry()
            .map(|entry| entry.path.clone())
    }

    fn project_target_path(&self, value: &str) -> Option<PathBuf> {
        let path = self.resolve_project_path(value);
        path.starts_with(&self.project.root).then_some(path)
    }

    fn create_project_file(&mut self, value: &str) {
        let Some(path) = self.project_target_path(value) else {
            self.message = "Path must be inside the project".to_string();
            return;
        };
        if path.exists() {
            self.message = "File already exists".to_string();
            return;
        }
        match path
            .parent()
            .map(fs::create_dir_all)
            .transpose()
            .and_then(|_| fs::File::create(&path).map(|_| ()))
        {
            Ok(()) => {
                let _ = self.project.refresh();
                self.message = format!("Created {}", path.display());
            }
            Err(error) => self.message = format!("Create failed: {error}"),
        }
    }

    fn create_project_directory(&mut self, value: &str) {
        let Some(path) = self.project_target_path(value) else {
            self.message = "Path must be inside the project".to_string();
            return;
        };
        match fs::create_dir(&path) {
            Ok(()) => {
                let _ = self.project.refresh();
                self.message = format!("Created {}", path.display());
            }
            Err(error) => self.message = format!("Create failed: {error}"),
        }
    }

    fn rename_selected_project_entry(&mut self, value: &str) {
        let Some(source) = self.selected_project_path() else {
            self.message = "Select a file or folder first".to_string();
            return;
        };
        let Some(destination) = source
            .parent()
            .and_then(|parent| (!value.is_empty()).then(|| parent.join(value)))
        else {
            self.message = "Usage: :rename new-name".to_string();
            return;
        };
        match fs::rename(&source, &destination) {
            Ok(()) => {
                let _ = self.project.refresh();
                self.message = format!("Renamed to {}", destination.display());
            }
            Err(error) => self.message = format!("Rename failed: {error}"),
        }
    }

    fn begin_rename_selected(&mut self) {
        let Some(path) = self.selected_project_path() else {
            self.message = "Select a file or folder first".to_string();
            return;
        };
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let prefix = "rename ";
        self.command_input = format!("{prefix}{name}");
        let stem_end = name
            .rfind('.')
            .filter(|index| *index > 0)
            .unwrap_or(name.len());
        self.command_cursor = prefix.len() + stem_end;
        self.command_selection = None;
        self.command_anchor = None;
        self.command_suggestion = 0;
        self.command_suggestion_scroll = 0;
        self.mode = Mode::Command;
        self.message = "Type a new name, then press Enter".to_string();
    }

    fn replace_command_selection(&mut self) {
        if let Some((start, end)) = self.command_selection.take() {
            if start <= end && end <= self.command_input.len() {
                self.command_input.replace_range(start..end, "");
                self.command_cursor = start;
            }
        }
        self.command_anchor = None;
    }

    fn duplicate_selected_project_file(&mut self, value: &str) {
        let Some(source) = self.selected_project_path() else {
            self.message = "Select a file first".to_string();
            return;
        };
        if source.is_dir() {
            self.message = "Duplicate currently supports files only".to_string();
            return;
        }
        let destination = if value.is_empty() {
            source
                .with_file_name(format!(
                    "{} copy",
                    source
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or("file")
                ))
                .with_extension(source.extension().unwrap_or_default())
        } else {
            source.parent().unwrap_or(&self.project.root).join(value)
        };
        match fs::copy(&source, &destination) {
            Ok(_) => {
                let _ = self.project.refresh();
                self.message = format!("Duplicated to {}", destination.display());
            }
            Err(error) => self.message = format!("Duplicate failed: {error}"),
        }
    }

    fn move_selected_project_entry(&mut self, value: &str) {
        let Some(source) = self.selected_project_path() else {
            self.message = "Select a file or folder first".to_string();
            return;
        };
        let Some(destination) = self.project_target_path(value) else {
            self.message = "Path must be inside the project".to_string();
            return;
        };
        match fs::rename(&source, &destination) {
            Ok(()) => {
                let _ = self.project.refresh();
                self.message = format!("Moved to {}", destination.display());
            }
            Err(error) => self.message = format!("Move failed: {error}"),
        }
    }

    fn delete_selected_project_entry(&mut self, force: bool) {
        let Some(path) = self.selected_project_path() else {
            self.message = "Select a file or folder first".to_string();
            return;
        };
        if !force {
            self.message = format!("Delete {} permanently? :delete! confirms", path.display());
            return;
        }
        let result = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match result {
            Ok(()) => {
                let _ = self.project.refresh();
                self.message = format!("Deleted {}", path.display());
            }
            Err(error) => self.message = format!("Delete failed: {error}"),
        }
    }

    fn git_selected(&mut self, stage: bool) {
        let Some(path) = self.selected_project_path() else {
            self.message = "Select a file first".to_string();
            return;
        };
        let relative = path.strip_prefix(&self.project.root).unwrap_or(&path);
        let relative = relative.to_string_lossy().to_string();
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.project.root);
        if stage {
            command.args(["add", "--", &relative]);
        } else {
            command.args(["restore", "--staged", "--", &relative]);
        }
        match command.output() {
            Ok(output) if output.status.success() => {
                self.project.refresh_git_status();
                self.refresh_git_line_changes();
                self.message = if stage {
                    "Staged selected file"
                } else {
                    "Unstaged selected file"
                }
                .to_string();
            }
            Ok(output) => {
                self.message = format!(
                    "Git failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )
            }
            Err(error) => self.message = format!("Git failed: {error}"),
        }
    }

    pub fn git_line_change(&self, line: usize) -> Option<GitLineChange> {
        self.git_line_changes.get(&line).copied()
    }

    fn refresh_git_line_changes(&mut self) {
        self.git_line_changes.clear();
        let Some(path) = self.editor.path.as_ref() else {
            return;
        };
        let Ok(relative) = path.strip_prefix(&self.project.root) else {
            return;
        };
        let relative_text = relative.to_string_lossy().to_string();

        let tracked = Command::new("git")
            .arg("-C")
            .arg(&self.project.root)
            .args(["ls-files", "--error-unmatch", "--", &relative_text])
            .output()
            .is_ok_and(|output| output.status.success());
        if !tracked {
            if path.exists() {
                for line in 0..self.editor.line_count() {
                    self.git_line_changes.insert(line, GitLineChange::Added);
                }
            }
            return;
        }

        let Ok(output) = Command::new("git")
            .arg("-C")
            .arg(&self.project.root)
            .args(["diff", "HEAD", "--unified=0", "--", &relative_text])
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        self.git_line_changes = parse_git_hunks(
            &String::from_utf8_lossy(&output.stdout),
            self.editor.line_count(),
        );
    }

    fn show_git_diff(&mut self) {
        let path = self
            .selected_project_path()
            .or_else(|| self.editor.path.clone());
        let Some(path) = path else {
            self.message = "Select or open a file first".to_string();
            return;
        };
        let relative = path
            .strip_prefix(&self.project.root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let run = |staged| {
            let mut command = Command::new("git");
            command.arg("-C").arg(&self.project.root).arg("diff");
            if staged {
                command.arg("--cached");
            }
            command.args(["--", &relative]).output()
        };
        let output = run(false)
            .ok()
            .filter(|output| output.status.success())
            .or_else(|| run(true).ok());
        let Some(output) = output else {
            self.message = "Git diff failed".to_string();
            return;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        self.git_diff_lines = if text.is_empty() {
            vec!["No unstaged or staged changes for this file.".to_string()]
        } else {
            text.lines().map(str::to_string).collect()
        };
        self.git_diff_scroll = 0;
        self.git_diff_title = "GIT DIFF".to_string();
        self.git_diff_return_mode = self.preferred_editor_mode();
        self.mode = Mode::GitDiff;
    }

    fn compare_external_change(&mut self) {
        let Some(path) = self.editor.path.clone() else {
            return;
        };
        let disk = match crate::document::read_text(&path) {
            Ok((text, _)) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                "[file no longer exists on disk]".to_string()
            }
            Err(error) => {
                self.message = format!("Could not compare disk version: {error}");
                return;
            }
        };
        let buffer = self.editor.text();
        let buffer_lines = buffer.lines().collect::<Vec<_>>();
        let disk_lines = disk.lines().collect::<Vec<_>>();
        let count = buffer_lines.len().max(disk_lines.len());
        self.git_diff_lines = (0..count)
            .flat_map(
                |line| match (buffer_lines.get(line), disk_lines.get(line)) {
                    (Some(left), Some(right)) if left == right => vec![format!("  {left}")],
                    (Some(left), Some(right)) => vec![format!("- {left}"), format!("+ {right}")],
                    (Some(left), None) => vec![format!("- {left}")],
                    (None, Some(right)) => vec![format!("+ {right}")],
                    (None, None) => Vec::new(),
                },
            )
            .collect();
        if self.git_diff_lines.is_empty() {
            self.git_diff_lines
                .push("No content differences; metadata changed on disk.".to_string());
        }
        self.git_diff_scroll = 0;
        self.git_diff_title = "EXTERNAL FILE CHANGE".to_string();
        self.git_diff_return_mode = Mode::ReloadConfirm;
        self.mode = Mode::GitDiff;
    }

    fn show_git_history(&mut self) {
        let path = self
            .selected_project_path()
            .or_else(|| self.editor.path.clone());
        let Some(path) = path else {
            self.message = "Select or open a file first".to_string();
            return;
        };
        let relative = path
            .strip_prefix(&self.project.root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.project.root)
            .args(["log", "--format=%h%x09%s", "-n", "30", "--", &relative])
            .output();
        let Ok(output) = output else {
            self.message = "Git history failed".to_string();
            return;
        };
        self.git_history = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                line.split_once('\t')
                    .map(|(hash, summary)| GitHistoryEntry {
                        hash: hash.to_string(),
                        summary: summary.to_string(),
                    })
            })
            .collect();
        self.git_history_selected = 0;
        self.mode = Mode::GitHistory;
    }

    fn show_selected_history_diff(&mut self) {
        let Some(entry) = self.git_history.get(self.git_history_selected) else {
            return;
        };
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.project.root)
            .args(["show", "--format=fuller", "--stat", &entry.hash])
            .output();
        let Ok(output) = output else {
            self.message = "Git show failed".to_string();
            return;
        };
        self.git_diff_lines = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
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
        self.remember_recent_file();
        self.refresh_git_line_changes();
        self.sync_lsp_active_file();
    }

    fn close_active_tab(&mut self, force: bool) {
        match self.editor.close_active(force) {
            Ok(title) => {
                self.pending_key = None;
                self.search_origin = self.editor.cursor;
                self.mode = self.preferred_editor_mode();
                self.message = format!("Closed {title}");
            }
            Err(message) => {
                self.message = message;
                self.mode = Mode::TabCloseConfirm;
            }
        }
    }

    fn handle_explorer(&mut self, key: KeyEvent) {
        // Ctrl shortcuts were already offered to the global registry; swallow
        // the rest so plain-letter tree bindings never fire with Ctrl held.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return;
        }

        if self.sidebar_view == SidebarView::Outline {
            let symbols = self.outline_symbols();
            match key.code {
                KeyCode::Esc => {
                    self.explorer_focused = false;
                    self.mode = self.preferred_editor_mode();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.outline_selected = self.outline_selected.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.outline_selected =
                        (self.outline_selected + 1).min(symbols.len().saturating_sub(1))
                }
                KeyCode::PageUp => {
                    self.outline_selected = self
                        .outline_selected
                        .saturating_sub(self.viewport_rows.saturating_sub(2))
                }
                KeyCode::PageDown => {
                    self.outline_selected = (self.outline_selected
                        + self.viewport_rows.saturating_sub(2))
                    .min(symbols.len().saturating_sub(1))
                }
                KeyCode::Home => self.outline_selected = 0,
                KeyCode::End => self.outline_selected = symbols.len().saturating_sub(1),
                KeyCode::Enter => {
                    if let Some(symbol) = symbols.get(self.outline_selected) {
                        self.editor.goto_line(symbol.start_line);
                        self.editor.move_line_start();
                        self.follow_cursor = true;
                        self.message = format!("{} · line {}", symbol.name, symbol.start_line + 1);
                    }
                }
                _ => {}
            }
            return;
        }

        // Typing mode for the tree filter: characters narrow the file list.
        if self.tree_filter_active {
            match key.code {
                KeyCode::Esc => {
                    self.tree_filter_active = false;
                    let _ = self.project.set_filter(String::new());
                    self.message = "Filter cleared".to_string();
                }
                KeyCode::Enter => {
                    if self.project.filter.is_empty() {
                        self.tree_filter_active = false;
                        self.message = "Filter closed".to_string();
                    } else if self.project.entries.is_empty() {
                        self.message = format!("No files match {}", self.project.filter);
                    } else {
                        self.tree_filter_active = false;
                        self.activate_project_entry();
                    }
                }
                KeyCode::Up => self.project.move_up(),
                KeyCode::Down => self.project.move_down(),
                KeyCode::Backspace => {
                    let mut filter = self.project.filter.clone();
                    filter.pop();
                    let _ = self.project.set_filter(filter);
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    let filter = format!("{}{character}", self.project.filter);
                    let _ = self.project.set_filter(filter);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::F(1) | KeyCode::Char('?') => {
                self.explorer_focused = false;
                self.open_help();
            }
            KeyCode::Esc if !self.project.filter.is_empty() => {
                let _ = self.project.set_filter(String::new());
                self.message = "Filter cleared".to_string();
            }
            KeyCode::Esc => {
                self.explorer_focused = false;
                self.mode = self.preferred_editor_mode();
                self.message = "Editor focused".to_string();
            }
            KeyCode::Char('/') | KeyCode::Char('f') => {
                self.tree_filter_active = true;
                self.message =
                    "Filter files · type to narrow · Enter opens · Esc clears".to_string();
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
            KeyCode::Char('n') => {
                self.command_input = "newfile ".to_string();
                self.command_suggestion = 0;
                self.command_suggestion_scroll = 0;
                self.mode = Mode::Command;
            }
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
                        self.remember_recent_file();
                        self.message = match disposition {
                            OpenDisposition::Opened => {
                                format!("Opened {} in a new tab", path.display())
                            }
                            OpenDisposition::Switched => format!("Switched to {}", path.display()),
                        };
                        self.refresh_git_line_changes();
                    }
                    Err(error) => self.message = format!("Open failed: {error}"),
                }
            }
            Ok(None) => {}
            Err(error) => self.message = format!("Folder error: {error}"),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) {
        if self.handle_column_select_key(key) {
            return;
        }
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
            KeyCode::F(1) | KeyCode::Char('?') => self.open_help(),
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
            KeyCode::Char('/') => self.open_search_panel(false),
            KeyCode::Char('n') => self.repeat_search(true),
            KeyCode::Char('N') => self.repeat_search(false),
            KeyCode::Char('%') => {
                if !self.editor.jump_to_matching_bracket() {
                    self.message = "No matching bracket".to_string();
                }
            }
            KeyCode::Char(':') | KeyCode::Char(';')
                if key.code == KeyCode::Char(':')
                    || key.modifiers.contains(KeyModifiers::SHIFT) =>
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
        if self.handle_column_select_key(key) {
            return;
        }
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && self.extend_selection(key.code)
        {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
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
            KeyCode::F(1) => self.open_help(),
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
            KeyCode::Enter => self.editor.insert_newline(),
            KeyCode::Backspace => {
                self.editor.smart_backspace();
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

    /// Alt-Shift-arrows grow a rectangular selection; any other key drops the
    /// column anchor so the next rectangle starts fresh.
    fn handle_column_select_key(&mut self, key: KeyEvent) -> bool {
        let is_column_key = key.modifiers.contains(KeyModifiers::ALT)
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
            );
        if !is_column_key {
            self.column_select_origin = None;
            return false;
        }

        let origin = self.column_select_origin.unwrap_or(self.editor.cursor);
        self.column_select_origin = Some(origin);
        match key.code {
            KeyCode::Up => self.editor.move_up(),
            KeyCode::Down => self.editor.move_down(),
            KeyCode::Left => self.editor.move_left(),
            KeyCode::Right => self.editor.move_right(),
            _ => return true,
        }
        self.editor.column_select(origin);
        self.message = format!(
            "Column select: {} cursor(s) · type to edit every line",
            self.editor.secondary_cursors.len() + 1
        );
        true
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

    /// Opens the find/replace panel.  A multi-line selection scopes the
    /// search to that selection; a short selection seeds the query.
    fn open_search_panel(&mut self, focus_replace: bool) {
        self.search_origin = self.editor.cursor;
        self.search_focus_replace = focus_replace;
        self.search_history_index = None;
        self.search_input.clear();
        self.search_scope = None;
        self.search_scoped = false;
        if let Some(range) = self.editor.selection_range() {
            let selected = self.editor.selected_text().unwrap_or_default();
            if selected.contains('\n') {
                self.search_scope = Some(range);
                self.search_scoped = true;
            } else if !selected.is_empty() && selected != self.last_search {
                // Seed the query from a deliberate selection, but not from
                // the still-selected result of the previous search.
                self.search_input = selected;
            }
        }
        self.editor.clear_selection();
        self.mode = Mode::Search;
        self.message = "Find · Tab replace field · Alt-C/W/R options · F3 next · Alt-A replace all"
            .to_string();
        self.recompile_search();
        self.preview_search();
    }

    fn handle_search(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('c' | 'C') => {
                    self.search_options.case_sensitive = !self.search_options.case_sensitive;
                    self.recompile_search();
                    self.preview_search();
                }
                KeyCode::Char('w' | 'W') => {
                    self.search_options.whole_word = !self.search_options.whole_word;
                    self.recompile_search();
                    self.preview_search();
                }
                KeyCode::Char('r' | 'R') => {
                    self.search_options.use_regex = !self.search_options.use_regex;
                    self.recompile_search();
                    self.preview_search();
                }
                KeyCode::Char('s' | 'S') => {
                    if self.search_scope.is_some() {
                        self.search_scoped = !self.search_scoped;
                        self.preview_search();
                    } else {
                        self.message =
                            "Select several lines before opening search to scope it".to_string();
                    }
                }
                KeyCode::Enter => self.search_replace_current(),
                KeyCode::Char('a' | 'A') => self.search_replace_all(),
                KeyCode::Char('n' | 'N') => self.search_step(true),
                KeyCode::Char('p' | 'P') => self.search_step(false),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.editor.clear_selection();
                self.editor.cursor = self.search_origin;
                self.mode = self.preferred_editor_mode();
                self.message = "Search closed".to_string();
            }
            KeyCode::Enter => {
                if self.search_input.is_empty() {
                    self.message = "Empty search".to_string();
                } else {
                    self.last_search = self.search_input.clone();
                    self.push_search_history();
                    self.message = format!("Search: {}", self.last_search);
                }
                self.mode = self.preferred_editor_mode();
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.search_focus_replace = !self.search_focus_replace;
            }
            KeyCode::F(3) => self.search_step(!key.modifiers.contains(KeyModifiers::SHIFT)),
            KeyCode::Up => self.recall_search_history(true),
            KeyCode::Down => self.recall_search_history(false),
            KeyCode::Backspace => {
                if self.search_focus_replace {
                    self.search_replace_input.pop();
                } else {
                    self.search_input.pop();
                    self.search_history_index = None;
                    self.recompile_search();
                    self.preview_search();
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if self.search_focus_replace {
                    self.search_replace_input.push(character);
                } else {
                    self.search_input.push(character);
                    self.search_history_index = None;
                    self.recompile_search();
                    self.preview_search();
                }
            }
            _ => {}
        }
    }

    fn recompile_search(&mut self) {
        if self.search_input.is_empty() {
            self.active_search = None;
            self.search_error = None;
            return;
        }
        match CompiledSearch::compile(&self.search_input, self.search_options) {
            Ok(search) => {
                self.active_search = Some(search);
                self.search_error = None;
            }
            Err(error) => {
                self.active_search = None;
                self.search_error = Some(error);
            }
        }
    }

    fn effective_search_scope(&self) -> Option<(usize, usize)> {
        if self.search_scoped {
            self.search_scope
        } else {
            None
        }
    }

    fn preview_search(&mut self) {
        self.editor.clear_selection();
        self.editor.cursor = self.search_origin;
        let Some(search) = self.active_search.clone() else {
            return;
        };
        let scope = self.effective_search_scope();
        self.editor.find_next_match(&search, true, scope);
    }

    fn search_step(&mut self, forward: bool) {
        let Some(search) = self.active_search.clone() else {
            self.message = self
                .search_error
                .clone()
                .unwrap_or_else(|| "Type a search first".to_string());
            return;
        };
        let scope = self.effective_search_scope();
        if !self.editor.find_next_match(&search, forward, scope) {
            self.message = format!("No match: {}", self.search_input);
        }
    }

    fn search_replace_current(&mut self) {
        let Some(search) = self.active_search.clone() else {
            self.message = self
                .search_error
                .clone()
                .unwrap_or_else(|| "Type a search first".to_string());
            return;
        };
        let replacement = self.search_replace_input.clone();
        let scope = self.effective_search_scope();
        self.editor.checkpoint();
        match self
            .editor
            .replace_current_match(&search, &replacement, scope)
        {
            ReplaceOutcome::Replaced { delta } => {
                if let Some((_, end)) = self.search_scope.as_mut() {
                    *end = end.saturating_add_signed(delta);
                }
                let scope = self.effective_search_scope();
                self.editor.find_next_match(&search, true, scope);
                self.message = "Replaced 1 occurrence".to_string();
            }
            ReplaceOutcome::SelectedNext => {
                self.message = "Selected the next match · Alt-Enter again replaces it".to_string();
            }
            ReplaceOutcome::NoMatches => {
                self.message = format!("No match: {}", self.search_input);
            }
        }
    }

    fn search_replace_all(&mut self) {
        let Some(search) = self.active_search.clone() else {
            self.message = self
                .search_error
                .clone()
                .unwrap_or_else(|| "Type a search first".to_string());
            return;
        };
        let replacement = self.search_replace_input.clone();
        let scope = self.effective_search_scope();
        self.editor.checkpoint();
        let count = self
            .editor
            .replace_all_matches(&search, &replacement, scope);
        if count == 0 {
            self.message = format!("No match: {}", self.search_input);
            return;
        }
        self.last_search = self.search_input.clone();
        self.push_search_history();
        self.mode = self.preferred_editor_mode();
        self.message = format!("Replaced {count} occurrence(s) · Ctrl-Z undoes");
    }

    fn push_search_history(&mut self) {
        let query = self.search_input.clone();
        if query.is_empty() {
            return;
        }
        self.search_history.retain(|entry| *entry != query);
        self.search_history.insert(0, query);
        self.search_history.truncate(50);
        self.search_history_index = None;
    }

    fn recall_search_history(&mut self, older: bool) {
        if self.search_focus_replace || self.search_history.is_empty() {
            return;
        }
        let next_index = match (self.search_history_index, older) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(index), true) => Some((index + 1).min(self.search_history.len() - 1)),
            (Some(0), false) => None,
            (Some(index), false) => Some(index - 1),
        };
        self.search_history_index = next_index;
        if let Some(index) = next_index {
            self.search_input = self.search_history[index].clone();
        } else {
            self.search_input.clear();
        }
        self.recompile_search();
        self.preview_search();
    }

    fn repeat_search(&mut self, forward: bool) {
        let search = match self.active_search.clone() {
            Some(search) => search,
            None => {
                if self.last_search.is_empty() {
                    self.message = "No previous search".to_string();
                    return;
                }
                match CompiledSearch::compile(&self.last_search, self.search_options) {
                    Ok(search) => {
                        self.active_search = Some(search.clone());
                        search
                    }
                    Err(error) => {
                        self.message = error;
                        return;
                    }
                }
            }
        };
        if !self.editor.find_next_match(&search, forward, None) {
            self.message = format!("No match: {}", search.pattern);
        }
    }

    /// Opens the Ctrl-P fuzzy file picker over every non-ignored project
    /// file.  Recently opened files rank first.
    fn open_file_picker(&mut self) {
        const MAX_PICKER_FILES: usize = 20_000;
        let mut files = Vec::new();
        let mut truncated = false;
        let walker = ignore::WalkBuilder::new(&self.project.root)
            .sort_by_file_path(std::cmp::Ord::cmp)
            .require_git(false)
            .hidden(!self.project.show_hidden)
            .build();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if files.len() >= MAX_PICKER_FILES {
                truncated = true;
                break;
            }
            let relative = entry
                .path()
                .strip_prefix(&self.project.root)
                .unwrap_or(entry.path());
            files.push(relative.display().to_string());
        }
        self.file_picker = FilePickerState {
            files,
            truncated,
            ..FilePickerState::default()
        };
        self.mode = Mode::FilePicker;
        self.message = "Open file · type to filter · Enter opens · Esc closes".to_string();
        self.refresh_file_picker();
    }

    fn refresh_file_picker(&mut self) {
        const MAX_PICKER_MATCHES: usize = 300;
        let recent: Vec<String> = self
            .settings
            .recent_files
            .iter()
            .filter_map(|path| path.strip_prefix(&self.project.root).ok())
            .map(|relative| relative.display().to_string())
            .collect();

        let state = &mut self.file_picker;
        state.matches.clear();
        state.selected = 0;
        state.scroll = 0;

        if state.input.is_empty() {
            let mut seen = std::collections::HashSet::new();
            for name in &recent {
                if let Some(index) = state.files.iter().position(|file| file == name) {
                    if seen.insert(index) {
                        state.matches.push(PickerMatch {
                            file_index: index,
                            positions: Vec::new(),
                            recent: true,
                        });
                    }
                }
            }
            for index in 0..state.files.len() {
                if state.matches.len() >= MAX_PICKER_MATCHES {
                    break;
                }
                if !seen.contains(&index) {
                    state.matches.push(PickerMatch {
                        file_index: index,
                        positions: Vec::new(),
                        recent: false,
                    });
                }
            }
            return;
        }

        let mut scored: Vec<(i32, PickerMatch)> = state
            .files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                crate::fuzzy::match_score(file, &state.input).map(|(score, positions)| {
                    let is_recent = recent.iter().any(|name| name == file);
                    (
                        score + if is_recent { 10 } else { 0 },
                        PickerMatch {
                            file_index: index,
                            positions,
                            recent: is_recent,
                        },
                    )
                })
            })
            .collect();
        scored.sort_by(|left, right| {
            right.0.cmp(&left.0).then_with(|| {
                state.files[left.1.file_index]
                    .len()
                    .cmp(&state.files[right.1.file_index].len())
            })
        });
        state.matches = scored
            .into_iter()
            .take(MAX_PICKER_MATCHES)
            .map(|(_, matched)| matched)
            .collect();
    }

    fn handle_file_picker(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = self.preferred_editor_mode();
                self.message = "File picker closed".to_string();
            }
            KeyCode::Enter => self.open_selected_picker_file(),
            KeyCode::Up => {
                self.file_picker.selected = self.file_picker.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.file_picker.selected = (self.file_picker.selected + 1)
                    .min(self.file_picker.matches.len().saturating_sub(1));
            }
            KeyCode::PageUp => {
                self.file_picker.selected = self.file_picker.selected.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.file_picker.selected = (self.file_picker.selected + 10)
                    .min(self.file_picker.matches.len().saturating_sub(1));
            }
            KeyCode::Home => self.file_picker.selected = 0,
            KeyCode::End => {
                self.file_picker.selected = self.file_picker.matches.len().saturating_sub(1)
            }
            KeyCode::Backspace => {
                self.file_picker.input.pop();
                self.refresh_file_picker();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.file_picker.input.push(character);
                self.refresh_file_picker();
            }
            _ => {}
        }
    }

    fn open_selected_picker_file(&mut self) {
        let Some(matched) = self
            .file_picker
            .matches
            .get(self.file_picker.selected)
            .cloned()
        else {
            self.message = "No file selected".to_string();
            return;
        };
        let path = self
            .project
            .root
            .join(&self.file_picker.files[matched.file_index]);
        let before = self.current_location();
        match self.editor.open_or_switch(&path) {
            Ok(disposition) => {
                self.commit_navigation(before);
                self.explorer_focused = false;
                self.mode = self.preferred_editor_mode();
                self.search_origin = self.editor.cursor;
                self.remember_recent_file();
                self.message = match disposition {
                    OpenDisposition::Opened => format!("Opened {}", path.display()),
                    OpenDisposition::Switched => format!("Switched to {}", path.display()),
                };
                self.refresh_git_line_changes();
            }
            Err(error) => self.message = format!("Open failed: {error}"),
        }
    }

    /// Records the active file at the top of the recently-opened list.
    fn remember_recent_file(&mut self) {
        let Some(path) = self.editor.path.clone() else {
            return;
        };
        let canonical = path.canonicalize().unwrap_or(path);
        if self.settings.recent_files.first() == Some(&canonical) {
            return;
        }
        self.settings
            .recent_files
            .retain(|existing| existing != &canonical);
        self.settings.recent_files.insert(0, canonical);
        self.settings.recent_files.truncate(30);
        self.persist_settings();
    }

    /// Opens the project-wide search panel, seeding the query from a short
    /// selection.  Previous results stay visible when reopened.
    fn open_project_search(&mut self) {
        if let Some(text) = self.editor.selected_text() {
            if !text.is_empty() && !text.contains('\n') {
                self.project_search.query = text;
                self.project_search.results.clear();
                self.project_search.ran_query.clear();
            }
        }
        self.project_search.focus_replace = false;
        self.project_search.confirm_replace = false;
        self.mode = Mode::ProjectSearch;
        self.message =
            "Project search · Enter searches, then opens · Tab replace field · Alt-A replace all"
                .to_string();
        if !self.project_search.query.is_empty() && self.project_search.results.is_empty() {
            self.run_project_search();
        }
    }

    fn handle_project_search(&mut self, key: KeyEvent) {
        let is_replace_key = key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('a' | 'A'));
        if !is_replace_key {
            self.project_search.confirm_replace = false;
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('c' | 'C') => {
                    self.search_options.case_sensitive = !self.search_options.case_sensitive;
                    self.run_project_search();
                }
                KeyCode::Char('w' | 'W') => {
                    self.search_options.whole_word = !self.search_options.whole_word;
                    self.run_project_search();
                }
                KeyCode::Char('r' | 'R') => {
                    self.search_options.use_regex = !self.search_options.use_regex;
                    self.run_project_search();
                }
                KeyCode::Char('a' | 'A') => self.project_replace_all(),
                KeyCode::Enter => self.open_selected_project_match(),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.mode = self.preferred_editor_mode();
                self.message = "Project search closed".to_string();
            }
            KeyCode::Enter => {
                if self.project_search.query != self.project_search.ran_query
                    || self.project_search.results.is_empty()
                {
                    self.run_project_search();
                } else {
                    self.open_selected_project_match();
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.project_search.focus_replace = !self.project_search.focus_replace;
            }
            KeyCode::Up => {
                self.project_search.selected = self.project_search.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.project_search.selected = (self.project_search.selected + 1)
                    .min(self.project_search.results.len().saturating_sub(1));
            }
            KeyCode::PageUp => {
                self.project_search.selected = self.project_search.selected.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.project_search.selected = (self.project_search.selected + 10)
                    .min(self.project_search.results.len().saturating_sub(1));
            }
            KeyCode::Home => self.project_search.selected = 0,
            KeyCode::End => {
                self.project_search.selected = self.project_search.results.len().saturating_sub(1)
            }
            KeyCode::Delete => {
                let selected = self.project_search.selected;
                if selected < self.project_search.results.len()
                    && !self.project_search.excluded.remove(&selected)
                {
                    self.project_search.excluded.insert(selected);
                }
            }
            KeyCode::Backspace => {
                if self.project_search.focus_replace {
                    self.project_search.replacement.pop();
                } else {
                    self.project_search.query.pop();
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if self.project_search.focus_replace {
                    self.project_search.replacement.push(character);
                } else {
                    self.project_search.query.push(character);
                }
            }
            _ => {}
        }
    }

    fn run_project_search(&mut self) {
        self.project_search.results.clear();
        self.project_search.excluded.clear();
        self.project_search.selected = 0;
        self.project_search.scroll = 0;
        self.project_search.truncated = false;
        self.project_search.files_with_matches = 0;
        self.project_search.error = None;
        self.project_search.confirm_replace = false;
        self.project_search.ran_query = self.project_search.query.clone();
        if self.project_search.query.is_empty() {
            return;
        }
        let search = match CompiledSearch::compile(&self.project_search.query, self.search_options)
        {
            Ok(search) => search,
            Err(error) => {
                self.project_search.error = Some(error);
                return;
            }
        };
        let limit = self.settings.max_search_results.max(50);
        let results = crate::project_search::search(&self.project.root, &search, limit);
        self.project_search.results = results.matches;
        self.project_search.truncated = results.truncated;
        self.project_search.files_with_matches = results.files_with_matches;
    }

    fn open_selected_project_match(&mut self) {
        let Some(found) = self
            .project_search
            .results
            .get(self.project_search.selected)
            .cloned()
        else {
            self.message = "No search result selected".to_string();
            return;
        };
        let before = self.current_location();
        match self.editor.open_or_switch(&found.path) {
            Ok(_) => {
                self.commit_navigation(before);
                self.editor.goto_line(found.line);
                let column = found.line_text[..found.byte_start.min(found.line_text.len())]
                    .chars()
                    .count();
                self.editor.cursor.column =
                    column.min(self.editor.line_len_chars(self.editor.cursor.line));
                self.explorer_focused = false;
                self.mode = self.preferred_editor_mode();
                self.after_tab_switch();
                self.remember_recent_file();
                self.message = format!("{}:{}", found.path.display(), found.line + 1);
            }
            Err(error) => self.message = format!("Open failed: {error}"),
        }
    }

    fn project_replace_all(&mut self) {
        if self.project_search.query != self.project_search.ran_query {
            self.message = "Press Enter to search first, then replace".to_string();
            return;
        }
        let included: Vec<usize> = (0..self.project_search.results.len())
            .filter(|index| !self.project_search.excluded.contains(index))
            .collect();
        if included.is_empty() {
            self.message = "No matches to replace".to_string();
            return;
        }
        let files: std::collections::BTreeSet<PathBuf> = included
            .iter()
            .map(|index| self.project_search.results[*index].path.clone())
            .collect();

        if !self.project_search.confirm_replace {
            self.project_search.confirm_replace = true;
            self.message = format!(
                "Replace {} match(es) in {} file(s) on disk? Alt-A again confirms",
                included.len(),
                files.len()
            );
            return;
        }
        self.project_search.confirm_replace = false;

        let search = match CompiledSearch::compile(&self.project_search.query, self.search_options)
        {
            Ok(search) => search,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let replacement = self.project_search.replacement.clone();

        let mut replaced = 0usize;
        let mut changed_files = 0usize;
        let mut skipped_dirty = 0usize;
        let mut errors = 0usize;
        for path in files {
            if self
                .editor
                .editor_for_path_mut(&path)
                .is_some_and(|editor| editor.dirty)
            {
                skipped_dirty += 1;
                continue;
            }
            let excluded: std::collections::HashSet<(usize, usize)> = self
                .project_search
                .excluded
                .iter()
                .filter_map(|index| self.project_search.results.get(*index))
                .filter(|found| found.path == path)
                .map(|found| (found.line, found.byte_start))
                .collect();
            match crate::project_search::replace_in_file(&path, &search, &replacement, &excluded) {
                Ok(count) if count > 0 => {
                    replaced += count;
                    changed_files += 1;
                    // Refresh a clean open tab so the buffer matches disk.
                    if let Some(editor) = self.editor.editor_for_path_mut(&path) {
                        let cursor = editor.cursor;
                        if editor.reload_from_disk().is_ok() {
                            editor.goto_line(cursor.line);
                            editor.cursor.column =
                                cursor.column.min(editor.line_len_chars(cursor.line));
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => errors += 1,
            }
        }

        self.refresh_git_line_changes();
        self.project.refresh_git_status();
        let mut summary = format!("Replaced {replaced} match(es) in {changed_files} file(s)");
        if skipped_dirty > 0 {
            summary.push_str(&format!(
                " · skipped {skipped_dirty} file(s) with unsaved changes"
            ));
        }
        if errors > 0 {
            summary.push_str(&format!(" · {errors} file(s) failed"));
        }
        self.message = summary;
        self.run_project_search();
    }

    fn handle_command_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = self.preferred_editor_mode();
                self.message = "Command cancelled".to_string();
            }
            KeyCode::Enter => {
                let command = self
                    .selected_command()
                    .unwrap_or_else(|| self.command_input.trim().to_string());
                self.command_selection = None;
                self.command_anchor = None;
                self.mode = self.preferred_editor_mode();
                self.execute_command(&command);
            }
            KeyCode::Backspace => {
                if self.command_selection.is_some() {
                    self.replace_command_selection();
                } else if self.command_cursor > 0 {
                    let start = previous_char_boundary(&self.command_input, self.command_cursor);
                    self.command_input
                        .replace_range(start..self.command_cursor, "");
                    self.command_cursor = start;
                }
                self.command_suggestion = 0;
                self.command_suggestion_scroll = 0;
            }
            KeyCode::Delete => {
                if self.command_selection.is_some() {
                    self.replace_command_selection();
                } else if self.command_cursor < self.command_input.len() {
                    let end = next_char_boundary(&self.command_input, self.command_cursor);
                    self.command_input
                        .replace_range(self.command_cursor..end, "");
                }
                self.command_suggestion = 0;
                self.command_suggestion_scroll = 0;
            }
            KeyCode::Home => {
                self.command_cursor = 0;
                self.command_anchor = None;
                self.command_selection = None;
            }
            KeyCode::End => {
                self.command_cursor = self.command_input.len();
                self.command_anchor = None;
                self.command_selection = None;
            }
            KeyCode::Left => {
                self.move_command_cursor(false, key.modifiers.contains(KeyModifiers::SHIFT))
            }
            KeyCode::Right => {
                self.move_command_cursor(true, key.modifiers.contains(KeyModifiers::SHIFT))
            }
            KeyCode::Up => {
                self.move_command_suggestion(-1);
            }
            KeyCode::Down => {
                self.move_command_suggestion(1);
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.replace_command_selection();
                self.command_input.insert(self.command_cursor, character);
                self.command_cursor += character.len_utf8();
                self.command_suggestion = 0;
                self.command_suggestion_scroll = 0;
            }
            _ => {}
        }
    }

    fn move_command_suggestion(&mut self, delta: isize) {
        let last = self.command_suggestions().len().saturating_sub(1);
        self.command_suggestion = if delta.is_negative() {
            self.command_suggestion.saturating_sub(delta.unsigned_abs())
        } else {
            self.command_suggestion
                .saturating_add(delta as usize)
                .min(last)
        };
        self.ensure_command_suggestion_visible();
    }

    fn command_suggestion_hover_is_locked(&self) -> bool {
        self.command_suggestion_hover_lock_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn theme_gallery_hover_is_locked(&self) -> bool {
        self.theme_gallery_hover_lock_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn ensure_command_suggestion_visible(&mut self) {
        const VISIBLE_ROWS: usize = 8;

        let total = self.command_suggestions().len();
        if total == 0 {
            self.command_suggestion = 0;
            self.command_suggestion_scroll = 0;
            return;
        }

        self.command_suggestion = self.command_suggestion.min(total - 1);
        let rows = total.min(VISIBLE_ROWS);
        self.command_suggestion_scroll = self.command_suggestion_scroll.min(total - rows);
        if self.command_suggestion < self.command_suggestion_scroll {
            self.command_suggestion_scroll = self.command_suggestion;
        } else if self.command_suggestion >= self.command_suggestion_scroll + rows {
            self.command_suggestion_scroll = self.command_suggestion + 1 - rows;
        }
    }

    fn move_command_cursor(&mut self, forward: bool, extend: bool) {
        if !extend && self.command_selection.is_some() {
            let (start, end) = self.command_selection.take().unwrap();
            self.command_cursor = if forward { end } else { start };
            self.command_anchor = None;
            return;
        }
        if extend && self.command_anchor.is_none() {
            self.command_anchor = Some(self.command_cursor);
        }
        self.command_cursor = if forward {
            next_char_boundary(&self.command_input, self.command_cursor)
        } else {
            previous_char_boundary(&self.command_input, self.command_cursor)
        };
        self.command_selection = self.command_anchor.and_then(|anchor| {
            (anchor != self.command_cursor).then_some((
                anchor.min(self.command_cursor),
                anchor.max(self.command_cursor),
            ))
        });
        if !extend {
            self.command_anchor = None;
        }
    }

    pub fn command_suggestions(&self) -> Vec<String> {
        const COMMANDS: &[&str] = &[
            "w",
            "wa",
            "q",
            "q!",
            "wq",
            "e",
            "new",
            "tabnew",
            "tabnext",
            "tabprev",
            "tree",
            "newfile",
            "newdir",
            "rename",
            "copy",
            "move",
            "delete",
            "delete!",
            "refresh",
            "gitrefresh",
            "stage",
            "unstage",
            "diff",
            "history",
            "symbols",
            "outline",
            "split",
            "vsplit",
            "only",
            "unsplit",
            "terminal",
            "terminalfocus",
            "terminalclose",
            "plugins",
            "doctor",
            "copydiagnostics",
            "recover",
            "recovercompare",
            "discardrecovery",
            "plugin",
            "pluginreload",
            "plugindir",
            "fold",
            "foldopen",
            "foldall",
            "unfoldall",
            "format",
            "lsp",
            "lspstop",
            "complete",
            "hover",
            "definition",
            "references",
            "actions",
            "diagnostics",
            "symbolrename",
            "find",
            "replace",
            "files",
            "projectsearch",
            "grep",
            "trim",
            "splitline",
            "selectoccurrences",
            "addcursorabove",
            "addcursorbelow",
            "set formatonsave",
            "set noformatonsave",
            "set reducedmotion",
            "set noreducedmotion",
            "set number",
            "set nonumber",
            "set autoindent",
            "set noautoindent",
            "set trimonsave",
            "set notrimonsave",
            "set finalnewline=preserve",
            "set finalnewline=always",
            "set finalnewline=strip",
            "set startup=session",
            "set startup=folder",
            "set startup=empty",
            "set startup=dashboard",
            "set",
            "theme",
            "themes",
            "keymap",
            "keymaps",
            "keybindings",
            "bind",
            "unbind",
            "bindreset",
            "help",
            "welcome",
        ];
        let query = self.command_input.trim().to_ascii_lowercase();
        let mut commands = COMMANDS
            .iter()
            .map(|command| (*command).to_string())
            .collect::<Vec<_>>();
        commands.extend(
            self.plugins
                .command_names()
                .into_iter()
                .map(|name| format!("plugin {name}")),
        );
        commands
            .into_iter()
            .filter(|command| command.to_ascii_lowercase().contains(&query))
            .collect()
    }

    pub fn command_cursor(&self) -> usize {
        self.command_cursor.min(self.command_input.len())
    }

    pub fn command_selection(&self) -> Option<(usize, usize)> {
        self.command_selection
    }

    fn selected_command(&self) -> Option<String> {
        let suggestions = self.command_suggestions();
        if self.command_input.trim().is_empty()
            || suggestions
                .iter()
                .any(|value| value == self.command_input.trim())
        {
            None
        } else {
            suggestions.get(self.command_suggestion).cloned()
        }
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
                                self.remember_recent_file();
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
                if argument.is_empty() {
                    self.message = "Usage: :newfile path".to_string();
                } else {
                    self.create_project_file(&argument);
                }
            }
            "newdir" | "mkdir" => {
                if argument.is_empty() {
                    self.message = "Usage: :newdir path".to_string();
                } else {
                    self.create_project_directory(&argument);
                }
            }
            "rename" => self.rename_selected_project_entry(&argument),
            "copy" => self.duplicate_selected_project_file(&argument),
            "move" => {
                if argument.is_empty() {
                    self.message = "Usage: :move path".to_string();
                } else {
                    self.move_selected_project_entry(&argument);
                }
            }
            "delete" | "rm" => self.delete_selected_project_entry(false),
            "delete!" | "rm!" => self.delete_selected_project_entry(true),
            "refresh" | "reloadtree" => match self.project.refresh() {
                Ok(()) => {
                    self.project.refresh_git_status();
                    self.message = "File tree refreshed".to_string()
                }
                Err(error) => self.message = format!("Refresh failed: {error}"),
            },
            "gitrefresh" => {
                self.project.refresh_git_status();
                self.refresh_git_line_changes();
                self.message = "Git status and gutter refreshed".to_string();
            }
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
            "find" | "search" => {
                self.open_search_panel(false);
                if !argument.is_empty() {
                    self.search_input = argument.clone();
                    self.recompile_search();
                    self.preview_search();
                }
            }
            "replace" | "findreplace" => {
                self.open_search_panel(true);
                if !argument.is_empty() {
                    self.search_input = argument.clone();
                    self.recompile_search();
                    self.preview_search();
                }
            }
            "files" | "openfile" => self.open_file_picker(),
            "projectsearch" | "grep" | "findall" => {
                if !argument.is_empty() {
                    self.project_search.query = argument.clone();
                    self.project_search.results.clear();
                    self.project_search.ran_query.clear();
                }
                self.open_project_search();
            }
            "trim" | "trimwhitespace" => {
                self.editor.checkpoint();
                let count = self.editor.trim_trailing_whitespace();
                self.message = if count == 0 {
                    "No trailing whitespace found".to_string()
                } else {
                    format!("Trimmed trailing whitespace on {count} line(s)")
                };
            }
            "splitline" => {
                self.editor.checkpoint();
                self.editor.split_line();
                self.message = "Split line at cursor".to_string();
            }
            "selectoccurrences" | "selectallmatches" => {
                let count = self.editor.select_all_occurrences();
                self.message = if count > 1 {
                    format!("Selected {count} occurrences · type to replace them all")
                } else {
                    "Select or place the cursor on a word first".to_string()
                };
            }
            "addcursorabove" => self.add_cursor_line(false),
            "addcursorbelow" => self.add_cursor_line(true),
            "indent" => self.indent_selection(false),
            "outdent" => self.indent_selection(true),
            "comment" | "togglecomment" => self.toggle_comments(),
            "symbols" | "outline" => self.show_symbols(),
            "split" => self.open_split(false),
            "vsplit" => self.open_split(true),
            "only" | "unsplit" => {
                self.split_views = None;
                self.message = "Split closed".to_string();
            }
            "terminal" | "term" => {
                if self.terminal.is_none() {
                    self.open_terminal();
                } else {
                    self.toggle_terminal_focus();
                }
            }
            "terminalfocus" | "termfocus" => self.open_terminal(),
            "terminalclose" | "termclose" => self.close_terminal(),
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
            "welcome" | "dashboard" => {
                self.mode = Mode::Dashboard;
                self.dashboard_selected = 0;
                self.dashboard_hover = None;
                self.message =
                    "Welcome · choose a recent project or open the current folder".to_string();
            }
            "set" => self.execute_set(&argument),
            "theme" => self.execute_theme(&argument),
            "themes" | "themegallery" => self.open_theme_gallery(),
            "keymap" => self.execute_keymap(&argument),
            "keymaps" | "keymapgallery" => self.open_keymap_gallery(),
            "keybindings" | "binds" | "shortcuts" => self.open_key_browser(),
            "bind" => {
                let mut values = argument.split_whitespace();
                match (values.next(), values.next()) {
                    (Some(action_id), Some(chord_text)) => self.execute_bind(action_id, chord_text),
                    _ => self.message =
                        "Usage: :bind <action> <keys>, e.g. :bind find ctrl+g — see :keybindings"
                            .to_string(),
                }
            }
            "unbind" => {
                if argument.is_empty() {
                    self.message = "Usage: :unbind <action> restores its default".to_string();
                } else {
                    self.execute_bind(argument.trim(), "default");
                }
            }
            "bindreset" => {
                let count = self.settings.custom_keys.len();
                self.settings.custom_keys.clear();
                self.rebuild_keys();
                self.persist_settings();
                self.message = format!("Reset {count} custom binding(s) to the defaults");
            }
            "plugins" => {
                let errors = self.plugins.errors().len();
                self.message = if errors == 0 {
                    self.plugins.summary()
                } else {
                    format!(
                        "{} · {errors} plugin error(s); see :plugindir",
                        self.plugins.summary()
                    )
                };
            }
            "plugindir" => self.message = config::plugins_dir().display().to_string(),
            "pluginreload" => {
                self.plugins = PluginRegistry::load(&config::plugins_dir());
                self.message = format!(
                    "Reloaded {} plugin(s) · {} error(s)",
                    self.plugins.count(),
                    self.plugins.errors().len()
                );
            }
            "plugin" => {
                let mut values = argument.split_whitespace();
                let Some(name) = values.next() else {
                    self.message = "Usage: :plugin command [args]".to_string();
                    return;
                };
                self.run_plugin_command(name, values.map(str::to_string).collect(), "command");
            }
            "config" => self.message = config::config_path().display().to_string(),
            "doctor" => self.show_diagnostic_report(),
            "copydiagnostics" => self.copy_diagnostic_report(),
            "recover" => match crate::recovery::load() {
                Ok(entries) if entries.is_empty() => {
                    self.message = "No recovery snapshots available".to_string()
                }
                Ok(entries) => {
                    let index = argument.parse::<usize>().unwrap_or(1).saturating_sub(1);
                    let Some(entry) = entries.get(index) else {
                        self.message = "Recovery snapshot index is out of range".to_string();
                        return;
                    };
                    self.editor.replace_text(&entry.text);
                    self.editor.cursor = Cursor {
                        line: entry.cursor_line,
                        column: entry.cursor_column,
                    };
                    self.message = format!(
                        "Recovered {} from {}",
                        entry
                            .path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "untitled buffer".to_string()),
                        entry.saved_unix_secs
                    );
                }
                Err(error) => self.message = format!("Recovery load failed: {error}"),
            },
            "recovercompare" => match crate::recovery::load() {
                Ok(entries) => {
                    let index = argument.parse::<usize>().unwrap_or(1).saturating_sub(1);
                    let Some(entry) = entries.get(index) else {
                        self.message = "Recovery snapshot index is out of range".to_string();
                        return;
                    };
                    let current = self.editor.text();
                    self.git_diff_lines = recovery_diff_lines(&current, &entry.text);
                    self.git_diff_scroll = 0;
                    self.git_diff_title = format!("RECOVERY SNAPSHOT {}", index + 1);
                    self.git_diff_return_mode = self.preferred_editor_mode();
                    self.mode = Mode::GitDiff;
                }
                Err(error) => self.message = format!("Recovery load failed: {error}"),
            },
            "discardrecovery" => match crate::recovery::discard() {
                Ok(()) => self.message = "Recovery snapshots discarded".to_string(),
                Err(error) => self.message = format!("Could not discard recovery: {error}"),
            },
            "lsp" => self.start_lsp(),
            "lspstop" => {
                self.lsp = None;
                self.lsp_status = LspStatus::Off;
                self.lsp_started_at = None;
                self.diagnostic_count = 0;
                self.message = "LSP stopped".to_string();
            }
            "hover" => self.request_hover(),
            "complete" => self.request_lsp_position("textDocument/completion"),
            "definition" | "def" => self.request_lsp_position("textDocument/definition"),
            "references" | "refs" => self.request_lsp_position("textDocument/references"),
            "actions" => self.request_lsp_position("textDocument/codeAction"),
            "diagnostics" => self.open_diagnostics_panel(),
            "symbolrename" | "lsprename" => {
                if argument.is_empty() {
                    self.message =
                        "Usage: :symbolrename newName (or press F2 in the editor)".to_string();
                } else {
                    self.request_lsp_rename(&argument);
                }
            }
            "format" => self.request_formatting(),
            "help" | "h" => self.open_help(),
            _ => self.message = format!("Unknown command: {command}"),
        }
    }

    fn change_project_root(&mut self, path: PathBuf) {
        match self.project.set_root(path) {
            Ok(()) => {
                self.project.visible = true;
                self.explorer_focused = true;
                self.mode = Mode::Normal;
                self.remember_current_project();
                self.message = format!("Project: {}", self.project.root.display());
            }
            Err(error) => self.message = format!("Cannot open folder: {error}"),
        }
    }

    pub fn recent_projects(&self) -> &[PathBuf] {
        &self.settings.recent_projects
    }

    fn remember_current_project(&mut self) {
        let project = self
            .project
            .root
            .canonicalize()
            .unwrap_or_else(|_| self.project.root.clone());
        self.settings.recent_projects.retain(|existing| {
            existing.canonicalize().unwrap_or_else(|_| existing.clone()) != project
        });
        self.settings.recent_projects.insert(0, project);
        self.settings.recent_projects.truncate(10);
        self.persist_settings();
    }

    fn open_selected_recent_project(&mut self) {
        if self.settings.recent_projects.is_empty() {
            self.open_current_folder_from_dashboard();
            return;
        }
        let index = self
            .dashboard_selected
            .min(self.settings.recent_projects.len() - 1);
        let path = self.settings.recent_projects[index].clone();
        if !path.is_dir() {
            self.settings.recent_projects.remove(index);
            self.dashboard_selected = self
                .dashboard_selected
                .min(self.settings.recent_projects.len().saturating_sub(1));
            self.persist_settings();
            self.message = format!("Removed missing project: {}", path.display());
            return;
        }
        self.change_project_root(path);
    }

    fn open_current_folder_from_dashboard(&mut self) {
        self.mode = Mode::Normal;
        self.project.visible = true;
        self.explorer_focused = true;
        self.remember_current_project();
        self.message = format!("Project: {}", self.project.root.display());
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
                self.settings.show_line_numbers = true;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Line numbers enabled".to_string();
            }
            "nonumber" | "nonu" => {
                self.settings.show_line_numbers = false;
                self.apply_editor_settings();
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
            "reducedmotion" | "reduce-motion" => {
                self.settings.reduced_motion = true;
                self.persist_settings();
                self.message = "Reduced motion enabled".to_string();
            }
            "noreducedmotion" | "no-reduce-motion" => {
                self.settings.reduced_motion = false;
                self.persist_settings();
                self.message = "Reduced motion disabled".to_string();
            }
            "autoindent" | "ai" => {
                self.settings.auto_indent = true;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Auto-indent enabled".to_string();
            }
            "noautoindent" | "noai" => {
                self.settings.auto_indent = false;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Auto-indent disabled".to_string();
            }
            "trimonsave" => {
                self.settings.trim_trailing_whitespace_on_save = true;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Trailing whitespace is trimmed on save".to_string();
            }
            "notrimonsave" => {
                self.settings.trim_trailing_whitespace_on_save = false;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Trailing whitespace is kept on save".to_string();
            }
            value if value.starts_with("finalnewline=") => {
                let choice = value.split_once('=').map(|(_, value)| value);
                let policy = match choice {
                    Some("preserve") => Some(FinalNewline::Preserve),
                    Some("always") => Some(FinalNewline::Always),
                    Some("strip") => Some(FinalNewline::Strip),
                    _ => None,
                };
                match policy {
                    Some(policy) => {
                        self.settings.final_newline = policy;
                        self.apply_editor_settings();
                        self.persist_settings();
                        self.message = format!("Final newline on save: {}", policy.name());
                    }
                    None => {
                        self.message =
                            "Final newline must be preserve, always, or strip".to_string()
                    }
                }
            }
            value if value.starts_with("undolimit=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());
                match number {
                    Some(number @ 10..=100_000) => {
                        self.settings.undo_history_limit = number;
                        self.apply_editor_settings();
                        self.persist_settings();
                        self.message = format!("Undo history limit: {number} steps");
                    }
                    _ => {
                        self.message = "Undo limit must be between 10 and 100000 steps".to_string()
                    }
                }
            }
            value if value.starts_with("tabstop=") || value.starts_with("ts=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());

                match number {
                    Some(number @ 1..=16) => {
                        self.settings.tab_width = number;
                        self.apply_editor_settings();
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
            value if value.starts_with("startup=") => {
                let choice = value.split_once('=').map(|(_, value)| value);
                let view = match choice {
                    Some("session") => Some(StartupView::Session),
                    Some("folder") => Some(StartupView::Folder),
                    Some("empty") => Some(StartupView::Empty),
                    Some("dashboard") => Some(StartupView::Dashboard),
                    _ => None,
                };
                match view {
                    Some(view) => {
                        self.settings.startup = view;
                        self.persist_settings();
                        self.message = format!(
                            "Startup view: {} (applies on next launch)",
                            choice.unwrap_or_default()
                        );
                    }
                    None => {
                        self.message =
                            "Startup must be session, folder, empty, or dashboard".to_string()
                    }
                }
            }
            _ => {
                self.message =
                    "Try :set number, :set startup=folder, :set formatonsave, or :set tabstop=4"
                        .to_string()
            }
        }
    }

    fn start_lsp(&mut self) {
        self.lsp = None;
        self.lsp_panel = None;
        self.lsp_status = LspStatus::Off;
        self.diagnostic_count = 0;
        self.diagnostics.clear();
        self.lsp_open_path = None;
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
                self.lsp_status = LspStatus::Starting;
                self.lsp_started_at = Some(Instant::now());
                self.last_background_animation = Instant::now();
                self.lsp_initialized = false;
                // csharp-ls may keep reporting workspace progress indefinitely
                // for a standalone file. Initialization is sufficient for
                // position and formatting requests, so never block on it.
                self.lsp_workspace_ready = true;
                self.lsp_last_text.clear();
                self.message = format!("Starting LSP: {server}");
            }
            Err(error) => {
                self.lsp_status = LspStatus::Error;
                self.message = format!("Could not start {server}: {error}");
            }
        }
    }

    fn handle_lsp_panel_key(&mut self, key: KeyEvent) {
        let Some(panel) = self.lsp_panel.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.lsp_panel = None;
                self.message = "LSP results closed".to_string();
            }
            KeyCode::Up | KeyCode::Char('k') => panel.selected = panel.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                panel.selected = (panel.selected + 1).min(panel.items.len().saturating_sub(1))
            }
            KeyCode::PageUp => panel.selected = panel.selected.saturating_sub(8),
            KeyCode::PageDown => {
                panel.selected = (panel.selected + 8).min(panel.items.len().saturating_sub(1))
            }
            KeyCode::Home => panel.selected = 0,
            KeyCode::End => panel.selected = panel.items.len().saturating_sub(1),
            KeyCode::Enter => self.activate_lsp_panel_item(),
            _ => {}
        }
    }

    fn activate_lsp_panel_item(&mut self) {
        let Some(panel) = self.lsp_panel.take() else {
            return;
        };
        let Some(item) = panel.items.get(panel.selected).cloned() else {
            return;
        };
        match panel.kind {
            LspPanelKind::Completion => self.apply_lsp_completion(&item.payload),
            LspPanelKind::Locations | LspPanelKind::Diagnostics => {
                self.open_lsp_location(&item.payload)
            }
            LspPanelKind::CodeActions => self.apply_lsp_code_action(&item.payload),
            LspPanelKind::Hover => self.message = "Hover closed".to_string(),
        }
    }

    fn open_diagnostics_panel(&mut self) {
        if self.diagnostics.is_empty() {
            self.message = "No LSP diagnostics for this file".to_string();
            return;
        }
        let items = self
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let severity = match diagnostic["diagnostic"]["severity"].as_u64() {
                    Some(1) => "Error",
                    Some(2) => "Warning",
                    Some(3) => "Info",
                    Some(4) => "Hint",
                    _ => "Diagnostic",
                };
                let line = diagnostic["diagnostic"]["range"]["start"]["line"]
                    .as_u64()
                    .unwrap_or(0)
                    + 1;
                LspPanelItem {
                    label: format!("{severity} · line {line}"),
                    detail: diagnostic["diagnostic"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .replace('\n', " "),
                    payload: diagnostic.clone(),
                }
            })
            .collect();
        self.lsp_panel = Some(LspPanel {
            title: format!("Diagnostics ({})", self.diagnostics.len()),
            items,
            selected: 0,
            kind: LspPanelKind::Diagnostics,
        });
        self.message = "↑↓ select · Enter jumps · Esc closes".to_string();
    }

    fn request_lsp_rename(&mut self, new_name: &str) {
        if !self.lsp_initialized {
            self.message = "Start LSP first with :lsp and wait until it is ready".to_string();
            return;
        }
        let Some(path) = self.editor.path.as_ref() else {
            self.message = "Save the buffer before renaming a symbol".to_string();
            return;
        };
        let character = self.lsp_cursor_character();
        let params = json!({
            "textDocument": { "uri": lsp::file_uri(path) },
            "position": { "line": self.editor.cursor.line, "character": character },
            "newName": new_name
        });
        match self
            .lsp
            .as_mut()
            .ok_or_else(|| "Start LSP first with :lsp".to_string())
            .and_then(|client| {
                client
                    .request("textDocument/rename", params)
                    .map_err(|error| error.to_string())
            }) {
            Ok(id) => {
                self.lsp_requests
                    .insert(id, "textDocument/rename".to_string());
                self.message = format!("Renaming symbol to {new_name}…");
            }
            Err(error) => self.message = error,
        }
    }

    fn apply_lsp_completion(&mut self, item: &Value) {
        let replacement = item["textEdit"]["newText"]
            .as_str()
            .or_else(|| item["insertText"].as_str())
            .or_else(|| item["label"].as_str())
            .unwrap_or_default();
        let replacement = if item["insertTextFormat"].as_u64() == Some(2) {
            strip_lsp_snippet(replacement)
        } else {
            replacement.to_string()
        };
        let range = item["textEdit"]
            .get("range")
            .or_else(|| item["textEdit"].get("replace"));
        if let Some(range) = range {
            let edit = json!({ "range": range, "newText": replacement });
            if let Some(text) =
                apply_lsp_text_edits(&self.editor.text(), std::slice::from_ref(&edit))
            {
                self.editor.replace_text(&text);
            }
        } else {
            self.editor.checkpoint();
            self.editor.delete_selection();
            self.editor.insert_text(&replacement);
        }
        self.message = format!(
            "Completed: {}",
            item["label"].as_str().unwrap_or(&replacement)
        );
    }

    fn open_lsp_location(&mut self, payload: &Value) {
        let (uri, range) = if payload.get("diagnostic").is_some() {
            (payload["uri"].as_str(), &payload["diagnostic"]["range"])
        } else {
            let range = if payload["range"].is_object() {
                &payload["range"]
            } else {
                &payload["targetSelectionRange"]
            };
            (
                payload["uri"]
                    .as_str()
                    .or_else(|| payload["targetUri"].as_str()),
                range,
            )
        };
        let (Some(uri), Some(path)) = (uri, uri.and_then(lsp::path_from_uri)) else {
            self.message = "LSP result has no usable file location".to_string();
            return;
        };
        let line = range["start"]["line"].as_u64().unwrap_or(0) as usize;
        let column = range["start"]["character"].as_u64().unwrap_or(0) as usize;
        let before = self.current_location();
        match self.editor.open_or_switch(&path) {
            Ok(_) => {
                self.commit_navigation(before);
                self.editor.goto_line(line);
                self.editor.cursor.column = lsp_cursor_column(&self.editor.text(), line, column)
                    .unwrap_or(0)
                    .min(self.editor.line_len_chars(line));
                self.explorer_focused = false;
                self.mode = self.preferred_editor_mode();
                self.after_tab_switch();
                self.message = format!("{} · line {}", path.display(), line + 1);
            }
            Err(error) => self.message = format!("Cannot open {uri}: {error}"),
        }
    }

    fn apply_lsp_code_action(&mut self, action: &Value) {
        let mut applied = 0;
        if action["edit"].is_object() {
            match self.apply_workspace_edit(&action["edit"]) {
                Ok(count) => applied = count,
                Err(error) => {
                    self.message = error;
                    return;
                }
            }
        }
        let command = action
            .get("command")
            .filter(|command| command.is_object())
            .unwrap_or(action);
        if command["command"].is_string() {
            let params = json!({ "command": command["command"], "arguments": command["arguments"].as_array().cloned().unwrap_or_default() });
            match self
                .lsp
                .as_mut()
                .and_then(|client| client.request("workspace/executeCommand", params).ok())
            {
                Some(id) => {
                    self.lsp_requests
                        .insert(id, "workspace/executeCommand".to_string());
                    self.message = format!(
                        "Running code action: {}",
                        action["title"].as_str().unwrap_or("command")
                    );
                }
                None => self.message = "Could not execute the LSP code action".to_string(),
            }
        } else {
            self.message = format!("Applied code action to {applied} file(s)");
        }
    }

    fn apply_workspace_edit(&mut self, edit: &Value) -> Result<usize, String> {
        let mut documents = Vec::<(String, Vec<Value>)>::new();
        if let Some(changes) = edit["changes"].as_object() {
            for (uri, edits) in changes {
                documents.push((uri.clone(), edits.as_array().cloned().unwrap_or_default()));
            }
        }
        if let Some(changes) = edit["documentChanges"].as_array() {
            for change in changes {
                if let (Some(uri), Some(edits)) = (
                    change["textDocument"]["uri"].as_str(),
                    change["edits"].as_array(),
                ) {
                    documents.push((uri.to_string(), edits.clone()));
                }
            }
        }
        let mut applied = 0;
        for (uri, edits) in documents {
            let path =
                lsp::path_from_uri(&uri).ok_or_else(|| format!("Unsupported LSP URI: {uri}"))?;
            if let Some(editor) = self.editor.editor_for_path_mut(&path) {
                let text = apply_lsp_text_edits(&editor.text(), &edits)
                    .ok_or_else(|| format!("Invalid text edits for {}", path.display()))?;
                editor.replace_text(&text);
            } else {
                let text = fs::read_to_string(&path)
                    .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
                let text = apply_lsp_text_edits(&text, &edits)
                    .ok_or_else(|| format!("Invalid text edits for {}", path.display()))?;
                fs::write(&path, text)
                    .map_err(|error| format!("Could not update {}: {error}", path.display()))?;
            }
            applied += 1;
        }
        let _ = self.project.refresh();
        self.project.refresh_git_status();
        self.refresh_git_line_changes();
        Ok(applied)
    }

    fn request_hover(&mut self) {
        if self.lsp.is_none() {
            self.message = "Start LSP first with :lsp".to_string();
            return;
        }
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
        let character = self.lsp_cursor_character();
        let lsp = self.lsp.as_mut().expect("LSP availability checked");
        match lsp.request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": lsp::file_uri(path) },
                "position": { "line": self.editor.cursor.line, "character": character }
            }),
        ) {
            Ok(id) => {
                self.lsp_requests.insert(id, "hover".to_string());
                self.message = "Hover requested".to_string();
            }
            Err(error) => self.message = format!("Hover request failed: {error}"),
        }
    }

    fn sync_lsp_document(&mut self) {
        if self.editor.path != self.lsp_open_path {
            self.sync_lsp_active_file();
        }
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

    fn sync_lsp_active_file(&mut self) {
        if !self.lsp_initialized {
            return;
        }
        let next = self.editor.path.clone();
        if next == self.lsp_open_path {
            return;
        }
        let Some(lsp) = &self.lsp else { return };
        if let Some(previous) = self.lsp_open_path.take() {
            let _ = lsp.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": lsp::file_uri(&previous) } }),
            );
        }
        if let Some(path) = next {
            let text = self.editor.text();
            let language_id = lsp_language_id(Language::from_path(Some(&path)));
            self.lsp_version = 1;
            if lsp.notify("textDocument/didOpen", json!({ "textDocument": { "uri": lsp::file_uri(&path), "languageId": language_id, "version": 1, "text": text } })).is_ok() {
                self.lsp_last_text = self.editor.text();
                self.lsp_open_path = Some(path);
            }
        }
    }

    fn request_lsp_position(&mut self, method: &str) {
        if !self.lsp_initialized {
            self.message = "Start LSP first with :lsp and wait until it is ready".to_string();
            return;
        }
        if !self.lsp_workspace_ready {
            self.message = "LSP is still loading the workspace".to_string();
            return;
        }
        let Some(path) = self.editor.path.as_ref() else {
            self.message = "Save the buffer before using LSP".to_string();
            return;
        };
        let character = self.lsp_cursor_character();
        let mut params = json!({
            "textDocument": { "uri": lsp::file_uri(path) },
            "position": { "line": self.editor.cursor.line, "character": character }
        });
        if method == "textDocument/references" {
            params["context"] = json!({ "includeDeclaration": true });
        } else if method == "textDocument/codeAction" {
            params["range"] = json!({
                "start": { "line": self.editor.cursor.line, "character": character },
                "end": { "line": self.editor.cursor.line, "character": character }
            });
            params["context"] = json!({ "diagnostics": self.diagnostics.iter().map(|value| value["diagnostic"].clone()).collect::<Vec<_>>() });
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
        if !self.lsp_initialized {
            self.message = "Start LSP first with :lsp and wait until it is ready".to_string();
            return;
        }
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
                Some("begin") => {
                    self.lsp_status = LspStatus::Loading;
                    self.message = "LSP loading workspace…".to_string();
                }
                Some("end") => {
                    self.lsp_workspace_ready = true;
                    self.lsp_status = LspStatus::Ready;
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
            if method == "workspace/applyEdit" {
                let id = id.clone();
                let result = match self.apply_workspace_edit(&message["params"]["edit"]) {
                    Ok(_) => json!({ "applied": true }),
                    Err(error) => json!({ "applied": false, "failureReason": error }),
                };
                if let Some(lsp) = &self.lsp {
                    let _ = lsp.respond(&id, result);
                }
                return;
            }
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
            if message.get("error").is_some() {
                self.lsp_status = LspStatus::Error;
                self.message = format!("LSP initialization failed: {}", message["error"]);
                return;
            }
            let Some(lsp) = &self.lsp else { return };
            if lsp.notify("initialized", json!({})).is_err() {
                self.lsp_status = LspStatus::Error;
                self.message = "LSP initialization failed".to_string();
                return;
            }
            let Some(path) = self.editor.path.as_ref() else {
                return;
            };
            let language_id = lsp_language_id(crate::syntax::Language::from_path(Some(path)));
            if lsp.notify("textDocument/didOpen", json!({ "textDocument": { "uri": lsp::file_uri(path), "languageId": language_id, "version": 1, "text": self.editor.text() } })).is_ok() {
                self.lsp_initialized = true;
                self.lsp_status = LspStatus::Ready;
                self.lsp_last_text = self.editor.text();
                self.lsp_open_path = Some(path.clone());
                self.message = "LSP ready".to_string();
            }
            return;
        }
        if request.is_some() && message.get("error").is_some() {
            self.message = format!("LSP request failed: {}", message["error"]);
            return;
        }
        if request.as_deref() == Some("textDocument/definition") {
            let location = message["result"]
                .as_array()
                .and_then(|locations| locations.first())
                .unwrap_or(&message["result"]);
            if location.is_object() {
                self.open_lsp_location(location);
            } else {
                self.message = "No definition found at cursor".to_string();
            }
            return;
        }
        if request.as_deref() == Some("textDocument/completion") {
            let values = message["result"]
                .as_array()
                .or_else(|| message["result"]["items"].as_array())
                .cloned()
                .unwrap_or_default();
            let items = values
                .into_iter()
                .map(|item| LspPanelItem {
                    label: item["label"].as_str().unwrap_or("completion").to_string(),
                    detail: item["detail"]
                        .as_str()
                        .or_else(|| item["documentation"]["value"].as_str())
                        .unwrap_or_default()
                        .replace('\n', " "),
                    payload: item,
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                self.message = "No completions at cursor".to_string();
            } else {
                self.lsp_panel = Some(LspPanel {
                    title: format!("Completions ({})", items.len()),
                    items,
                    selected: 0,
                    kind: LspPanelKind::Completion,
                });
                self.message = "↑↓ select · Enter inserts · Esc closes".to_string();
            }
            return;
        }
        if request.as_deref() == Some("hover") {
            let text = lsp_hover_text(&message["result"]);
            let items = text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| LspPanelItem {
                    label: line.to_string(),
                    detail: String::new(),
                    payload: Value::Null,
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                self.message = "No hover information at cursor".to_string();
            } else {
                self.lsp_panel = Some(LspPanel {
                    title: "Hover".to_string(),
                    items,
                    selected: 0,
                    kind: LspPanelKind::Hover,
                });
                self.message = "Esc closes hover".to_string();
            }
            return;
        }
        if request.as_deref() == Some("textDocument/references") {
            let values = message["result"].as_array().cloned().unwrap_or_default();
            let items = values
                .into_iter()
                .map(|location| {
                    let uri = location["uri"].as_str().unwrap_or_default();
                    let path = lsp::path_from_uri(uri).unwrap_or_default();
                    let line = location["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
                    LspPanelItem {
                        label: format!(
                            "{}:{line}",
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(uri)
                        ),
                        detail: path.display().to_string(),
                        payload: location,
                    }
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                self.message = "No references found".to_string();
            } else {
                self.lsp_panel = Some(LspPanel {
                    title: format!("References ({})", items.len()),
                    items,
                    selected: 0,
                    kind: LspPanelKind::Locations,
                });
                self.message = "↑↓ select · Enter jumps · Esc closes".to_string();
            }
            return;
        }
        if request.as_deref() == Some("textDocument/codeAction") {
            let values = message["result"].as_array().cloned().unwrap_or_default();
            let items = values
                .into_iter()
                .map(|action| LspPanelItem {
                    label: action["title"]
                        .as_str()
                        .unwrap_or("Code action")
                        .to_string(),
                    detail: action["kind"].as_str().unwrap_or_default().to_string(),
                    payload: action,
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                self.message = "No code actions at cursor".to_string();
            } else {
                self.lsp_panel = Some(LspPanel {
                    title: format!("Code actions ({})", items.len()),
                    items,
                    selected: 0,
                    kind: LspPanelKind::CodeActions,
                });
                self.message = "↑↓ select · Enter applies · Esc closes".to_string();
            }
            return;
        }
        if request.as_deref() == Some("textDocument/rename") {
            match self.apply_workspace_edit(&message["result"]) {
                Ok(count) => self.message = format!("Renamed symbol in {count} file(s)"),
                Err(error) => self.message = error,
            }
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
            let uri = message["params"]["uri"].as_str().unwrap_or_default();
            self.diagnostics
                .retain(|value| value["uri"].as_str() != Some(uri));
            self.diagnostics.extend(
                message["params"]["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|diagnostic| json!({ "uri": uri, "diagnostic": diagnostic })),
            );
            self.diagnostic_count = self.diagnostics.len();
            self.message = format!(
                "LSP diagnostics: {} · :diagnostics to inspect",
                self.diagnostic_count
            );
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
        let Some(edits) = result.as_array() else {
            return false;
        };
        let mut text = self.editor.text();
        let mut changes = Vec::new();
        for edit in edits {
            let Some(replacement) = edit["newText"].as_str() else {
                continue;
            };
            let range = &edit["range"];
            let Some(start_line) = range["start"]["line"].as_u64() else {
                continue;
            };
            let Some(start_character) = range["start"]["character"].as_u64() else {
                continue;
            };
            let Some(end_line) = range["end"]["line"].as_u64() else {
                continue;
            };
            let Some(end_character) = range["end"]["character"].as_u64() else {
                continue;
            };
            let Some(start) = lsp_text_offset(&text, start_line as usize, start_character as usize)
            else {
                continue;
            };
            let Some(end) = lsp_text_offset(&text, end_line as usize, end_character as usize)
            else {
                continue;
            };
            changes.push((start, end, replacement.to_string()));
        }
        changes.sort_by_key(|(start, _, _)| *start);
        if changes.is_empty() {
            return false;
        }
        for (start, end, replacement) in changes.into_iter().rev() {
            if start <= end && end <= text.len() {
                text.replace_range(start..end, &replacement);
            }
        }
        if text == self.editor.text() {
            return false;
        }
        self.editor.replace_text(&text);
        true
    }

    fn plugin_context(&self, arguments: Vec<String>, event: &str) -> PluginContext {
        PluginContext {
            project: self.project.root.display().to_string(),
            file: self
                .editor
                .path
                .as_ref()
                .map(|path| path.display().to_string()),
            language: self.language_name(),
            text: self.editor.text(),
            selection: self.editor.selected_text(),
            cursor_line: self.editor.cursor.line,
            cursor_column: self.editor.cursor.column,
            arguments,
            event: event.to_string(),
        }
    }

    fn run_plugin_command(&mut self, name: &str, arguments: Vec<String>, event: &str) {
        let context = self.plugin_context(arguments, event);
        match self.plugins.run(name, &context) {
            Ok(response) => self.apply_plugin_response(response),
            Err(error) => self.message = error,
        }
    }

    fn apply_plugin_response(&mut self, response: PluginResponse) {
        if let Some(text) = response.replace_document {
            self.editor.replace_text(&text);
        } else if let Some(text) = response.replace_selection {
            if self.editor.selection_range().is_some() {
                self.editor.checkpoint();
                self.editor.delete_selection();
                self.editor.insert_text(&text);
            } else {
                self.message =
                    "Plugin requested replace_selection, but nothing is selected".to_string();
            }
        } else if let Some(text) = response.insert_text {
            self.editor.checkpoint();
            self.editor.delete_selection();
            self.editor.insert_text(&text);
        }

        if let Some(path) = response.open {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                self.project.root.join(path)
            };
            match self.editor.open_or_switch(&path) {
                Ok(_) => {
                    self.explorer_focused = false;
                    self.mode = self.preferred_editor_mode();
                    self.after_tab_switch();
                }
                Err(error) => {
                    self.message = format!("Plugin could not open {}: {error}", path.display())
                }
            }
        }
        if let Some(message) = response.message {
            self.message = message;
        }
    }

    fn run_save_hooks(&mut self) -> Result<usize, String> {
        let commands = self.plugins.on_save_commands();
        for command in &commands {
            let context = self.plugin_context(Vec::new(), "on_save");
            let response = self.plugins.run(command, &context)?;
            self.apply_plugin_response(response);
        }
        if self.editor.dirty {
            self.editor
                .save()
                .map_err(|error| format!("Plugin save hook failed to save changes: {error}"))?;
        }
        Ok(commands.len())
    }

    fn execute_theme(&mut self, argument: &str) {
        if let Some(theme) = self.plugins.theme(argument) {
            self.theme = theme;
            self.active_custom_theme = Some(argument.to_string());
            self.settings.custom_theme = Some(argument.to_string());
            self.persist_settings();
            self.message = format!("Theme: {argument} (plugin)");
            return;
        }
        let normalized = argument.trim().to_ascii_lowercase();
        let kind = if normalized == "gallery" {
            self.open_theme_gallery();
            return;
        } else if let Some(kind) = ThemeKind::parse(&normalized) {
            kind
        } else {
            let custom = self.plugins.theme_names();
            let built_in = ThemeKind::ALL
                .iter()
                .map(|kind| kind.name())
                .collect::<Vec<_>>()
                .join(", ");
            self.message = if custom.is_empty() {
                format!("Themes: {built_in}, gallery")
            } else {
                format!(
                    "Themes: {built_in}, gallery · plugins: {}",
                    custom.join(", ")
                )
            };
            return;
        };

        self.theme_kind = kind;
        self.theme = Theme::for_kind(kind);
        self.active_custom_theme = None;
        self.settings.theme = kind;
        self.settings.custom_theme = None;
        self.persist_settings();
        self.message = format!("Theme: {argument}");
    }

    fn open_theme_gallery(&mut self) {
        self.theme_gallery_return_mode = if self.mode == Mode::Command {
            self.preferred_editor_mode()
        } else {
            self.mode
        };
        self.theme_gallery_original = self.theme_kind;
        self.theme_gallery_original_theme = self.theme;
        self.theme_gallery_original_custom = self.active_custom_theme.clone();
        self.theme_gallery_selected = ThemeKind::ALL
            .iter()
            .position(|kind| *kind == self.theme_kind)
            .unwrap_or(0);
        self.theme_gallery_hover_lock_until = None;
        self.mode = Mode::ThemeGallery;
        self.preview_gallery_theme();
    }

    fn preview_gallery_theme(&mut self) {
        self.theme_kind = ThemeKind::ALL[self.theme_gallery_selected];
        self.theme = Theme::for_kind(self.theme_kind);
    }

    pub fn keymap_profile(&self) -> KeymapProfile {
        self.settings.keymap
    }

    fn preferred_editor_mode(&self) -> Mode {
        match self.settings.keymap {
            KeymapProfile::Vim => Mode::Normal,
            KeymapProfile::Caret | KeymapProfile::Conventional => Mode::Insert,
        }
    }

    fn open_keymap_gallery(&mut self) {
        self.keymap_gallery_return_mode = if self.mode == Mode::Command {
            self.preferred_editor_mode()
        } else {
            self.mode
        };
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
        self.mode = self.keymap_gallery_return_mode;
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
        // Tests must never overwrite the user's real configuration file.
        if cfg!(test) {
            return;
        }
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
                let hooks = match self.run_save_hooks() {
                    Ok(count) => count,
                    Err(error) => {
                        self.message = error;
                        return;
                    }
                };
                let _ = self.project.refresh();
                self.project.refresh_git_status();
                self.refresh_git_line_changes();
                self.message = if hooks == 0 {
                    format!("Saved {}", path.display())
                } else {
                    format!("Saved {} · ran {hooks} plugin hook(s)", path.display())
                };
                self.request_formatting_after_save();
            }
            Err(error) => self.message = format!("Save failed: {error}"),
        }
    }

    fn save_internal(&mut self) -> bool {
        if self.editor.has_pending_external_change() {
            self.pending_save_after_disk_change = true;
            self.mode = Mode::ReloadConfirm;
            self.message = "Disk changes were kept — [R] Reload   [K] Overwrite with current buffer   [C] Compare   [Esc] Cancel".to_string();
            return false;
        }
        match self.editor.save() {
            Ok(()) => {
                let hooks = match self.run_save_hooks() {
                    Ok(count) => count,
                    Err(error) => {
                        self.message = error;
                        return false;
                    }
                };
                let _ = self.project.refresh();
                self.project.refresh_git_status();
                self.refresh_git_line_changes();
                let name = self
                    .editor
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                self.message = if hooks == 0 {
                    format!("Saved {name}")
                } else {
                    format!("Saved {name} · ran {hooks} plugin hook(s)")
                };
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

fn parse_git_hunks(diff: &str, line_count: usize) -> HashMap<usize, GitLineChange> {
    let mut changes = HashMap::new();
    if line_count == 0 {
        return changes;
    }
    for line in diff.lines().filter(|line| line.starts_with("@@ ")) {
        let Some(header) = line.split(" @@").next() else {
            continue;
        };
        let mut ranges = header.split_whitespace().skip(1);
        let Some(old) = ranges.next().and_then(|range| parse_diff_range(range, '-')) else {
            continue;
        };
        let Some(new) = ranges.next().and_then(|range| parse_diff_range(range, '+')) else {
            continue;
        };
        let new_index = new.0.saturating_sub(1);
        if old.1 == 0 {
            for index in new_index..new_index.saturating_add(new.1).min(line_count) {
                changes.insert(index, GitLineChange::Added);
            }
            continue;
        }
        if new.1 == 0 {
            changes.insert(new_index.min(line_count - 1), GitLineChange::Deleted);
            continue;
        }
        let common = old.1.min(new.1);
        for index in new_index..new_index.saturating_add(common).min(line_count) {
            changes.insert(index, GitLineChange::Modified);
        }
        for index in
            new_index.saturating_add(common)..new_index.saturating_add(new.1).min(line_count)
        {
            changes.insert(index, GitLineChange::Added);
        }
        if old.1 > new.1 {
            let marker = new_index
                .saturating_add(new.1.saturating_sub(1))
                .min(line_count - 1);
            changes.insert(marker, GitLineChange::Deleted);
        }
    }
    changes
}

fn parse_diff_range(value: &str, prefix: char) -> Option<(usize, usize)> {
    let value = value.strip_prefix(prefix)?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
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
    let line_start = if line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 1)
            .map(|(index, _)| index + 1)?
    };
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |offset| line_start + offset);
    let prefix = &text[line_start..line_end];
    let mut utf16_column = 0;
    let mut byte_offset = prefix.len();
    for (offset, value) in prefix.char_indices() {
        if utf16_column >= character {
            byte_offset = offset;
            break;
        }
        utf16_column += value.len_utf16();
        if utf16_column > character {
            return None;
        }
    }
    Some(line_start + byte_offset)
}

fn apply_lsp_text_edits(text: &str, edits: &[Value]) -> Option<String> {
    let mut changes = Vec::new();
    for edit in edits {
        let replacement = edit["newText"].as_str()?;
        let range = &edit["range"];
        let start = lsp_text_offset(
            text,
            range["start"]["line"].as_u64()? as usize,
            range["start"]["character"].as_u64()? as usize,
        )?;
        let end = lsp_text_offset(
            text,
            range["end"]["line"].as_u64()? as usize,
            range["end"]["character"].as_u64()? as usize,
        )?;
        if start > end || end > text.len() {
            return None;
        }
        changes.push((start, end, replacement.to_string()));
    }
    changes.sort_by_key(|(start, _, _)| *start);
    let mut result = text.to_string();
    for (start, end, replacement) in changes.into_iter().rev() {
        result.replace_range(start..end, &replacement);
    }
    Some(result)
}

fn lsp_cursor_column(text: &str, line: usize, character: usize) -> Option<usize> {
    let offset = lsp_text_offset(text, line, character)?;
    let line_start = if line == 0 {
        0
    } else {
        text.match_indices('\n')
            .nth(line - 1)
            .map(|(index, _)| index + 1)?
    };
    Some(text[line_start..offset].chars().count())
}

fn strip_lsp_snippet(snippet: &str) -> String {
    let mut output = String::new();
    let mut chars = snippet.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '$' {
            output.push(character);
            continue;
        }
        if chars.peek() == Some(&'{') {
            chars.next();
            let mut body = String::new();
            for value in chars.by_ref() {
                if value == '}' {
                    break;
                }
                body.push(value);
            }
            let value = body.split_once(':').map_or("", |(_, default)| default);
            output.push_str(value);
        } else {
            while chars.peek().is_some_and(|value| value.is_ascii_digit()) {
                chars.next();
            }
        }
    }
    output
}

fn lsp_hover_text(result: &Value) -> String {
    let contents = &result["contents"];
    if let Some(text) = contents.as_str() {
        return text.to_string();
    }
    if let Some(text) = contents["value"].as_str() {
        return text.to_string();
    }
    contents
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.as_str().or_else(|| part["value"].as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    text[..index.min(text.len())]
        .char_indices()
        .last()
        .map_or(0, |(offset, _)| offset)
}

fn copy_message(result: io::Result<crate::clipboard::CopyMethod>, action: &str) -> String {
    match result {
        Ok(crate::clipboard::CopyMethod::System) => format!("{action} to clipboard"),
        Ok(crate::clipboard::CopyMethod::TerminalOsc52) => {
            format!("{action} via terminal clipboard")
        }
        Err(error) => format!("{action} (internal clipboard only: {error})"),
    }
}

fn recovery_diff_lines(current: &str, recovered: &str) -> Vec<String> {
    let current = current.lines().collect::<Vec<_>>();
    let recovered = recovered.lines().collect::<Vec<_>>();
    let count = current.len().max(recovered.len());
    let mut lines = Vec::new();
    for index in 0..count {
        match (current.get(index), recovered.get(index)) {
            (Some(left), Some(right)) if left == right => lines.push(format!("  {left}")),
            (Some(left), Some(right)) => {
                lines.push(format!("- {left}"));
                lines.push(format!("+ {right}"));
            }
            (Some(left), None) => lines.push(format!("- {left}")),
            (None, Some(right)) => lines.push(format!("+ {right}")),
            (None, None) => {}
        }
    }
    if lines.is_empty() {
        lines.push("No content differences.".to_string());
    }
    lines
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    text[index.min(text.len())..]
        .chars()
        .next()
        .map_or(text.len(), |character| index + character.len_utf8())
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

    fn type_text(app: &mut App, text: &str) {
        for character in text.chars() {
            app.handle_key(key(character));
        }
    }

    #[test]
    fn dirty_tab_close_requires_confirmation_before_discarding() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
        app.mode = Mode::Insert;
        app.editor.insert_text("unsaved");
        app.editor.finish_undo_group();

        app.close_active_tab(false);
        assert_eq!(app.mode, Mode::TabCloseConfirm);
        assert!(app.message.contains("unsaved changes"), "{}", app.message);

        app.handle_key(key('x'));
        assert_eq!(app.mode, Mode::TabCloseConfirm);
        assert!(app.editor.tab_dirty(0));

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Insert);
        assert!(app.editor.tab_dirty(0));

        app.close_active_tab(false);
        assert_eq!(app.mode, Mode::TabCloseConfirm);

        app.handle_key(key('d'));
        assert_eq!(app.mode, Mode::Insert);
        assert!(!app.editor.tab_dirty(0));
        assert!(app.message.contains("Closed Untitled 1"), "{}", app.message);
    }

    #[test]
    fn search_panel_replaces_all_occurrences_as_one_undo_step() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
        app.mode = Mode::Insert;
        app.editor.insert_text("foo bar foo");
        app.editor.finish_undo_group();

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Search);
        type_text(&mut app, "foo");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "baz");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));

        assert_eq!(app.editor.text(), "baz bar baz");
        assert!(app.message.contains("Replaced 2"), "{}", app.message);
        assert!(app.editor.undo());
        assert_eq!(app.editor.text(), "foo bar foo");
    }

    #[test]
    fn search_options_narrow_matches_case_then_whole_word() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
        app.mode = Mode::Insert;
        app.editor.insert_text("Cat cat concat");
        app.editor.finish_undo_group();

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        type_text(&mut app, "cat");
        assert!(
            app.search_panel_text().contains("1/3"),
            "{}",
            app.search_panel_text()
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT));
        assert!(
            app.search_panel_text().contains("1/2"),
            "{}",
            app.search_panel_text()
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT));
        assert!(
            app.search_panel_text().contains("1/1"),
            "{}",
            app.search_panel_text()
        );
        assert_eq!(app.editor.selected_text().as_deref(), Some("cat"));
    }

    #[test]
    fn search_history_recalls_previous_queries() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
        app.mode = Mode::Insert;
        app.editor.insert_text("alpha beta");
        app.editor.finish_undo_group();

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        type_text(&mut app, "alpha");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        type_text(&mut app, "beta");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(app.search_input.is_empty());
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.search_input, "beta");
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.search_input, "alpha");
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.search_input, "beta");
    }

    #[test]
    fn project_search_finds_excludes_and_replaces_across_files() {
        let root = std::env::temp_dir().join(format!("caret-app-psearch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "alpha();\nalpha();\n").unwrap();
        std::fs::write(root.join("b.txt"), "alpha note\n").unwrap();

        let mut app = App::new(None).expect("create app");
        app.project.set_root(root.clone()).expect("set root");
        app.open_project_search();
        assert_eq!(app.mode, Mode::ProjectSearch);

        type_text(&mut app, "alpha");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.project_search.results.len(), 3);
        assert_eq!(app.project_search.files_with_matches, 2);

        // Exclude the first match in src/a.rs (results are path-sorted:
        // b.txt first, then src/a.rs line 1 and line 2).
        app.project_search.selected = 1;
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        type_text(&mut app, "omega");

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
        assert!(app.message.contains("Alt-A again"), "{}", app.message);
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
        assert!(app.message.contains("Replaced 2"), "{}", app.message);

        assert_eq!(
            std::fs::read_to_string(root.join("src/a.rs")).unwrap(),
            "alpha();\nomega();\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("b.txt")).unwrap(),
            "omega note\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_search_enter_opens_the_selected_match() {
        let root = std::env::temp_dir().join(format!("caret-app-popen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("notes.txt"), "first\ntarget here\n").unwrap();

        let mut app = App::new(None).expect("create app");
        app.project.set_root(root.clone()).expect("set root");
        app.open_project_search();
        type_text(&mut app, "target");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.project_search.results.len(), 1);

        // Second Enter (query unchanged) opens the selected result.
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_ne!(app.mode, Mode::ProjectSearch);
        assert_eq!(app.editor.cursor.line, 1);
        assert!(app
            .editor
            .path
            .as_ref()
            .is_some_and(|path| path.ends_with("notes.txt")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fuzzy_file_picker_filters_and_opens_files() {
        let root = std::env::temp_dir().join(format!("caret-app-picker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("src/helper.rs"), "fn helper() {}").unwrap();
        std::fs::write(root.join("readme.md"), "docs").unwrap();

        let mut app = App::new(None).expect("create app");
        app.project.set_root(root.clone()).expect("set root");

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::FilePicker);
        assert_eq!(app.file_picker.files.len(), 3);

        type_text(&mut app, "mainrs");
        assert_eq!(app.file_picker.matches.len(), 1);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_ne!(app.mode, Mode::FilePicker);
        assert!(app
            .editor
            .path
            .as_ref()
            .is_some_and(|path| path.ends_with("main.rs")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tree_filter_narrows_the_project_tree_and_opens_a_match() {
        let root = std::env::temp_dir().join(format!("caret-app-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/target_file.rs"), "code").unwrap();
        std::fs::write(root.join("other.md"), "text").unwrap();

        let mut app = App::new(None).expect("create app");
        app.project.set_root(root.clone()).expect("set root");
        app.explorer_focused = true;
        app.mode = Mode::Normal;

        app.handle_key(key('/'));
        type_text(&mut app, "target");
        assert_eq!(app.project.entries.len(), 1);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app
            .editor
            .path
            .as_ref()
            .is_some_and(|path| path.ends_with("target_file.rs")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn custom_bindings_rebind_global_shortcuts() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
        app.mode = Mode::Insert;

        app.execute_command("bind find ctrl+g");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Search);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        // The old chord no longer opens search after the rebind.
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert_ne!(app.mode, Mode::Search);

        // Conflicts are rejected and name the other action.
        app.execute_command("bind replace ctrl+g");
        assert!(app.message.contains("find"), "{}", app.message);

        // Reset restores the default chord.
        app.execute_command("bind find default");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, Mode::Search);
    }

    #[test]
    fn keybinding_browser_lists_and_searches_bindings() {
        let mut app = App::new(None).expect("create app");
        app.execute_command("keybindings");
        assert_eq!(app.mode, Mode::KeyBrowser);

        let all = app.keybinding_rows().len();
        assert!(all > 30, "expected a full catalog, got {all}");

        type_text(&mut app, "undo");
        let filtered = app.keybinding_rows();
        assert!(filtered.len() < all);
        assert!(filtered
            .iter()
            .any(|(_, description, _)| description.contains("Undo")));
    }

    #[test]
    fn invalid_regex_shows_an_error_instead_of_crashing() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
        app.mode = Mode::Insert;
        app.editor.insert_text("(text)");
        app.editor.finish_undo_group();

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT));
        type_text(&mut app, "(unclosed");
        assert!(app.search_error.is_some());
        assert!(app.search_panel_text().contains("Regex error"));
    }

    #[test]
    fn records_and_replays_a_macro() {
        let mut app = App::new(None).expect("create app");
        app.explorer_focused = false;
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
        app.explorer_focused = false;
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

    #[test]
    fn command_suggestion_scrolls_when_keyboard_selection_reaches_the_edge() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Command;

        for _ in 0..9 {
            app.handle_command_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }

        assert_eq!(app.command_suggestion, 9);
        assert_eq!(app.command_suggestion_scroll, 2);
    }

    #[test]
    fn command_suggestion_hover_lock_expires() {
        let mut app = App::new(None).expect("create app");
        app.command_suggestion_hover_lock_until = Some(Instant::now() + Duration::from_secs(1));
        assert!(app.command_suggestion_hover_is_locked());

        app.command_suggestion_hover_lock_until = Some(Instant::now() - Duration::from_secs(1));
        assert!(!app.command_suggestion_hover_is_locked());
    }

    #[test]
    fn theme_gallery_hover_lock_expires() {
        let mut app = App::new(None).expect("create app");
        app.theme_gallery_hover_lock_until = Some(Instant::now() + Duration::from_secs(1));
        assert!(app.theme_gallery_hover_is_locked());

        app.theme_gallery_hover_lock_until = Some(Instant::now() - Duration::from_secs(1));
        assert!(!app.theme_gallery_hover_is_locked());
    }

    #[test]
    fn context_menu_actions_execute_and_restore_editor_mode() {
        let mut app = App::new(None).expect("create app");
        app.editor.insert_text("hello");
        app.context_menu_previous_mode = Mode::Insert;
        app.context_menu = Some(ContextMenu {
            x: 1,
            y: 1,
            selected: 0,
            actions: vec![ContextAction::SelectAll],
        });
        app.mode = Mode::ContextMenu;

        app.execute_context_action();

        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.editor.selected_text().as_deref(), Some("hello"));
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn parses_added_modified_and_deleted_git_hunks_for_the_gutter() {
        let diff =
            "@@ -2,1 +2,1 @@\n-old\n+new\n@@ -4,0 +5,2 @@\n+a\n+b\n@@ -8,2 +9,0 @@\n-old\n-old\n";
        let changes = parse_git_hunks(diff, 12);

        assert_eq!(changes.get(&1), Some(&GitLineChange::Modified));
        assert_eq!(changes.get(&4), Some(&GitLineChange::Added));
        assert_eq!(changes.get(&5), Some(&GitLineChange::Added));
        assert_eq!(changes.get(&8), Some(&GitLineChange::Deleted));
    }

    #[test]
    fn help_returns_to_the_recent_project_dashboard() {
        let mut app = App::new(None).expect("create app");
        app.execute_command("welcome");
        assert_eq!(app.mode, Mode::Dashboard);

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(app.mode, Mode::Help);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(app.mode, Mode::Dashboard);
    }

    #[test]
    fn default_startup_opens_the_file_tree_not_the_dashboard() {
        let app = App::new(None).expect("create app");
        assert_ne!(app.mode, Mode::Dashboard);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.explorer_focused);
        assert!(app.project.visible);
    }

    #[test]
    fn welcome_command_opens_the_dashboard_on_demand() {
        let mut app = App::new(None).expect("create app");
        assert_ne!(app.mode, Mode::Dashboard);
        app.execute_command("welcome");
        assert_eq!(app.mode, Mode::Dashboard);
    }

    #[test]
    fn long_running_lsp_work_is_reported_as_a_warning() {
        let mut app = App::new(None).expect("create app");
        app.lsp_status = LspStatus::Loading;
        app.lsp_started_at = Some(Instant::now() - Duration::from_secs(31));

        let (label, state) = app.background_status().expect("background status");

        assert_eq!(state, BackgroundState::Warning);
        assert!(label.contains("still loading"));
    }

    #[test]
    fn lsp_offsets_use_utf16_columns() {
        let text = "a😀b\nnext";
        assert_eq!(lsp_text_offset(text, 0, 0), Some(0));
        assert_eq!(lsp_text_offset(text, 0, 1), Some(1));
        assert_eq!(lsp_text_offset(text, 0, 2), None);
        assert_eq!(lsp_text_offset(text, 0, 3), Some(5));
        assert_eq!(lsp_cursor_column(text, 0, 3), Some(2));
        assert_eq!(lsp_text_offset(text, 1, 2), Some(9));
    }

    #[test]
    fn applies_lsp_edits_from_the_end_and_expands_snippets() {
        let edits = vec![
            json!({ "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }, "newText": "A" }),
            json!({ "range": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 3 } }, "newText": "C" }),
        ];
        assert_eq!(apply_lsp_text_edits("abc", &edits).as_deref(), Some("AbC"));
        assert_eq!(strip_lsp_snippet("write(${1:value})$0"), "write(value)");
    }

    #[test]
    fn completion_panel_inserts_selected_item() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Insert;
        app.lsp_panel = Some(LspPanel {
            title: "Completions".to_string(),
            selected: 0,
            kind: LspPanelKind::Completion,
            items: vec![LspPanelItem {
                label: "Console".to_string(),
                detail: String::new(),
                payload: json!({ "label": "Console", "insertText": "Console" }),
            }],
        });
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.text(), "Console");
        assert!(app.lsp_panel.is_none());
    }

    #[test]
    fn colon_opens_the_command_palette_from_the_file_explorer() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Normal;
        app.explorer_focused = true;

        app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT));

        assert_eq!(app.mode, Mode::Command);
        assert_eq!(app.message, "Command palette");
    }

    #[test]
    fn colon_opens_the_command_palette_in_normal_mode_without_shift_metadata() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Normal;

        app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));

        assert_eq!(app.mode, Mode::Command);
    }

    #[test]
    fn explicit_file_startup_focuses_the_editor() {
        let path = std::env::temp_dir().join(format!("caret-startup-{}.txt", std::process::id()));
        fs::write(&path, "content").expect("write file");

        let app = App::new(Some(&path)).expect("create app");

        assert!(!app.explorer_focused);
        assert!(!app.project.visible);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn explicit_folder_startup_shows_and_focuses_the_tree() {
        let dir = std::env::temp_dir().join(format!("caret-startup-dir-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create dir");

        let app = App::new(Some(&dir)).expect("create app");

        assert!(app.explorer_focused);
        assert!(app.project.visible);
        let _ = fs::remove_dir_all(dir);
    }
}
