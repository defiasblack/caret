use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor,
        SetForegroundColor,
    },
    terminal::{self, BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    app::{App, HoverTarget, Mode},
    editor::display_width,
    syntax::{self, Language},
};

#[derive(Debug, Clone, Copy)]
pub struct ScreenLayout {
    pub content_top: u16,
    pub content_height: usize,
    pub sidebar_width: usize,
    pub editor_x: usize,
    pub editor_width: usize,
    pub gutter_width: usize,
    pub status_row: u16,
    pub prompt_row: u16,
    pub hotkey_row: u16,
}

pub fn screen_layout(app: &App, width: u16, height: u16) -> ScreenLayout {
    let content_top = 2u16;
    let content_height = height.saturating_sub(5) as usize;
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
    let content_top = layout.content_top;
    let content_height = layout.content_height;
    let sidebar_width = layout.sidebar_width;
    let editor_x = layout.editor_x;
    let editor_width = layout.editor_width;
    let gutter_width = layout.gutter_width;
    let content_width = editor_width.saturating_sub(gutter_width);

    app.viewport_rows = content_height.max(1);
    app.viewport_columns = content_width.max(1);
    app.editor
        .ensure_cursor_visible(app.viewport_rows, app.viewport_columns);
    app.project
        .ensure_selected_visible(content_height.saturating_sub(1));

    draw_top_bar(out, app, width)?;
    draw_tab_bar(out, app, 1, width)?;

    if sidebar_width > 0 {
        draw_project_tree(
            out,
            app,
            content_top,
            content_height,
            sidebar_width,
        )?;
        draw_vertical_separator(
            out,
            app,
            sidebar_width as u16,
            content_top,
            content_height,
        )?;
    }

    draw_editor(
        out,
        app,
        content_top,
        content_height,
        editor_x as u16,
        editor_width as u16,
        gutter_width,
    )?;
    draw_status_bar(out, app, layout.status_row, width)?;
    draw_prompt_bar(out, app, layout.prompt_row, width)?;
    draw_hotkey_bar(out, app, layout.hotkey_row, width)?;

    if app.mode == Mode::Help {
        draw_help(out, app, width, height)?;
    }

    place_cursor(
        out,
        app,
        content_top,
        content_height,
        editor_x,
        editor_width,
        gutter_width,
        width,
        height,
    )?;

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
    let title = format!("  CARET  │ [FILES] │  {filename}{dirty}");
    let right = format!(" {}  │ [F1 Help] │ [Quit] ", app.project.root_name());

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

fn draw_tab_bar<W: Write>(
    out: &mut W,
    app: &App,
    row: u16,
    width: u16,
) -> io::Result<()> {
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
    let dirty = if app.editor.tab_dirty(index) { " ●" } else { "" };
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
            if entry.expanded { "▾" } else { "▸" }
        } else if active_file {
            "●"
        } else {
            "·"
        };
        let indent = "│  ".repeat(entry.depth);
        let branch = "├─";
        let suffix = if entry.is_dir { "/" } else { "" };
        let kind = if entry.is_dir { "DIR" } else { "   " };
        let label = format!(
            " {indent}{branch}{icon} {kind} {}{suffix}",
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

    for screen_row in 0..rows {
        let terminal_row = top + screen_row as u16;
        let line_index = app.editor.scroll_line + screen_row;

        queue!(
            out,
            MoveTo(editor_x, terminal_row),
            SetBackgroundColor(app.theme.background),
            SetForegroundColor(app.theme.foreground),
            Print(" ".repeat(editor_width as usize))
        )?;

        if line_index >= app.editor.line_count() {
            if gutter_width > 0 {
                queue!(
                    out,
                    MoveTo(editor_x + (gutter_width - 2) as u16, terminal_row),
                    SetForegroundColor(app.theme.gutter),
                    Print("~")
                )?;
            }
            continue;
        }

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
            let number_width = gutter_width.saturating_sub(2);
            let number = format!("{:>width$}  ", line_index + 1, width = number_width);
            let number_color = if is_current {
                app.theme.gutter_current
            } else {
                app.theme.gutter
            };

            queue!(out, SetForegroundColor(number_color), Print(number))?;
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
        let selected = selections.iter().any(|(start, end)| {
            (*start..*end).contains(&(line_start + character_index))
        });
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

fn draw_status_bar<W: Write>(
    out: &mut W,
    app: &App,
    row: u16,
    width: u16,
) -> io::Result<()> {
    let mode_color = if app.explorer_focused {
        app.theme.normal_mode
    } else {
        match app.mode {
            Mode::Normal => app.theme.normal_mode,
            Mode::Insert => app.theme.insert_mode,
            Mode::Search => app.theme.search_mode,
            Mode::Command | Mode::Help => app.theme.command_mode,
            Mode::QuitConfirm => app.theme.error,
        }
    };

    let language = Language::from_path(app.editor.path.as_deref());
    let left = format!(
        " {}  │  Tab {}/{}  │  {} lines  │  {} ",
        app.active_panel_label(),
        app.editor.active_index() + 1,
        app.editor.len(),
        app.editor.line_count(),
        language.name()
    );
    let right = if app.explorer_focused {
        format!(
            " {}/{} items  ",
            app.project.selected.saturating_add(1).min(app.project.entries.len()),
            app.project.entries.len()
        )
    } else {
        format!(
            " Ln {}, Col {}  ",
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
    )
}

fn draw_prompt_bar<W: Write>(
    out: &mut W,
    app: &App,
    row: u16,
    width: u16,
) -> io::Result<()> {
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
            format!(
                " Unsaved changes — [S] Save all & quit   [D] Discard & quit   [Esc] Cancel"
            ),
            app.theme.error,
            app.theme.background,
        ),
        _ => (
            format!(" {}", app.message),
            app.theme.prompt_bar,
            app.theme.prompt_text,
        ),
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

fn draw_hotkey_bar<W: Write>(
    out: &mut W,
    app: &App,
    row: u16,
    width: u16,
) -> io::Result<()> {
    let mode_color = if app.explorer_focused {
        app.theme.normal_mode
    } else {
        match app.mode {
            Mode::Normal => app.theme.normal_mode,
            Mode::Insert => app.theme.insert_mode,
            Mode::Search => app.theme.search_mode,
            Mode::Command | Mode::Help => app.theme.command_mode,
            Mode::QuitConfirm => app.theme.error,
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
            SetAttribute(if clickable { Attribute::Bold } else { Attribute::Reset }),
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
    if app.explorer_focused {
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

    match app.mode {
        Mode::Insert => &[
            ("Esc", "Normal"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("Alt-N", "NextTab"),
            ("Alt-P", "PrevTab"),
            ("Ctrl-S", "Save"),
            ("Ctrl-E", "Files"),
        ],
        Mode::Normal => &[
            ("i", "Insert"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("Alt-N", "NextTab"),
            ("Alt-P", "PrevTab"),
            ("Ctrl-E", "Files"),
            (":", "Command"),
        ],
        Mode::Search => &[
            ("Enter", "Accept"),
            ("Esc", "Cancel"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
        ],
        Mode::Command => &[
            ("Enter", "Run"),
            ("Esc", "Cancel"),
            ("Alt-←", "Back"),
            ("Alt-→", "Forward"),
            ("F1", "Help"),
        ],
        Mode::Help => &[
            ("←/→", "Page"),
            ("1-4", "Section"),
            ("Esc", "Close"),
            ("F1", "Close"),
        ],
        Mode::QuitConfirm => &[
            ("S", "Save+Quit"),
            ("D", "Discard+Quit"),
            ("Esc", "Cancel"),
        ],
    }
}

fn draw_help<W: Write>(
    out: &mut W,
    app: &App,
    terminal_width: u16,
    terminal_height: u16,
) -> io::Result<()> {
    const PAGES: [&str; 4] = ["EDITING", "NAVIGATION", "FILES", "COMMANDS"];
    const EDITING: [(&str, &str); 19] = [
        ("Type normally", "Enter text while in Insert mode"),
        ("Esc", "Switch to Normal mode"),
        ("F7", "Duplicate current line"),
        ("Ctrl + Left / Right", "Move by word"),
        ("Ctrl + Shift + Left / Right", "Select by word"),
        ("Double-click", "Select the clicked word"),
        ("Shift + Arrow/Home/End", "Select text with the keyboard"),
        ("Mouse drag", "Select text with the mouse"),
        ("Ctrl-C / Ctrl-X / Ctrl-V", "Copy / Cut / Paste selection"),
        ("Ctrl-D", "Select next occurrence; type to edit all selections"),
        ("Backspace / Delete", "Delete text or selection"),
        ("Ctrl-S", "Save the current file"),
        ("i / a  (Normal)", "Insert before / after cursor"),
        ("x / dd  (Normal)", "Delete character / line"),
        ("yy / p  (Normal)", "Copy line / paste below"),
        ("q{register} / @{register}", "Record / replay a macro in Normal mode"),
        ("Tab / Shift-Tab  (Normal)", "Indent / outdent the line or selection"),
        ("Ctrl-/", "Toggle language-aware comments"),
        ("u / Ctrl-R  (Normal)", "Undo / Redo"),
    ];
    const NAVIGATION: [(&str, &str); 11] = [
        ("Arrows or h j k l", "Move the cursor"),
        ("w / b", "Next / previous word"),
        ("0 / $", "Start / end of line"),
        ("gg / G", "Top / bottom of file"),
        ("PageUp / PageDown", "Move one screen"),
        ("Alt-Left / Alt-Right", "Go back / forward in history"),
        ("/", "Search for text"),
        ("n / N", "Next / previous search result"),
        ("Ctrl-T / Ctrl-W", "Open / close a tab"),
        ("Alt-N / Alt-P", "Next / previous tab"),
        ("Alt-1 ... Alt-9", "Select a tab directly"),
    ];
    const FILES: [(&str, &str); 11] = [
        ("Ctrl-B", "Show or hide the explorer"),
        ("Click FILES", "Show or hide the explorer"),
        ("Ctrl-E", "Switch between editor and files"),
        ("Up / Down", "Select a file or folder"),
        ("Enter", "Open a file or expand a folder"),
        ("Left / Right", "Collapse / expand a folder"),
        ("Backspace", "Move to the parent folder"),
        ("Shift-Left / Shift-Right", "Collapse recursively / expand one level"),
        ("* / -", "Expand all / collapse all"),
        (".", "Show or hide hidden files"),
        ("r", "Refresh the explorer"),
    ];
    const COMMANDS: [(&str, &str); 11] = [
        (":  (from Normal mode)", "Open the command prompt"),
        (":w  /  :w file", "Save / Save as"),
        (":q  /  :q!", "Quit / Force quit"),
        (":e path", "Open a file or folder"),
        (":new [file]", "Create a new tab"),
        (":tab 2", "Select tab 2"),
        (":bd  /  :bd!", "Close tab / Force close tab"),
        (":tree", "Show or hide the explorer"),
        (":set number / nonumber", "Show or hide line numbers"),
        (":theme oxide / mono", "Change the color theme"),
        ("Ctrl-Q", "Save, discard, or cancel quitting"),
    ];

    let page = app.help_page.min(PAGES.len() - 1);
    let rows: &[(&str, &str)] = match page {
        0 => &EDITING,
        1 => &NAVIGATION,
        2 => &FILES,
        _ => &COMMANDS,
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
        Print(format!("Page {}/4", page + 1))
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
            SetAttribute(if active { Attribute::Bold } else { Attribute::Reset }),
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
        Print("←/→ or 1-4 change page"),
        MoveTo(start_x + box_width.saturating_sub(25) as u16, start_y + box_height.saturating_sub(2) as u16),
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
    if matches!(app.mode, Mode::Help | Mode::QuitConfirm) || app.explorer_focused {
        return queue!(out, Hide);
    }

    if app.mode == Mode::Command || app.mode == Mode::Search {
        let prefix_width = 1usize;
        let input = if app.mode == Mode::Command {
            &app.command_input
        } else {
            &app.search_input
        };
        let x = (prefix_width + UnicodeWidthStr::width(input.as_str()))
            .min(terminal_width.saturating_sub(1) as usize) as u16;

        return queue!(out, MoveTo(x, terminal_height - 2), Show);
    }

    if app.editor.cursor.line < app.editor.scroll_line
        || app.editor.cursor.line >= app.editor.scroll_line + content_height
    {
        return queue!(out, Hide);
    }

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

    let y = content_top + (app.editor.cursor.line - app.editor.scroll_line) as u16;
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
