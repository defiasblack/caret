use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use ropey::Rope;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone)]
struct Snapshot {
    buffer: Rope,
    cursor: Cursor,
    selection_anchor: Option<Cursor>,
    secondary_cursors: Vec<SecondaryCursor>,
    dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondaryCursor {
    pub cursor: Cursor,
    pub selection_anchor: Option<Cursor>,
}

pub struct Editor {
    buffer: Rope,
    pub path: Option<PathBuf>,
    pub cursor: Cursor,
    pub selection_anchor: Option<Cursor>,
    pub secondary_cursors: Vec<SecondaryCursor>,
    pub scroll_line: usize,
    pub scroll_column: usize,
    pub dirty: bool,
    pub show_line_numbers: bool,
    pub tab_width: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    preferred_column: Option<usize>,
}

impl Editor {
    pub fn new(path: Option<&Path>) -> io::Result<Self> {
        match path {
            Some(path) if path.exists() => Self::from_file(path),
            Some(path) => Ok(Self {
                buffer: Rope::new(),
                path: Some(path.to_path_buf()),
                cursor: Cursor::default(),
                selection_anchor: None,
                secondary_cursors: Vec::new(),
                scroll_line: 0,
                scroll_column: 0,
                dirty: false,
                show_line_numbers: true,
                tab_width: 4,
                undo: Vec::new(),
                redo: Vec::new(),
                preferred_column: None,
            }),
            None => Ok(Self::blank()),
        }
    }

    pub fn blank() -> Self {
        Self {
            buffer: Rope::new(),
            path: None,
            cursor: Cursor::default(),
            selection_anchor: None,
            secondary_cursors: Vec::new(),
            scroll_line: 0,
            scroll_column: 0,
            dirty: false,
            show_line_numbers: true,
            tab_width: 4,
            undo: Vec::new(),
            redo: Vec::new(),
            preferred_column: None,
        }
    }

    pub fn from_file(path: &Path) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        Ok(Self {
            buffer: Rope::from_str(&contents),
            path: Some(path.to_path_buf()),
            cursor: Cursor::default(),
            selection_anchor: None,
            secondary_cursors: Vec::new(),
            scroll_line: 0,
            scroll_column: 0,
            dirty: false,
            show_line_numbers: true,
            tab_width: 4,
            undo: Vec::new(),
            redo: Vec::new(),
            preferred_column: None,
        })
    }

    pub fn open(&mut self, path: &Path) -> io::Result<()> {
        let replacement = Self::new(Some(path))?;
        let show_line_numbers = self.show_line_numbers;
        let tab_width = self.tab_width;
        *self = replacement;
        self.show_line_numbers = show_line_numbers;
        self.tab_width = tab_width;
        Ok(())
    }

    pub fn new_buffer(&mut self) {
        let show_line_numbers = self.show_line_numbers;
        let tab_width = self.tab_width;
        *self = Self::blank();
        self.show_line_numbers = show_line_numbers;
        self.tab_width = tab_width;
    }

