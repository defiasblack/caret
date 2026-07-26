use std::{
    io::{self, Write},
    path::{Component, Path, Prefix, MAIN_SEPARATOR},
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{
        App, BackgroundState, ExplorerInputKind, HoverTarget, ManagerConfirmation,
        ManagerInputKind, Mode, SidebarView, COMMAND_PALETTE_ROWS,
    },
    config::{IconMode, KeymapProfile},
    editor::display_width,
    file_manager::{human_size, unix_time_label, FileEntry, Preview},
    project::{GitStatus, TreeLoadState},
    syntax::{self, Language},
    theme::ThemeKind,
};

#[derive(Debug, Clone, Copy)]
pub struct ScreenLayout {
    pub content_top: u16,
    pub content_height: usize,
    pub sidebar_width: usize,
    pub editor_x: usize,
    pub editor_width: usize,
    pub gutter_width: usize,
    pub terminal_top: u16,
    pub terminal_height: usize,
    pub status_row: u16,
    pub prompt_row: u16,
    pub hotkey_row: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileManagerLayout {
    parent_x: usize,
    parent_width: usize,
    current_x: usize,
    current_width: usize,
    preview_x: usize,
    preview_width: usize,
}

#[derive(Debug, Clone, Copy)]
struct ManagerPanel {
    top: u16,
    rows: usize,
    x: usize,
    width: usize,
}

impl FileManagerLayout {
    fn calculate(width: usize, parent_percent: u8, current_percent: u8) -> Self {
        if width >= 110 {
            let parent_width = (width * usize::from(parent_percent) / 100).clamp(22, width / 3);
            let current_width = (width * usize::from(current_percent) / 100).clamp(34, width / 2);
            let preview_x = parent_width + current_width + 2;
            Self {
                parent_x: 0,
                parent_width,
                current_x: parent_width + 1,
                current_width,
                preview_x,
                preview_width: width.saturating_sub(preview_x),
            }
        } else if width >= 68 {
            let current_width = (width * 54 / 100).max(34);
            Self {
                parent_x: 0,
                parent_width: 0,
                current_x: 0,
                current_width,
                preview_x: current_width + 1,
                preview_width: width.saturating_sub(current_width + 1),
            }
        } else {
            Self {
                parent_x: 0,
                parent_width: 0,
                current_x: 0,
                current_width: width,
                preview_x: width,
                preview_width: 0,
            }
        }
    }
}

pub fn screen_layout(app: &App, width: u16, height: u16) -> ScreenLayout {
    let content_top = 2u16;
    let available_height = height.saturating_sub(5) as usize;
    let terminal_height = if app.terminal_visible() && available_height >= 10 {
        (available_height / 3).clamp(5, 12)
    } else {
        0
    };
    let terminal_separator = usize::from(terminal_height > 0);
    let content_height = available_height.saturating_sub(terminal_height + terminal_separator);
    let sidebar_width = effective_sidebar_width(app, width);
    let separator_width = usize::from(sidebar_width > 0);
    let editor_x = sidebar_width + separator_width;
    let editor_width = (width as usize).saturating_sub(editor_x);
    let gutter_width = app
        .editor
        .line_number_width()
        .min(editor_width.saturating_sub(1));

    ScreenLayout {
        content_top,
        content_height,
        sidebar_width,
        editor_x,
        editor_width,
        gutter_width,
        terminal_top: content_top + content_height as u16 + terminal_separator as u16,
        terminal_height,
        status_row: height.saturating_sub(3),
        prompt_row: height.saturating_sub(2),
        hotkey_row: height.saturating_sub(1),
    }
}

pub fn draw<W: Write>(out: &mut W, app: &mut App) -> io::Result<()> {
    let (width, height) = terminal::size()?;

    queue!(
        out,
        BeginSynchronizedUpdate,
        Hide,
        SetBackgroundColor(app.theme.background),
        SetForegroundColor(app.theme.foreground)
    )?;

    if width < 44 || height < 8 {
        queue!(
            out,
            MoveTo(0, 0),
            SetForegroundColor(app.theme.error),
            Print("Terminal is too small for Caret.")
        )?;
        out.flush()?;
        return Ok(());
    }

    let layout = screen_layout(app, width, height);
    if layout.terminal_height > 0 {
        app.resize_terminal(layout.terminal_height, width as usize);
    }
    let content_top = layout.content_top;
    let content_height = layout.content_height;
    let sidebar_width = layout.sidebar_width;
    let editor_x = layout.editor_x;
    let editor_width = layout.editor_width;
    let gutter_width = layout.gutter_width;
    let content_width = editor_width.saturating_sub(gutter_width);

    app.viewport_rows = content_height.max(1);
    app.viewport_columns = content_width.max(1);
    if app.follow_cursor {
        app.editor
            .ensure_cursor_visible(app.viewport_rows, app.viewport_columns);
    }
    app.project
        .ensure_selected_visible(content_height.saturating_sub(1));

    draw_top_bar(out, app, width)?;
    draw_tab_bar(out, app, 1, width)?;

    if sidebar_width > 0 {
        if app.sidebar_view == SidebarView::Files {
            draw_project_tree(out, app, content_top, content_height, sidebar_width)?;
        } else {
            draw_outline(out, app, content_top, content_height, sidebar_width)?;
        }
        draw_vertical_separator(out, app, sidebar_width as u16, content_top, content_height)?;
    }

    if let Some(views) = app.split_views {
        let focused = (
            app.editor.active_index(),
            app.editor.cursor,
            app.editor.scroll_line,
            app.editor.scroll_column,
        );
        if views.vertical {
            let pane_width = editor_width.saturating_sub(1) / 2;
            let pane_gutter = app
                .editor
                .line_number_width()
                .min(pane_width.saturating_sub(1));
            app.editor.select(views.primary.tab_index);
            app.editor.cursor = views.primary.cursor;
            app.editor.scroll_line = views.primary.scroll_line;
            app.editor.scroll_column = views.primary.scroll_column;
            draw_editor(
                out,
                app,
                content_top,
                content_height,
                editor_x as u16,
                pane_width as u16,
                pane_gutter,
            )?;
            let divider = editor_x + pane_width;
            draw_vertical_separator(out, app, divider as u16, content_top, content_height)?;
            app.editor.select(views.secondary.tab_index);
            app.editor.cursor = views.secondary.cursor;
            app.editor.scroll_line = views.secondary.scroll_line;
            app.editor.scroll_column = views.secondary.scroll_column;
            draw_editor(
                out,
                app,
                content_top,
                content_height,
                (divider + 1) as u16,
                pane_width as u16,
                pane_gutter,
            )?;
        } else {
            let pane_rows = content_height.saturating_sub(1) / 2;
            app.editor.select(views.primary.tab_index);
            app.editor.cursor = views.primary.cursor;
            app.editor.scroll_line = views.primary.scroll_line;
            app.editor.scroll_column = views.primary.scroll_column;
            draw_editor(
                out,
                app,
                content_top,
                pane_rows,
                editor_x as u16,
                editor_width as u16,
                gutter_width,
            )?;
            let divider = content_top + pane_rows as u16;
            queue!(
                out,
                MoveTo(editor_x as u16, divider),
                SetBackgroundColor(app.theme.background),
                SetForegroundColor(app.theme.border),
                Print("─".repeat(editor_width))
            )?;
            app.editor.select(views.secondary.tab_index);
            app.editor.cursor = views.secondary.cursor;
            app.editor.scroll_line = views.secondary.scroll_line;
            app.editor.scroll_column = views.secondary.scroll_column;
            draw_editor(
                out,
                app,
                divider + 1,
                pane_rows,
                editor_x as u16,
                editor_width as u16,
                gutter_width,
            )?;
        }
        app.editor.select(focused.0);
        app.editor.cursor = focused.1;
        app.editor.scroll_line = focused.2;
        app.editor.scroll_column = focused.3;
    } else {
        draw_editor(
            out,
            app,
            content_top,
            content_height,
            editor_x as u16,
            editor_width as u16,
            gutter_width,
        )?;
    }
    if layout.terminal_height > 0 {
        draw_terminal(out, app, layout.terminal_top, layout.terminal_height, width)?;
    }
    if app.mode == Mode::FileManager {
        app.file_manager
            .ensure_selected_visible(content_height.saturating_sub(5));
        draw_file_manager(out, app, content_top, content_height, width)?;
    }
    draw_status_bar(out, app, layout.status_row, width)?;
    draw_command_palette(out, app, width, height)?;
    draw_prompt_bar(out, app, layout.prompt_row, width)?;
    draw_hotkey_bar(out, app, layout.hotkey_row, width)?;

    if app.mode == Mode::Help {
        draw_help(out, app, width, height)?;
    }
    if app.mode == Mode::ProjectSearch {
        draw_project_search(out, app, width, height)?;
    }
    if app.mode == Mode::FilePicker {
        draw_file_picker(out, app, width, height)?;
    }
    if app.mode == Mode::KeyBrowser {
        draw_key_browser(out, app, width, height)?;
    }
    if app.mode == Mode::SettingsBrowser {
        draw_settings_browser(out, app, width, height)?;
    }
    if app.mode == Mode::GitDiff {
        draw_git_diff(out, app, width, height)?;
    }
    if app.mode == Mode::GitHistory {
        draw_git_history(out, app, width, height)?;
    }
    if app.mode == Mode::ThemeGallery {
        draw_theme_gallery(out, app, width, height)?;
    }
    if app.mode == Mode::KeymapGallery {
        draw_keymap_gallery(out, app, width, height)?;
    }
    if app.mode == Mode::ContextMenu {
        draw_context_menu(out, app, width, height)?;
    }
    if app.mode == Mode::TabCloseConfirm {
        draw_tab_close_confirm(out, app, width, height)?;
    }
    if app.mode == Mode::Dashboard {
        draw_dashboard(out, app, width, height)?;
    }
    if app.mode == Mode::FileManager
        && (app.manager_confirmation.is_some() || app.manager_conflicts > 0)
    {
        draw_manager_confirmation(out, app, width, height)?;
    }
    if app.mode == Mode::FileManager && app.manager_context_menu.is_some() {
        draw_manager_context_menu(out, app, width, height)?;
    }
    if app.lsp_panel.is_some() {
        draw_lsp_panel(out, app, width, height)?;
    }

    let (cursor_editor_x, cursor_editor_width, cursor_gutter_width) =
        if let Some(views) = app.split_views {
            let pane_width = editor_width.saturating_sub(1) / 2;
            let pane_gutter = app
                .editor
                .line_number_width()
                .min(pane_width.saturating_sub(1));
            let x = if views.vertical && views.secondary_active {
                editor_x + pane_width + 1
            } else {
                editor_x
            };
            (x, pane_width, pane_gutter)
        } else {
            (editor_x, editor_width, gutter_width)
        };
    let (cursor_content_top, cursor_content_height) = if let Some(views) = app.split_views {
        if !views.vertical && views.secondary_active {
            let pane_rows = content_height.saturating_sub(1) / 2;
            (content_top + pane_rows as u16 + 1, pane_rows)
        } else if !views.vertical {
            (content_top, content_height.saturating_sub(1) / 2)
        } else {
            (content_top, content_height)
        }
    } else {
        (content_top, content_height)
    };
    if app.lsp_panel.is_some() || app.mode == Mode::FileManager {
        queue!(out, Hide)?;
    } else if app.terminal_focused && layout.terminal_height > 0 {
        let (row, column) = app.terminal_cursor_position();
        let x = column.min(width.saturating_sub(1) as usize) as u16;
        let y = layout.terminal_top + row.min(layout.terminal_height.saturating_sub(1)) as u16;
        queue!(out, MoveTo(x, y), Show)?;
    } else {
        place_cursor(
            out,
            app,
            cursor_content_top,
            cursor_content_height,
            cursor_editor_x,
            cursor_editor_width,
            cursor_gutter_width,
            width,
            height,
        )?;
    }

    queue!(
        out,
        ResetColor,
        SetAttribute(Attribute::Reset),
        EndSynchronizedUpdate
    )?;
    out.flush()
}

fn effective_sidebar_width(app: &App, terminal_width: u16) -> usize {
    if !app.project.visible {
        return 0;
    }

    let maximum = (terminal_width as usize).saturating_sub(28);
    if maximum < 22 {
        return 0;
    }

    app.project.width.clamp(22, maximum)
}

/// Everything in `title_bar_right` except the project name.
const TITLE_BAR_RIGHT_FIXED: u16 = 12;
/// Columns the left half needs before `[FILES]` is fully drawn.
const TITLE_BAR_FILES_END: u16 = 18;

/// The right-aligned segment of the title bar.  `title_bar_targets` hard-codes
/// offsets into this string, and a test pins the two together.
fn title_bar_right(root: &str) -> String {
    format!(" {root}  │ [Menu] ")
}

/// The clickable controls in the title bar.  Drawing, hover, and click
/// handling all read this one table so the hit zones cannot drift from what
/// is actually painted.
///
/// `fit_bar` right-aligns the trailing segment and truncates the title to
/// fit, so a control is only where these offsets claim while the segment
/// still fits: on a narrow terminal the entries are dropped rather than
/// pointing at whatever ended up in that column.
fn title_bar_targets(width: u16, root_width: u16) -> Vec<(HoverTarget, u16, &'static str)> {
    let right_width = root_width.saturating_add(TITLE_BAR_RIGHT_FIXED);
    let mut targets = Vec::with_capacity(2);

    if width >= right_width.saturating_add(TITLE_BAR_FILES_END) {
        targets.push((HoverTarget::Files, 11, "[FILES]"));
    }
    if width > right_width {
        targets.push((HoverTarget::Menu, width - 7, "[Menu]"));
    }
    targets
}

/// The title-bar control under a column, or `None` for the gaps between them.
pub fn title_bar_target_at(app: &App, width: u16, column: u16) -> Option<HoverTarget> {
    let root_width = UnicodeWidthStr::width(app.project.root_name().as_str()) as u16;
    title_bar_targets(width, root_width)
        .into_iter()
        .find(|(_, x, label)| column >= *x && column < x.saturating_add(label.len() as u16))
        .map(|(target, _, _)| target)
}

fn draw_top_bar<W: Write>(out: &mut W, app: &App, width: u16) -> io::Result<()> {
    let filename = app
        .editor
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("[No Name]");

    let dirty = if app.editor.dirty { " ●" } else { "" };
    let breadcrumb = app.current_breadcrumbs();
    let location = if breadcrumb.is_empty() {
        String::new()
    } else {
        format!("  › {breadcrumb}")
    };
    let title = format!("  CARET  │ [FILES] │  {filename}{dirty}{location}");
    let root = app.project.root_name();
    let right = title_bar_right(&root);

    queue!(
        out,
        MoveTo(0, 0),
        SetBackgroundColor(app.theme.top_bar),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(fit_bar(&title, &right, width as usize)),
        SetAttribute(Attribute::Reset)
    )?;

    let root_width = UnicodeWidthStr::width(root.as_str()) as u16;
    for (target, x, label) in title_bar_targets(width, root_width) {
        if app.hover_target == Some(target) {
            queue!(
                out,
                MoveTo(x, 0),
                SetBackgroundColor(app.theme.heading),
                SetForegroundColor(app.theme.background),
                SetAttribute(Attribute::Bold),
                Print(label),
                SetAttribute(Attribute::Reset)
            )?;
        }
    }

    Ok(())
}

fn draw_tab_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    let available = width as usize;
    queue!(
        out,
        MoveTo(0, row),
        SetBackgroundColor(app.theme.prompt_bar),
        SetForegroundColor(app.theme.muted),
        Print(" ".repeat(available))
    )?;

    if available == 0 || app.editor.is_empty() {
        return Ok(());
    }

    let active = app.editor.active_index();
    let mut start = active;
    let active_label = tab_label(app, active);
    let mut required = UnicodeWidthStr::width(active_label.as_str());

    while start > 0 {
        let previous_label = tab_label(app, start - 1);
        let previous_width = UnicodeWidthStr::width(previous_label.as_str());
        let left_indicator = if start - 1 > 0 { 2 } else { 0 };
        if required + previous_width + left_indicator > available {
            break;
        }
        start -= 1;
        required += previous_width;
    }

    let mut x = 0usize;
    if start > 0 && available >= 2 {
        queue!(
            out,
            MoveTo(0, row),
            SetBackgroundColor(app.theme.prompt_bar),
            SetForegroundColor(app.theme.muted),
            Print("‹ ")
        )?;
        x = 2;
    }

    let mut last_rendered = start.saturating_sub(1);
    for index in start..app.editor.len() {
        let label = tab_label(app, index);
        let label_width = UnicodeWidthStr::width(label.as_str());
        let needs_right_indicator = index + 1 < app.editor.len();
        let reserve = usize::from(needs_right_indicator) * 2;

        if x + label_width + reserve > available {
            break;
        }

        let active_tab = index == active;
        queue!(
            out,
            MoveTo(x as u16, row),
            SetBackgroundColor(if active_tab {
                app.theme.current_line
            } else {
                app.theme.prompt_bar
            }),
            SetForegroundColor(if active_tab {
                app.theme.top_bar_text
            } else if app.editor.tab_dirty(index) {
                app.theme.error
            } else {
                app.theme.muted
            }),
            SetAttribute(if active_tab {
                Attribute::Bold
            } else {
                Attribute::Reset
            }),
            Print(&label),
            SetAttribute(Attribute::Reset)
        )?;

        x += label_width;
        last_rendered = index;
    }

