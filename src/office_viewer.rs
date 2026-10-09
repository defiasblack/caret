use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

use calamine::{open_workbook_auto_from_rs, DataType, Reader};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

use crate::preview;

const MAX_SHEET_ROWS: usize = 100_000;
const MAX_SHEET_COLUMNS: usize = 512;
const MAX_CELL_CHARACTERS: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficeViewer {
    pub path: PathBuf,
    pub title: String,
    pub content: OfficeContent,
    pub search_query: String,
    pub search_editing: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OfficeContent {
    Spreadsheet(SpreadsheetView),
    Document(DocumentView),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpreadsheetView {
    pub sheets: Vec<SheetView>,
    pub active_sheet: usize,
    pub row: usize,
    pub column: usize,
    pub scroll_row: usize,
    pub scroll_column: usize,
    pub detail_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetView {
    pub name: String,
    pub cells: BTreeMap<usize, BTreeMap<usize, String>>,
    pub formulas: BTreeMap<usize, BTreeMap<usize, String>>,
    pub row_count: usize,
    pub column_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub const ROW_HEADER_WIDTH: usize = 7;
pub const CELL_WIDTH: usize = 24;
pub const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_CELLS: usize = 1_000_000;
#[derive(Debug, Clone, Copy)]
pub struct ViewerViewport {
    pub rows: usize,
    pub columns: usize,
}
impl ViewerViewport {
    pub fn spreadsheet(height: usize, width: usize) -> Self {
        Self {
            rows: height.saturating_sub(5),
            columns: width.saturating_sub(ROW_HEADER_WIDTH) / CELL_WIDTH,
        }
    }
    pub fn document(height: usize) -> Self {
        Self {
            rows: height.saturating_sub(3),
            columns: 1,
        }
    }
}

impl OfficeViewer {
    pub fn open(path: &Path) -> io::Result<Self> {
        if std::fs::metadata(path)?.len() > MAX_INPUT_BYTES {
            return Err(io::Error::other(
                "Office file exceeds the 32 MiB input budget",
            ));
        }
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
        let viewport = ViewerViewport::spreadsheet(rows, columns);
        let visible_rows = viewport.rows.max(1);
        if self.row < self.scroll_row {
            self.scroll_row = self.row;
        } else if self.row >= self.scroll_row + visible_rows {
            self.scroll_row = self.row + 1 - visible_rows;
        }
        let visible_columns = viewport.columns.max(1);
        if self.column < self.scroll_column {
            self.scroll_column = self.column;
        } else if self.column >= self.scroll_column + visible_columns {
            self.scroll_column = self.column + 1 - visible_columns;
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
        let start = (self.row, self.column);
        let matching = sheet
            .cells
            .iter()
            .flat_map(|(row, values)| {
                values
                    .iter()
                    .map(move |(column, value)| ((*row, *column), value))
            })
            .filter(|(_, value)| value.to_ascii_lowercase().contains(needle))
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        let next = if reverse {
            matching
                .iter()
                .rev()
                .find(|position| **position < start)
                .or_else(|| matching.last())
        } else {
            matching
                .iter()
                .find(|position| **position > start)
                .or_else(|| matching.first())
        };
        if let Some(&(row, column)) = next {
            self.row = row;
            self.column = column;
            return true;
        }
        false
    }
}

impl SheetView {
    fn empty(name: String) -> Self {
        Self {
            name,
            cells: BTreeMap::new(),
            formulas: BTreeMap::new(),
            row_count: 0,
            column_count: 0,
            truncated: false,
        }
    }
    #[cfg(test)]
    pub fn from_rows(name: &str, rows: Vec<Vec<String>>) -> Self {
        let mut sheet = Self::empty(name.to_string());
        for (row, cells) in rows.into_iter().enumerate() {
            sheet.row_count = row + 1;
            sheet.column_count = sheet.column_count.max(cells.len());
            for (column, text) in cells.into_iter().enumerate() {
                if !text.is_empty() {
                    sheet.cells.entry(row).or_default().insert(column, text);
                }
            }
        }
        sheet
    }
    pub fn dimensions(&self) -> (usize, usize) {
        (self.row_count, self.column_count)
    }
    pub fn cell(&self, row: usize, column: usize) -> Option<&str> {
        self.cells
            .get(&row)
            .and_then(|values| values.get(&column))
            .map(String::as_str)
    }
    pub fn formula(&self, row: usize, column: usize) -> Option<&str> {
        self.formulas
            .get(&row)
            .and_then(|values| values.get(&column))
            .map(String::as_str)
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
        let visible = ViewerViewport::document(rows).rows.max(1);
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
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(io::Error::other(
            "Office file exceeds the 32 MiB input budget",
        ));
    }
    let mut workbook = open_workbook_auto_from_rs(std::io::Cursor::new(bytes))
        .map_err(|error| io::Error::other(error.to_string()))?;
    let mut sheets = Vec::new();
    let mut cell_count = 0;
    for name in workbook.sheet_names() {
        let mut sheet = SheetView::empty(name.clone());
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let (start_row, start_column) = range.start().unwrap_or((0, 0));
        for (row, column, cell) in range.used_cells() {
            if cell.is_empty() {
                continue;
            }
            add_cell(
                &mut sheet,
                row + start_row as usize,
                column + start_column as usize,
                &cell.to_string(),
                false,
                &mut cell_count,
            )?;
        }
        drop(range);
        let formulas = workbook
            .worksheet_formula(&name)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let (start_row, start_column) = formulas.start().unwrap_or((0, 0));
        for (row, column, formula) in formulas.used_cells() {
            if !formula.is_empty() {
                add_cell(
                    &mut sheet,
                    row + start_row as usize,
                    column + start_column as usize,
                    formula,
                    true,
                    &mut cell_count,
                )?;
            }
        }
        sheets.push(sheet);
    }
    if sheets.is_empty() {
        sheets.push(SheetView::empty("Sheet1".into()));
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

fn add_cell(
    sheet: &mut SheetView,
    row: usize,
    column: usize,
    value: &str,
    formula: bool,
    count: &mut usize,
) -> io::Result<()> {
    *count += 1;
    if *count > MAX_CELLS {
        return Err(io::Error::other(
            "workbook exceeds the one million cell budget",
        ));
    }
    if row >= MAX_SHEET_ROWS || column >= MAX_SHEET_COLUMNS {
        sheet.truncated = true;
        return Ok(());
    }
    sheet.row_count = sheet.row_count.max(row + 1);
    sheet.column_count = sheet.column_count.max(column + 1);
    let cells = if formula {
        &mut sheet.formulas
    } else {
        &mut sheet.cells
    };
    if value.chars().count() > MAX_CELL_CHARACTERS {
        sheet.truncated = true;
    }
    cells
        .entry(row)
        .or_default()
        .insert(column, bounded_cell(value));
    Ok(())
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
            sheets: vec![SheetView::from_rows(
                "Orders",
                vec![
                    vec!["ID".to_string(), "Description".to_string()],
                    vec!["42".to_string(), "Blue widget".to_string()],
                ],
            )],
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

    #[test]
    fn offset_value_and_formula_ranges_keep_absolute_coordinates() {
        let path = std::env::temp_dir().join(format!("caret-offset-{}.xlsx", std::process::id()));
        crate::test_support::write_workbook(
            &path,
            r#"<row r="3"><c r="B3"><v>7</v></c></row><row r="5"><c r="D5"><f>SUM(B3)</f></c></row>"#,
        );
        let viewer = OfficeViewer::open(&path).unwrap();
        let OfficeContent::Spreadsheet(view) = viewer.content else {
            panic!("spreadsheet");
        };
        let sheet = view.active().unwrap();
        assert_eq!(sheet.cell(2, 1), Some("7"));
        assert_eq!(sheet.formula(4, 3), Some("SUM(B3)"));
        assert_eq!(sheet.cell(0, 0), None);
        assert_eq!(sheet.formula(0, 0), None);
        assert_eq!(sheet.dimensions(), (5, 4));
        assert_eq!(sheet.cells.values().map(BTreeMap::len).sum::<usize>(), 1);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn sparse_formula_only_and_empty_sheets_obey_budgets() {
        let mut sheet = SheetView::empty("empty".into());
        let mut count = 0;
        assert_eq!(sheet.dimensions(), (0, 0));
        add_cell(&mut sheet, 99_999, 511, "A1", true, &mut count).unwrap();
        assert_eq!(sheet.dimensions(), (100_000, 512));
        assert!(sheet.cells.is_empty());
        assert_eq!(sheet.formulas.len(), 1);
        add_cell(&mut sheet, 100_000, 512, "hidden", false, &mut count).unwrap();
        assert!(sheet.truncated);
        count = MAX_CELLS;
        assert!(add_cell(&mut sheet, 0, 0, "too many", false, &mut count).is_err());
    }
    #[test]
    fn paging_and_resizing_keep_selection_inside_the_drawn_viewport() {
        let mut viewer = OfficeViewer {
            path: PathBuf::from("big.xlsx"),
            title: "big".into(),
            content: OfficeContent::Spreadsheet(SpreadsheetView {
                sheets: vec![SheetView::from_rows("big", vec![vec!["x".into(); 50]; 100])],
                active_sheet: 0,
                row: 0,
                column: 0,
                scroll_row: 0,
                scroll_column: 0,
                detail_open: false,
            }),
            search_query: String::new(),
            search_editing: false,
            status: String::new(),
        };
        for (height, width) in [(40, 120), (9, 55), (6, 31), (4, 5), (30, 200)] {
            let viewport = ViewerViewport::spreadsheet(height, width);
            viewer.handle_key(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                viewport.rows,
            );
            viewer.handle_key(
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
                viewport.rows,
            );
            viewer.ensure_visible(height, width);
            let OfficeContent::Spreadsheet(view) = &viewer.content else {
                unreachable!()
            };
            assert!(
                view.row >= view.scroll_row && view.row < view.scroll_row + viewport.rows.max(1)
            );
            assert!(
                view.column >= view.scroll_column
                    && view.column < view.scroll_column + viewport.columns.max(1)
            );
        }
    }
}
