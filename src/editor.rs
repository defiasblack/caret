use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
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
    disk_modified: Option<SystemTime>,
    external_change_pending: bool,
    folded_ranges: BTreeMap<usize, usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    undo_group_active: bool,
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
                disk_modified: None,
                external_change_pending: false,
                folded_ranges: BTreeMap::new(),
                undo: Vec::new(),
                redo: Vec::new(),
                undo_group_active: false,
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
            disk_modified: None,
            external_change_pending: false,
            folded_ranges: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group_active: false,
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
            disk_modified: fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok(),
            external_change_pending: false,
            folded_ranges: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_group_active: false,
            preferred_column: None,
        })
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
        self.disk_modified = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok();
        self.external_change_pending = false;
        Ok(())
    }

    pub fn changed_on_disk(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let modified = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok();
        modified.is_some() && modified != self.disk_modified
    }

    pub fn acknowledge_disk_change(&mut self) {
        self.disk_modified = self.path.as_ref().and_then(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        });
    }

    pub fn keep_disk_change(&mut self) {
        self.acknowledge_disk_change();
        self.external_change_pending = true;
    }

    pub fn has_pending_external_change(&self) -> bool {
        self.external_change_pending
    }

    pub fn clear_pending_external_change(&mut self) {
        self.external_change_pending = false;
    }

    pub fn reload_from_disk(&mut self) -> io::Result<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let replacement = Self::from_file(&path)?;
        let show_line_numbers = self.show_line_numbers;
        let tab_width = self.tab_width;
        *self = replacement;
        self.show_line_numbers = show_line_numbers;
        self.tab_width = tab_width;
        Ok(())
    }

    pub fn checkpoint(&mut self) {
        self.push_undo_snapshot();
        self.undo_group_active = true;
    }

    /// Starts an undo group unless one is already active.  Consecutive typing
    /// and deletion operations share a single undo entry until navigation or
    /// a mode change ends the group.
    fn begin_undo_group(&mut self) {
        self.folded_ranges.clear();
        if !self.undo_group_active {
            self.push_undo_snapshot();
            self.undo_group_active = true;
        }
    }

    pub fn finish_undo_group(&mut self) {
        self.undo_group_active = false;
    }

    fn push_undo_snapshot(&mut self) {
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
        self.finish_undo_group();
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
        self.finish_undo_group();
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

    pub fn text(&self) -> String {
        self.buffer.to_string()
    }

    pub fn replace_text(&mut self, text: &str) {
        self.finish_undo_group();
        self.push_undo_snapshot();
        self.buffer = Rope::from_str(text);
        self.dirty = true;
        self.clamp_cursor();
    }

    pub fn line_count(&self) -> usize {
        self.buffer.len_lines()
    }

    pub fn line_number_width(&self) -> usize {
        if !self.show_line_numbers {
            return 0;
        }

        // One column for Git changes, one for fold state, and one spacer.
        self.line_count().max(1).to_string().len() + 3
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
        self.buffer
            .line_to_char(line.min(self.line_count().saturating_sub(1)))
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

    pub fn select_word_at_cursor(&mut self) -> bool {
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
        self.set_cursor_from_char_index(end);
        self.selection_anchor = Some(self.cursor_from_char_index(start));
        true
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
            !occupied
                .iter()
                .any(|(used_start, used_end)| *start < *used_end && end > *used_start)
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
        self.begin_undo_group();
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
        (anchor_index != cursor_index).then(|| {
            (
                anchor_index.min(cursor_index),
                anchor_index.max(cursor_index),
            )
        })
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
        self.cursor.column = index
            .saturating_sub(line_start)
            .min(self.line_len_chars(line));
        self.preferred_column = None;
    }

    pub fn set_cursor_from_display_position(&mut self, line: usize, display_column: usize) {
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
        self.begin_undo_group();
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

    pub fn insert_pair(&mut self, opening: char, closing: char) {
        self.begin_undo_group();
        if let Some((start, end)) = self.selection_range() {
            if self.secondary_cursors.is_empty() {
                self.buffer.insert_char(end, closing);
                self.buffer.insert_char(start, opening);
                self.set_cursor_from_char_index(end + 2);
                self.selection_anchor = None;
                self.dirty = true;
                self.preferred_column = None;
                return;
            }
        }

        if self.secondary_cursors.is_empty() {
            let index = self.current_char_index();
            self.delete_selection();
            self.buffer.insert(index, &format!("{opening}{closing}"));
            self.set_cursor_from_char_index(index + 1);
            self.dirty = true;
            self.preferred_column = None;
        } else {
            self.replace_at_cursors(&format!("{opening}{closing}"));
            self.move_all_left();
        }
    }

    pub fn skip_closing_character(&mut self, closing: char) -> bool {
        if !self.secondary_cursors.is_empty() || self.selection_range().is_some() {
            return false;
        }
        let index = self.current_char_index();
        if self.buffer.get_char(index) == Some(closing) {
            self.move_right();
            true
        } else {
            false
        }
    }

    fn replace_at_cursors(&mut self, replacement: &str) {
        self.begin_undo_group();
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

    fn move_all_left(&mut self) {
        self.move_left();
        self.secondary_cursors = self
            .secondary_cursors
            .iter()
            .map(|cursor| {
                let index = self.buffer.line_to_char(cursor.cursor.line) + cursor.cursor.column;
                SecondaryCursor {
                    cursor: self.cursor_from_char_index(index.saturating_sub(1)),
                    selection_anchor: None,
                }
            })
            .collect();
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

        self.begin_undo_group();

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

    pub fn backspace_pair(&mut self) -> bool {
        if !self.secondary_cursors.is_empty() || self.selection_range().is_some() {
            return self.backspace();
        }
        let index = self.current_char_index();
        if index > 0
            && index < self.buffer.len_chars()
            && matching_close(self.buffer.char(index - 1)) == Some(self.buffer.char(index))
        {
            self.begin_undo_group();
            self.buffer.remove(index - 1..index + 1);
            self.set_cursor_from_char_index(index - 1);
            self.dirty = true;
            self.preferred_column = None;
            return true;
        }
        self.backspace()
    }

    pub fn delete_at_cursor(&mut self) -> bool {
        if self.delete_selection().is_some() {
            return true;
        }
        let index = self.current_char_index();

        if index >= self.buffer.len_chars() {
            return false;
        }

        self.begin_undo_group();

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

        self.begin_undo_group();

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
        self.cursor.column = self
            .cursor
            .column
            .min(self.line_len_chars(self.cursor.line));
        self.dirty = true;
        self.preferred_column = None;
        Some(removed)
    }

    pub fn paste_line_below(&mut self, text: &str) {
        self.begin_undo_group();
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

    pub fn duplicate_line(&mut self) {
        self.begin_undo_group();
        let mut lines = self.logical_lines();
        let line = self.cursor.line.min(lines.len().saturating_sub(1));
        lines.insert(line, lines[line].clone());
        self.replace_logical_lines(&lines, self.has_trailing_newline());
        self.cursor.line += 1;
        self.selection_anchor = None;
        self.secondary_cursors.clear();
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn move_line(&mut self, down: bool) -> bool {
        let logical_line_count = self.logical_lines().len();
        let line = self.cursor.line.min(logical_line_count.saturating_sub(1));
        let target = if down {
            line.checked_add(1)
                .filter(|next| *next < logical_line_count)
        } else {
            line.checked_sub(1)
        };
        let Some(target) = target else {
            return false;
        };

        self.begin_undo_group();

        let mut lines = self.logical_lines();
        lines.swap(line, target);
        self.replace_logical_lines(&lines, self.has_trailing_newline());
        self.cursor.line = target;
        self.cursor.column = self.cursor.column.min(self.line_len_chars(target));
        self.selection_anchor = None;
        self.secondary_cursors.clear();
        self.dirty = true;
        self.preferred_column = None;
        true
    }

    pub fn join_line_below(&mut self) -> bool {
        let line = self.cursor.line;
        if line + 1 >= self.line_count() {
            return false;
        }

        self.begin_undo_group();

        let left = self.line_text(line);
        let right = self.line_text(line + 1);
        let line_end = self.buffer.line_to_char(line) + self.line_with_ending(line).chars().count();
        let newline_start =
            line_end.saturating_sub(if self.line_with_ending(line).ends_with("\r\n") {
                2
            } else {
                1
            });
        let mut right_start = 0;
        for character in right.chars() {
            if !character.is_whitespace() {
                break;
            }
            right_start += 1;
        }
        self.buffer.remove(newline_start..line_end + right_start);
        if !left.is_empty()
            && right.chars().nth(right_start).is_some()
            && !left.ends_with(char::is_whitespace)
        {
            self.buffer.insert_char(newline_start, ' ');
        }
        self.cursor.column = self.cursor.column.min(self.line_len_chars(line));
        self.selection_anchor = None;
        self.secondary_cursors.clear();
        self.dirty = true;
        self.preferred_column = None;
        true
    }

    pub fn sort_selected_lines(&mut self) -> usize {
        self.begin_undo_group();
        let mut lines = self.logical_lines();
        let start = self.cursor.line.min(lines.len().saturating_sub(1));
        let mut end = start;
        if let Some((_, selection_end)) = self.selection_range() {
            let end_cursor = self.cursor_from_char_index(selection_end);
            end = end_cursor.line.min(lines.len().saturating_sub(1));
            if end_cursor.column == 0 && end > start {
                end -= 1;
            }
        }
        lines[start..=end].sort_by_key(|line| line.trim().to_lowercase());
        self.replace_logical_lines(&lines, self.has_trailing_newline());
        self.cursor.line = start;
        self.cursor.column = self.cursor.column.min(self.line_len_chars(start));
        self.selection_anchor = None;
        self.secondary_cursors.clear();
        self.dirty = true;
        self.preferred_column = None;
        end - start + 1
    }

    pub fn indent_selected_lines(&mut self) -> usize {
        let (start, end, selected) = self.selected_line_range();
        let width = self.tab_width;
        self.begin_undo_group();

        let mut lines = self.logical_lines();
        for line in &mut lines[start..=end] {
            line.insert_str(0, &" ".repeat(width));
        }
        self.replace_logical_lines(&lines, self.has_trailing_newline());
        self.finish_line_edit(start, end, selected, width as isize);
        end - start + 1
    }

    pub fn outdent_selected_lines(&mut self) -> usize {
        let (start, end, selected) = self.selected_line_range();
        self.begin_undo_group();

        let mut lines = self.logical_lines();
        let mut current_removed = 0usize;
        for (offset, line) in lines[start..=end].iter_mut().enumerate() {
            let removed = leading_indent_width(line, self.tab_width);
            line.drain(..line_char_byte_index(line, removed));
            if start + offset == self.cursor.line {
                current_removed = removed;
            }
        }
        self.replace_logical_lines(&lines, self.has_trailing_newline());
        self.finish_line_edit(start, end, selected, -(current_removed as isize));
        end - start + 1
    }

    pub fn toggle_line_comments(&mut self, prefix: &str, suffix: Option<&str>) -> Option<bool> {
        let (start, end, selected) = self.selected_line_range();
        let mut lines = self.logical_lines();
        let nonblank = lines[start..=end]
            .iter()
            .filter(|line| !line.trim().is_empty())
            .count();
        if nonblank == 0 {
            return None;
        }

        let uncomment = lines[start..=end]
            .iter()
            .filter(|line| !line.trim().is_empty())
            .all(|line| is_commented(line, prefix, suffix));
        self.begin_undo_group();

        let mut current_delta = 0isize;
        for (offset, line) in lines[start..=end].iter_mut().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let indent_end = line
                .char_indices()
                .find_map(|(index, character)| (!character.is_whitespace()).then_some(index))
                .unwrap_or(line.len());
            let (indent, content) = line.split_at(indent_end);
            let replacement = if uncomment {
                uncomment_line(content, prefix, suffix)
            } else if let Some(suffix) = suffix {
                format!("{prefix} {content} {suffix}")
            } else {
                format!("{prefix} {content}")
            };
            if start + offset == self.cursor.line {
                current_delta =
                    replacement.chars().count() as isize - content.chars().count() as isize;
            }
            *line = format!("{indent}{replacement}");
        }

        self.replace_logical_lines(&lines, self.has_trailing_newline());
        self.finish_line_edit(start, end, selected, current_delta);
        Some(!uncomment)
    }

    fn selected_line_range(&self) -> (usize, usize, bool) {
        let maximum = self.logical_lines().len().saturating_sub(1);
        let start = self.cursor.line.min(maximum);
        let Some((selection_start, selection_end)) = self.selection_range() else {
            return (start, start, false);
        };
        let start_cursor = self.cursor_from_char_index(selection_start);
        let end_cursor = self.cursor_from_char_index(selection_end);
        let start = start_cursor.line.min(maximum);
        let mut end = end_cursor.line.min(maximum);
        if end_cursor.column == 0 && end > start {
            end -= 1;
        }
        (start, end.max(start), true)
    }

    fn finish_line_edit(&mut self, start: usize, end: usize, selected: bool, current_delta: isize) {
        if selected {
            self.selection_anchor = Some(Cursor {
                line: start,
                column: 0,
            });
            self.cursor = Cursor {
                line: end,
                column: self.line_len_chars(end),
            };
        } else if current_delta.is_negative() {
            self.cursor.column = self
                .cursor
                .column
                .saturating_sub(current_delta.unsigned_abs());
        } else {
            self.cursor.column = (self.cursor.column + current_delta as usize)
                .min(self.line_len_chars(self.cursor.line));
        }
        self.secondary_cursors.clear();
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn open_line_below(&mut self) {
        self.begin_undo_group();
        let insertion = if self.cursor.line + 1 < self.line_count() {
            self.buffer.line_to_char(self.cursor.line + 1)
        } else {
            self.buffer.len_chars()
        };

        self.buffer.insert_char(insertion, '\n');
        self.cursor.line += 1;

        self.cursor.column = 0;
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn open_line_above(&mut self) {
        self.begin_undo_group();
        let insertion = self.buffer.line_to_char(self.cursor.line);
        self.buffer.insert_char(insertion, '\n');
        self.cursor.column = 0;
        self.dirty = true;
        self.preferred_column = None;
    }

    pub fn move_left(&mut self) {
        self.finish_undo_group();
        if self.cursor.column > 0 {
            self.cursor.column -= 1;
        } else if self.cursor.line > 0 {
            self.cursor.line -= 1;
            self.cursor.column = self.line_len_chars(self.cursor.line);
        }
        self.preferred_column = None;
    }

    pub fn move_right(&mut self) {
        self.finish_undo_group();
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
        self.finish_undo_group();
        if self.cursor.line == 0 {
            return;
        }

        let preferred = self.preferred_column.unwrap_or(self.cursor.column);
        self.cursor.line = self.previous_visible_line(self.cursor.line).unwrap_or(0);
        self.cursor.column = preferred.min(self.line_len_chars(self.cursor.line));
        self.preferred_column = Some(preferred);
    }

    pub fn move_down(&mut self) {
        self.finish_undo_group();
        if self.cursor.line + 1 >= self.line_count() {
            return;
        }

        let preferred = self.preferred_column.unwrap_or(self.cursor.column);
        let Some(line) = self.next_visible_line(self.cursor.line) else {
            return;
        };
        self.cursor.line = line;
        self.cursor.column = preferred.min(self.line_len_chars(self.cursor.line));
        self.preferred_column = Some(preferred);
    }

    pub fn page_up(&mut self, amount: usize) {
        self.finish_undo_group();
        for _ in 0..amount {
            self.move_up();
        }
    }

    pub fn page_down(&mut self, amount: usize) {
        self.finish_undo_group();
        for _ in 0..amount {
            self.move_down();
        }
    }

    pub fn move_line_start(&mut self) {
        self.finish_undo_group();
        self.cursor.column = 0;
        self.preferred_column = None;
    }

    pub fn move_line_end(&mut self) {
        self.finish_undo_group();
        self.cursor.column = self.line_len_chars(self.cursor.line);
        self.preferred_column = None;
    }

    pub fn move_file_start(&mut self) {
        self.finish_undo_group();
        self.cursor = Cursor::default();
        self.preferred_column = None;
    }

    pub fn move_file_end(&mut self) {
        self.finish_undo_group();
        self.cursor.line = self.line_count().saturating_sub(1);
        self.cursor.column = self.line_len_chars(self.cursor.line);
        self.preferred_column = None;
    }

    pub fn move_word_forward(&mut self) {
        self.finish_undo_group();
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
        self.finish_undo_group();
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

    pub fn jump_to_matching_bracket(&mut self) -> bool {
        self.finish_undo_group();
        let characters = self.buffer.chars().collect::<Vec<_>>();
        let mut index = self.current_char_index();
        if index >= characters.len() || !is_bracket(characters[index]) {
            if index == 0 || !is_bracket(characters[index - 1]) {
                return false;
            }
            index -= 1;
        }

        let (opening, closing, forward) = match characters[index] {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => return false,
        };
        let mut depth = 0usize;
        if forward {
            for (offset, character) in characters[index..].iter().enumerate() {
                if *character == opening {
                    depth += 1;
                } else if *character == closing {
                    depth -= 1;
                    if depth == 0 {
                        self.set_cursor_from_char_index(index + offset);
                        return true;
                    }
                }
            }
        } else {
            for target in (0..=index).rev() {
                if characters[target] == closing {
                    depth += 1;
                } else if characters[target] == opening {
                    depth -= 1;
                    if depth == 0 {
                        self.set_cursor_from_char_index(target);
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn extend_word_forward(&mut self) {
        let characters = self.buffer.chars().collect::<Vec<_>>();
        let mut index = self.current_char_index();

        while index < characters.len() && !is_word_character(characters[index]) {
            index += 1;
        }
        while index < characters.len() && is_word_character(characters[index]) {
            index += 1;
        }
        self.set_cursor_from_char_index(index);
    }

    pub fn extend_word_backward(&mut self) {
        let characters = self.buffer.chars().collect::<Vec<_>>();
        let mut index = self.current_char_index();

        while index > 0 && !is_word_character(characters[index - 1]) {
            index -= 1;
        }
        while index > 0 && is_word_character(characters[index - 1]) {
            index -= 1;
        }
        self.set_cursor_from_char_index(index);
    }

    pub fn goto_line(&mut self, line: usize) {
        self.finish_undo_group();
        self.cursor.line = line.min(self.line_count().saturating_sub(1));
        self.cursor.column = self
            .cursor
            .column
            .min(self.line_len_chars(self.cursor.line));
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
        display_width(
            &line.chars().take(self.cursor.column).collect::<String>(),
            self.tab_width,
        )
    }

    pub fn ensure_cursor_visible(&mut self, rows: usize, columns: usize) {
        if rows == 0 || columns == 0 {
            return;
        }

        self.reveal_line(self.cursor.line);
        if self.cursor.line < self.scroll_line {
            self.scroll_line = self.cursor.line;
        } else if self
            .visible_distance(self.scroll_line, self.cursor.line)
            .is_some_and(|d| d >= rows)
        {
            let mut line = self.cursor.line;
            for _ in 1..rows {
                line = self.previous_visible_line(line).unwrap_or(0);
            }
            self.scroll_line = line;
        }

        let display_column = self.cursor_display_column();

        if display_column < self.scroll_column {
            self.scroll_column = display_column;
        } else if display_column >= self.scroll_column + columns {
            self.scroll_column = display_column + 1 - columns;
        }
    }

    pub fn scroll_vertical(&mut self, delta: isize, _viewport_rows: usize) {
        if delta < 0 {
            for _ in 0..delta.unsigned_abs() {
                self.scroll_line = self.previous_visible_line(self.scroll_line).unwrap_or(0);
            }
        } else {
            for _ in 0..delta as usize {
                let Some(line) = self.next_visible_line(self.scroll_line) else {
                    break;
                };
                self.scroll_line = line;
            }
        }
    }

    pub fn toggle_fold(&mut self, ranges: &[(usize, usize)]) -> Option<bool> {
        if let Some(start) = self.fold_containing(self.cursor.line) {
            self.folded_ranges.remove(&start);
            return Some(false);
        }
        let (start, end) = ranges
            .iter()
            .copied()
            .filter(|(start, end)| *start <= self.cursor.line && self.cursor.line <= *end)
            .min_by_key(|(start, end)| end - start)?;
        self.folded_ranges.insert(start, end);
        self.cursor.line = start;
        Some(true)
    }

    pub fn close_fold(&mut self, ranges: &[(usize, usize)]) -> bool {
        self.fold_containing(self.cursor.line).is_none() && self.toggle_fold(ranges) == Some(true)
    }

    pub fn open_fold(&mut self) -> bool {
        let Some(start) = self.fold_containing(self.cursor.line) else {
            return false;
        };
        self.folded_ranges.remove(&start);
        true
    }

    pub fn close_all_folds(&mut self, ranges: &[(usize, usize)]) -> usize {
        self.folded_ranges.clear();
        for &(start, end) in ranges {
            self.folded_ranges
                .entry(start)
                .and_modify(|old| *old = (*old).max(end))
                .or_insert(end);
        }
        self.reveal_line(self.cursor.line);
        self.folded_ranges.len()
    }

    pub fn open_all_folds(&mut self) -> usize {
        let count = self.folded_ranges.len();
        self.folded_ranges.clear();
        count
    }

    pub fn folded_end(&self, line: usize) -> Option<usize> {
        self.folded_ranges.get(&line).copied()
    }

    pub fn visible_line_at(&self, start: usize, row: usize) -> Option<usize> {
        let mut line = start;
        for _ in 0..row {
            line = self.next_visible_line(line)?;
        }
        (line < self.line_count()).then_some(line)
    }

    fn fold_containing(&self, line: usize) -> Option<usize> {
        self.folded_ranges
            .iter()
            .find_map(|(&start, &end)| (start <= line && line <= end).then_some(start))
    }

    fn reveal_line(&mut self, line: usize) {
        let hidden = self
            .folded_ranges
            .iter()
            .filter_map(|(&start, &end)| (start < line && line <= end).then_some(start))
            .collect::<Vec<_>>();
        for start in hidden {
            self.folded_ranges.remove(&start);
        }
    }

    fn next_visible_line(&self, line: usize) -> Option<usize> {
        let next = self.folded_end(line).map_or(line + 1, |end| end + 1);
        (next < self.line_count()).then_some(next)
    }

    fn previous_visible_line(&self, line: usize) -> Option<usize> {
        if line == 0 {
            return None;
        }
        let candidate = line - 1;
        Some(
            self.folded_ranges
                .iter()
                .find_map(|(&start, &end)| (start < candidate && candidate <= end).then_some(start))
                .unwrap_or(candidate),
        )
    }

    fn visible_distance(&self, start: usize, target: usize) -> Option<usize> {
        let mut line = start;
        for distance in 0..self.line_count() {
            if line == target {
                return Some(distance);
            }
            line = self.next_visible_line(line)?;
        }
        None
    }

    fn clamp_cursor(&mut self) {
        self.cursor.line = self.cursor.line.min(self.line_count().saturating_sub(1));
        self.cursor.column = self
            .cursor
            .column
            .min(self.line_len_chars(self.cursor.line));
    }

    fn has_trailing_newline(&self) -> bool {
        self.buffer.len_chars() > 0 && self.buffer.char(self.buffer.len_chars() - 1) == '\n'
    }

    fn line_ending(&self) -> &'static str {
        if self.buffer.to_string().contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }

    fn logical_lines(&self) -> Vec<String> {
        let count = self
            .line_count()
            .saturating_sub(self.has_trailing_newline() as usize);
        if count == 0 {
            vec![String::new()]
        } else {
            (0..count).map(|line| self.line_text(line)).collect()
        }
    }

    fn replace_logical_lines(&mut self, lines: &[String], trailing_newline: bool) {
        let ending = self.line_ending();
        let mut text = lines.join(ending);
        if trailing_newline {
            text.push_str(ending);
        }
        self.buffer = Rope::from_str(&text);
    }
}

fn is_word_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn is_bracket(character: char) -> bool {
    matches!(character, '(' | ')' | '[' | ']' | '{' | '}')
}

fn matching_close(character: char) -> Option<char> {
    match character {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '\'' => Some('\''),
        '"' => Some('"'),
        _ => None,
    }
}

fn leading_indent_width(line: &str, tab_width: usize) -> usize {
    if line.starts_with('\t') {
        1
    } else {
        line.chars()
            .take_while(|character| *character == ' ')
            .take(tab_width)
            .count()
    }
}

fn line_char_byte_index(line: &str, characters: usize) -> usize {
    line.char_indices()
        .nth(characters)
        .map_or(line.len(), |(index, _)| index)
}

fn is_commented(line: &str, prefix: &str, suffix: Option<&str>) -> bool {
    let content = line.trim_start();
    content.starts_with(prefix) && suffix.is_none_or(|suffix| content.trim_end().ends_with(suffix))
}

fn uncomment_line(content: &str, prefix: &str, suffix: Option<&str>) -> String {
    let content = content
        .strip_prefix(prefix)
        .unwrap_or(content)
        .trim_start_matches(' ');
    let content = if let Some(suffix) = suffix {
        content
            .trim_end()
            .strip_suffix(suffix)
            .unwrap_or(content)
            .trim_end_matches(' ')
    } else {
        content
    };
    content.to_string()
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

    #[test]
    fn line_operations_preserve_line_boundaries() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("beta\nalpha\ngamma");

        editor.duplicate_line();
        assert_eq!(editor.buffer.to_string(), "beta\nbeta\nalpha\ngamma");
        assert!(editor.move_line(true));
        assert_eq!(editor.buffer.to_string(), "beta\nalpha\nbeta\ngamma");
        editor.cursor = Cursor { line: 0, column: 0 };
        assert!(editor.join_line_below());
        assert_eq!(editor.buffer.to_string(), "beta alpha\nbeta\ngamma");

        editor.set_cursor_from_char_index(0);
        assert_eq!(editor.sort_selected_lines(), 1);
        assert_eq!(editor.buffer.to_string(), "beta alpha\nbeta\ngamma");
    }

    #[test]
    fn sorting_a_selection_sorts_the_selected_lines() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("zebra\nAlpha\nmiddle\n");
        editor.cursor = Cursor { line: 0, column: 0 };
        editor.selection_anchor = Some(Cursor { line: 2, column: 6 });

        assert_eq!(editor.sort_selected_lines(), 3);
        assert_eq!(editor.buffer.to_string(), "Alpha\nmiddle\nzebra\n");
    }

    #[test]
    fn selecting_word_at_cursor_selects_the_whole_word() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("one two_three");
        editor.set_cursor_from_char_index(6);

        assert!(editor.select_word_at_cursor());
        assert_eq!(editor.selected_text().as_deref(), Some("two_three"));
    }

    #[test]
    fn extending_by_word_stops_at_word_boundaries() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("version = \"2.2.0\"");
        editor.begin_selection();
        editor.extend_word_forward();
        assert_eq!(editor.selected_text().as_deref(), Some("version"));

        editor.clear_selection();
        editor.set_cursor_from_char_index(7);
        editor.begin_selection();
        editor.extend_word_backward();
        assert_eq!(editor.selected_text().as_deref(), Some("version"));
    }

    #[test]
    fn pairs_skip_delete_and_find_matching_brackets() {
        let mut editor = Editor::blank();
        editor.insert_pair('(', ')');
        assert_eq!(editor.buffer.to_string(), "()");
        assert!(editor.skip_closing_character(')'));
        assert_eq!(editor.current_char_index(), 2);
        editor.move_left();
        assert!(editor.backspace_pair());
        assert_eq!(editor.buffer.to_string(), "");

        editor.buffer = Rope::from_str("fn main() { call([1]); }");
        editor.set_cursor_from_char_index(7);
        assert!(editor.jump_to_matching_bracket());
        assert_eq!(editor.current_char_index(), 8);
        editor.set_cursor_from_char_index(17);
        assert!(editor.jump_to_matching_bracket());
        assert_eq!(editor.current_char_index(), 19);
    }

    #[test]
    fn undo_groups_consecutive_edits_and_splits_after_navigation() {
        let mut editor = Editor::blank();

        editor.insert_char('a');
        editor.insert_char('b');
        editor.insert_char('c');
        assert_eq!(editor.buffer.to_string(), "abc");
        assert!(editor.undo());
        assert_eq!(editor.buffer.to_string(), "");

        assert!(editor.redo());
        editor.move_left();
        editor.insert_char('!');
        assert_eq!(editor.buffer.to_string(), "ab!c");
        assert!(editor.undo());
        assert_eq!(editor.buffer.to_string(), "abc");
        assert!(editor.undo());
        assert_eq!(editor.buffer.to_string(), "");
    }

    #[test]
    fn indent_and_outdent_apply_to_all_selected_lines() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("one\n  two\nthree");
        editor.cursor = Cursor { line: 2, column: 5 };
        editor.selection_anchor = Some(Cursor { line: 0, column: 0 });

        assert_eq!(editor.indent_selected_lines(), 3);
        assert_eq!(editor.buffer.to_string(), "    one\n      two\n    three");
        assert_eq!(editor.selection_anchor, Some(Cursor { line: 0, column: 0 }));
        assert_eq!(editor.cursor, Cursor { line: 2, column: 9 });

        assert_eq!(editor.outdent_selected_lines(), 3);
        assert_eq!(editor.buffer.to_string(), "one\n  two\nthree");
    }

    #[test]
    fn comments_selected_lines_and_preserves_markdown_delimiters() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("fn main() {}\n\n    run();");
        editor.cursor = Cursor {
            line: 2,
            column: 10,
        };
        editor.selection_anchor = Some(Cursor { line: 0, column: 0 });

        assert_eq!(editor.toggle_line_comments("//", None), Some(true));
        assert_eq!(
            editor.buffer.to_string(),
            "// fn main() {}\n\n    // run();"
        );
        assert_eq!(editor.toggle_line_comments("//", None), Some(false));
        assert_eq!(editor.buffer.to_string(), "fn main() {}\n\n    run();");

        editor.cursor = Cursor { line: 0, column: 0 };
        editor.selection_anchor = None;
        assert_eq!(editor.toggle_line_comments("<!--", Some("-->")), Some(true));
        assert_eq!(
            editor.buffer.to_string(),
            "<!-- fn main() {} -->\n\n    run();"
        );
        assert_eq!(
            editor.toggle_line_comments("<!--", Some("-->")),
            Some(false)
        );
        assert_eq!(editor.buffer.to_string(), "fn main() {}\n\n    run();");
    }

    #[test]
    fn folded_lines_are_skipped_by_rendering_and_navigation() {
        let mut editor = Editor::blank();
        editor.buffer = Rope::from_str("start\ninside\nend\nafter");
        assert!(editor.close_fold(&[(0, 2)]));
        assert_eq!(editor.visible_line_at(0, 1), Some(3));
        editor.move_down();
        assert_eq!(editor.cursor.line, 3);
        editor.move_up();
        assert_eq!(editor.cursor.line, 0);
        assert!(editor.open_fold());
        assert_eq!(editor.visible_line_at(0, 1), Some(1));
    }
}