    if last_rendered + 1 < app.editor.len() && x + 2 <= available {
        queue!(
            out,
            MoveTo((available - 2) as u16, row),
            SetBackgroundColor(app.theme.prompt_bar),
            SetForegroundColor(app.theme.muted),
            Print(" ›")
        )?;
    }

    Ok(())
}

fn tab_label(app: &App, index: usize) -> String {
    let dirty = if app.editor.tab_dirty(index) {
        " ●"
    } else {
        ""
    };
    let title = compact_text(&app.editor.tab_title(index), 24);
    format!(" {} {}{} ", index + 1, title, dirty)
}

pub fn tab_index_at(app: &App, width: u16, column: u16) -> Option<usize> {
    let available = width as usize;
    let column = column as usize;

    if available == 0 || app.editor.is_empty() || column >= available {
        return None;
    }

    let active = app.editor.active_index();
    let mut start = active;
    let active_label = tab_label(app, active);
    let mut required = UnicodeWidthStr::width(active_label.as_str());

    while start > 0 {
        let previous_label = tab_label(app, start - 1);
        let previous_width = UnicodeWidthStr::width(previous_label.as_str());
        let left_indicator = if start - 1 > 0 { 2 } else { 0 };

        if required + previous_width + left_indicator > available {
            break;
        }

        start -= 1;
        required += previous_width;
    }

    let mut x = if start > 0 && available >= 2 { 2 } else { 0 };

    for index in start..app.editor.len() {
        let label = tab_label(app, index);
        let label_width = UnicodeWidthStr::width(label.as_str());
        let reserve = usize::from(index + 1 < app.editor.len()) * 2;

        if x + label_width + reserve > available {
            break;
        }

        if column >= x && column < x + label_width {
            return Some(index);
        }

        x += label_width;
    }

    None
}

fn compact_text(text: &str, maximum_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= maximum_width {
        return text.to_string();
    }

    let target = maximum_width.saturating_sub(1);
    let mut output = String::new();
    let mut used = 0usize;

    for character in text.chars() {
        let width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used + width > target {
            break;
        }
        output.push(character);
        used += width;
    }

    output.push('…');
    output
}

fn soft_selection_background(app: &App) -> Color {
    match (app.theme.overlay, app.theme.foreground) {
        (
            Color::Rgb {
                r: base_r,
                g: base_g,
                b: base_b,
            },
            Color::Rgb {
                r: text_r,
                g: text_g,
                b: text_b,
            },
        ) => Color::Rgb {
            r: blend_channel(base_r, text_r, 14),
            g: blend_channel(base_g, text_g, 14),
            b: blend_channel(base_b, text_b, 14),
        },
        _ => app.theme.current_line,
    }
}

fn blend_channel(base: u8, accent: u8, accent_percent: u16) -> u8 {
    let base_percent = 100 - accent_percent;
    ((u16::from(base) * base_percent + u16::from(accent) * accent_percent) / 100) as u8
}

fn manager_display_line(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut column = 0usize;
    for character in text.chars() {
        if character == '\t' {
            let spaces = 4 - column % 4;
            output.push_str(&" ".repeat(spaces));
            column += spaces;
        } else if character.is_control() {
            output.push('�');
            column += 1;
        } else {
            output.push(character);
            column += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }
    output
}

fn draw_project_tree<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    width: usize,
) -> io::Result<()> {
    for screen_row in 0..rows {
        let y = top + screen_row as u16;
        queue!(
            out,
            MoveTo(0, y),
            SetBackgroundColor(app.theme.prompt_bar),
            SetForegroundColor(app.theme.foreground),
            Print(" ".repeat(width))
        )?;

        if screen_row == 0 {
            let hidden_marker = if app.project.show_hidden {
                " · hidden"
            } else {
                ""
            };
            let filter_marker = if app.project.filter.is_empty() {
                String::new()
            } else {
                format!(" · filter: {}", app.project.filter)
            };
            let git_marker = if app.project.git_refreshing {
                " · git…"
            } else if app.project.git_error.is_some() {
                " · git!"
            } else {
                ""
            };
            let tree_marker = match &app.project.tree_state {
                TreeLoadState::Loading => " · scan…",
                TreeLoadState::Empty => " · empty",
                TreeLoadState::PermissionDenied(_) => " · denied",
                TreeLoadState::Missing(_) => " · missing",
                TreeLoadState::Error(_) => " · scan!",
                TreeLoadState::Ready => "",
            };
            let root = format!(
                " 󰉋  {}  {}{}{}{}{}",
                explorer_breadcrumb(app, width.saturating_sub(30)),
                app.project.entries.len(),
                hidden_marker,
                filter_marker,
                git_marker,
                tree_marker
            );
            let controls = explorer_header_controls(width);
            queue!(
                out,
                MoveTo(0, y),
                SetForegroundColor(app.theme.top_bar_text),
                SetAttribute(Attribute::Bold),
                Print(fit_bar(&root, controls, width)),
                SetAttribute(Attribute::NormalIntensity)
            )?;
            continue;
        }

        if screen_row == 1 && app.project.entries.is_empty() {
            let message = match &app.project.tree_state {
                TreeLoadState::Loading => "  Scanning project…",
                TreeLoadState::Empty => "  This folder is empty",
                state => state.message().unwrap_or("  No project entries"),
            };
            queue!(
                out,
                MoveTo(0, y),
                SetForegroundColor(match &app.project.tree_state {
                    TreeLoadState::PermissionDenied(_)
                    | TreeLoadState::Missing(_)
                    | TreeLoadState::Error(_) => app.theme.error,
                    _ => app.theme.muted,
                }),
                Print(pad_or_truncate(message, width))
            )?;
            continue;
        }

        let visual_index = app.project.scroll + screen_row - 1;
        let insert_index = matches!(
            app.explorer_input_kind,
            Some(ExplorerInputKind::NewFile | ExplorerInputKind::NewDirectory)
        )
        .then_some(app.project.selected.saturating_add(1));
        if insert_index == Some(visual_index) {
            let kind = app.explorer_input_kind.unwrap();
            let parent_depth = app
                .project
                .selected_entry()
                .map_or(0, |entry| entry.depth + usize::from(entry.is_dir));
            let indent = "   ".repeat(parent_depth);
            let icon = if kind == ExplorerInputKind::NewDirectory {
                "▸"
            } else {
                "+"
            };
            let placeholder = if app.explorer_input.is_empty() {
                if kind == ExplorerInputKind::NewDirectory {
                    "folder name"
                } else {
                    "file name"
                }
            } else {
                &app.explorer_input
            };
            let line = format!(" {indent}└─{icon} {placeholder}_");
            queue!(
                out,
                MoveTo(0, y),
                SetBackgroundColor(app.theme.current_line),
                SetForegroundColor(app.theme.search_foreground),
                SetAttribute(Attribute::Bold),
                Print(pad_or_truncate(&line, width)),
                SetAttribute(Attribute::NormalIntensity)
            )?;
            continue;
        }
        let entry_index = visual_index.saturating_sub(usize::from(
            insert_index.is_some_and(|insert| visual_index > insert),
        ));
        let Some(entry) = app.project.entries.get(entry_index) else {
            continue;
        };

        let selected = entry_index == app.project.selected;
        let hovered = app.explorer_hovered == Some(entry_index);
        let active_file = app.editor.path.as_ref() == Some(&entry.path);
        let background = if selected && app.explorer_focused {
            app.theme.normal_mode
        } else if selected {
            app.theme.current_line
        } else if hovered {
            soft_selection_background(app)
        } else {
            app.theme.prompt_bar
        };
        let foreground = if selected && app.explorer_focused {
            app.theme.background
        } else if entry.git_status == Some(GitStatus::Conflicted) {
            app.theme.error
        } else if active_file {
            app.theme.success
        } else {
            explorer_entry_foreground(app, entry)
        };

        let icon = match (app.icon_mode(), entry.is_dir, entry.is_symlink, active_file) {
            (IconMode::Ascii, true, _, _) => {
                if entry.expanded {
                    "v"
                } else {
                    ">"
                }
            }
            (IconMode::Ascii, false, true, _) => "@",
            (IconMode::Ascii, false, false, true) => "*",
            (IconMode::Ascii, false, false, false) => "-",
            (IconMode::Nerd, true, _, _) => {
                if entry.expanded {
                    "󰝰"
                } else {
                    "󰉋"
                }
            }
            (IconMode::Nerd, false, true, _) => "󰌷",
            (IconMode::Nerd, false, false, true) => "󰈔",
            (IconMode::Nerd, false, false, false) => "󰈔",
            (IconMode::Unicode, true, _, _) => {
                if entry.expanded {
                    "▾"
                } else {
                    "▸"
                }
            }
            (IconMode::Unicode, false, true, _) => "↗",
            (IconMode::Unicode, false, false, true) => "●",
            (IconMode::Unicode, false, false, false) => "·",
        };
        let git = match entry.git_status {
            Some(GitStatus::Modified) => " M ",
            Some(GitStatus::Added) => " A ",
            Some(GitStatus::Deleted) => " D ",
            Some(GitStatus::Renamed) => " R ",
            Some(GitStatus::Conflicted) => " U ",
            Some(GitStatus::Untracked) => " ? ",
            None => "",
        };
        let metadata = if app.project.show_metadata && width >= 68 {
            let size = if entry.is_dir {
                "—".to_string()
            } else {
                entry
                    .size
                    .map(human_size)
                    .unwrap_or_else(|| "…".to_string())
            };
            format!(
                " {:>8} {:>8}",
                compact_text(&size, 8),
                compact_text(
                    &entry
                        .modified_unix_secs
                        .map(|seconds| unix_time_label(Some(seconds)))
                        .unwrap_or_else(|| "…".to_string()),
                    8
                )
            )
        } else if app.project.show_metadata && width >= 52 {
            let size = if entry.is_dir {
                "—".to_string()
            } else {
                entry
                    .size
                    .map(human_size)
                    .unwrap_or_else(|| "…".to_string())
            };
            format!(" {:>8}", compact_text(&size, 8))
        } else {
            String::new()
        };
        let right = format!("{metadata}{git}");
        let indent = entry
            .guides
            .iter()
            .map(|continued| if *continued { "│  " } else { "   " })
            .collect::<String>();
        let branch = if entry.is_last { "└─" } else { "├─" };
        let suffix = match (entry.is_dir, entry.is_symlink) {
            (true, true) => "/@",
            (true, false) => "/",
            (false, true) => "@",
            (false, false) => "",
        };
        let display_name = if app.explorer_input_kind == Some(ExplorerInputKind::Rename)
            && entry_index == app.project.selected
        {
            format!("{}_", app.explorer_input)
        } else {
            entry.name.clone()
        };
        let prefix = format!(" {indent}{branch}{icon} ");
        let left = format!("{prefix}{display_name}{suffix}");
        let label = fit_bar(&left, &right, width);

        queue!(
            out,
            MoveTo(0, y),
            SetBackgroundColor(background),
            SetForegroundColor(foreground),
            SetAttribute(if selected || active_file {
                Attribute::Bold
            } else {
                Attribute::NormalIntensity
            }),
            Print(label),
            SetAttribute(Attribute::NormalIntensity)
        )?;
        if app.explorer_input_kind.is_none() && !app.project.filter.is_empty() {
            if let Some((match_start, match_end)) =
                case_insensitive_match_range(&display_name, &app.project.filter)
            {
                let x = UnicodeWidthStr::width(prefix.as_str())
                    + UnicodeWidthStr::width(&display_name[..match_start]);
                let matched = &display_name[match_start..match_end];
                let matched_width = UnicodeWidthStr::width(matched);
                let right_width = UnicodeWidthStr::width(right.as_str());
                if x + matched_width < width.saturating_sub(right_width) {
                    queue!(
                        out,
                        MoveTo(x as u16, y),
                        SetBackgroundColor(app.theme.search_background),
                        SetForegroundColor(app.theme.search_foreground),
                        SetAttribute(Attribute::Bold),
                        Print(matched),
                        SetAttribute(Attribute::NormalIntensity)
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn case_insensitive_match_range(text: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    if text.is_ascii() && needle.is_ascii() {
        let start = text
            .to_ascii_lowercase()
            .find(&needle.to_ascii_lowercase())?;
        return Some((start, start + needle.len()));
    }
    text.find(needle).map(|start| (start, start + needle.len()))
}

fn explorer_entry_foreground(app: &App, entry: &crate::project::ProjectEntry) -> Color {
    if entry.ignored || entry.hidden {
        return app.theme.muted;
    }
    if entry.is_symlink {
        return app.theme.punctuation;
    }
    if entry.is_dir {
        return app.theme.heading;
    }
    if entry.is_executable == Some(true) {
        return app.theme.success;
    }

    match entry
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some(
            "rs" | "go" | "cs" | "py" | "js" | "jsx" | "ts" | "tsx" | "java" | "c" | "h" | "cpp"
            | "hpp",
        ) => app.theme.keyword,
        Some("md" | "txt" | "rst" | "adoc") => app.theme.string,
        Some("json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf") => app.theme.number,
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico") => app.theme.type_name,
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar") => app.theme.comment,
        _ => app.theme.foreground,
    }
}

fn explorer_breadcrumb(app: &App, max_width: usize) -> String {
    let selected = app.project.entries.get(app.project.selected);
    let directory = selected.map_or(app.project.root.as_path(), |entry| {
        if entry.is_dir {
            entry.path.as_path()
        } else {
            entry.path.parent().unwrap_or(app.project.root.as_path())
        }
    });
    let relative = directory
        .strip_prefix(&app.project.root)
        .unwrap_or(directory);
    let mut breadcrumb = app.project.root_name().to_string();
    for component in relative.components() {
        let name = component.as_os_str().to_string_lossy();
        if !name.is_empty() {
            breadcrumb.push_str(" › ");
            breadcrumb.push_str(&name);
        }
    }
    compact_text(&breadcrumb, max_width)
}

fn explorer_header_controls(width: usize) -> &'static str {
    if width >= 36 {
        "[+] [D] [R] [-]"
    } else {
        ""
    }
}

pub fn explorer_header_action_at(width: usize, column: usize) -> Option<char> {
    let controls = explorer_header_controls(width);
    if controls.is_empty() || column >= width {
        return None;
    }
    let start = width.saturating_sub(UnicodeWidthStr::width(controls));
    let offset = column.checked_sub(start)?;
    match offset {
        1 => Some('+'),
        5 => Some('D'),
        9 => Some('R'),
        13 => Some('-'),
        _ => None,
    }
}

fn draw_outline<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    width: usize,
) -> io::Result<()> {
    let symbols = app.outline_symbols();
    for row in 0..rows {
        let y = top + row as u16;
        queue!(
            out,
            MoveTo(0, y),
            SetBackgroundColor(app.theme.prompt_bar),
            Print(" ".repeat(width))
        )?;
        if row == 0 {
            queue!(
                out,
                MoveTo(0, y),
                SetForegroundColor(app.theme.top_bar_text),
                SetAttribute(Attribute::Bold),
                Print(pad_or_truncate(
                    &format!(" SYMBOLS ▾ {} items", symbols.len()),
                    width
                )),
                SetAttribute(Attribute::NormalIntensity)
            )?;
            continue;
        }
        let index = app.outline_scroll + row - 1;
        let Some(symbol) = symbols.get(index) else {
            continue;
        };
        let selected = index == app.outline_selected;
        let background = if selected && app.explorer_focused {
            app.theme.normal_mode
        } else if selected {
            app.theme.current_line
        } else {
            app.theme.prompt_bar
        };
        let foreground = if selected && app.explorer_focused {
            app.theme.background
        } else if symbol.kind == "type" {
            app.theme.type_name
        } else {
            app.theme.foreground
        };
        let label = format!(
            " {}{} {}  {}",
            "  ".repeat(symbol.depth),
            if symbol.kind == "type" { "◆" } else { "ƒ" },
            symbol.name,
            symbol.start_line + 1
        );
        queue!(
            out,
            MoveTo(0, y),
            SetBackgroundColor(background),
            SetForegroundColor(foreground),
            SetAttribute(if selected {
                Attribute::Bold
            } else {
                Attribute::NormalIntensity
            }),
            Print(pad_or_truncate(&label, width)),
            SetAttribute(Attribute::NormalIntensity)
        )?;
    }
    Ok(())
}

fn draw_vertical_separator<W: Write>(
    out: &mut W,
    app: &App,
    x: u16,
    top: u16,
    rows: usize,
) -> io::Result<()> {
    for row in 0..rows {
        queue!(
            out,
            MoveTo(x, top + row as u16),
            SetBackgroundColor(app.theme.background),
            SetForegroundColor(app.theme.border),
            Print("│")
        )?;
    }
    Ok(())
}

fn draw_terminal<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    width: u16,
) -> io::Result<()> {
    if rows == 0 {
        return Ok(());
    }
    let separator_row = top.saturating_sub(1);
    let cwd = app
        .terminal_cwd()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let title = format!(" TERMINAL · {} · {} ", app.terminal_shell_name(), cwd);
    let close_hint = " Ctrl-Backtick focus · Ctrl-Shift-Backtick close ";
    queue!(
        out,
        MoveTo(0, separator_row),
        SetBackgroundColor(app.theme.background),
        SetForegroundColor(if app.terminal_exited() {
            app.theme.error
        } else if app.terminal_focused {
            app.theme.insert_mode
        } else {
            app.theme.border
        }),
        SetAttribute(Attribute::Bold),
        Print(fit_bar(&title, close_hint, width as usize)),
        SetAttribute(Attribute::Reset)
    )?;

    let lines = app.terminal_lines(rows);
    let blank_rows = rows.saturating_sub(lines.len());
    for row in 0..rows {
        let text = if row < blank_rows {
            ""
        } else {
            lines[row - blank_rows].as_str()
        };
        queue!(
            out,
            MoveTo(0, top + row as u16),
            SetBackgroundColor(app.theme.prompt_bar),
            SetForegroundColor(app.theme.prompt_text),
            Print(pad_or_truncate(text, width as usize))
        )?;
    }
    Ok(())
}

fn draw_editor<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    editor_x: u16,
    editor_width: u16,
    gutter_width: usize,
) -> io::Result<()> {
    let fold_ranges = app.editor.syntax_fold_ranges();

    for screen_row in 0..rows {
        let terminal_row = top + screen_row as u16;
        let line_index = app
            .editor
            .visible_line_at(app.editor.scroll_line, screen_row);

        queue!(
            out,
            MoveTo(editor_x, terminal_row),
            SetBackgroundColor(app.theme.background),
            SetForegroundColor(app.theme.foreground),
            Print(" ".repeat(editor_width as usize))
        )?;

        let Some(line_index) = line_index else {
            if gutter_width > 0 {
                queue!(
                    out,
                    MoveTo(editor_x + (gutter_width - 2) as u16, terminal_row),
                    SetForegroundColor(app.theme.gutter),
                    Print("~")
                )?;
            }
            continue;
        };

        let is_current = line_index == app.editor.cursor.line;
        let line_background = if is_current {
            app.theme.current_line
        } else {
            app.theme.background
        };

        queue!(
            out,
            MoveTo(editor_x, terminal_row),
            SetBackgroundColor(line_background)
        )?;

        if gutter_width > 0 {
            let number_width = gutter_width.saturating_sub(3);
            let marker = if app.editor.folded_end(line_index).is_some() {
                "▶"
            } else if fold_ranges.iter().any(|(start, _)| *start == line_index) {
                "▼"
            } else {
                " "
            };
            let number = format!("{:>width$}{marker} ", line_index + 1, width = number_width);
            let number_color = if is_current {
                app.theme.gutter_current
            } else {
                app.theme.gutter
            };
            let (git_marker, git_color) = match app.git_line_change(line_index) {
                Some(crate::app::GitLineChange::Added) => ("+", app.theme.success),
                Some(crate::app::GitLineChange::Modified) => ("~", app.theme.search_mode),
                Some(crate::app::GitLineChange::Deleted) => ("-", app.theme.error),
                None => (" ", number_color),
            };

            queue!(
                out,
                SetForegroundColor(git_color),
                Print(git_marker),
                SetForegroundColor(number_color),
                Print(number)
            )?;
        }

        let line = app.editor.line_text(line_index);
        let colors = app
            .editor
            .syntax_colors_for_line(line_index, &line, &app.theme);
        let search_hits = app.search_line_hits(&line);
        let text_width = editor_width.saturating_sub(gutter_width as u16) as usize;
        let line_start = app.editor.buffer_line_to_char(line_index);
        let selections = app.editor.selection_ranges();

        render_line_text(
            out,
            &line,
            &colors,
            &search_hits,
            editor_x + gutter_width as u16,
            terminal_row,
            text_width,
            app.editor.scroll_column,
            app.editor.tab_width,
            line_background,
            app.theme.search_foreground,
            app.theme.search_background,
            line_start,
            &selections,
        )?;

        if let Some(end) = app.editor.folded_end(line_index) {
            let label = format!("  ⋯ {} lines folded", end - line_index);
            let text_column = display_width(&line, app.editor.tab_width);
            if text_column >= app.editor.scroll_column {
                let screen_column = text_column - app.editor.scroll_column;
                if screen_column < text_width {
                    queue!(
                        out,
                        MoveTo(
                            editor_x + gutter_width as u16 + screen_column as u16,
                            terminal_row
                        ),
                        SetBackgroundColor(line_background),
                        SetForegroundColor(app.theme.muted),
                        Print(pad_or_truncate(&label, text_width - screen_column))
                    )?;
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_line_text<W: Write>(
    out: &mut W,
    line: &str,
    colors: &[Color],
    search_hits: &[bool],
    x: u16,
    y: u16,
    viewport_width: usize,
    scroll_column: usize,
    tab_width: usize,
    base_background: Color,
    search_foreground: Color,
    search_background: Color,
    line_start: usize,
    selections: &[(usize, usize)],
) -> io::Result<()> {
    if viewport_width == 0 {
        return Ok(());
    }

    let mut display_column = 0usize;
    let mut screen_column = 0usize;
    let mut last_foreground = None;
    let mut last_background = None;

    for (character_index, character) in line.chars().enumerate() {
        let width = if character == '\t' {
            tab_width - (display_column % tab_width)
        } else {
            UnicodeWidthChar::width(character).unwrap_or(0)
        };

        let start = display_column;
        let end = display_column + width;
        display_column = end;

        if end <= scroll_column {
            continue;
        }

        if start >= scroll_column + viewport_width {
            break;
        }

        let highlighted = search_hits.get(character_index).copied().unwrap_or(false);
        let selected = selections
            .iter()
            .any(|(start, end)| (*start..*end).contains(&(line_start + character_index)));
        let foreground = if highlighted {
            search_foreground
        } else {
            colors.get(character_index).copied().unwrap_or(Color::White)
        };
        let background = if selected {
            Color::DarkGrey
        } else if highlighted {
            search_background
        } else {
            base_background
        };

        if last_foreground != Some(foreground) {
            queue!(out, SetForegroundColor(foreground))?;
            last_foreground = Some(foreground);
        }
        if last_background != Some(background) {
            queue!(out, SetBackgroundColor(background))?;
            last_background = Some(background);
        }

        let visible_width = width.min(viewport_width.saturating_sub(screen_column));
        if visible_width == 0 {
            break;
        }

        queue!(out, MoveTo(x + screen_column as u16, y))?;

        if character == '\t' {
            queue!(out, Print(" ".repeat(visible_width)))?;
        } else if start < scroll_column && width > 1 {
            queue!(out, Print(" ".repeat(visible_width)))?;
        } else {
            queue!(out, Print(character))?;
        }

        screen_column += visible_width;
        if screen_column >= viewport_width {
            break;
        }
    }

    if screen_column < viewport_width {
        queue!(
            out,
            MoveTo(x + screen_column as u16, y),
            SetBackgroundColor(base_background),
            Print(" ".repeat(viewport_width - screen_column))
        )?;
    }

    Ok(())
}

fn draw_file_manager<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    width: u16,
) -> io::Result<()> {
    let width = width as usize;
    if rows == 0 || width == 0 {
        return Ok(());
    }
    for row in 0..rows {
        queue!(
            out,
            MoveTo(0, top + row as u16),
            SetBackgroundColor(app.theme.background),
            SetForegroundColor(app.theme.foreground),
            Print(" ".repeat(width))
        )?;
    }

    let selection_count = app.file_manager.selected_paths.len();
    let left_header = format!(
        " 󰉋  {}",
        manager_breadcrumb(&app.file_manager.current_dir, width.saturating_sub(28))
    );
    let right_header = if app.file_manager.loading {
        " scanning… ".to_string()
    } else if selection_count > 0 {
        format!(" {selection_count} selected ")
    } else {
        format!(" {} items ", app.file_manager.visible_entries().len())
    };
    queue!(
        out,
        MoveTo(0, top),
        SetBackgroundColor(app.theme.top_bar),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(fit_bar(&left_header, &right_header, width)),
        SetAttribute(Attribute::NormalIntensity)
    )?;

    if rows < 3 {
        return Ok(());
    }
    let body_top = top + 1;
    let body_rows = rows.saturating_sub(2);
    let (parent_percent, current_percent) = app.manager_pane_ratios();
    let manager_layout = FileManagerLayout::calculate(width, parent_percent, current_percent);
    if manager_layout.parent_width > 0 {
        draw_manager_parent_pane(
            out,
            app,
            body_top,
            body_rows,
            manager_layout.parent_x,
            manager_layout.parent_width,
        )?;
    }
    draw_manager_current_pane(
        out,
        app,
        body_top,
        body_rows,
        manager_layout.current_x,
        manager_layout.current_width,
    )?;
    if manager_layout.preview_width > 0 {
        draw_manager_preview_pane(
            out,
            app,
            body_top,
            body_rows,
            manager_layout.preview_x,
            manager_layout.preview_width,
        )?;
    }

    let operation = app.file_manager.progress.as_ref().map_or_else(
        || {
            app.file_manager.last_operation.as_ref().map_or_else(
                || "Ready".to_string(),
                |summary| {
                    format!(
                        "{:?}: {} complete · {} skipped · {} failed{}",
                        summary.kind,
                        summary.completed,
                        summary.skipped,
                        summary.failures.len(),
                        if summary.cancelled {
                            " · cancelled"
                        } else {
                            ""
                        }
                    )
                },
            )
        },
        |progress| {
            format!(
                "{:?}: {}/{} · {} · Ctrl-C cancels",
                progress.kind,
                progress.completed,
                progress.total,
                progress.current.display()
            )
        },
    );
    let context_hints = if app.file_manager.progress.is_some() {
        " Ctrl-C cancel "
    } else if selection_count > 0 {
        " c copy  x cut  p paste  d duplicate  Del trash "
    } else {
        " Enter open  Ctrl-Enter split  Space select  / filter  g go to "
    };
    let left_footer = if let Some(kind) = app.manager_input_kind {
        let label = match kind {
            ManagerInputKind::Rename => "Rename",
            ManagerInputKind::BulkRename => "Bulk rename",
            ManagerInputKind::NewFile => "New file",
            ManagerInputKind::NewDirectory => "New folder",
            ManagerInputKind::GoTo => "Go to",
        };
        format!(" {label}  {}_", app.manager_input)
    } else if !app.file_manager.filter.is_empty() {
        format!(" Filter  {}_", app.file_manager.filter)
    } else {
        format!(" {operation}")
    };
    queue!(
        out,
        MoveTo(0, top + rows.saturating_sub(1) as u16),
        SetBackgroundColor(app.theme.status_bar),
        SetForegroundColor(app.theme.status_text),
        Print(fit_bar(&left_footer, context_hints, width))
    )?;
    Ok(())
}

fn manager_breadcrumb(path: &Path, max_width: usize) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                let label = match prefix.kind() {
                    Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                        format!("{}:", char::from(drive))
                    }
                    Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                        format!(
                            "⌁ {}{}{}",
                            server.to_string_lossy(),
                            MAIN_SEPARATOR,
                            share.to_string_lossy()
                        )
                    }
                    Prefix::DeviceNS(device) | Prefix::Verbatim(device) => {
                        device.to_string_lossy().into_owned()
                    }
                };
                parts.push(manager_display_line(&label));
            }
            Component::RootDir if parts.is_empty() => {
                parts.push(MAIN_SEPARATOR.to_string());
            }
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => parts.push("..".to_string()),
            Component::Normal(part) => {
                parts.push(manager_display_line(&part.to_string_lossy()));
            }
        }
    }
    if parts.is_empty() {
        return manager_display_line(&path.display().to_string());
    }

    let separator = "  ›  ";
    let full = parts.join(separator);
    if UnicodeWidthStr::width(full.as_str()) <= max_width {
        return full;
    }

    let mut compact = Vec::new();
    compact.push(parts[0].clone());
    if parts.len() > 3 {
        compact.push("…".to_string());
    }
    compact.extend(
        parts
            .iter()
            .skip(parts.len().saturating_sub(2).max(1))
            .cloned(),
    );
    compact.dedup();
    compact_text(&compact.join(separator), max_width)
}

