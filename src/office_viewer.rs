use std::{
    io,
    path::{Path, PathBuf},
};

use calamine::{open_workbook_auto, Reader};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::preview;

const MAX_SHEET_ROWS: usize = 100_000;
const MAX_SHEET_COLUMNS: usize = 512;
const MAX_CELL_CHARACTERS: usize = 512;

#[derive(Debug, Clone)]
pub struct OfficeViewer {
    pub path: PathBuf,
    pub title: String,
    pub content: OfficeContent,
    pub search_query: String,
    pub search_editing: bool,
    pub status: String,
}

#[derive(Debug, Clone)]
pub enum OfficeContent {
    Spreadsheet(SpreadsheetView),
    Document(DocumentView),
}

#[derive(Debug, Clone)]
pub struct SpreadsheetView {
    pub sheets: Vec<SheetView>,
    pub active_sheet: usize,
    pub row: usize,
    pub column: usize,
    pub scroll_row: usize,
    pub scroll_column: usize,
    pub detail_open: bool,
}

#[derive(Debug, Clone)]
pub struct SheetView {
    pub name: String,
    pub cells: Vec<Vec<String>>,
    pub formulas: Vec<Vec<String>>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct DocumentView {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub scroll_line: usize,
    pub detail_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerAction {
    None,
    Close,
    Copy(String),
}

impl OfficeViewer {
    pub fn open(path: &Path) -> io::Result<Self> {
        let extension = extension(path);
        let content = if extension == "docx" {
            OfficeContent::Document(load_document(path)?)
        } else {
            OfficeContent::Spreadsheet(load_spreadsheet(path)?)
        };
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Office document")
            .to_string();
        Ok(Self {
            path: path.to_path_buf(),
            title,
            content,
            search_query: String::new(),
            search_editing: false,
            status: String::new(),
        })
    }

    pub fn kind_label(&self) -> &'static str {
        match self.content {
            OfficeContent::Spreadsheet(_) => "SPREADSHEET",
            OfficeContent::Document(_) => "DOCUMENT",
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, page_rows: usize) -> ViewerAction {
        if self.search_editing {
            return self.handle_search_key(key);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            return ViewerAction::Copy(self.current_text());
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return ViewerAction::Close,
            KeyCode::Char('/') => {
                self.search_editing = true;
                self.search_query.clear();
                self.status = "Search: type a term, then Enter".to_string();
                return ViewerAction::None;
            }
            KeyCode::Char('n') => {
                self.find_next(false);
                return ViewerAction::None;
            }
            KeyCode::Char('N') => {
                self.find_next(true);
                return ViewerAction::None;
            }
            KeyCode::Char('c') => return ViewerAction::Copy(self.current_text()),
            _ => {}
        }
        match &mut self.content {
            OfficeContent::Spreadsheet(sheet) => sheet.handle_key(key, page_rows),
            OfficeContent::Document(document) => document.handle_key(key, page_rows),
        }
        ViewerAction::None
    }

    pub fn ensure_visible(&mut self, rows: usize, columns: usize) {
        match &mut self.content {
            OfficeContent::Spreadsheet(sheet) => sheet.ensure_visible(rows, columns),
            OfficeContent::Document(document) => document.ensure_visible(rows),
        }
    }

    pub fn current_text(&self) -> String {
        match &self.content {
            OfficeContent::Spreadsheet(view) => view
                .active()
                .and_then(|sheet| sheet.cell(view.row, view.column))
                .unwrap_or_default()
                .to_string(),
            OfficeContent::Document(view) => view
                .lines
                .get(view.cursor_line)
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> ViewerAction {
        match key.code {
            KeyCode::Esc => {
                self.search_editing = false;
                self.status = "Search cancelled".to_string();
            }
            KeyCode::Enter => {
                self.search_editing = false;
                self.find_next(false);
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.status = format!("Search: {}", self.search_query);
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.search_query.push(character);
                self.status = format!("Search: {}", self.search_query);
            }
            _ => {}
        }
        ViewerAction::None
    }

    fn find_next(&mut self, reverse: bool) {
        if self.search_query.is_empty() {
            self.status = "Press / and enter a search term".to_string();
            return;
        }
        let needle = self.search_query.to_ascii_lowercase();
        let found = match &mut self.content {
            OfficeContent::Spreadsheet(view) => view.find(&needle, reverse),
            OfficeContent::Document(view) => view.find(&needle, reverse),
        };
        self.status = if found {
            format!("Found “{}”", self.search_query)
        } else {
            format!("No match for “{}”", self.search_query)
        };
    }
}

impl SpreadsheetView {
    pub fn active(&self) -> Option<&SheetView> {
        self.sheets.get(self.active_sheet)
    }

    pub fn dimensions(&self) -> (usize, usize) {
        self.active().map_or((0, 0), SheetView::dimensions)
    }

    fn handle_key(&mut self, key: KeyEvent, page_rows: usize) {
        let (rows, columns) = self.dimensions();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.row = self.row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.row = (self.row + 1).min(rows.saturating_sub(1))
            }
            KeyCode::Left | KeyCode::Char('h') => self.column = self.column.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => {
                self.column = (self.column + 1).min(columns.saturating_sub(1))
            }
            KeyCode::PageUp => self.row = self.row.saturating_sub(page_rows.max(1)),
            KeyCode::PageDown => {
                self.row = (self.row + page_rows.max(1)).min(rows.saturating_sub(1))
            }
            KeyCode::Home => self.column = 0,
            KeyCode::End => self.column = columns.saturating_sub(1),
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => self.switch_sheet(-1),
            KeyCode::BackTab => self.switch_sheet(-1),
            KeyCode::Tab => self.switch_sheet(1),
            KeyCode::Enter => self.detail_open = !self.detail_open,
            _ => {}
        }
    }

    fn switch_sheet(&mut self, direction: isize) {
        if self.sheets.is_empty() {
            return;
        }
        self.active_sheet = if direction < 0 {
            self.active_sheet
                .checked_sub(1)
                .unwrap_or(self.sheets.len() - 1)
        } else {
            (self.active_sheet + 1) % self.sheets.len()
        };
        self.row = 0;
        self.column = 0;
        self.scroll_row = 0;
        self.scroll_column = 0;
        self.detail_open = false;
    }

    fn ensure_visible(&mut self, rows: usize, columns: usize) {
        let visible_rows = rows.saturating_sub(4).max(1);
        if self.row < self.scroll_row {
            self.scroll_row = self.row;
        } else if self.row >= self.scroll_row + visible_rows {
            self.scroll_row = self.row + 1 - visible_rows;
        }
        let approximate_visible_columns = (columns.saturating_sub(8) / 14).max(1);
        if self.column < self.scroll_column {
            self.scroll_column = self.column;
        } else if self.column >= self.scroll_column + approximate_visible_columns {
            self.scroll_column = self.column + 1 - approximate_visible_columns;
        }
    }

    fn find(&mut self, needle: &str, reverse: bool) -> bool {
        let Some(sheet) = self.active() else {
            return false;
        };
        let (row_count, column_count) = sheet.dimensions();
        if row_count == 0 || column_count == 0 {
            return false;
        }
        let total = row_count.saturating_mul(column_count);
        let start = self
            .row
            .saturating_mul(column_count)
            .saturating_add(self.column);
        for offset in 1..=total {
            let index = if reverse {
                (start + total - (offset % total)) % total
            } else {
                (start + offset) % total
            };
            let row = index / column_count;
            let column = index % column_count;
            if sheet
                .cell(row, column)
                .is_some_and(|cell| cell.to_ascii_lowercase().contains(needle))
            {
                self.row = row;
                self.column = column;
                return true;
            }
        }
        false
    }
}

impl SheetView {
    pub fn dimensions(&self) -> (usize, usize) {
        (
            self.cells.len(),
            self.cells.iter().map(Vec::len).max().unwrap_or(0),
        )
    }

    pub fn cell(&self, row: usize, column: usize) -> Option<&str> {
        self.cells
            .get(row)
            .and_then(|values| values.get(column))
            .map(String::as_str)
    }

    pub fn formula(&self, row: usize, column: usize) -> Option<&str> {
        self.formulas
            .get(row)
            .and_then(|values| values.get(column))
            .map(String::as_str)
            .filter(|formula| !formula.is_empty())
    }
}

impl DocumentView {
    fn handle_key(&mut self, key: KeyEvent, page_rows: usize) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor_line = self.cursor_line.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor_line = (self.cursor_line + 1).min(self.lines.len().saturating_sub(1))
            }
            KeyCode::PageUp => self.cursor_line = self.cursor_line.saturating_sub(page_rows.max(1)),
            KeyCode::PageDown => {
                self.cursor_line =
                    (self.cursor_line + page_rows.max(1)).min(self.lines.len().saturating_sub(1))
            }
            KeyCode::Home => self.cursor_line = 0,
            KeyCode::End => self.cursor_line = self.lines.len().saturating_sub(1),
            KeyCode::Enter => self.detail_open = !self.detail_open,
            _ => {}
        }
    }

    fn ensure_visible(&mut self, rows: usize) {
        let visible = rows.saturating_sub(3).max(1);
        if self.cursor_line < self.scroll_line {
            self.scroll_line = self.cursor_line;
        } else if self.cursor_line >= self.scroll_line + visible {
            self.scroll_line = self.cursor_line + 1 - visible;
        }
    }

    fn find(&mut self, needle: &str, reverse: bool) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        for offset in 1..=self.lines.len() {
            let index = if reverse {
                (self.cursor_line + self.lines.len() - (offset % self.lines.len()))
                    % self.lines.len()
            } else {
                (self.cursor_line + offset) % self.lines.len()
            };
            if self.lines[index].to_ascii_lowercase().contains(needle) {
                self.cursor_line = index;
                return true;
            }
        }
        false
    }
}