    pub fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no filename; use :w filename",
            ));
        };

        self.save_as(&path)
    }

    pub fn save_as(&mut self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut file = fs::File::create(path)?;
        for chunk in self.buffer.chunks() {
            file.write_all(chunk.as_bytes())?;
        }
        file.flush()?;
        file.sync_all()?;

        self.path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    pub fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
            secondary_cursors: self.secondary_cursors.clone(),
            dirty: self.dirty,
        });

        const HISTORY_LIMIT: usize = 200;
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }

        self.redo.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };

        self.redo.push(Snapshot {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
            secondary_cursors: self.secondary_cursors.clone(),
            dirty: self.dirty,
        });

        self.buffer = snapshot.buffer;
        self.cursor = snapshot.cursor;
        self.selection_anchor = snapshot.selection_anchor;
        self.secondary_cursors = snapshot.secondary_cursors;
        self.dirty = snapshot.dirty;
        self.clamp_cursor();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };

        self.undo.push(Snapshot {
            buffer: self.buffer.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
            secondary_cursors: self.secondary_cursors.clone(),
            dirty: self.dirty,
        });

        self.buffer = snapshot.buffer;
        self.cursor = snapshot.cursor;
        self.selection_anchor = snapshot.selection_anchor;
        self.secondary_cursors = snapshot.secondary_cursors;
        self.dirty = snapshot.dirty;
        self.clamp_cursor();
        true
    }

    pub fn len_chars(&self) -> usize {
        self.buffer.len_chars()
    }

    pub fn line_count(&self) -> usize {
        self.buffer.len_lines()
    }

    pub fn line_number_width(&self) -> usize {
        if !self.show_line_numbers {
            return 0;
        }

        self.line_count().max(1).to_string().len() + 2
    }

    pub fn line_text(&self, line: usize) -> String {
        if line >= self.line_count() {
            return String::new();
        }

        let mut text = self.buffer.line(line).to_string();

        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }

        text
    }

    pub fn line_with_ending(&self, line: usize) -> String {
        if line >= self.line_count() {
            String::new()
        } else {
            self.buffer.line(line).to_string()
        }
    }

    pub fn line_len_chars(&self, line: usize) -> usize {
        if line >= self.line_count() {
            return 0;
        }

        let slice = self.buffer.line(line);
        let mut length = slice.len_chars();

        if length > 0 && slice.char(length - 1) == '\n' {
            length -= 1;
            if length > 0 && slice.char(length - 1) == '\r' {
                length -= 1;
            }
        }

        length
    }

    pub fn buffer_line_to_char(&self, line: usize) -> usize {
        self.buffer.line_to_char(line.min(self.line_count().saturating_sub(1)))
    }

    pub fn current_char_index(&self) -> usize {
        self.buffer.line_to_char(self.cursor.line) + self.cursor.column
    }

    pub fn selection_range(&self) -> Option<(usize, usize)> {
        Self::cursor_selection_range(&self.buffer, self.cursor, self.selection_anchor)
    }

    pub fn selection_ranges(&self) -> Vec<(usize, usize)> {
        let mut ranges = self
            .secondary_cursors
            .iter()
            .filter_map(|cursor| {
                Self::cursor_selection_range(&self.buffer, cursor.cursor, cursor.selection_anchor)
            })
            .collect::<Vec<_>>();
        if let Some(range) = self.selection_range() {
            ranges.push(range);
        }
        ranges
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        Some(self.buffer.slice(start..end).to_string())
    }

    pub fn begin_selection(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
        self.secondary_cursors.clear();
    }

    pub fn select_next_occurrence(&mut self) -> bool {
        let query = if let Some(text) = self.selected_text() {
            text
        } else {
            let characters = self.buffer.chars().collect::<Vec<_>>();
            let mut start = self.current_char_index();
            if start >= characters.len() || !is_word_character(characters[start]) {
                if start == 0 || !is_word_character(characters[start - 1]) {
                    return false;
                }
                start -= 1;
            }
            while start > 0 && is_word_character(characters[start - 1]) {
                start -= 1;
            }
            let mut end = start;
            while end < characters.len() && is_word_character(characters[end]) {
                end += 1;
            }
            if start == end {
                return false;
            }
            let text = self.buffer.slice(start..end).to_string();
            self.set_cursor_from_char_index(end);
            self.selection_anchor = Some(self.cursor_from_char_index(start));
            text
        };

        let query_len = query.chars().count();
        let text = self.buffer.to_string();
        let occupied = self.selection_ranges();
        let after = self.selection_range().map(|(_, end)| end).unwrap_or(0);
        let mut matches = text
            .match_indices(&query)
            .map(|(byte, _)| text[..byte].chars().count())
            .collect::<Vec<_>>();
        matches.sort_by_key(|start| (*start < after, *start));

        let Some(start) = matches.into_iter().find(|start| {
            let end = *start + query_len;
            !occupied.iter().any(|(used_start, used_end)| *start < *used_end && end > *used_start)
        }) else {
            return false;
        };

        self.secondary_cursors.push(SecondaryCursor {
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
        });
        self.set_cursor_from_char_index(start + query_len);
        self.selection_anchor = Some(self.cursor_from_char_index(start));
        true
    }

    pub fn delete_selection(&mut self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let text = self.buffer.slice(start..end).to_string();
        self.buffer.remove(start..end);
        self.set_cursor_from_char_index(start);
        self.selection_anchor = None;
        self.dirty = true;
        Some(text)
    }

    fn cursor_selection_range(
        buffer: &Rope,
        cursor: Cursor,
        anchor: Option<Cursor>,
    ) -> Option<(usize, usize)> {
        let anchor = anchor?;
        let anchor_index = buffer.line_to_char(anchor.line) + anchor.column;
        let cursor_index = buffer.line_to_char(cursor.line) + cursor.column;
        (anchor_index != cursor_index).then(|| (anchor_index.min(cursor_index), anchor_index.max(cursor_index)))
    }

    fn cursor_from_char_index(&self, index: usize) -> Cursor {
        let index = index.min(self.buffer.len_chars());
        let line = self.buffer.char_to_line(index);
        Cursor {
            line,
            column: index
                .saturating_sub(self.buffer.line_to_char(line))
                .min(self.line_len_chars(line)),
        }
    }

    pub fn set_cursor_from_char_index(&mut self, index: usize) {
        let index = index.min(self.buffer.len_chars());
        let line = self.buffer.char_to_line(index);
        let line_start = self.buffer.line_to_char(line);

        self.cursor.line = line;
        self.cursor.column = index.saturating_sub(line_start).min(self.line_len_chars(line));
        self.preferred_column = None;
    }

    pub fn set_cursor_from_display_position(
        &mut self,
        line: usize,
        display_column: usize,
    ) {
        let line = line.min(self.line_count().saturating_sub(1));
        let text = self.line_text(line);
        let mut visual = 0usize;
        let mut character_column = 0usize;

        for character in text.chars() {
            let width = if character == '\t' {
                self.tab_width - (visual % self.tab_width)
            } else {
                UnicodeWidthChar::width(character).unwrap_or(0)
            };

            let next = visual + width;

            if display_column < next {
                // Clicking on the right half of a wide character or tab moves
                // the cursor after it; clicking on the left half stays before it.
                if width > 0 && display_column.saturating_sub(visual) * 2 >= width {
                    character_column += 1;
                }
                break;
            }

            visual = next;
            character_column += 1;
        }

        self.cursor.line = line;
        self.cursor.column = character_column.min(self.line_len_chars(line));
        self.preferred_column = None;
    }

    pub fn insert_char(&mut self, character: char) {
        if !self.secondary_cursors.is_empty() {
            self.replace_at_cursors(&character.to_string());
            return;
        }
        self.delete_selection();
        let index = self.current_char_index();

        if character == '\n' {
            self.buffer.insert_char(index, '\n');
            self.cursor.line += 1;
            self.cursor.column = 0;
        } else {
            self.buffer.insert_char(index, character);
            self.cursor.column += 1;
        }

        self.dirty = true;
        self.preferred_column = None;
    }

    fn replace_at_cursors(&mut self, replacement: &str) {
        let primary = (self.current_char_index(), self.selection_range());
        let mut targets = self
            .secondary_cursors
            .iter()
            .map(|cursor| {
                let index = self.buffer.line_to_char(cursor.cursor.line) + cursor.cursor.column;
                let range = Self::cursor_selection_range(
                    &self.buffer,
                    cursor.cursor,
                    cursor.selection_anchor,
                );
                (false, index, range)
            })
            .collect::<Vec<_>>();
        targets.push((true, primary.0, primary.1));
        targets.sort_by_key(|(_, index, range)| range.map(|(start, _)| start).unwrap_or(*index));

        let replacement_len = replacement.chars().count();
        let mut offset: isize = 0;
        let mut primary_index = 0;
        let mut secondary_indices = Vec::new();
        for (is_primary, index, range) in targets {
            let (start, end) = range.unwrap_or((index, index));
            let start = (start as isize + offset) as usize;
            let end = (end as isize + offset) as usize;
            self.buffer.remove(start..end);
            self.buffer.insert(start, replacement);
            let next = start + replacement_len;
            if is_primary {
                primary_index = next;
            } else {
                secondary_indices.push(next);
            }
            offset += replacement_len as isize - (end - start) as isize;
        }

        self.set_cursor_from_char_index(primary_index);
        self.selection_anchor = None;
        self.secondary_cursors = secondary_indices
            .into_iter()
            .map(|index| SecondaryCursor {
                cursor: self.cursor_from_char_index(index),
                selection_anchor: None,
            })
            .collect();
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        for character in text.chars() {
            self.insert_char(character);
        }
    }

    pub fn insert_tab(&mut self) {
        let display_column = self.cursor_display_column();
        let spaces = self.tab_width - (display_column % self.tab_width);
        self.insert_text(&" ".repeat(spaces));
    }

    pub fn backspace(&mut self) -> bool {
        if self.delete_selection().is_some() {
            return true;
        }
        let index = self.current_char_index();

        if index == 0 {
            return false;
        }

        if self.cursor.column > 0 {
            self.buffer.remove(index - 1..index);
            self.cursor.column -= 1;
        } else {
            let previous_line = self.cursor.line - 1;
            let previous_length = self.line_len_chars(previous_line);
            let mut start = index - 1;

            if index >= 2
                && self.buffer.char(index - 1) == '\n'
                && self.buffer.char(index - 2) == '\r'
            {
                start = index - 2;
            }

            self.buffer.remove(start..index);
            self.cursor.line = previous_line;
            self.cursor.column = previous_length;
        }

        self.dirty = true;
        self.preferred_column = None;
        true
    }

    pub fn delete_at_cursor(&mut self) -> bool {
        if self.delete_selection().is_some() {
            return true;
        }
        let index = self.current_char_index();

        if index >= self.buffer.len_chars() {
            return false;
        }

        let current = self.buffer.char(index);

        if current == '\r'
            && index + 1 < self.buffer.len_chars()
            && self.buffer.char(index + 1) == '\n'
        {
            self.buffer.remove(index..index + 2);
        } else {
            self.buffer.remove(index..index + 1);
        }

        self.dirty = true;
        self.clamp_cursor();
        true
    }

    pub fn delete_line(&mut self) -> Option<String> {
        if self.line_count() == 0 {
            return None;
        }

        let line = self.cursor.line;
        let removed = self.line_with_ending(line);
        let start = self.buffer.line_to_char(line);
        let end = if line + 1 < self.line_count() {
            self.buffer.line_to_char(line + 1)
        } else {
            self.buffer.len_chars()
        };

        if start < end {
            self.buffer.remove(start..end);
        } else if self.buffer.len_chars() > 0 {
            self.buffer.remove(start.saturating_sub(1)..start);
        }

        self.cursor.line = self.cursor.line.min(self.line_count().saturating_sub(1));
        self.cursor.column = self.cursor.column.min(self.line_len_chars(self.cursor.line));
        self.dirty = true;
        self.preferred_column = None;
        Some(removed)
    }

    pub fn paste_line_below(&mut self, text: &str) {
        let mut line_text = text.to_string();
        if !line_text.ends_with('\n') {
            line_text.push('\n');
        }

        let insertion = if self.cursor.line + 1 < self.line_count() {
            self.buffer.line_to_char(self.cursor.line + 1)
        } else {
            self.buffer.len_chars()
        };

        if insertion == self.buffer.len_chars()
            && insertion > 0
            && self.buffer.char(insertion - 1) != '\n'
        {
            line_text.insert(0, '\n');
        }

        self.buffer.insert(insertion, &line_text);
        self.cursor.line = (self.cursor.line + 1).min(self.line_count().saturating_sub(1));
        self.cursor.column = 0;
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn open_line_below(&mut self) {
        let insertion = if self.cursor.line + 1 < self.line_count() {
            self.buffer.line_to_char(self.cursor.line + 1)
        } else {
            self.buffer.len_chars()
        };

        if insertion == self.buffer.len_chars()
            && insertion > 0
            && self.buffer.char(insertion - 1) != '\n'
        {
            self.buffer.insert_char(insertion, '\n');
            self.cursor.line += 1;
        } else {
            self.buffer.insert_char(insertion, '\n');
            self.cursor.line += 1;
        }

        self.cursor.column = 0;
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn open_line_above(&mut self) {
        let insertion = self.buffer.line_to_char(self.cursor.line);
        self.buffer.insert_char(insertion, '\n');
        self.cursor.column = 0;
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn move_left(&mut self) {
        if self.cursor.column > 0 {
            self.cursor.column -= 1;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.column = self.line_len_chars(self.cursor.line);
        }
        self.preferred_column = None;
    }

    pub fn move_right(&mut self) {
        let line_length = self.line_len_chars(self.cursor.line);

        if self.cursor.column < line_length {
            self.cursor.column += 1;
        } else if self.cursor.line + 1 < self.line_count() {
            self.cursor.line += 1;
            self.cursor.column = 0;
        }
        self.preferred_column = None;
    }

    pub fn move_up(&mut self) {
        if self.cursor.line == 0 {
            return;
        }

        let preferred = self.preferred_column.unwrap_or(self.cursor.column);
        self.cursor.line -= 1;
        self.cursor.column = preferred.min(self.line_len_chars(self.cursor.line));
        self.preferred_column = Some(preferred);
    }

    pub fn move_down(&mut self) {
        if self.cursor.line + 1 >= self.line_count() {
            return;
        }

        let preferred = self.preferred_column.unwrap_or(self.cursor.column);
        self.cursor.line += 1;
        self.cursor.column = preferred.min(self.line_len_chars(self.cursor.line));
        self.preferred_column = Some(preferred);
    }

    pub fn page_up(&mut self, amount: usize) {
        for _ in 0..amount {
            self.move_up();
        }
    }

    pub fn page_down(&mut self, amount: usize) {
        for _ in 0..amount {
            self.move_down();
        }
    }

    pub fn move_line_start(&mut self) {
        self.cursor.column = 0;
        self.preferred_column = None;
    }

    pub fn move_line_end(&mut self) {
        self.cursor.column = self.line_len_chars(self.cursor.line);
        self.preferred_column = None;
    }

    pub fn move_file_start(&mut self) {
        self.cursor = Cursor::default();
        self.preferred_column = None;
    }

    pub fn move_file_end(&mut self) {
        self.cursor.line = self.line_count().saturating_sub(1);
        self.cursor.column = self.line_len_chars(self.cursor.line);
        self.preferred_column = None;
    }

    pub fn move_word_forward(&mut self) {
        let characters: Vec<char> = self.buffer.chars().collect();
        let mut index = self.current_char_index();

        if index >= characters.len() {
            return;
        }

        if is_word_character(characters[index]) {
            while index < characters.len() && is_word_character(characters[index]) {
                index += 1;
            }
        }

        while index < characters.len() && !is_word_character(characters[index]) {
            index += 1;
        }

        self.set_cursor_from_char_index(index);
    }

    pub fn move_word_backward(&mut self) {
        let characters: Vec<char> = self.buffer.chars().collect();
        let mut index = self.current_char_index();

        if index == 0 || characters.is_empty() {
            return;
        }

        index -= 1;

        while index > 0 && !is_word_character(characters[index]) {
            index -= 1;
        }

        while index > 0 && is_word_character(characters[index - 1]) {
            index -= 1;
        }

        self.set_cursor_from_char_index(index);
    }

    pub fn goto_line(&mut self, line: usize) {
        self.cursor.line = line.min(self.line_count().saturating_sub(1));
        self.cursor.column = self.cursor.column.min(self.line_len_chars(self.cursor.line));
        self.preferred_column = None;
    }

    pub fn find_next(&mut self, query: &str, forward: bool) -> bool {
        if query.is_empty() {
            return false;
        }

        let text = self.buffer.to_string();
        let mut matches = Vec::new();

        for (byte_index, _) in text.match_indices(query) {
            let char_index = text[..byte_index].chars().count();
            matches.push(char_index);
        }

        if matches.is_empty() {
            return false;
        }

        let current = self.current_char_index();

        let target = if forward {
            matches
                .iter()
                .copied()
                .find(|index| *index > current)
                .unwrap_or(matches[0])
        } else {
            matches
                .iter()
                .copied()
                .rev()
                .find(|index| *index < current)
                .unwrap_or(*matches.last().unwrap())
        };

        self.set_cursor_from_char_index(target);
        true
    }

    pub fn cursor_display_column(&self) -> usize {
        let line = self.line_text(self.cursor.line);
        display_width(&line.chars().take(self.cursor.column).collect::<String>(), self.tab_width)
    }

    pub fn ensure_cursor_visible(&mut self, rows: usize, columns: usize) {
        if rows == 0 || columns == 0 {
            return;
        }

        if self.cursor.line < self.scroll_line {
            self.scroll_line = self.cursor.line;
        } else if self.cursor.line >= self.scroll_line + rows {
            self.scroll_line = self.cursor.line + 1 - rows;
        }

        let display_column = self.cursor_display_column();

        if display_column < self.scroll_column {
            self.scroll_column = display_column;
        } else if display_column >= self.scroll_column + columns {
            self.scroll_column = display_column + 1 - columns;
        }
    }

    pub fn scroll_vertical(&mut self, delta: isize, viewport_rows: usize) {
        if delta < 0 {
            self.scroll_line = self.scroll_line.saturating_sub(delta.unsigned_abs());
        } else {
            let maximum = self.line_count().saturating_sub(viewport_rows.max(1));
            self.scroll_line = (self.scroll_line + delta as usize).min(maximum);
        }
    }

    fn clamp_cursor(&mut self) {
        self.cursor.line = self.cursor.line.min(self.line_count().saturating_sub(1));
        self.cursor.column = self.cursor.column.min(self.line_len_chars(self.cursor.line));
    }
}

fn is_word_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

pub fn display_width(text: &str, tab_width: usize) -> usize {
    let mut column = 0;

    for character in text.chars() {
        if character == '\t' {
            column += tab_width - (column % tab_width);
        } else {
            column += UnicodeWidthChar::width(character).unwrap_or(0);
        }
    }

    column
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_next_occurrence_and_replace_all() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("alpha beta alpha alpha");
        editor.set_cursor_from_char_index(0);

        assert!(editor.select_next_occurrence());
        assert!(editor.select_next_occurrence());
        assert_eq!(editor.selection_ranges().len(), 3);

        editor.insert_char('z');
        assert_eq!(editor.buffer.to_string(), "z beta z z");
        editor.insert_char('!');
        assert_eq!(editor.buffer.to_string(), "z! beta z! z!");
    }
}