pub fn file_manager_entry_at(
    app: &App,
    terminal_width: u16,
    terminal_height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    let layout = screen_layout(app, terminal_width, terminal_height);
    let (parent_percent, current_percent) = app.manager_pane_ratios();
    let manager_layout =
        FileManagerLayout::calculate(terminal_width as usize, parent_percent, current_percent);
    if (column as usize) <= manager_layout.current_x
        || (column as usize) >= manager_layout.current_x + manager_layout.current_width - 1
    {
        return None;
    }
    let first_entry_row = layout.content_top + 3;
    let body_bottom = layout.content_top + layout.content_height.saturating_sub(2) as u16;
    if row < first_entry_row || row >= body_bottom {
        return None;
    }
    let index = app.file_manager.scroll + (row - first_entry_row) as usize;
    (index < app.file_manager.visible_entries().len()).then_some(index)
}

fn draw_manager_parent_pane<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    x: usize,
    width: usize,
) -> io::Result<()> {
    let parent_name = app
        .file_manager
        .current_dir
        .parent()
        .and_then(Path::file_name)
        .map(|name| manager_display_line(&name.to_string_lossy()))
        .unwrap_or_else(|| "ROOT".to_string());
    draw_manager_panel_frame(
        out,
        app,
        ManagerPanel {
            top,
            rows,
            x,
            width,
        },
        &format!("PARENT  ‹  {parent_name}"),
        None,
        false,
    )?;
    let current_name = app
        .file_manager
        .current_dir
        .file_name()
        .map(|name| name.to_string_lossy());
    for (row, entry) in app
        .file_manager
        .parent_entries
        .iter()
        .take(rows.saturating_sub(2))
        .enumerate()
    {
        let active = current_name.as_deref() == Some(entry.name.as_str());
        draw_manager_entry(
            out,
            app,
            entry,
            top + row as u16 + 1,
            x + 1,
            width.saturating_sub(2),
            active,
            false,
        )?;
    }
    Ok(())
}

fn draw_manager_current_pane<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    x: usize,
    width: usize,
) -> io::Result<()> {
    let directory_name = app
        .file_manager
        .current_dir
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| app.file_manager.current_dir.display().to_string().into());
    let entries = app.file_manager.visible_entries();
    let position = if entries.is_empty() {
        "0 items".to_string()
    } else {
        format!(
            "{}/{}",
            app.file_manager
                .selected
                .saturating_add(1)
                .min(entries.len()),
            entries.len()
        )
    };
    draw_manager_panel_frame(
        out,
        app,
        ManagerPanel {
            top,
            rows,
            x,
            width,
        },
        &format!("FILES  ·  {directory_name}"),
        Some(&position),
        true,
    )?;
    let filter_text = if app.file_manager.filter.is_empty() {
        "(/) Type to filter".to_string()
    } else {
        format!("(/) {}_", app.file_manager.filter)
    };
    queue!(
        out,
        MoveTo((x + 1) as u16, top + 1),
        SetBackgroundColor(app.theme.prompt_bar),
        SetForegroundColor(if app.file_manager.filter.is_empty() {
            app.theme.muted
        } else {
            app.theme.heading
        }),
        Print(pad_or_truncate(
            &format!(" 🔎 {filter_text}"),
            width.saturating_sub(2)
        ))
    )?;
    for (screen_row, entry) in entries
        .iter()
        .skip(app.file_manager.scroll)
        .take(rows.saturating_sub(3))
        .enumerate()
    {
        let index = app.file_manager.scroll + screen_row;
        draw_manager_entry(
            out,
            app,
            entry,
            top + screen_row as u16 + 2,
            x + 1,
            width.saturating_sub(2),
            index == app.file_manager.selected,
            app.file_manager.selected_paths.contains(&entry.path),
        )?;
    }
    if let Some(error) = app.file_manager.error.as_deref() {
        queue!(
            out,
            MoveTo((x + 1) as u16, top + 2),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.error),
            Print(pad_or_truncate(
                &format!(" {error}"),
                width.saturating_sub(2)
            ))
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_manager_entry<W: Write>(
    out: &mut W,
    app: &App,
    entry: &FileEntry,
    row: u16,
    x: usize,
    width: usize,
    active: bool,
    selected: bool,
) -> io::Result<()> {
    let background = if active {
        soft_selection_background(app)
    } else if selected {
        app.theme.prompt_bar
    } else {
        app.theme.overlay
    };
    let foreground = if active {
        app.theme.heading
    } else if entry.hidden {
        app.theme.muted
    } else if entry.is_dir || entry.is_symlink {
        app.theme.heading
    } else {
        app.theme.foreground
    };
    let icon = manager_icon(entry, app.icon_mode());
    let mark = if active {
        "▌"
    } else if selected {
        "●"
    } else {
        " "
    };
    let suffix = if entry.is_dir {
        "/"
    } else if entry.is_symlink {
        "@"
    } else {
        ""
    };
    let metadata = if width >= 56 {
        format!(
            "  {:>8}  {}",
            human_size(entry.size),
            unix_time_label(entry.modified_unix_secs)
        )
    } else if width >= 38 {
        format!("  {:>8}", human_size(entry.size))
    } else {
        String::new()
    };
    let safe_name = manager_display_line(&entry.name);
    let label = fit_bar(
        &format!(" {mark} {icon} {safe_name}{suffix}"),
        &metadata,
        width,
    );
    queue!(
        out,
        MoveTo(x as u16, row),
        SetBackgroundColor(background),
        SetForegroundColor(foreground),
        Print(label)
    )?;
    Ok(())
}

fn manager_icon(entry: &FileEntry, mode: IconMode) -> &'static str {
    match (mode, entry.is_dir, entry.is_symlink) {
        (IconMode::Ascii, true, _) => "[D]",
        (IconMode::Ascii, false, true) => "[L]",
        (IconMode::Ascii, false, false) => "[F]",
        (IconMode::Nerd, true, _) => "󰉋",
        (IconMode::Nerd, false, true) => "󰌷",
        (IconMode::Nerd, false, false) => "󰈔",
        (IconMode::Unicode, true, _) => "▸",
        (IconMode::Unicode, false, true) => "↗",
        (IconMode::Unicode, false, false) => "·",
    }
}