fn load_spreadsheet(path: &Path) -> io::Result<SpreadsheetView> {
    let mut workbook =
        open_workbook_auto(path).map_err(|error| io::Error::other(error.to_string()))?;
    let names = workbook.sheet_names();
    let mut sheets = Vec::with_capacity(names.len());
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let formulas = workbook.worksheet_formula(&name).unwrap_or_default();
        let source_rows = range.rows().take(MAX_SHEET_ROWS + 1).collect::<Vec<_>>();
        let truncated_rows = source_rows.len() > MAX_SHEET_ROWS;
        let cells = source_rows
            .iter()
            .take(MAX_SHEET_ROWS)
            .map(|row| {
                row.iter()
                    .take(MAX_SHEET_COLUMNS)
                    .map(|cell| bounded_cell(&cell.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let formula_rows = formulas
            .rows()
            .take(MAX_SHEET_ROWS)
            .map(|row| {
                row.iter()
                    .take(MAX_SHEET_COLUMNS)
                    .map(|formula| bounded_cell(formula))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let truncated_columns = source_rows
            .iter()
            .take(MAX_SHEET_ROWS)
            .any(|row| row.len() > MAX_SHEET_COLUMNS);
        sheets.push(SheetView {
            name,
            cells,
            formulas: formula_rows,
            truncated: truncated_rows || truncated_columns,
        });
    }
    if sheets.is_empty() {
        sheets.push(SheetView {
            name: "Sheet1".to_string(),
            cells: vec![Vec::new()],
            formulas: Vec::new(),
            truncated: false,
        });
    }
    Ok(SpreadsheetView {
        sheets,
        active_sheet: 0,
        row: 0,
        column: 0,
        scroll_row: 0,
        scroll_column: 0,
        detail_open: false,
    })
}

fn load_document(path: &Path) -> io::Result<DocumentView> {
    let content = preview::office_text_content(path)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a readable DOCX file"))?;
    let lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    Ok(DocumentView {
        lines: if lines.is_empty() {
            vec!["(empty document)".to_string()]
        } else {
            lines
        },
        cursor_line: 0,
        scroll_line: 0,
        detail_open: false,
    })
}

fn bounded_cell(value: &str) -> String {
    value.chars().take(MAX_CELL_CHARACTERS).collect()
}

pub fn supports(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "docx" | "xlsx" | "xls" | "xlsm" | "xlsb" | "ods"
    )
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub fn column_label(mut column: usize) -> String {
    let mut label = String::new();
    loop {
        label.insert(0, (b'A' + (column % 26) as u8) as char);
        if column < 26 {
            break;
        }
        column = column / 26 - 1;
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_column_labels_match_excel() {
        assert_eq!(column_label(0), "A");
        assert_eq!(column_label(25), "Z");
        assert_eq!(column_label(26), "AA");
        assert_eq!(column_label(701), "ZZ");
    }

    #[test]
    fn supported_formats_are_explicit() {
        assert!(supports(Path::new("report.docx")));
        assert!(supports(Path::new("book.xlsb")));
        assert!(!supports(Path::new("archive.zip")));
    }

    #[test]
    fn spreadsheet_navigation_and_search_are_bounded() {
        let mut view = SpreadsheetView {
            sheets: vec![SheetView {
                name: "Orders".to_string(),
                cells: vec![
                    vec!["ID".to_string(), "Description".to_string()],
                    vec!["42".to_string(), "Blue widget".to_string()],
                ],
                formulas: Vec::new(),
                truncated: false,
            }],
            active_sheet: 0,
            row: 0,
            column: 0,
            scroll_row: 0,
            scroll_column: 0,
            detail_open: false,
        };

        view.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 10);
        assert_eq!(view.row, 0);
        assert!(view.find("widget", false));
        assert_eq!((view.row, view.column), (1, 1));
        view.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        assert_eq!(view.row, 1);
    }

    #[test]
    fn document_search_wraps_through_lines() {
        let mut view = DocumentView {
            lines: vec!["alpha".to_string(), "beta".to_string()],
            cursor_line: 1,
            scroll_line: 0,
            detail_open: false,
        };

        assert!(view.find("alpha", false));
        assert_eq!(view.cursor_line, 0);
        assert!(!view.find("missing", false));
    }
}
