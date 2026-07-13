use std::io::{self, Write};

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
    app::{App, BackgroundState, HoverTarget, Mode, SidebarView},
    config::KeymapProfile,
    editor::display_width,
    project::GitStatus,
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
    draw_status_bar(out, app, layout.status_row, width)?;
    draw_command_palette(out, app, width, height)?;
    draw_prompt_bar(out, app, layout.prompt_row, width)?;
    draw_hotkey_bar(out, app, layout.hotkey_row, width)?;

    if app.mode == Mode::Help {
        draw_help(out, app, width, height)?;
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
    if app.mode == Mode::Dashboard {
        draw_dashboard(out, app, width, height)?;
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
    if app.lsp_panel.is_some() {
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
    let right = format!(
        " {}  │ [Themes] │ [F1 Help] │ [Quit] ",
        app.project.root_name()
    );

    queue!(
        out,
        MoveTo(0, 0),
        SetBackgroundColor(app.theme.top_bar),
        SetForegroundColor(app.theme.top_bar_text),
        SetAttribute(Attribute::Bold),
        Print(fit_bar(&title, &right, width as usize)),
        SetAttribute(Attribute::Reset)
    )?;

    for (target, x, label) in [
        (HoverTarget::Files, 11u16, "[FILES]"),
        (HoverTarget::Themes, width.saturating_sub(30), "[Themes]"),
        (HoverTarget::Help, width.saturating_sub(19), "[F1 Help]"),
        (HoverTarget::Quit, width.saturating_sub(7), "[Quit]"),
    ] {
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
            let root = format!(
                " PROJECT ▾ {} · {} items{}",
                app.project.root_name(),
                app.project.entries.len(),
                hidden_marker
            );
            queue!(
                out,
                MoveTo(0, y),
                SetForegroundColor(app.theme.top_bar_text),
                SetAttribute(Attribute::Bold),
                Print(pad_or_truncate(&root, width)),
                SetAttribute(Attribute::Reset)
            )?;
            continue;
        }

        let entry_index = app.project.scroll + screen_row - 1;
        let Some(entry) = app.project.entries.get(entry_index) else {
            continue;
        };

        let selected = entry_index == app.project.selected;
        let active_file = app.editor.path.as_ref() == Some(&entry.path);
        let background = if selected && app.explorer_focused {
            app.theme.normal_mode
        } else if selected {
            app.theme.current_line
        } else {
            app.theme.prompt_bar
        };
        let foreground = if selected && app.explorer_focused {
            app.theme.background
        } else if entry.is_dir {
            app.theme.heading
        } else if active_file {
            app.theme.success
        } else {
            app.theme.foreground
        };

        let icon = if entry.is_dir {
            if entry.expanded {
                "▾"
            } else {
                "▸"
            }
        } else if active_file {
            "●"
        } else {
            "·"
        };
        let git = match entry.git_status {
            Some(GitStatus::Modified) => "M",
            Some(GitStatus::Added) => "A",
            Some(GitStatus::Deleted) => "D",
            Some(GitStatus::Untracked) => "?",
            None => " ",
        };
        let indent = "│  ".repeat(entry.depth);
        let branch = "├─";
        let suffix = if entry.is_dir { "/" } else { "" };
        let kind = if entry.is_dir { "DIR" } else { "   " };
        let label = format!(
            " {indent}{branch}{icon} {git} {kind} {}{suffix}",
            entry.name
        );

        queue!(
            out,
            MoveTo(0, y),
            SetBackgroundColor(background),
            SetForegroundColor(foreground),
            SetAttribute(if selected {
                Attribute::Bold
            } else {
                Attribute::Reset
            }),
            Print(pad_or_truncate(&label, width)),
            SetAttribute(Attribute::Reset)
        )?;
    }

    Ok(())
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
                SetAttribute(Attribute::Reset)
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
                Attribute::Reset
            }),
            Print(pad_or_truncate(&label, width)),
            SetAttribute(Attribute::Reset)
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
    let language = Language::from_path(app.editor.path.as_deref());
    let search_query = app.active_search_query();
    let fold_ranges = syntax::fold_ranges(&app.editor.text(), language);

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
        let colors = syntax::highlight_line(&line, language, &app.theme);
        let search_hits = search_hit_map(&line, search_query);
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