fn draw_manager_preview_pane<W: Write>(
    out: &mut W,
    app: &App,
    top: u16,
    rows: usize,
    x: usize,
    width: usize,
) -> io::Result<()> {
    let title = app
        .file_manager
        .selected_entry()
        .map(|entry| entry.name.as_str())
        .unwrap_or("nothing selected");
    draw_manager_panel_frame(
        out,
        app,
        ManagerPanel {
            top,
            rows,
            x,
            width,
        },
        &format!("PREVIEW · {title}"),
        None,
        false,
    )?;
    let mut lines: Vec<(String, Option<Language>)> = Vec::new();
    if let Some(entry) = app.file_manager.selected_entry() {
        lines.push((format!("Size: {}", human_size(entry.size)), None));
        lines.push((
            format!("Modified: {}", unix_time_label(entry.modified_unix_secs)),
            None,
        ));
        lines.push((String::new(), None));
    }
    match &app.file_manager.preview {
        Preview::Loading => lines.push(("Loading preview…".to_string(), None)),
        Preview::Empty => lines.push(("Preview disabled or unavailable".to_string(), None)),
        Preview::Cancelled => lines.push(("Preview cancelled".to_string(), None)),
        Preview::Unsupported { reason } => {
            lines.push((format!("Preview unsupported: {reason}"), None));
        }
        Preview::Directory {
            children,
            directories,
            files,
            total_bytes,
            truncated,
        } => {
            lines.push((format!("{children} children"), None));
            lines.push((format!("{directories} directories"), None));
            lines.push((format!("{files} files"), None));
            lines.push((
                format!("{} immediate file data", human_size(*total_bytes)),
                None,
            ));
            if *truncated {
                lines.push(("Count truncated".to_string(), None));
            }
        }
        Preview::Text {
            lines: preview_lines,
            truncated,
            structured,
            language,
        } => {
            if let Some(kind) = structured {
                lines.push((format!("{kind} preview"), None));
                lines.push((String::new(), None));
            }
            lines.extend(
                preview_lines
                    .iter()
                    .cloned()
                    .map(|line| (line, Some(*language))),
            );
            if *truncated {
                lines.push(("… preview truncated".to_string(), None));
            }
        }
        Preview::Binary {
            size,
            header,
            kind,
            dimensions,
        } => {
            lines.push(((*kind).to_string(), None));
            lines.push((format!("Size: {}", human_size(*size)), None));
            if let Some((width, height)) = dimensions {
                lines.push((format!("Dimensions: {width} × {height}"), None));
            }
            lines.push((String::new(), None));
            lines.push(("Header:".to_string(), None));
            lines.push((header.clone(), None));
        }
        Preview::Symlink { target, exists } => {
            lines.push((format!("Target: {}", target.display()), None));
            lines.push((
                if *exists {
                    "Target exists".to_string()
                } else {
                    "Broken symlink".to_string()
                },
                None,
            ));
        }
        Preview::Error(error) => lines.push((format!("Preview error: {error}"), None)),
    }
    for (index, (line, language)) in lines.iter().take(rows.saturating_sub(2)).enumerate() {
        let y = top + index as u16 + 1;
        if let Some(language) = language {
            let colors = syntax::highlight_line(line, *language, &app.theme);
            render_line_text(
                out,
                line,
                &colors,
                &[],
                (x + 2) as u16,
                y,
                width.saturating_sub(4),
                0,
                4,
                app.theme.overlay,
                app.theme.foreground,
                app.theme.search_background,
                0,
                &[],
            )?;
            continue;
        }
        let safe_line = manager_display_line(line);
        let metadata = line.starts_with("Size:")
            || line.starts_with("Modified:")
            || line.starts_with("Dimensions:")
            || line.starts_with("Target:")
            || line == "Header:";
        queue!(
            out,
            MoveTo((x + 1) as u16, y),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(
                if line.starts_with("Preview error") || line == "Broken symlink" {
                    app.theme.error
                } else if metadata {
                    app.theme.heading
                } else {
                    app.theme.foreground
                }
            ),
            Print(pad_or_truncate(
                &format!(" {safe_line}"),
                width.saturating_sub(2)
            ))
        )?;
    }
    Ok(())
}

fn draw_manager_panel_frame<W: Write>(
    out: &mut W,
    app: &App,
    panel: ManagerPanel,
    title: &str,
    footer: Option<&str>,
    focused: bool,
) -> io::Result<()> {
    let ManagerPanel {
        top,
        rows,
        x,
        width,
    } = panel;
    if width < 3 || rows < 2 {
        return Ok(());
    }
    let border = if focused {
        app.theme.heading
    } else {
        app.theme.border
    };
    for offset in 0..rows {
        queue!(
            out,
            MoveTo(x as u16, top + offset as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(width))
        )?;
    }
    let inner = width.saturating_sub(2);
    let safe_title = manager_display_line(title);
    let label = compact_text(&safe_title, inner.saturating_sub(4));
    let label_width = UnicodeWidthStr::width(label.as_str());
    let remaining = inner.saturating_sub(label_width + 3);
    queue!(
        out,
        MoveTo(x as u16, top),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(border)
    )?;
    if focused {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    queue!(out, Print(format!("╭─ {label} {}╮", "─".repeat(remaining))))?;
    if focused {
        queue!(out, SetAttribute(Attribute::NormalIntensity))?;
    }
    let bottom = footer.map_or_else(
        || format!("╰{}╯", "─".repeat(inner)),
        |footer| {
            let footer = compact_text(&manager_display_line(footer), inner.saturating_sub(3));
            let footer_width = UnicodeWidthStr::width(footer.as_str());
            format!(
                "╰{} {footer} ─╯",
                "─".repeat(inner.saturating_sub(footer_width + 3))
            )
        },
    );
    queue!(
        out,
        MoveTo(x as u16, top + rows as u16 - 1),
        SetForegroundColor(border),
        Print(bottom)
    )?;
    for offset in 1..rows.saturating_sub(1) {
        queue!(
            out,
            MoveTo(x as u16, top + offset as u16),
            SetForegroundColor(border),
            Print("│"),
            MoveTo((x + width - 1) as u16, top + offset as u16),
            Print("│")
        )?;
    }
    Ok(())
}

fn draw_manager_confirmation<W: Write>(
    out: &mut W,
    app: &App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    if app.manager_conflicts > 0 {
        let box_width = (width as usize).saturating_sub(4).clamp(34, 76);
        let x = width.saturating_sub(box_width as u16) / 2;
        let y = height.saturating_sub(6) / 2;
        for row in 0..6u16 {
            queue!(
                out,
                MoveTo(x, y + row),
                SetBackgroundColor(app.theme.prompt_bar),
                SetForegroundColor(app.theme.foreground),
                Print(" ".repeat(box_width))
            )?;
        }
        queue!(
            out,
            MoveTo(x, y),
            SetForegroundColor(app.theme.heading),
            SetAttribute(Attribute::Bold),
            Print(pad_or_truncate(
                &format!(" {} paste conflict(s)", app.manager_conflicts),
                box_width
            )),
            MoveTo(x, y + 2),
            SetAttribute(Attribute::Reset),
            SetForegroundColor(app.theme.foreground),
            Print(pad_or_truncate(
                " Choose one policy for all conflicting items:",
                box_width
            )),
            MoveTo(x, y + 4),
            SetForegroundColor(app.theme.heading),
            Print(pad_or_truncate(
                " O overwrite · S skip · R rename · Esc cancel",
                box_width
            ))
        )?;
        return Ok(());
    }
    let paths = app.file_manager.selected_or_cursor_paths();
    let action = match app.manager_confirmation {
        Some(ManagerConfirmation::Trash) => "Move to trash",
        Some(ManagerConfirmation::Delete) => "PERMANENTLY DELETE",
        None => return Ok(()),
    };
    let detail = if paths.len() == 1 {
        paths[0].display().to_string()
    } else {
        format!("{} selected items", paths.len())
    };
    let box_width = (width as usize).saturating_sub(4).clamp(30, 76);
    let x = width.saturating_sub(box_width as u16) / 2;
    let y = height.saturating_sub(5) / 2;
    for row in 0..5u16 {
        queue!(
            out,
            MoveTo(x, y + row),
            SetBackgroundColor(app.theme.prompt_bar),
            SetForegroundColor(app.theme.foreground),
            Print(" ".repeat(box_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x, y),
        SetForegroundColor(app.theme.error),
        SetAttribute(Attribute::Bold),
        Print(pad_or_truncate(&format!(" {action}?"), box_width)),
        MoveTo(x, y + 2),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(app.theme.foreground),
        Print(pad_or_truncate(&format!(" {detail}"), box_width)),
        MoveTo(x, y + 4),
        SetForegroundColor(app.theme.heading),
        Print(pad_or_truncate(
            " Enter/Y confirm · Esc/N cancel",
            box_width
        ))
    )?;
    Ok(())
}

fn draw_manager_context_menu<W: Write>(
    out: &mut W,
    app: &App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let Some((column, row)) = app.manager_context_menu else {
        return Ok(());
    };
    let menu_width = 28usize.min(width.saturating_sub(2) as usize);
    let menu_height = 9u16;
    let x = column.min(width.saturating_sub(menu_width as u16 + 1));
    let y = row.min(height.saturating_sub(menu_height + 1));
    let items = [
        (" Enter", "Open"),
        (" Ctrl-Enter", "Open in split"),
        (" c / x", "Copy / cut"),
        (" p", "Paste"),
        (" d", "Duplicate"),
        (" F2", "Rename"),
        (" Delete", "Move to trash"),
    ];
    for offset in 0..menu_height {
        queue!(
            out,
            MoveTo(x, y + offset),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(menu_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x, y),
        SetBackgroundColor(app.theme.normal_mode),
        SetForegroundColor(app.theme.background),
        SetAttribute(Attribute::Bold),
        Print(pad_or_truncate("  FILE ACTIONS", menu_width)),
        SetAttribute(Attribute::Reset)
    )?;
    for (index, (key, label)) in items.iter().enumerate() {
        queue!(
            out,
            MoveTo(x, y + index as u16 + 1),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.heading),
            Print(pad_or_truncate(key, 12)),
            SetForegroundColor(app.theme.overlay_text),
            Print(pad_or_truncate(label, menu_width.saturating_sub(12)))
        )?;
    }
    queue!(
        out,
        MoveTo(x, y + menu_height - 1),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(" Esc  close", menu_width))
    )
}

fn draw_status_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    let mode_color = if app.terminal_focused {
        app.theme.insert_mode
    } else if app.explorer_focused {
        app.theme.normal_mode
    } else {
        match app.mode {
            Mode::Normal => app.theme.normal_mode,
            Mode::Insert => app.theme.insert_mode,
            Mode::Search | Mode::ProjectSearch | Mode::FilePicker => app.theme.search_mode,
            Mode::KeyBrowser | Mode::SettingsBrowser => app.theme.command_mode,
            Mode::Command | Mode::Help => app.theme.command_mode,
            Mode::QuitConfirm | Mode::TabCloseConfirm | Mode::ReloadConfirm => app.theme.error,
            Mode::GitDiff
            | Mode::GitHistory
            | Mode::ThemeGallery
            | Mode::KeymapGallery
            | Mode::ContextMenu
            | Mode::Dashboard
            | Mode::FileManager => app.theme.command_mode,
        }
    };

    let language_name = app.language_name();
    let left = format!(
        " {}  │  {} keys  │  Tab {}/{}  │  {} lines  │  {} ",
        app.active_panel_label(),
        app.keymap_profile().name(),
        app.editor.active_index() + 1,
        app.editor.len(),
        app.editor.line_count(),
        language_name
    );
    let background = app.background_status();
    let background_label = background.as_ref().map(|(label, _)| label.as_str());
    let right = if app.explorer_focused {
        format!(
            " {}{} {}/{} items  ",
            background_label.unwrap_or(""),
            if background_label.is_some() {
                "  │ "
            } else {
                ""
            },
            app.project
                .selected
                .saturating_add(1)
                .min(app.project.entries.len()),
            app.project.entries.len()
        )
    } else {
        format!(
            " {}{}Ln {}, Col {}  ",
            background_label.unwrap_or(""),
            if background_label.is_some() {
                "  │ "
            } else {
                ""
            },
            app.editor.cursor.line + 1,
            app.editor.cursor.column + 1
        )
    };

    queue!(
        out,
        MoveTo(0, row),
        SetBackgroundColor(app.theme.status_bar),
        SetForegroundColor(mode_color),
        SetAttribute(Attribute::Bold),
        Print(fit_bar(&left, &right, width as usize)),
        SetAttribute(Attribute::NormalIntensity)
    )?;

    if let Some((label, state)) = background {
        let right_width = UnicodeWidthStr::width(right.as_str());
        let x = (width as usize)
            .saturating_sub(right_width)
            .saturating_add(1) as u16;
        let color = match state {
            BackgroundState::Working => app.theme.search_mode,
            BackgroundState::Ready => app.theme.success,
            BackgroundState::Warning => app.theme.gutter_current,
            BackgroundState::Error => app.theme.error,
        };
        queue!(
            out,
            MoveTo(x, row),
            SetBackgroundColor(app.theme.status_bar),
            SetForegroundColor(color),
            SetAttribute(Attribute::Bold),
            Print(label),
            SetAttribute(Attribute::NormalIntensity)
        )?;
    }

    if app.mode == Mode::Command {
        if let Some((start, end)) = app.command_selection() {
            if start <= end && end <= app.command_input.len() {
                let before = UnicodeWidthStr::width(&app.command_input[..start]);
                let selected = &app.command_input[start..end];
                queue!(
                    out,
                    MoveTo((1 + before).min(width as usize) as u16, row),
                    SetBackgroundColor(app.theme.search_background),
                    SetForegroundColor(app.theme.search_foreground),
                    Print(pad_or_truncate(
                        selected,
                        width.saturating_sub(1 + before as u16) as usize
                    ))
                )?;
            }
        }
    }
    Ok(())
}

fn draw_git_diff<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let panel_width = width.saturating_sub(6) as usize;
    let panel_height = height.saturating_sub(4) as usize;
    let x = 3u16;
    let y = 2u16;
    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x, y),
        SetAttribute(Attribute::Bold),
        SetForegroundColor(app.theme.top_bar_text),
        Print(pad_or_truncate(
            &format!(" {}  ·  Esc closes  ·  ↑↓ scroll", app.git_diff_title),
            panel_width
        )),
        SetAttribute(Attribute::Reset)
    )?;
    for (row, line) in app
        .git_diff_lines
        .iter()
        .skip(app.git_diff_scroll)
        .take(panel_height.saturating_sub(2))
        .enumerate()
    {
        let color = if line.starts_with('+') {
            app.theme.success
        } else if line.starts_with('-') {
            app.theme.error
        } else if line.starts_with("@@") {
            app.theme.heading
        } else {
            app.theme.overlay_text
        };
        queue!(
            out,
            MoveTo(x, y + 1 + row as u16),
            SetForegroundColor(color),
            Print(pad_or_truncate(line, panel_width))
        )?;
    }
    Ok(())
}

fn draw_git_history<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let panel_width = width.saturating_sub(6) as usize;
    let panel_height = height.saturating_sub(4) as usize;
    let x = 3u16;
    let y = 2u16;
    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x, y),
        SetAttribute(Attribute::Bold),
        SetForegroundColor(app.theme.top_bar_text),
        Print(pad_or_truncate(
            " GIT HISTORY  ·  Enter inspects commit  ·  Esc closes",
            panel_width
        )),
        SetAttribute(Attribute::Reset)
    )?;
    if app.git_history.is_empty() {
        queue!(out, MoveTo(x, y + 2), Print(" No commits for this file."))?;
    }
    for (index, entry) in app
        .git_history
        .iter()
        .take(panel_height.saturating_sub(2))
        .enumerate()
    {
        let selected = index == app.git_history_selected;
        let label = format!(" {}  {}", entry.hash, entry.summary);
        queue!(
            out,
            MoveTo(x, y + 1 + index as u16),
            SetBackgroundColor(if selected {
                app.theme.command_mode
            } else {
                app.theme.overlay
            }),
            SetForegroundColor(if selected {
                app.theme.background
            } else {
                app.theme.overlay_text
            }),
            Print(pad_or_truncate(&label, panel_width))
        )?;
    }
    Ok(())
}

fn draw_key_browser<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let panel_width = (width as usize).saturating_sub(6).min(96);
    let panel_height = (height as usize).saturating_sub(4);
    if panel_width < 30 || panel_height < 6 {
        return Ok(());
    }
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = 1u16;
    let list_rows = panel_height.saturating_sub(3);

    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }

    queue!(
        out,
        MoveTo(x + 2, y),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(pad_or_truncate(
            &format!(
                "KEY BINDINGS · {} profile · search: {}▏",
                app.keymap_profile().name(),
                app.key_browser_input
            ),
            panel_width.saturating_sub(4)
        )),
        SetAttribute(Attribute::Reset)
    )?;

    let rows = app.keybinding_rows();
    let scroll = app.key_browser_scroll.min(rows.len().saturating_sub(1));
    let chord_width = 18usize;
    let description_width = (panel_width.saturating_sub(4 + chord_width + 2)) / 2;
    for row_index in 0..list_rows {
        let Some((chord, description, note)) = rows.get(scroll + row_index) else {
            break;
        };
        let row_y = y + 1 + row_index as u16;
        let text = format!(
            "{}  {}  {note}",
            pad_or_truncate(chord, chord_width),
            pad_or_truncate(description, description_width),
        );
        queue!(
            out,
            MoveTo(x + 2, row_y),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(if note.contains('⚠') {
                app.theme.gutter_current
            } else if note.starts_with("custom") {
                app.theme.success
            } else {
                app.theme.overlay_text
            }),
            Print(pad_or_truncate(&text, panel_width.saturating_sub(4)))
        )?;
    }

    queue!(
        out,
        MoveTo(x + 2, y + panel_height as u16 - 1),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(
            &format!(
                "{} binding(s) · :bind <action> <keys> rebinds · :bind <action> default resets · :bindreset resets all · Esc closes",
                rows.len()
            ),
            panel_width.saturating_sub(4)
        ))
    )?;
    Ok(())
}

fn draw_settings_browser<W: Write>(
    out: &mut W,
    app: &mut App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let panel_width = (width as usize).saturating_sub(6).min(112);
    let panel_height = (height as usize).saturating_sub(4);
    if panel_width < 48 || panel_height < 8 {
        return Ok(());
    }
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = 1u16;
    let visible_rows = panel_height.saturating_sub(3) / 2;
    let rows = app.setting_rows();
    app.settings_browser_selected = app
        .settings_browser_selected
        .min(rows.len().saturating_sub(1));
    if app.settings_browser_selected < app.settings_browser_scroll {
        app.settings_browser_scroll = app.settings_browser_selected;
    } else if visible_rows > 0
        && app.settings_browser_selected >= app.settings_browser_scroll + visible_rows
    {
        app.settings_browser_scroll = app.settings_browser_selected + 1 - visible_rows;
    }

    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }

    queue!(
        out,
        MoveTo(x + 2, y),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(pad_or_truncate(
            &format!("SETTINGS · search: {}▏", app.settings_browser_input),
            panel_width.saturating_sub(4),
        )),
        SetAttribute(Attribute::Reset)
    )?;

    if rows.is_empty() {
        queue!(
            out,
            MoveTo(x + 2, y + 2),
            SetForegroundColor(app.theme.muted),
            Print(pad_or_truncate(
                &format!(
                    "No settings match {}",
                    if app.settings_browser_input.is_empty() {
                        "the current search"
                    } else {
                        &app.settings_browser_input
                    }
                ),
                panel_width.saturating_sub(4),
            ))
        )?;
    } else {
        for row_index in 0..visible_rows {
            let Some(index) = app
                .settings_browser_scroll
                .checked_add(row_index)
                .filter(|index| *index < rows.len())
            else {
                break;
            };
            let setting = &rows[index];
            let selected = index == app.settings_browser_selected;
            let background = if selected {
                app.theme.command_mode
            } else {
                app.theme.overlay
            };
            let foreground = if selected {
                app.theme.background
            } else {
                app.theme.overlay_text
            };
            let restart = if setting.restart_required {
                "↻ next launch"
            } else {
                "live"
            };
            let headline = format!(
                "{:<20}  {:<18}  default {:<10}  {}",
                setting.name, setting.current, setting.default, restart
            );
            let headline_row = y + 1 + (row_index * 2) as u16;
            queue!(
                out,
                MoveTo(x + 2, headline_row),
                SetBackgroundColor(background),
                SetForegroundColor(foreground),
                SetAttribute(if selected {
                    Attribute::Bold
                } else {
                    Attribute::Reset
                }),
                Print(pad_or_truncate(&headline, panel_width.saturating_sub(4))),
                SetAttribute(Attribute::Reset),
                MoveTo(x + 4, headline_row + 1),
                SetBackgroundColor(app.theme.overlay),
                SetForegroundColor(if selected {
                    app.theme.heading
                } else {
                    app.theme.muted
                }),
                Print(pad_or_truncate(
                    &format!("{} · valid: {}", setting.description, setting.validation),
                    panel_width.saturating_sub(8),
                ))
            )?;
        }
    }

    queue!(
        out,
        MoveTo(x + 2, y + panel_height as u16 - 1),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(
            &format!(
                "{} setting(s) · :set changes values · Enter inspects · Esc closes",
                rows.len()
            ),
            panel_width.saturating_sub(4),
        ))
    )?;
    Ok(())
}

fn draw_file_picker<W: Write>(
    out: &mut W,
    app: &mut App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let panel_width = 72usize.min(width.saturating_sub(4) as usize);
    let panel_height = (height as usize).saturating_sub(6).clamp(6, 24);
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = 2u16;
    let list_rows = panel_height.saturating_sub(3);

    let state = &mut app.file_picker;
    state.selected = state.selected.min(state.matches.len().saturating_sub(1));
    if state.selected < state.scroll {
        state.scroll = state.selected;
    } else if list_rows > 0 && state.selected >= state.scroll + list_rows {
        state.scroll = state.selected + 1 - list_rows;
    }

    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }

    let state = &app.file_picker;
    queue!(
        out,
        MoveTo(x + 2, y),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(pad_or_truncate(
            &format!(" OPEN FILE  {}▏", state.input),
            panel_width.saturating_sub(4)
        )),
        SetAttribute(Attribute::Reset)
    )?;

    for row in 0..list_rows {
        let index = state.scroll + row;
        let Some(matched) = state.matches.get(index) else {
            break;
        };
        let selected = index == state.selected;
        let name = &state.files[matched.file_index];
        let marker = if matched.recent { "★" } else { " " };
        let row_y = y + 1 + row as u16;
        let available = panel_width.saturating_sub(4);
        let (row_background, row_foreground) = if selected {
            (app.theme.command_mode, app.theme.background)
        } else {
            (app.theme.overlay, app.theme.overlay_text)
        };
        queue!(
            out,
            MoveTo(x + 2, row_y),
            SetBackgroundColor(row_background),
            SetForegroundColor(row_foreground),
            Print(pad_or_truncate(&format!("{marker} {name}"), available))
        )?;

        // Emphasize the matched characters when they fit on screen.
        if !selected {
            let name_chars: Vec<char> = name.chars().collect();
            for position in &matched.positions {
                let offset: usize = 2 + name_chars[..*position]
                    .iter()
                    .map(|character| UnicodeWidthChar::width(*character).unwrap_or(0))
                    .sum::<usize>();
                let Some(character) = name_chars.get(*position) else {
                    continue;
                };
                let character_width = UnicodeWidthChar::width(*character).unwrap_or(0);
                if offset + character_width > available {
                    continue;
                }
                queue!(
                    out,
                    MoveTo(x + 2 + offset as u16, row_y),
                    SetForegroundColor(app.theme.search_foreground),
                    SetBackgroundColor(app.theme.search_background),
                    Print(character)
                )?;
            }
        }
    }

    let status = if state.files.is_empty() {
        "No files found in this project".to_string()
    } else if state.matches.is_empty() {
        format!("No files match {}", state.input)
    } else {
        format!(
            "{} of {} file(s){} · ★ recent",
            state.matches.len(),
            state.files.len(),
            if state.truncated {
                " (list capped)"
            } else {
                ""
            }
        )
    };
    queue!(
        out,
        MoveTo(x + 2, y + panel_height as u16 - 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(
            &format!("{status} · Enter opens · Esc closes"),
            panel_width.saturating_sub(4)
        ))
    )?;
    Ok(())
}

/// Rows available for results inside the project-search panel.
fn project_search_list_rows(height: u16) -> usize {
    (height as usize).saturating_sub(3 + 5)
}

fn draw_project_search<W: Write>(
    out: &mut W,
    app: &mut App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let x = 2u16;
    let panel_width = (width as usize).saturating_sub(4);
    let y = 1u16;
    let panel_height = (height as usize).saturating_sub(3);
    if panel_width < 20 || panel_height < 6 {
        return Ok(());
    }
    let list_rows = project_search_list_rows(height);

    // Keep the selection visible.
    let state = &mut app.project_search;
    state.selected = state.selected.min(state.results.len().saturating_sub(1));
    if state.selected < state.scroll {
        state.scroll = state.selected;
    } else if list_rows > 0 && state.selected >= state.scroll + list_rows {
        state.scroll = state.selected + 1 - list_rows;
    }

    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }

    queue!(
        out,
        MoveTo(x + 2, y),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(pad_or_truncate(
            &format!("PROJECT SEARCH · {}", app.project.root.display()),
            panel_width.saturating_sub(4)
        )),
        SetAttribute(Attribute::Reset)
    )?;

    let mut flags = String::new();
    if app.search_options.case_sensitive {
        flags.push_str("  Aa");
    }
    if app.search_options.whole_word {
        flags.push_str("  Word");
    }
    if app.search_options.use_regex {
        flags.push_str("  .*");
    }
    let state = &app.project_search;
    let query_caret = if state.focus_replace { " " } else { "▏" };
    let replace_caret = if state.focus_replace { "▏" } else { " " };
    queue!(
        out,
        MoveTo(x + 2, y + 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.overlay_text),
        Print(pad_or_truncate(
            &fit_bar(
                &format!(" FIND     {}{query_caret}", state.query),
                &flags,
                panel_width.saturating_sub(4)
            ),
            panel_width.saturating_sub(4)
        )),
        MoveTo(x + 2, y + 2),
        Print(pad_or_truncate(
            &format!(" REPLACE  {}{replace_caret}", state.replacement),
            panel_width.saturating_sub(4)
        ))
    )?;

    let status = if let Some(error) = &state.error {
        error.clone()
    } else if state.ran_query.is_empty() {
        "Type a query and press Enter to search the project".to_string()
    } else {
        let mut text = format!(
            "{} match(es) in {} file(s)",
            state.results.len(),
            state.files_with_matches
        );
        if state.truncated {
            text.push_str(" · result limit reached (see :set in config)");
        }
        if !state.excluded.is_empty() {
            text.push_str(&format!(" · {} excluded", state.excluded.len()));
        }
        text
    };
    queue!(
        out,
        MoveTo(x + 2, y + 3),
        SetForegroundColor(if state.error.is_some() {
            app.theme.error
        } else {
            app.theme.muted
        }),
        Print(pad_or_truncate(&status, panel_width.saturating_sub(4)))
    )?;

    for row in 0..list_rows {
        let index = state.scroll + row;
        let Some(found) = state.results.get(index) else {
            break;
        };
        let selected = index == state.selected;
        let excluded = state.excluded.contains(&index);
        let relative = found
            .path
            .strip_prefix(&app.project.root)
            .unwrap_or(&found.path);
        let marker = if excluded { "✗" } else { " " };
        let prefix = format!("{marker} {}:{}: ", relative.display(), found.line + 1);

        let row_y = y + 4 + row as u16;
        let available = panel_width.saturating_sub(4);
        let (row_background, row_foreground) = if selected {
            (app.theme.command_mode, app.theme.background)
        } else if excluded {
            (app.theme.overlay, app.theme.muted)
        } else {
            (app.theme.overlay, app.theme.overlay_text)
        };
        queue!(
            out,
            MoveTo(x + 2, row_y),
            SetBackgroundColor(row_background),
            SetForegroundColor(row_foreground),
            Print(pad_or_truncate(
                &format!("{prefix}{}", found.line_text),
                available
            ))
        )?;

        // Repaint the matched fragment in the search highlight color when it
        // is inside the visible width.
        if !selected && !excluded {
            let before_width = UnicodeWidthStr::width(&found.line_text[..found.byte_start]);
            let match_text = &found.line_text[found.byte_start..found.byte_end];
            let match_width = UnicodeWidthStr::width(match_text);
            let offset = UnicodeWidthStr::width(prefix.as_str()) + before_width;
            if offset + match_width <= available {
                queue!(
                    out,
                    MoveTo(x + 2 + offset as u16, row_y),
                    SetBackgroundColor(app.theme.search_background),
                    SetForegroundColor(app.theme.search_foreground),
                    Print(match_text)
                )?;
            }
        }
    }

    queue!(
        out,
        MoveTo(x + 2, y + panel_height as u16 - 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(
            "Enter search/open · ↑↓ select · Del exclude · Tab replace · Alt-C/W/R options · Alt-A replace all · Esc close",
            panel_width.saturating_sub(4)
        ))
    )?;
    Ok(())
}

/// Which result row (if any) sits at a screen position, for mouse clicks.
pub fn project_search_result_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.mode != Mode::ProjectSearch || width < 24 {
        return None;
    }
    let list_rows = project_search_list_rows(height);
    let first_row = 5u16;
    if row < first_row || (row - first_row) as usize >= list_rows || column < 2 {
        return None;
    }
    let index = app.project_search.scroll + (row - first_row) as usize;
    (index < app.project_search.results.len()).then_some(index)
}

fn draw_theme_gallery<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let (x, y, panel_width, visible_rows, first) = theme_gallery_geometry(app, width, height);
    let panel_height = visible_rows + 4;
    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            Print(" ".repeat(panel_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x + 2, y + 1),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print("THEME GALLERY · live preview"),
        SetAttribute(Attribute::Reset)
    )?;
    for (row, (index, kind)) in ThemeKind::ALL
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_rows)
        .enumerate()
    {
        let selected = index == app.theme_gallery_selected;
        queue!(
            out,
            MoveTo(x + 2, y + 2 + row as u16),
            SetBackgroundColor(if selected {
                app.theme.command_mode
            } else {
                app.theme.overlay
            }),
            SetForegroundColor(if selected {
                app.theme.background
            } else {
                app.theme.overlay_text
            }),
            Print(pad_or_truncate(
                &format!(" {}  {}", if selected { "▶" } else { " " }, kind.name()),
                panel_width.saturating_sub(4)
            ))
        )?;
    }
    queue!(
        out,
        MoveTo(x + 2, y + panel_height as u16 - 1),
        SetForegroundColor(app.theme.muted),
        Print("Hover preview · Click apply · ↑↓/Enter · Esc cancel")
    )?;
    Ok(())
}

fn theme_gallery_geometry(app: &App, width: u16, height: u16) -> (u16, u16, usize, usize, usize) {
    let panel_width = 46usize.min(width.saturating_sub(4) as usize);
    let panel_height = (ThemeKind::ALL.len() + 4).min(height.saturating_sub(2) as usize);
    let visible_rows = panel_height
        .saturating_sub(4)
        .max(1)
        .min(ThemeKind::ALL.len());
    let first = app
        .theme_gallery_selected
        .saturating_sub(visible_rows.saturating_sub(1))
        .min(ThemeKind::ALL.len().saturating_sub(visible_rows));
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height as u16) / 2;
    (x, y, panel_width, visible_rows, first)
}

fn draw_keymap_gallery<W: Write>(
    out: &mut W,
    app: &App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let panel_width = 62usize.min(width.saturating_sub(4) as usize);
    let panel_height = KeymapProfile::ALL.len() + 5;
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height as u16) / 2;
    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x + 2, y + 1),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print("KEYMAP PROFILES"),
        SetAttribute(Attribute::Reset)
    )?;
    for (index, profile) in KeymapProfile::ALL.iter().enumerate() {
        let selected = index == app.keymap_gallery_selected;
        let active = *profile == app.keymap_profile();
        let label = format!(
            " {} {:<13} {}{}",
            if selected { "▶" } else { " " },
            profile.name(),
            profile.description(),
            if active { "  ✓" } else { "" }
        );
        queue!(
            out,
            MoveTo(x + 2, y + 2 + index as u16),
            SetBackgroundColor(if selected {
                app.theme.command_mode
            } else {
                app.theme.overlay
            }),
            SetForegroundColor(if selected {
                app.theme.background
            } else {
                app.theme.overlay_text
            }),
            Print(pad_or_truncate(&label, panel_width.saturating_sub(4)))
        )?;
    }
    queue!(
        out,
        MoveTo(x + 2, y + panel_height as u16 - 1),
        SetForegroundColor(app.theme.muted),
        Print("Click/Enter apply · ↑↓ select · Esc cancel")
    )?;
    Ok(())
}

pub fn keymap_gallery_item_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.mode != Mode::KeymapGallery {
        return None;
    }
    let panel_width = 62usize.min(width.saturating_sub(4) as usize);
    let panel_height = KeymapProfile::ALL.len() + 5;
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height as u16) / 2;
    let first = y.saturating_add(2);
    if column < x
        || column >= x.saturating_add(panel_width as u16)
        || row < first
        || row >= first.saturating_add(KeymapProfile::ALL.len() as u16)
    {
        return None;
    }
    Some((row - first) as usize)
}

fn context_menu_geometry(app: &App, width: u16, height: u16) -> Option<(u16, u16, usize)> {
    let menu = app.context_menu.as_ref()?;
    let content_width = menu
        .actions
        .iter()
        .map(|action| {
            let hint = action.hint();
            UnicodeWidthStr::width(action.label())
                + if hint.is_empty() {
                    0
                } else {
                    UnicodeWidthStr::width(hint) + 2
                }
        })
        .max()
        .unwrap_or(12)
        + 4;
    let menu_width = content_width.clamp(20, width.saturating_sub(2) as usize);
    let menu_height = menu.actions.len().saturating_add(2) as u16;
    let x = menu.x.min(width.saturating_sub(menu_width as u16 + 1));
    let y = menu.y.min(height.saturating_sub(menu_height + 1));
    Some((x, y, menu_width))
}