fn draw_status_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    let mode_color = if app.terminal_focused {
        app.theme.insert_mode
    } else if app.explorer_focused {
        app.theme.normal_mode
    } else {
        match app.mode {
            Mode::Normal => app.theme.normal_mode,
            Mode::Insert => app.theme.insert_mode,
            Mode::Search => app.theme.search_mode,
            Mode::Command | Mode::Help => app.theme.command_mode,
            Mode::QuitConfirm | Mode::ReloadConfirm => app.theme.error,
            Mode::GitDiff
            | Mode::GitHistory
            | Mode::ThemeGallery
            | Mode::KeymapGallery
            | Mode::ContextMenu
            | Mode::Dashboard => app.theme.command_mode,
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
        SetAttribute(Attribute::Reset)
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
            SetForegroundColor(color),
            SetAttribute(Attribute::Bold),
            Print(label),
            SetAttribute(Attribute::Reset)
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
            " GIT DIFF  ·  Esc closes  ·  ↑↓ scroll",
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

fn draw_theme_gallery<W: Write>(out: &mut W, app: &App, width: u16, height: u16) -> io::Result<()> {
    let panel_width = 46usize.min(width.saturating_sub(4) as usize);
    let panel_height = ThemeKind::ALL.len() + 4;
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height as u16) / 2;
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
    for (index, kind) in ThemeKind::ALL.iter().enumerate() {
        let selected = index == app.theme_gallery_selected;
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
    let panel_width = 46usize.min(width.saturating_sub(4) as usize);
    let panel_height = ThemeKind::ALL.len() + 4;
    let x = width.saturating_sub(panel_width as u16) / 2;
    let y = height.saturating_sub(panel_height as u16) / 2;
    if column < x || column >= x.saturating_add(panel_width as u16) {
        return None;
    }
    let first = y.saturating_add(2);
    if row < first || row >= first.saturating_add(ThemeKind::ALL.len() as u16) {
        return None;
    }
    Some((row - first) as usize)
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
    let suggestions = app.command_suggestions();
    let rows = suggestions.len().min(8);
    let panel_width = 30usize.min(width.saturating_sub(2) as usize);
    let start_row = height.saturating_sub(2 + rows as u16);
    for (index, command) in suggestions.into_iter().take(rows).enumerate() {
        let selected = index == app.command_suggestion;
        queue!(
            out,
            MoveTo(1, start_row + index as u16),
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
            Print(pad_or_truncate(&format!("  :{command}"), panel_width))
        )?;
    }
    Ok(())
}

pub fn command_suggestion_at(
    app: &App,
    width: u16,
    height: u16,
    column: u16,
    row: u16,
) -> Option<usize> {
    if app.mode != Mode::Command
        || column == 0
        || column as usize > 30usize.min(width.saturating_sub(2) as usize)
    {
        return None;
    }
    let rows = app.command_suggestions().len().min(8);
    let start = height.saturating_sub(2 + rows as u16);
    if row >= start && row < start.saturating_add(rows as u16) {
        Some((row - start) as usize)
    } else {
        None
    }
}

fn draw_prompt_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    let (prompt, background, foreground) = match app.mode {
        Mode::Search => (
            format!("/{}", app.search_input),
            app.theme.prompt_bar,
            app.theme.prompt_text,
        ),
        Mode::Command => (
            format!(":{}", app.command_input),
            app.theme.prompt_bar,
            app.theme.prompt_text,
        ),
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
        SetAttribute(if app.mode == Mode::QuitConfirm {
            Attribute::Bold
        } else {
            Attribute::Reset
        }),
        Print(pad_or_truncate(&prompt, width as usize)),
        SetAttribute(Attribute::Reset)
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

fn draw_hotkey_bar<W: Write>(out: &mut W, app: &App, row: u16, width: u16) -> io::Result<()> {
    let mode_color = if app.terminal_focused {
        app.theme.insert_mode
    } else if app.explorer_focused {
        app.theme.normal_mode
    } else {
        match app.mode {
            Mode::Normal => app.theme.normal_mode,
            Mode::Insert => app.theme.insert_mode,
            Mode::Search => app.theme.search_mode,
            Mode::Command | Mode::Help => app.theme.command_mode,
            Mode::QuitConfirm | Mode::ReloadConfirm => app.theme.error,
            Mode::GitDiff
            | Mode::GitHistory
            | Mode::ThemeGallery
            | Mode::KeymapGallery
            | Mode::ContextMenu
            | Mode::Dashboard => app.theme.command_mode,
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
            SetAttribute(Attribute::Reset)
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
                Attribute::Reset
            }),
            Print(&description_text)
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
        (Mode::Insert, KeymapProfile::Conventional) => &[
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
            ("Esc", "Cancel"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
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
        (Mode::ReloadConfirm, _) => &[("R", "Reload"), ("K", "Keep"), ("Esc", "Later")],
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
    const FILES: [(&str, &str); 13] = [
        ("Ctrl-B", "Show or hide the explorer"),
        ("Click FILES", "Show or hide the explorer"),
        ("Ctrl-E", "Switch between editor and files"),
        ("Up / Down", "Select a file or folder"),
        ("Enter", "Open a file or expand a folder"),
        ("Right-click file/tab", "Open actions for that item"),
        (
            "+ / ~ / - in gutter",
            "Added / modified / deleted Git lines",
        ),
        ("Left / Right", "Collapse / expand a folder"),
        ("Backspace", "Move to the parent folder"),
        (
            "Shift-Left / Shift-Right",
            "Collapse recursively / expand one level",
        ),
        ("* / -", "Expand all / collapse all"),
        (".", "Show or hide hidden files"),
        ("r", "Refresh the explorer"),
    ];
    const COMMANDS: [(&str, &str); 12] = [
        (":  (from Normal mode)", "Open the command prompt"),
        (":w  /  :w file", "Save / Save as"),
        (":q  /  :q!", "Quit / Force quit"),
        (":e path", "Open a file or folder"),
        (":terminal / Ctrl-`", "Open or focus the integrated shell"),
        ("Ctrl-Space", "Show LSP completions"),
        ("F2", "Rename the symbol at the cursor"),
        ("F12 / Shift-F12", "Definition / references"),
        (":hover", "Show type and documentation"),
        (":actions", "Show and apply code actions"),
        (":diagnostics", "Inspect errors and warnings"),
        (":plugins", "List loaded plugins and commands"),
    ];
    const CUSTOMIZE: [(&str, &str); 11] = [
        ("Click Themes / :themes", "Open the live theme gallery"),
        ("Hover a theme", "Temporarily preview its colors"),
        ("Click / Enter", "Apply and remember the selected theme"),
        (":keymaps", "Open the keymap profile chooser"),
        (":keymap caret", "Insert-first Caret workflow"),
        (":keymap vim", "Modal Vim-style workflow"),
        (
            ":keymap conventional",
            "Always-type workflow with Ctrl shortcuts",
        ),
        ("Ctrl-Shift-P", "Open the command palette from any profile"),
        (":config", "Show the saved configuration path"),
        (
            ":theme mono / NO_COLOR=1",
            "Use the high-contrast monochrome theme",
        ),
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
            | Mode::QuitConfirm
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

    if app.mode == Mode::Command || app.mode == Mode::Search {
        let prefix_width = 1usize;
        let input = if app.mode == Mode::Command {
            &app.command_input
        } else {
            &app.search_input
        };
        let typed = if app.mode == Mode::Command {
            &input[..app.command_cursor()]
        } else {
            input.as_str()
        };
        let x = (prefix_width + UnicodeWidthStr::width(typed))
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

fn search_hit_map(line: &str, query: &str) -> Vec<bool> {
    let char_count = line.chars().count();
    let mut hits = vec![false; char_count];

    if query.is_empty() {
        return hits;
    }

    for (byte_start, matched) in line.match_indices(query) {
        let start = line[..byte_start].chars().count();
        let length = matched.chars().count();
        let end = (start + length).min(hits.len());

        for hit in &mut hits[start..end] {
            *hit = true;
        }
    }

    hits
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

    #[test]
    fn truncation_respects_wide_unicode_cells() {
        let rendered = pad_or_truncate("a界b", 3);
        assert_eq!(UnicodeWidthStr::width(rendered.as_str()), 3);
        assert!(rendered.starts_with("a界"));
    }
}