fn draw_context_menu<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let Some(menu) = app.context_menu.as_ref() else {
        return Ok(());
    };
    let Some((x, y, menu_width)) = context_menu_geometry(app, width, height) else {
        return Ok(());
    };
    queue!(
        out,
        MoveTo(x, y),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.border),
        Print(format!("┌{}┐", "─".repeat(menu_width.saturating_sub(2))))
    )?;
    for (index, action) in menu.actions.iter().enumerate() {
        let selected = index == menu.selected;
        let hint = action.hint();
        let inner = menu_width.saturating_sub(2);
        let label = if hint.is_empty() {
            format!("  {}", action.label())
        } else {
            let left = format!("  {}", action.label());
            let used = UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(hint);
            format!(
                "{}{}{} ",
                left,
                " ".repeat(inner.saturating_sub(used + 1)),
                hint
            )
        };
        queue!(
            out,
            MoveTo(x, y + 1 + index as u16),
            SetBackgroundColor(if selected {
                app.theme.command_mode
            } else {
                app.theme.overlay
            }),
            SetForegroundColor(if selected {
                app.theme.background
            } else {
                app.theme.overlay_text
            }),
            Print("│"),
            Print(pad_or_truncate(&label, inner)),
            Print("│")
        )?;
    }
    queue!(
        out,
        MoveTo(x, y + menu.actions.len() as u16 + 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.border),
        Print(format!("└{}┘", "─".repeat(menu_width.saturating_sub(2))))
    )?;
    Ok(())
}

fn lsp_panel_geometry(
    app: &App,
    width: u16,
    height: u16,
) -> Option<(u16, u16, usize, usize, usize)> {
    let panel = app.lsp_panel.as_ref()?;
    let panel_width = (width as usize).saturating_sub(4).clamp(30, 90);
    let visible = panel
        .items
        .len()
        .min((height as usize).saturating_sub(8).clamp(1, 14));
    let panel_height = visible + 2;
    let x = (width.saturating_sub(panel_width as u16)) / 2;
    let y = (height.saturating_sub(panel_height as u16)) / 2;
    let start = panel
        .selected
        .saturating_sub(visible.saturating_sub(1))
        .min(panel.items.len().saturating_sub(visible));
    Some((x, y, panel_width, visible, start))
}

fn draw_lsp_panel<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let Some(panel) = app.lsp_panel.as_ref() else {
        return Ok(());
    };
    let Some((x, y, panel_width, visible, start)) = lsp_panel_geometry(app, width, height) else {
        return Ok(());
    };
    let title = format!(
        "┌ {} {}┐",
        panel.title,
        "─".repeat(panel_width.saturating_sub(panel.title.chars().count() + 4))
    );
    queue!(
        out,
        MoveTo(x, y),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.border),
        Print(pad_or_truncate(&title, panel_width))
    )?;
    for row in 0..visible {
        let index = start + row;
        let item = &panel.items[index];
        let selected = index == panel.selected;
        let text = if item.detail.is_empty() {
            item.label.clone()
        } else {
            format!("{}  ·  {}", item.label, item.detail)
        };
        let body = format!("│ {}", text);
        queue!(
            out,
            MoveTo(x, y + row as u16 + 1),
            SetBackgroundColor(if selected {
                app.theme.command_mode
            } else {
                app.theme.overlay
            }),
            SetForegroundColor(if selected {
                app.theme.background
            } else {
                app.theme.overlay_text
            }),
            Print(pad_or_truncate(&body, panel_width.saturating_sub(1))),
            Print("│")
        )?;
    }
    queue!(
        out,
        MoveTo(x, y + visible as u16 + 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.border),
        Print(format!("└{}┘", "─".repeat(panel_width.saturating_sub(2))))
    )?;
    Ok(())
}

pub fn lsp_panel_item_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    let (x, y, panel_width, visible, start) = lsp_panel_geometry(app, width, height)?;
    if column <= x || column >= x + panel_width as u16 - 1 || row <= y || row > y + visible as u16 {
        return None;
    }
    Some(start + (row - y - 1) as usize)
}

pub fn context_menu_action_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    let menu = app.context_menu.as_ref()?;
    let (x, y, menu_width) = context_menu_geometry(app, width, height)?;
    if column <= x || column >= x.saturating_add(menu_width as u16).saturating_sub(1) {
        return None;
    }
    let first = y.saturating_add(1);
    if row < first || row >= first.saturating_add(menu.actions.len() as u16) {
        return None;
    }
    Some((row - first) as usize)
}

fn dashboard_geometry(app: &App, width: u16, height: u16) -> (u16, u16, usize, usize) {
    let panel_width = 72usize.min(width.saturating_sub(4) as usize);
    let recent_rows = app.recent_projects().len().clamp(1, 10);
    let panel_height = recent_rows + 11;
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height as u16) / 2;
    (x, y, panel_width, recent_rows)
}

fn draw_dashboard<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let (x, y, panel_width, recent_rows) = dashboard_geometry(app, width, height);
    let panel_height = recent_rows + 11;
    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }
    let title = "CARET";
    queue!(
        out,
        MoveTo(
            x + (panel_width.saturating_sub(title.len()) / 2) as u16,
            y + 1
        ),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(title),
        SetAttribute(Attribute::Reset)
    )?;
    let subtitle = "Fast, focused editing in any terminal";
    queue!(
        out,
        MoveTo(
            x + (panel_width.saturating_sub(subtitle.len()) / 2) as u16,
            y + 2
        ),
        SetForegroundColor(app.theme.muted),
        Print(subtitle)
    )?;
    queue!(
        out,
        MoveTo(x + 3, y + 4),
        SetForegroundColor(app.theme.heading),
        SetAttribute(Attribute::Bold),
        Print("RECENT PROJECTS"),
        SetAttribute(Attribute::Reset)
    )?;

    if app.recent_projects().is_empty() {
        queue!(
            out,
            MoveTo(x + 3, y + 5),
            SetForegroundColor(app.theme.muted),
            Print("No recent projects yet — open the current folder to begin.")
        )?;
    } else {
        for (index, path) in app.recent_projects().iter().take(10).enumerate() {
            let selected = index == app.dashboard_selected;
            let hovered = app.dashboard_hover == Some(index);
            let active = selected || hovered;
            let exists = path.is_dir();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Project");
            let label = format!(
                " {} {:<20}  {}{}",
                if active { "▶" } else { " " },
                name,
                path.display(),
                if exists { "" } else { "  (missing)" }
            );
            queue!(
                out,
                MoveTo(x + 2, y + 5 + index as u16),
                SetBackgroundColor(if active {
                    app.theme.command_mode
                } else {
                    app.theme.overlay
                }),
                SetForegroundColor(if active {
                    app.theme.background
                } else if exists {
                    app.theme.overlay_text
                } else {
                    app.theme.error
                }),
                Print(pad_or_truncate(&label, panel_width.saturating_sub(4)))
            )?;
        }
    }

    let actions_y = y + 6 + recent_rows as u16;
    let current_hit = app.recent_projects().len();
    let path_hit = current_hit + 1;
    queue!(
        out,
        MoveTo(x + 3, actions_y),
        SetBackgroundColor(if app.dashboard_hover == Some(current_hit) {
            app.theme.command_mode
        } else {
            app.theme.overlay
        }),
        SetForegroundColor(if app.dashboard_hover == Some(current_hit) {
            app.theme.background
        } else {
            app.theme.top_bar_text
        }),
        SetAttribute(Attribute::Bold),
        Print(" [C] Open Current Folder "),
        SetAttribute(Attribute::Reset)
    )?;
    queue!(
        out,
        MoveTo(x + 3, actions_y + 1),
        SetBackgroundColor(if app.dashboard_hover == Some(path_hit) {
            app.theme.command_mode
        } else {
            app.theme.overlay
        }),
        SetForegroundColor(if app.dashboard_hover == Some(path_hit) {
            app.theme.background
        } else {
            app.theme.top_bar_text
        }),
        SetAttribute(Attribute::Bold),
        Print(" [E] Open Path… "),
        SetAttribute(Attribute::Reset)
    )?;
    queue!(
        out,
        MoveTo(x + 3, actions_y + 3),
        SetForegroundColor(app.theme.muted),
        Print("↑↓ select · Enter open · F1 help · Q quit")
    )?;
    Ok(())
}

/// Recent rows use 0..N; N is Current Folder and N+1 is Open Path.
pub fn dashboard_hit_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.mode != Mode::Dashboard {
        return None;
    }
    let (x, y, panel_width, recent_rows) = dashboard_geometry(app, width, height);
    if column < x || column >= x.saturating_add(panel_width as u16) {
        return None;
    }
    let recent_start = y + 5;
    if !app.recent_projects().is_empty()
        && row >= recent_start
        && row < recent_start.saturating_add(app.recent_projects().len().min(10) as u16)
    {
        return Some((row - recent_start) as usize);
    }
    let actions_y = y + 6 + recent_rows as u16;
    if row == actions_y && column >= x + 3 && column < x + 27 {
        return Some(app.recent_projects().len());
    }
    if row == actions_y + 1 && column >= x + 3 && column < x + 22 {
        return Some(app.recent_projects().len() + 1);
    }
    None
}

/// Returns the theme row under the mouse, if it is inside the gallery list.
pub fn theme_gallery_item_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.mode != Mode::ThemeGallery {
        return None;
    }
    let (x, y, panel_width, visible_rows, first_index) = theme_gallery_geometry(app, width, height);
    if column < x || column >= x.saturating_add(panel_width as u16) {
        return None;
    }
    let list_start = y.saturating_add(2);
    if row < list_start || row >= list_start.saturating_add(visible_rows as u16) {
        return None;
    }
    Some(first_index + (row - list_start) as usize)
}

pub fn theme_gallery_contains(app: &App, width: u16, height: u16, column: u16, row: u16) -> bool {
    if app.mode != Mode::ThemeGallery {
        return false;
    }
    let (x, y, panel_width, visible_rows, _) = theme_gallery_geometry(app, width, height);
    column >= x
        && column < x.saturating_add(panel_width as u16)
        && row >= y
        && row < y.saturating_add((visible_rows + 4) as u16)
}

/// Rows above the list inside the palette: framed search field and spacing.
const PALETTE_LIST_TOP: u16 = 5;
const PALETTE_ITEM_HEIGHT: usize = 2;
/// The list rows plus `PALETTE_LIST_TOP` and the footer.
const PALETTE_CHROME_ROWS: usize = PALETTE_LIST_TOP as usize + 2;

/// `(x, y, panel_width, visible_rows, first_index)` for the command palette.
fn command_palette_geometry(app: &App, width: u16, height: u16) -> (u16, u16, usize, usize, usize) {
    let total = app.command_suggestions().len();
    let available_width = width.saturating_sub(4) as usize;
    let panel_width = (width as usize * 3 / 5)
        .max(74.min(available_width))
        .min(110)
        .min(available_width)
        .max(1);
    let visible_rows = total
        .min(COMMAND_PALETTE_ROWS)
        .min((height as usize).saturating_sub(PALETTE_CHROME_ROWS + 2) / PALETTE_ITEM_HEIGHT)
        .max(1);
    let panel_height = visible_rows * PALETTE_ITEM_HEIGHT + PALETTE_CHROME_ROWS;
    let first = command_suggestion_window_start(total, app.command_suggestion_scroll, visible_rows);
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = if height as usize >= panel_height + 4 {
        2
    } else {
        height.saturating_sub(panel_height as u16) / 2
    };
    (x, y, panel_width, visible_rows, first)
}

fn draw_command_palette<W: Write>(
    out: &mut W,
    app: &App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    if app.mode != Mode::Command {
        return Ok(());
    }

    let (x, y, panel_width, visible_rows, first) = command_palette_geometry(app, width, height);
    let matches = app.command_matches();
    let inner = panel_width.saturating_sub(6);
    let panel_height = visible_rows * PALETTE_ITEM_HEIGHT + PALETTE_CHROME_ROWS;

    for row in 0..panel_height {
        queue!(
            out,
            MoveTo(x, y + row as u16),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.overlay_text),
            Print(" ".repeat(panel_width))
        )?;
    }
    queue!(
        out,
        MoveTo(x, y),
        SetForegroundColor(app.theme.border),
        Print(format!("╭{}╮", "─".repeat(panel_width.saturating_sub(2)))),
        MoveTo(x, y + panel_height as u16 - 1),
        Print(format!("╰{}╯", "─".repeat(panel_width.saturating_sub(2))))
    )?;
    for offset in 1..panel_height.saturating_sub(1) {
        queue!(
            out,
            MoveTo(x, y + offset as u16),
            SetForegroundColor(app.theme.border),
            Print("│"),
            MoveTo(x + panel_width as u16 - 1, y + offset as u16),
            Print("│")
        )?;
    }

    // The framed input and roomy rows intentionally mirror a graphical
    // launcher while remaining terminal-native.
    let empty = app.command_input.is_empty();
    let field = if empty {
        "> Type a command name…".to_string()
    } else {
        format!("> {}", app.command_input)
    };
    queue!(
        out,
        MoveTo(x + 2, y + 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.border),
        Print(format!("╭{}╮", "─".repeat(panel_width.saturating_sub(6)))),
        MoveTo(x + 2, y + 2),
        SetBackgroundColor(app.theme.prompt_bar),
        SetForegroundColor(app.theme.border),
        Print("│"),
        SetForegroundColor(if empty {
            app.theme.muted
        } else {
            app.theme.prompt_text
        }),
        Print(pad_or_truncate(&field, inner)),
        SetForegroundColor(app.theme.border),
        Print("│"),
        MoveTo(x + 2, y + 3),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.heading),
        Print(format!("╰{}╯", "─".repeat(panel_width.saturating_sub(6))))
    )?;

    if matches.is_empty() {
        queue!(
            out,
            MoveTo(x + 3, y + PALETTE_LIST_TOP + 1),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(app.theme.muted),
            Print(pad_or_truncate("No commands match", inner))
        )?;
    }

    for (row, (index, entry)) in matches
        .iter()
        .enumerate()
        .skip(first)
        .take(visible_rows)
        .enumerate()
    {
        let selected = index == app.command_suggestion;
        let item_y = y + PALETTE_LIST_TOP + (row * PALETTE_ITEM_HEIGHT) as u16;
        let item_width = panel_width.saturating_sub(4);
        let left = format!("  {}", entry.description);

        queue!(
            out,
            MoveTo(x + 2, item_y),
            SetBackgroundColor(if selected {
                soft_selection_background(app)
            } else {
                app.theme.overlay
            }),
            SetForegroundColor(app.theme.overlay_text),
            Print(pad_or_truncate(&left, item_width))
        )?;
        if selected {
            queue!(
                out,
                MoveTo(x + 2, item_y),
                SetForegroundColor(app.theme.heading),
                SetAttribute(Attribute::Bold),
                Print("▌"),
                SetAttribute(Attribute::Reset)
            )?;
        }
        if let Some(chord) = entry.chord.as_deref() {
            let badge = format!(" {chord} ");
            let badge_width = UnicodeWidthStr::width(badge.as_str());
            if badge_width + 4 < item_width {
                queue!(
                    out,
                    MoveTo(x + panel_width as u16 - badge_width as u16 - 3, item_y),
                    SetBackgroundColor(app.theme.prompt_bar),
                    SetForegroundColor(if selected {
                        app.theme.heading
                    } else {
                        app.theme.muted
                    }),
                    Print(badge)
                )?;
            }
        }
    }

    let counter = if matches.is_empty() {
        String::new()
    } else {
        format!("{} of {} ", app.command_suggestion + 1, matches.len())
    };
    queue!(
        out,
        MoveTo(x + 3, y + panel_height as u16 - 2),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.muted),
        Print(fit_bar(
            " ↑↓ Navigate   Enter Run   Esc Close",
            &counter,
            inner
        ))
    )?;

    Ok(())
}

fn command_suggestion_window_start(total: usize, scroll: usize, rows: usize) -> usize {
    if rows == 0 || total <= rows {
        return 0;
    }

    scroll.min(total - rows)
}

pub fn command_suggestion_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.mode != Mode::Command {
        return None;
    }
    let (x, y, panel_width, visible_rows, first) = command_palette_geometry(app, width, height);
    if column < x || column >= x.saturating_add(panel_width as u16) {
        return None;
    }
    let start = y.saturating_add(PALETTE_LIST_TOP);
    let list_height = (visible_rows * PALETTE_ITEM_HEIGHT) as u16;
    if row >= start && row < start.saturating_add(list_height) {
        let index = first + (row - start) as usize / PALETTE_ITEM_HEIGHT;
        (index < app.command_suggestions().len()).then_some(index)
    } else {
        None
    }
}

fn draw_prompt_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    let (prompt, background, foreground) = match app.mode {
        Mode::Search => (
            app.search_panel_text(),
            app.theme.prompt_bar,
            app.theme.prompt_text,
        ),
        // The palette owns the input now, so the bar carries the hint instead
        // of repeating what is already on screen.
        Mode::Command => (String::new(), app.theme.background, app.theme.foreground),
        Mode::Help => (
            " Esc, F1, or ? closes help".to_string(),
            app.theme.prompt_bar,
            app.theme.prompt_text,
        ),
        Mode::QuitConfirm => (
            " Unsaved changes — [S] Save all & quit   [D] Discard & quit   [Esc] Cancel"
                .to_string(),
            app.theme.error,
            app.theme.background,
        ),
        Mode::TabCloseConfirm => (
            " Unsaved changes — [D] Discard & close   [Esc] Keep tab open".to_string(),
            app.theme.error,
            app.theme.background,
        ),
        Mode::ReloadConfirm => (
            format!(" {}", app.message),
            app.theme.error,
            app.theme.background,
        ),
        _ => {
            let (icon, foreground) = notification_presentation(&app.message, app);
            let prompt = if app.message.is_empty() {
                String::new()
            } else {
                format!(" {icon} {}", app.message)
            };
            (prompt, app.theme.prompt_bar, foreground)
        }
    };

    queue!(
        out,
        MoveTo(0, row),
        SetBackgroundColor(background),
        SetForegroundColor(foreground),
        SetAttribute(
            if matches!(app.mode, Mode::QuitConfirm | Mode::TabCloseConfirm) {
                Attribute::Bold
            } else {
                Attribute::NormalIntensity
            }
        ),
        Print(pad_or_truncate(&prompt, width as usize)),
        SetAttribute(Attribute::NormalIntensity)
    )
}

fn notification_presentation(message: &str, app: &App) -> (&'static str, Color) {
    let lower = message.to_ascii_lowercase();
    if lower.contains("failed")
        || lower.contains("error")
        || lower.contains("cannot")
        || lower.contains("unknown command")
    {
        ("✕", app.theme.error)
    } else if lower.contains("warning")
        || lower.contains("still loading")
        || lower.contains("missing")
        || lower.starts_with("no ")
    {
        ("⚠", app.theme.gutter_current)
    } else if lower.contains("saved")
        || lower.contains("ready")
        || lower.contains("created")
        || lower.contains("opened")
        || lower.contains("applied")
    {
        ("✓", app.theme.success)
    } else {
        ("•", app.theme.prompt_text)
    }
}

fn draw_tab_close_confirm<W: Write>(
    out: &mut W,
    app: &App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let panel_width = 56usize.min(width.saturating_sub(4) as usize);
    let panel_height = 7u16.min(height.saturating_sub(2));
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height) / 2;
    let inner_width = panel_width.saturating_sub(2);

    for row in 0..panel_height {
        let border = row == 0 || row + 1 == panel_height;
        let line = if border {
            if row == 0 {
                format!("┌{}┐", "─".repeat(inner_width))
            } else {
                format!("└{}┘", "─".repeat(inner_width))
            }
        } else {
            format!("│{}│", " ".repeat(inner_width))
        };
        queue!(
            out,
            MoveTo(x, y + row),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(if border {
                app.theme.error
            } else {
                app.theme.overlay_text
            }),
            Print(line)
        )?;
    }

    queue!(
        out,
        MoveTo(x + 2, y + 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.error),
        SetAttribute(Attribute::Bold),
        Print("UNSAVED CHANGES"),
        SetAttribute(Attribute::Reset),
        MoveTo(x + 2, y + 2),
        SetForegroundColor(app.theme.overlay_text),
        Print(pad_or_truncate(
            &format!("{} has unsaved changes.", app.editor.active_title()),
            inner_width.saturating_sub(2),
        )),
        MoveTo(x + 2, y + 4),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(
            "[D] Discard & close   [Esc] Keep tab open",
            inner_width.saturating_sub(2),
        )),
        MoveTo(x + 2, y + 5),
        SetForegroundColor(app.theme.muted),
        Print(pad_or_truncate(
            "To discard without prompting: :tabclose!",
            inner_width.saturating_sub(2),
        ))
    )?;
    Ok(())
}

fn draw_hotkey_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    if app.mode == Mode::Command {
        return queue!(
            out,
            MoveTo(0, row),
            SetBackgroundColor(app.theme.background),
            Print(" ".repeat(width as usize))
        );
    }
    let mode_color = if app.terminal_focused {
        app.theme.insert_mode
    } else if app.explorer_focused {
        app.theme.normal_mode
    } else {
        match app.mode {
            Mode::Normal => app.theme.normal_mode,
            Mode::Insert => app.theme.insert_mode,
            Mode::Search | Mode::ProjectSearch | Mode::FilePicker => app.theme.search_mode,
            Mode::KeyBrowser | Mode::SettingsBrowser => app.theme.command_mode,
            Mode::Command | Mode::Help => app.theme.command_mode,
            Mode::QuitConfirm | Mode::TabCloseConfirm | Mode::ReloadConfirm => app.theme.error,
            Mode::GitDiff
            | Mode::GitHistory
            | Mode::ThemeGallery
            | Mode::KeymapGallery
            | Mode::ContextMenu
            | Mode::Dashboard
            | Mode::FileManager => app.theme.command_mode,
        }
    };

    queue!(
        out,
        MoveTo(0, row),
        SetBackgroundColor(app.theme.current_line),
        SetForegroundColor(app.theme.status_text),
        Print(" ".repeat(width as usize))
    )?;

    let mut x = 1usize;

    for (key, description) in hotkeys_for_app(app) {
        let key_text = format!(" {key} ");
        let description_text = format!(" {description}  ");
        let key_width = UnicodeWidthStr::width(key_text.as_str());
        let description_width = UnicodeWidthStr::width(description_text.as_str());
        let segment_width = key_width + description_width;

        if x + segment_width > width as usize {
            break;
        }

        queue!(
            out,
            MoveTo(x as u16, row),
            SetBackgroundColor(mode_color),
            SetForegroundColor(app.theme.background),
            SetAttribute(Attribute::Bold),
            Print(&key_text),
            SetAttribute(Attribute::NormalIntensity)
        )?;
        x += key_width;

        let clickable = *description == "Command";
        let hovered = clickable && app.hover_target == Some(HoverTarget::Command);
        queue!(
            out,
            MoveTo(x as u16, row),
            SetBackgroundColor(if hovered {
                app.theme.heading
            } else if clickable {
                mode_color
            } else {
                app.theme.current_line
            }),
            SetForegroundColor(if clickable {
                app.theme.background
            } else {
                app.theme.status_text
            }),
            SetAttribute(if clickable {
                Attribute::Bold
            } else {
                Attribute::NormalIntensity
            }),
            Print(&description_text),
            SetAttribute(Attribute::NormalIntensity)
        )?;
        x += description_width;
    }

    Ok(())
}

pub fn hotkey_action_at(app: &App, width: u16, column: u16) -> Option<&'static str> {
    let mut x = 1usize;
    let column = column as usize;

    for (key, description) in hotkeys_for_app(app) {
        let key_width = UnicodeWidthStr::width(format!(" {key} ").as_str());
        let description_width = UnicodeWidthStr::width(format!(" {description}  ").as_str());
        let end = x + key_width + description_width;
        if end > width as usize {
            break;
        }
        if (x..end).contains(&column) {
            return Some(*description);
        }
        x = end;
    }

    None
}

fn hotkeys_for_app(app: &App) -> &'static [(&'static str, &'static str)] {
    if app.lsp_panel.is_some() {
        return &[("↑↓", "Select"), ("Enter", "Open/Apply"), ("Esc", "Close")];
    }
    if app.terminal_focused {
        return &[
            ("Enter", "Run"),
            ("↑↓", "History"),
            ("PgUp/Dn", "Scroll"),
            ("Ctrl-L", "Clear"),
            ("Ctrl-`", "Editor"),
            ("Ctrl-Shift-`", "Close"),
        ];
    }
    if app.explorer_focused {
        if app.sidebar_view == SidebarView::Outline {
            return &[
                ("↑↓ / j k", "Select"),
                ("Enter", "Jump"),
                ("PgUp/Dn", "Page"),
                ("Home/End", "Ends"),
                ("Ctrl-O", "Files"),
                ("Ctrl-E / Esc", "Editor"),
            ];
        }
        return &[
            ("↑↓", "Move"),
            ("Enter", "Open"),
            ("←→", "Fold"),
            ("*", "ExpandAll"),
            ("-", "CollapseAll"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("Ctrl-E", "Editor"),
        ];
    }

    match (app.mode, app.keymap_profile()) {
        // The non-modal profiles never leave Insert, so this row must not
        // promise an Esc that returns to Normal mode.
        (Mode::Insert, KeymapProfile::Caret | KeymapProfile::Conventional) => &[
            ("Ctrl-S", "Save"),
            ("Ctrl-F", "Find"),
            ("Ctrl-Z/Y", "Undo/Redo"),
            ("Ctrl-Shift-P", "Command"),
            ("Ctrl-E", "Files"),
        ],
        (Mode::Insert, _) => &[
            ("Esc", "Normal"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("Alt-N", "NextTab"),
            ("Alt-P", "PrevTab"),
            ("Ctrl-S", "Save"),
            ("Ctrl-E", "Files"),
        ],
        (Mode::Normal, KeymapProfile::Vim) => &[
            ("i/a/o", "Insert"),
            ("h j k l", "Move"),
            ("/", "Find"),
            ("u/Ctrl-R", "Undo/Redo"),
            ("Ctrl-E", "Files"),
            (":", "Command"),
        ],
        (Mode::Normal, _) => &[
            ("i", "Insert"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("Alt-N", "NextTab"),
            ("Alt-P", "PrevTab"),
            ("Ctrl-E", "Files"),
            (":", "Command"),
        ],
        (Mode::Search, _) => &[
            ("Enter", "Accept"),
            ("Tab", "Replace field"),
            ("F3", "Next"),
            ("Alt-Enter", "Replace"),
            ("Alt-A", "Replace all"),
            ("Alt-C/W/R", "Case/Word/Regex"),
            ("↑↓", "History"),
            ("Esc", "Close"),
        ],
        (Mode::ProjectSearch, _) => &[
            ("Enter", "Search/Open"),
            ("↑↓", "Select"),
            ("Del", "Exclude"),
            ("Tab", "Replace field"),
            ("Alt-A", "Replace all"),
            ("Esc", "Close"),
        ],
        (Mode::FilePicker, _) => &[
            ("Type", "Filter"),
            ("↑↓", "Select"),
            ("Enter", "Open"),
            ("Esc", "Close"),
        ],
        (Mode::KeyBrowser, _) => &[
            ("Type", "Search"),
            ("↑↓", "Scroll"),
            (":bind", "Rebind"),
            ("Esc", "Close"),
        ],
        (Mode::SettingsBrowser, _) => &[
            ("Type", "Search"),
            ("↑↓", "Scroll"),
            ("Enter", "Inspect"),
            ("Esc", "Close"),
        ],
        (Mode::Command, _) => &[
            ("Enter", "Run"),
            ("Esc", "Cancel"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("F1", "Help"),
        ],
        (Mode::Help, _) => &[
            ("←/→", "Page"),
            ("1-5", "Section"),
            ("Esc", "Close"),
            ("F1", "Close"),
        ],
        (Mode::QuitConfirm, _) => &[("S", "Save+Quit"), ("D", "Discard+Quit"), ("Esc", "Cancel")],
        (Mode::TabCloseConfirm, _) => &[("D", "Discard+Close"), ("Esc", "Keep open")],
        (Mode::ReloadConfirm, _) => &[
            ("R", "Reload"),
            ("K", "Keep"),
            ("C", "Compare"),
            ("Esc", "Later"),
        ],
        (Mode::GitDiff, _) => &[("↑↓", "Scroll"), ("Esc", "Close")],
        (Mode::GitHistory, _) => &[("↑↓", "Select"), ("Enter", "Inspect"), ("Esc", "Close")],
        (Mode::ThemeGallery, _) => &[("↑↓", "Preview"), ("Enter", "Apply"), ("Esc", "Cancel")],
        (Mode::KeymapGallery, _) => &[("↑↓", "Select"), ("Enter", "Apply"), ("Esc", "Cancel")],
        (Mode::ContextMenu, _) => &[("↑↓", "Select"), ("Enter", "Apply"), ("Esc", "Close")],
        (Mode::Dashboard, _) => &[
            ("↑↓", "Select"),
            ("Enter", "Open"),
            ("C", "Current"),
            ("E", "Path"),
            ("F1", "Help"),
            ("Q", "Quit"),
        ],
        (Mode::FileManager, _) => &[
            ("↑↓←→", "Navigate"),
            ("Space", "Select"),
            ("C/X/P", "Copy/Cut/Paste"),
            ("Z", "Undo"),
            ("B", "Bulk rename"),
            ("Del", "Trash"),
            ("D", "Delete"),
            ("/", "Filter"),
            ("Esc", "Close"),
        ],
    }
}

fn draw_help<W: Write>(
    out: &mut W,
    app: &App,
    terminal_width: u16,
    terminal_height: u16,
) -> io::Result<()> {
    const PAGES: [&str; 5] = ["EDITING", "NAVIGATION", "FILES", "COMMANDS", "CUSTOMIZE"];
    const EDITING: [(&str, &str); 20] = [
        ("Type normally", "Enter text while in Insert mode"),
        ("Esc", "Switch to Normal mode"),
        ("F7", "Duplicate current line"),
        ("Ctrl + Left / Right", "Move by word"),
        ("Ctrl + Shift + Left / Right", "Select by word"),
        ("Double-click", "Select the clicked word"),
        ("Shift + Arrow/Home/End", "Select text with the keyboard"),
        ("Mouse drag", "Select text with the mouse"),
        ("Right-click editor", "Open selection and editing actions"),
        ("Ctrl-C / Ctrl-X / Ctrl-V", "Copy / Cut / Paste selection"),
        (
            "Ctrl-D",
            "Select next occurrence; type to edit all selections",
        ),
        ("Backspace / Delete", "Delete text or selection"),
        ("Ctrl-S", "Save the current file"),
        ("i / a  (Normal)", "Insert before / after cursor"),
        ("x / dd  (Normal)", "Delete character / line"),
        ("yy / p  (Normal)", "Copy line / paste below"),
        (
            "q{register} / @{register}",
            "Record / replay a macro in Normal mode",
        ),
        (
            "Tab / Shift-Tab  (Normal)",
            "Indent / outdent the line or selection",
        ),
        ("Ctrl-/", "Toggle language-aware comments"),
        ("u / Ctrl-R  (Normal)", "Undo / Redo"),
    ];
    const NAVIGATION: [(&str, &str); 12] = [
        ("Arrows or h j k l", "Move the cursor"),
        ("w / b", "Next / previous word"),
        ("0 / $", "Start / end of line"),
        ("gg / G", "Top / bottom of file"),
        ("PageUp / PageDown", "Move one screen"),
        (
            "zc / zo / za / zM / zR",
            "Close, open, toggle, fold all, unfold all",
        ),
        ("Alt-Left / Alt-Right", "Go back / forward in history"),
        ("/", "Search for text"),
        ("n / N", "Next / previous search result"),
        ("Ctrl-T / Ctrl-W", "Open / close a tab"),
        ("Alt-N / Alt-P", "Next / previous tab"),
        ("Alt-1 ... Alt-9", "Select a tab directly"),
    ];
    const FILES: [(&str, &str); 12] = [
        ("Ctrl-B", "Show or hide the explorer"),
        ("Ctrl-E", "Switch between editor and files"),
        ("Up / Down", "Select a file or folder"),
        ("Enter", "Open a file or expand a folder"),
        ("Right-click file/tab", "Open actions for that item"),
        ("Left / Right", "Collapse / expand a folder"),
        ("Backspace", "Move to the parent folder"),
        ("* / -", "Expand all / collapse all"),
        (".", "Show or hide hidden files"),
        ("/ / Ctrl-P", "Filter tree / fuzzy-open a file"),
        (":manager / :fm", "Open full filesystem workspace"),
        ("Space / c x p / Del", "Select and operate in manager"),
    ];
    const COMMANDS: [(&str, &str); 13] = [
        (":  (from Normal mode)", "Open the command prompt"),
        (":w  /  :w file", "Save / Save as"),
        (":q  /  :q!", "Quit / Force quit"),
        (":e path", "Open a file or folder"),
        (":terminal / Ctrl-`", "Open or focus the integrated shell"),
        (
            "Ctrl-F / Ctrl-H",
            "Find or replace with options and history",
        ),
        (
            "Ctrl-Shift-F / :grep",
            "Search and replace across the project",
        ),
        ("Ctrl-P / :files", "Fuzzy-open project and recent files"),
        ("Ctrl-Space", "Show LSP completions"),
        ("F12 / Shift-F12", "Definition / references"),
        (
            ":actions / :diagnostics",
            "Apply code actions / inspect issues",
        ),
        (":settings", "Search settings and inspect their metadata"),
        (":manager / :fm", "Open the full file manager"),
    ];
    const CUSTOMIZE: [(&str, &str); 10] = [
        (
            ":settings",
            "Search all settings and inspect their metadata",
        ),
        (
            "Type / ↑↓ / Enter",
            "Filter, navigate, and inspect a setting",
        ),
        (
            ":set <value>",
            "Apply a validated setting; some apply next launch",
        ),
        (":themes / :theme <name>", "Open or apply the theme gallery"),
        ("Hover / wheel / Enter", "Preview, scroll, and apply themes"),
        (":keymaps / :keymap", "Choose an editing workflow"),
        (
            ":keybindings / :bind",
            "Search or customize keyboard bindings",
        ),
        ("Ctrl-Shift-P", "Open the command palette from any profile"),
        (":set icons=ascii", "Use portable ASCII filesystem icons"),
        (
            ":set reducedmotion",
            "Disable animated background indicators",
        ),
    ];

    let page = app.help_page.min(PAGES.len() - 1);
    let rows: &[(&str, &str)] = match page {
        0 => &EDITING,
        1 => &NAVIGATION,
        2 => &FILES,
        3 => &COMMANDS,
        _ => &CUSTOMIZE,
    };

    let box_width = 76usize.min(terminal_width.saturating_sub(4) as usize);
    let box_height = 20usize.min(terminal_height.saturating_sub(2) as usize);
    let start_x = terminal_width.saturating_sub(box_width as u16) / 2;
    let start_y = terminal_height.saturating_sub(box_height as u16) / 2;

    // Paint one stable panel first, then layer the structured content over it.
    for row in 0..box_height {
        let y = start_y + row as u16;
        let content = if row == 0 {
            format!("┌{}┐", "─".repeat(box_width.saturating_sub(2)))
        } else if row + 1 == box_height {
            format!("└{}┘", "─".repeat(box_width.saturating_sub(2)))
        } else {
            format!("│{}│", " ".repeat(box_width.saturating_sub(2)))
        };

        queue!(
            out,
            MoveTo(start_x, y),
            SetBackgroundColor(app.theme.overlay),
            SetForegroundColor(if row == 0 || row + 1 == box_height {
                app.theme.border
            } else {
                app.theme.overlay_text
            }),
            Print(content)
        )?;
    }

    queue!(
        out,
        MoveTo(start_x + 3, start_y + 1),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.heading),
        SetAttribute(Attribute::Bold),
        Print("CARET HELP"),
        SetAttribute(Attribute::Reset),
        MoveTo(start_x + box_width.saturating_sub(13) as u16, start_y + 1),
        SetForegroundColor(app.theme.muted),
        Print(format!("Page {}/{}", page + 1, PAGES.len()))
    )?;

    let mut tab_x = start_x + 3;
    for (index, label) in PAGES.iter().enumerate() {
        let active = index == page;
        let tab = format!(" {} {} ", index + 1, label);
        queue!(
            out,
            MoveTo(tab_x, start_y + 3),
            SetBackgroundColor(if active {
                app.theme.command_mode
            } else {
                app.theme.current_line
            }),
            SetForegroundColor(if active {
                app.theme.background
            } else {
                app.theme.muted
            }),
            SetAttribute(if active {
                Attribute::Bold
            } else {
                Attribute::Reset
            }),
            Print(&tab),
            SetAttribute(Attribute::Reset)
        )?;
        tab_x += UnicodeWidthStr::width(tab.as_str()) as u16 + 1;
    }

    queue!(
        out,
        MoveTo(start_x + 3, start_y + 5),
        SetBackgroundColor(app.theme.overlay),
        SetForegroundColor(app.theme.muted),
        Print("KEY / ACTION"),
        MoveTo(start_x + 30, start_y + 5),
        Print("WHAT IT DOES")
    )?;

    for (index, (key, action)) in rows.iter().enumerate() {
        let y = start_y + 6 + index as u16;
        if y >= start_y + box_height.saturating_sub(2) as u16 {
            break;
        }
        queue!(
            out,
            MoveTo(start_x + 3, y),
            SetForegroundColor(app.theme.top_bar_text),
            SetAttribute(Attribute::Bold),
            Print(pad_or_truncate(key, 25)),
            SetAttribute(Attribute::Reset),
            MoveTo(start_x + 30, y),
            SetForegroundColor(app.theme.overlay_text),
            Print(pad_or_truncate(action, box_width.saturating_sub(33)))
        )?;
    }

    queue!(
        out,
        MoveTo(start_x + 3, start_y + box_height.saturating_sub(2) as u16),
        SetForegroundColor(app.theme.muted),
        Print("←/→ or 1-5 change page"),
        MoveTo(
            start_x + box_width.saturating_sub(25) as u16,
            start_y + box_height.saturating_sub(2) as u16
        ),
        Print("Esc / F1 / ? closes")
    )?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn place_cursor<W: Write>(
    out: &mut W,
    app: &App,
    content_top: u16,
    content_height: usize,
    editor_x: usize,
    editor_width: usize,
    gutter_width: usize,
    terminal_width: u16,
    terminal_height: u16,
) -> io::Result<()> {
    if matches!(
        app.mode,
        Mode::Help
            | Mode::ProjectSearch
            | Mode::FilePicker
            | Mode::SettingsBrowser
            | Mode::KeyBrowser
            | Mode::QuitConfirm
            | Mode::TabCloseConfirm
            | Mode::ReloadConfirm
            | Mode::GitDiff
            | Mode::GitHistory
            | Mode::ThemeGallery
            | Mode::KeymapGallery
            | Mode::ContextMenu
            | Mode::Dashboard
    ) || (app.explorer_focused && !matches!(app.mode, Mode::Command | Mode::Search))
    {
        return queue!(out, Hide);
    }

    if app.mode == Mode::Command {
        // Inside the palette's search field: panel edge, padding, then "> ".
        let (panel_x, panel_y, ..) = command_palette_geometry(app, terminal_width, terminal_height);
        let typed = &app.command_input[..app.command_cursor()];
        let x = (panel_x as usize + 5 + UnicodeWidthStr::width(typed))
            .min(terminal_width.saturating_sub(1) as usize) as u16;

        return queue!(out, MoveTo(x, panel_y + 2), Show);
    }

    if app.mode == Mode::Search {
        let (prefix, typed) = app.search_cursor_offset();
        let x = (UnicodeWidthStr::width(prefix.as_str()) + UnicodeWidthStr::width(typed.as_str()))
            .min(terminal_width.saturating_sub(1) as usize) as u16;

        return queue!(out, MoveTo(x, terminal_height - 2), Show);
    }

    let Some(screen_row) = (0..content_height).find(|row| {
        app.editor.visible_line_at(app.editor.scroll_line, *row) == Some(app.editor.cursor.line)
    }) else {
        return queue!(out, Hide);
    };

    let line = app.editor.line_text(app.editor.cursor.line);
    let prefix: String = line.chars().take(app.editor.cursor.column).collect();
    let display_column = display_width(&prefix, app.editor.tab_width);

    if display_column < app.editor.scroll_column {
        return queue!(out, Hide);
    }

    let x = editor_x + gutter_width + display_column - app.editor.scroll_column;
    if x >= editor_x + editor_width || x >= terminal_width as usize {
        return queue!(out, Hide);
    }

    let y = content_top + screen_row as u16;
    queue!(out, MoveTo(x as u16, y), Show)
}

fn fit_bar(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let right_width = UnicodeWidthStr::width(right);
    if right_width >= width {
        return pad_or_truncate(right, width);
    }

    let available_left = width - right_width;
    let left = pad_or_truncate(left, available_left);
    format!("{left}{right}")
}

fn pad_or_truncate(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut output = String::new();
    let mut used = 0usize;

    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);

        if used + character_width > width {
            break;
        }

        output.push(character);
        used += character_width;
    }

    if used < width {
        output.push_str(&" ".repeat(width - used));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_layout_remains_usable_across_terminal_matrix() {
        let app = App::new(None).expect("create app");
        for (width, height) in [(44, 8), (80, 24), (120, 30), (200, 60)] {
            let layout = screen_layout(&app, width, height);
            assert!(layout.content_height >= 3, "{width}x{height}");
            assert!(layout.editor_width >= 1, "{width}x{height}");
            assert!(
                layout.gutter_width < layout.editor_width,
                "{width}x{height}"
            );
            assert!(layout.hotkey_row < height, "{width}x{height}");
        }
    }

    /// The offsets in `title_bar_targets` are hand-counted from
    /// `title_bar_right`, so pin them to what is actually drawn -- otherwise
    /// editing a label silently moves every click zone to its right.
    #[test]
    fn title_bar_controls_are_clickable_where_they_are_drawn() {
        for root in ["caret", "a-rather-long-project-name"] {
            for width in [80u16, 100, 200] {
                let right = title_bar_right(root);
                let bar = fit_bar("  CARET  │ [FILES] │  main.rs", &right, width as usize);
                let columns: Vec<char> = bar.chars().collect();
                assert_eq!(columns.len(), width as usize, "bar should fill the width");

                let root_width = UnicodeWidthStr::width(root) as u16;
                let targets = title_bar_targets(width, root_width);
                assert!(!targets.is_empty(), "{width} wide should have controls");

                for (target, x, label) in targets {
                    let start = x as usize;
                    let end = start + label.chars().count();
                    let drawn: String = columns[start..end].iter().collect();
                    assert_eq!(
                        drawn, label,
                        "{target:?} is not drawn at {x} ({width} wide, root {root:?})"
                    );
                }
            }
        }
    }

    /// Controls disappear only when the compact trailing segment cannot fit;
    /// FILES drops independently when there is not enough room for both sides.
    #[test]
    fn title_bar_controls_are_dropped_when_they_do_not_fit() {
        assert!(title_bar_targets(44, 40).is_empty());

        // FILES goes first: the title is truncated before the right segment is.
        let cramped = title_bar_targets(60, 40);
        assert!(!cramped
            .iter()
            .any(|(target, _, _)| *target == HoverTarget::Files));
        assert!(cramped
            .iter()
            .any(|(target, _, _)| *target == HoverTarget::Menu));
    }

    #[test]
    fn truncation_respects_wide_unicode_cells() {
        let rendered = pad_or_truncate("a界b", 3);
        assert_eq!(UnicodeWidthStr::width(rendered.as_str()), 3);
        assert!(rendered.starts_with("a界"));
    }

    #[test]
    fn fitted_tree_rows_keep_status_badges_right_aligned() {
        let rendered = fit_bar(" ├─▸ a very long directory name/", " M ", 20);
        assert_eq!(UnicodeWidthStr::width(rendered.as_str()), 20);
        assert!(rendered.ends_with(" M "));
    }

    #[test]
    fn explorer_filter_match_ranges_preserve_original_case() {
        let (start, end) = case_insensitive_match_range("src/ProjectTree.rs", "project").unwrap();
        assert_eq!(&"src/ProjectTree.rs"[start..end], "Project");
        assert!(case_insensitive_match_range("main.rs", "missing").is_none());
    }

    #[test]
    fn sidebar_rows_do_not_reset_their_theme_colors() {
        let app = App::new(None).expect("create app");
        let mut tree = Vec::new();
        draw_project_tree(&mut tree, &app, 2, 4, 40).expect("draw project tree");
        assert!(
            !tree
                .windows(b"\x1b[0m".len())
                .any(|window| window == b"\x1b[0m"),
            "an SGR reset after row colors would restore the terminal's black background"
        );

        let mut outline = Vec::new();
        draw_outline(&mut outline, &app, 2, 4, 40).expect("draw outline");
        assert!(!outline
            .windows(b"\x1b[0m".len())
            .any(|window| window == b"\x1b[0m"));
    }

    #[test]
    fn bottom_chrome_does_not_reset_its_theme_colors() {
        let app = App::new(None).expect("create app");
        let mut output = Vec::new();
        draw_status_bar(&mut output, &app, 20, 100).expect("draw status");
        draw_prompt_bar(&mut output, &app, 21, 100).expect("draw prompt");
        draw_hotkey_bar(&mut output, &app, 22, 100).expect("draw hotkeys");

        assert!(
            !output
                .windows(b"\x1b[0m".len())
                .any(|window| window == b"\x1b[0m"),
            "an SGR reset would leak the terminal background into the bottom rows"
        );
    }

    #[test]
    fn manager_lines_expand_tabs_and_never_emit_terminal_controls() {
        let safe = manager_display_line("Order\tCustomer\r\nNext");
        assert_eq!(safe, "Order   Customer��Next");
        assert!(!safe.chars().any(char::is_control));

        let rendered = pad_or_truncate(&format!(" {safe}"), 18);
        assert_eq!(UnicodeWidthStr::width(rendered.as_str()), 18);
        assert!(!rendered.chars().any(char::is_control));
    }

    #[test]
    fn manager_breadcrumb_preserves_context_without_exceeding_its_width() {
        let breadcrumb = manager_breadcrumb(
            Path::new("/home/nightcat/Documents/code/superfile/src/internal"),
            34,
        );
        assert!(UnicodeWidthStr::width(breadcrumb.as_str()) <= 34);
        assert!(breadcrumb.contains('…'));
        assert!(breadcrumb.ends_with("internal"));
        assert!(breadcrumb.contains('›'));
    }

    #[cfg(windows)]
    #[test]
    fn manager_breadcrumb_hides_windows_verbatim_prefixes() {
        let breadcrumb =
            manager_breadcrumb(Path::new(r"\\?\C:\Users\Admin\Documents\oxide-editor"), 80);
        assert!(breadcrumb.starts_with("C:"));
        assert!(breadcrumb.ends_with("oxide-editor"));
        assert!(!breadcrumb.contains(r"\\?\"));
    }

    #[test]
    fn command_suggestion_window_clamps_to_the_available_results() {
        assert_eq!(command_suggestion_window_start(10, 0, 8), 0);
        assert_eq!(command_suggestion_window_start(10, 1, 8), 1);
        assert_eq!(command_suggestion_window_start(10, 2, 8), 2);
        assert_eq!(command_suggestion_window_start(10, 9, 8), 2);
    }

    #[test]
    fn command_suggestion_hit_testing_tracks_the_scrolled_window() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Command;
        app.command_suggestion = 8;
        app.command_suggestion_scroll = 1;

        let (x, y, panel_width, visible_rows, first) = command_palette_geometry(&app, 80, 24);
        assert_eq!(first, 1, "scroll should offset the window");
        let inside = x + panel_width as u16 / 2;
        let list_top = y + PALETTE_LIST_TOP;

        assert_eq!(
            command_suggestion_at(&app, 80, 24, inside, list_top),
            Some(1)
        );
        assert_eq!(
            command_suggestion_at(
                &app,
                80,
                24,
                inside,
                list_top + (visible_rows * PALETTE_ITEM_HEIGHT) as u16 - 1
            ),
            Some(first + visible_rows - 1)
        );

        // Outside the panel and above the list are both misses.
        assert_eq!(
            command_suggestion_at(&app, 80, 24, x.saturating_sub(1), list_top),
            None
        );
        assert_eq!(
            command_suggestion_at(&app, 80, 24, inside, list_top - 1),
            None
        );
    }

    #[test]
    fn command_palette_is_centred_and_fits_the_terminal() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Command;

        for (width, height) in [(80u16, 24u16), (120, 40), (60, 14)] {
            let (x, y, panel_width, visible_rows, _) =
                command_palette_geometry(&app, width, height);
            let panel_height = visible_rows * PALETTE_ITEM_HEIGHT + PALETTE_CHROME_ROWS;

            assert!(visible_rows >= 1, "{width}x{height} should show a row");
            assert!(
                x as usize + panel_width <= width as usize,
                "panel overflows {width}x{height}"
            );
            assert!(
                y as usize + panel_height <= height as usize,
                "panel is taller than {width}x{height}"
            );
        }
    }

    #[test]
    fn file_manager_hit_testing_tracks_responsive_current_pane() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::FileManager;
        app.file_manager.entries = vec![FileEntry {
            path: std::path::PathBuf::from("alpha.txt"),
            name: "alpha.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            hidden: false,
            size: 5,
            modified_unix_secs: None,
        }];
        app.file_manager.loading = false;

        // At wide widths the parent pane occupies the left quarter.
        assert_eq!(file_manager_entry_at(&app, 120, 30, 31, 5), Some(0));
        assert_eq!(file_manager_entry_at(&app, 120, 30, 2, 4), None);
        // Narrow layouts collapse to one current-directory pane.
        assert_eq!(file_manager_entry_at(&app, 60, 20, 2, 5), Some(0));
    }

    #[test]
    fn explorer_header_controls_share_drawn_offsets_with_hit_testing() {
        assert_eq!(explorer_header_action_at(40, 26), Some('+'));
        assert_eq!(explorer_header_action_at(40, 30), Some('D'));
        assert_eq!(explorer_header_action_at(40, 34), Some('R'));
        assert_eq!(explorer_header_action_at(40, 38), Some('-'));
        assert_eq!(explorer_header_action_at(30, 20), None);
    }

    #[test]
    fn command_palette_lists_chords_beside_the_commands_that_share_them() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Command;
        app.command_input = "w".to_string();

        let save = app
            .command_matches()
            .into_iter()
            .find(|entry| entry.name == "w")
            .expect(":w should match the query \"w\"");

        assert_eq!(save.description, "Save the current file");
        let expected = if cfg!(target_os = "macos") {
            "⌃S"
        } else {
            "Ctrl+S"
        };
        assert_eq!(save.chord.as_deref(), Some(expected));
    }

    #[test]
    fn command_palette_matches_descriptions_after_names() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::Command;
        // "quit" is the description of :q and the name of nothing.
        app.command_input = "quit".to_string();

        let matches = app.command_matches();
        assert!(
            matches.iter().any(|entry| entry.name == "q"),
            "description matches should be reachable"
        );

        // A name match must outrank a description-only match.
        app.command_input = "theme".to_string();
        let matches = app.command_matches();
        assert!(
            matches[0].name.contains("theme"),
            "name match should sort first, got {:?}",
            matches[0].name
        );
    }

    #[test]
    fn theme_gallery_hit_testing_tracks_the_scrolled_window() {
        let mut app = App::new(None).expect("create app");
        app.mode = Mode::ThemeGallery;
        app.theme_gallery_selected = 20;

        assert!(theme_gallery_contains(&app, 80, 24, 18, 1));
        assert_eq!(theme_gallery_item_at(&app, 80, 24, 18, 3), Some(3));
    }
}
