use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use flate2::read::DeflateDecoder;

use crate::syntax::Language;

const MAX_DIRECTORY_ENTRIES: usize = 50_000;
const READ_CHUNK_BYTES: usize = 16 * 1024;
const MAX_RENDERED_LINE_CHARS: usize = 240;
const MAX_OFFICE_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_OFFICE_ENTRY_NAME_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Loading,
    Empty,
    Cancelled,
    Unsupported {
        reason: String,
    },
    Directory {
        children: usize,
        directories: usize,
        files: usize,
        total_bytes: u64,
        truncated: bool,
    },
    Text {
        lines: Vec<String>,
        truncated: bool,
        structured: Option<&'static str>,
        language: Language,
    },
    Office {
        kind: &'static str,
        summary: Vec<String>,
        lines: Vec<String>,
        truncated: bool,
    },
    Binary {
        size: u64,
        header: String,
        kind: &'static str,
        dimensions: Option<(u32, u32)>,
    },
    Symlink {
        target: PathBuf,
        exists: bool,
    },
    Error(String),
}

#[derive(Clone)]
pub struct PreviewRequest {
    pub path: PathBuf,
    pub max_bytes: usize,
    pub max_lines: usize,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

impl PreviewRequest {
    pub fn new(
        path: PathBuf,
        max_bytes: usize,
        max_lines: usize,
        timeout: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            path,
            max_bytes: max_bytes.max(1),
            max_lines: max_lines.max(1),
            cancelled,
            deadline: Instant::now() + timeout,
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    fn interrupted(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }
}

pub trait PreviewProvider: Sync {
    fn supports(&self, metadata: &fs::Metadata) -> bool;
    fn build(&self, request: &PreviewRequest, metadata: &fs::Metadata) -> Preview;
}

struct SymlinkProvider;
struct DirectoryProvider;
struct RegularFileProvider;

static PROVIDERS: [&dyn PreviewProvider; 3] =
    [&SymlinkProvider, &DirectoryProvider, &RegularFileProvider];

pub fn generate(request: &PreviewRequest) -> Preview {
    if request.interrupted() {
        return Preview::Cancelled;
    }
    let metadata = match fs::symlink_metadata(&request.path) {
        Ok(metadata) => metadata,
        Err(error) => return Preview::Error(error.to_string()),
    };
    for provider in PROVIDERS {
        if provider.supports(&metadata) {
            return provider.build(request, &metadata);
        }
    }
    Preview::Unsupported {
        reason: "this filesystem object type has no safe preview provider".to_string(),
    }
}

impl PreviewProvider for SymlinkProvider {
    fn supports(&self, metadata: &fs::Metadata) -> bool {
        metadata.file_type().is_symlink()
    }

    fn build(&self, request: &PreviewRequest, _metadata: &fs::Metadata) -> Preview {
        if request.interrupted() {
            return Preview::Cancelled;
        }
        match fs::read_link(&request.path) {
            Ok(target) => {
                let resolved = request
                    .path
                    .parent()
                    .map(|parent| parent.join(&target))
                    .unwrap_or_else(|| target.clone());
                Preview::Symlink {
                    target,
                    exists: resolved.exists(),
                }
            }
            Err(error) => Preview::Error(error.to_string()),
        }
    }
}

impl PreviewProvider for DirectoryProvider {
    fn supports(&self, metadata: &fs::Metadata) -> bool {
        metadata.is_dir()
    }

    fn build(&self, request: &PreviewRequest, _metadata: &fs::Metadata) -> Preview {
        let read_dir = match fs::read_dir(&request.path) {
            Ok(read_dir) => read_dir,
            Err(error) => return Preview::Error(error.to_string()),
        };
        let mut children = 0usize;
        let mut directories = 0usize;
        let mut files = 0usize;
        let mut total_bytes = 0u64;
        for result in read_dir.take(MAX_DIRECTORY_ENTRIES + 1) {
            if request.interrupted() {
                return Preview::Cancelled;
            }
            let entry = match result {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            children += 1;
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    directories += 1;
                } else {
                    files += 1;
                    total_bytes = total_bytes.saturating_add(metadata.len());
                }
            }
        }
        Preview::Directory {
            children: children.min(MAX_DIRECTORY_ENTRIES),
            directories,
            files,
            total_bytes,
            truncated: children > MAX_DIRECTORY_ENTRIES,
        }
    }
}

impl PreviewProvider for RegularFileProvider {
    fn supports(&self, metadata: &fs::Metadata) -> bool {
        metadata.is_file()
    }

    fn build(&self, request: &PreviewRequest, metadata: &fs::Metadata) -> Preview {
        match office_preview(request) {
            Ok(Some(preview)) => return preview,
            Ok(None) => {}
            Err(BuildError::Cancelled) => return Preview::Cancelled,
            Err(BuildError::Io(error)) => {
                return Preview::Error(format!("Office preview could not be read: {error}"));
            }
        }
        let bytes = match read_bounded(request) {
            Ok(bytes) => bytes,
            Err(BuildError::Cancelled) => return Preview::Cancelled,
            Err(BuildError::Io(error)) => return Preview::Error(error.to_string()),
        };
        let truncated_bytes = bytes.len() > request.max_bytes;
        let bounded = &bytes[..bytes.len().min(request.max_bytes)];
        if looks_binary(bounded) {
            return Preview::Binary {
                size: metadata.len(),
                header: hex_header(bounded),
                kind: binary_kind(&request.path, bounded),
                dimensions: image_dimensions(bounded),
            };
        }
        let text = String::from_utf8_lossy(bounded);
        let mut lines = text
            .lines()
            .take(request.max_lines + 1)
            .map(|line| line.chars().take(MAX_RENDERED_LINE_CHARS).collect())
            .collect::<Vec<String>>();
        let truncated_lines = lines.len() > request.max_lines;
        lines.truncate(request.max_lines);
        Preview::Text {
            lines,
            truncated: truncated_bytes || truncated_lines,
            structured: structured_kind(&request.path, &text),
            language: Language::from_path(Some(&request.path)),
        }
    }
}

#[derive(Debug, Clone)]
struct OfficeZipEntry {
    compression: u16,
    flags: u16,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header_offset: u64,
}

struct OfficePackage {
    path: PathBuf,
    file_len: u64,
    entries: HashMap<String, OfficeZipEntry>,
}

struct ExtractedOfficeText {
    text: String,
    truncated: bool,
}

fn office_preview(request: &PreviewRequest) -> Result<Option<Preview>, BuildError> {
    let Some((kind, main_part)) = office_kind(&request.path) else {
        return Ok(None);
    };
    let Some(package) = OfficePackage::open(request)? else {
        return Ok(None);
    };
    if !package.entries.contains_key("[Content_Types].xml")
        || !package.entries.contains_key(main_part)
    {
        return Ok(None);
    }

    match kind {
        "Word document" => preview_word_document(request, &package).map(Some),
        "Excel workbook" => preview_excel_workbook(request, &package).map(Some),
        _ => Ok(None),
    }
}

pub fn office_text_content(path: &Path) -> io::Result<Option<String>> {
    let request = PreviewRequest::new(
        path.to_path_buf(),
        8 * 1024 * 1024,
        10_000,
        Duration::from_secs(3),
        Arc::new(AtomicBool::new(false)),
    );
    match office_preview(&request) {
        Ok(Some(Preview::Office {
            kind,
            summary,
            lines,
            truncated,
        })) => {
            let mut content = vec![kind.to_string()];
            content.extend(summary);
            if !lines.is_empty() {
                content.push(String::new());
                content.extend(lines);
            }
            if truncated {
                content.push("… document preview truncated".to_string());
            }
            Ok(Some(content.join("\n")))
        }
        Ok(_) => Ok(None),
        Err(BuildError::Cancelled) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "document preview timed out",
        )),
        Err(BuildError::Io(error)) => Err(error),
    }
}

fn office_kind(path: &Path) -> Option<(&'static str, &'static str)> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "docx" => Some(("Word document", "word/document.xml")),
        "xlsx" => Some(("Excel workbook", "xl/workbook.xml")),
        _ => None,
    }
}

impl OfficePackage {
    fn open(request: &PreviewRequest) -> Result<Option<Self>, BuildError> {
        let mut file = fs::File::open(&request.path).map_err(BuildError::Io)?;
        let file_len = file.metadata().map_err(BuildError::Io)?.len();
        let tail_len = file_len.min(22 + u16::MAX as u64) as usize;
        if tail_len < 22 {
            return Ok(None);
        }
        if request.interrupted() {
            return Err(BuildError::Cancelled);
        }
        file.seek(SeekFrom::End(-(tail_len as i64)))
            .map_err(BuildError::Io)?;
        let mut tail = vec![0; tail_len];
        file.read_exact(&mut tail).map_err(BuildError::Io)?;
        let Some(eocd) = find_zip_end_record(&tail) else {
            return Ok(None);
        };
        let entry_count = le_u16(&tail[eocd + 10..eocd + 12]) as usize;
        let central_size = le_u32(&tail[eocd + 12..eocd + 16]) as u64;
        let central_offset = le_u32(&tail[eocd + 16..eocd + 20]) as u64;
        if entry_count == u16::MAX as usize
            || central_size == u32::MAX as u64
            || central_offset == u32::MAX as u64
        {
            return Err(invalid_office_archive(
                "Zip64 Office documents are not previewed",
            ));
        }
        if entry_count > MAX_OFFICE_ARCHIVE_ENTRIES {
            return Err(invalid_office_archive("archive contains too many entries"));
        }
        let central_end = central_offset
            .checked_add(central_size)
            .filter(|end| *end <= file_len)
            .ok_or_else(|| invalid_office_archive("invalid central directory bounds"))?;
        if central_size > request.max_bytes as u64 {
            return Err(invalid_office_archive(
                "archive directory exceeds the preview limit",
            ));
        }
        file.seek(SeekFrom::Start(central_offset))
            .map_err(BuildError::Io)?;
        let mut entries = HashMap::new();
        for _ in 0..entry_count {
            if request.interrupted() {
                return Err(BuildError::Cancelled);
            }
            let position = file.stream_position().map_err(BuildError::Io)?;
            if position.checked_add(46).is_none_or(|end| end > central_end) {
                return Err(invalid_office_archive("truncated central directory entry"));
            }
            let mut header = [0; 46];
            file.read_exact(&mut header).map_err(BuildError::Io)?;
            if &header[..4] != b"PK\x01\x02" {
                return Err(invalid_office_archive("invalid central directory entry"));
            }
            let name_len = le_u16(&header[28..30]) as usize;
            let extra_len = le_u16(&header[30..32]) as usize;
            let comment_len = le_u16(&header[32..34]) as usize;
            let variable_len = name_len
                .checked_add(extra_len)
                .and_then(|length| length.checked_add(comment_len))
                .ok_or_else(|| invalid_office_archive("invalid entry length"))?;
            let next = file
                .stream_position()
                .map_err(BuildError::Io)?
                .checked_add(variable_len as u64)
                .filter(|end| *end <= central_end)
                .ok_or_else(|| invalid_office_archive("truncated entry name"))?;
            if name_len > MAX_OFFICE_ENTRY_NAME_BYTES {
                file.seek(SeekFrom::Start(next)).map_err(BuildError::Io)?;
                continue;
            }
            let mut name = vec![0; name_len];
            file.read_exact(&mut name).map_err(BuildError::Io)?;
            file.seek(SeekFrom::Start(next)).map_err(BuildError::Io)?;
            let Ok(name) = String::from_utf8(name) else {
                continue;
            };
            entries.insert(
                name,
                OfficeZipEntry {
                    compression: le_u16(&header[10..12]),
                    flags: le_u16(&header[8..10]),
                    compressed_size: le_u32(&header[20..24]) as u64,
                    uncompressed_size: le_u32(&header[24..28]) as u64,
                    local_header_offset: le_u32(&header[42..46]) as u64,
                },
            );
        }
        Ok(Some(Self {
            path: request.path.clone(),
            file_len,
            entries,
        }))
    }

    fn text(
        &self,
        request: &PreviewRequest,
        name: &str,
    ) -> Result<Option<ExtractedOfficeText>, BuildError> {
        let Some(entry) = self.entries.get(name) else {
            return Ok(None);
        };
        let mut file = fs::File::open(&self.path).map_err(BuildError::Io)?;
        if entry.flags & 1 != 0 {
            return Err(invalid_office_archive(
                "encrypted Office documents are not previewed",
            ));
        }
        if entry.compressed_size > request.max_bytes as u64 {
            return Err(invalid_office_archive(
                "document content exceeds the preview limit",
            ));
        }
        let header_end = entry
            .local_header_offset
            .checked_add(30)
            .filter(|end| *end <= self.file_len)
            .ok_or_else(|| invalid_office_archive("invalid local entry header"))?;
        file.seek(SeekFrom::Start(entry.local_header_offset))
            .map_err(BuildError::Io)?;
        let mut header = [0; 30];
        file.read_exact(&mut header).map_err(BuildError::Io)?;
        if &header[..4] != b"PK\x03\x04" {
            return Err(invalid_office_archive("invalid local entry header"));
        }
        let name_len = le_u16(&header[26..28]) as u64;
        let extra_len = le_u16(&header[28..30]) as u64;
        let data_offset = header_end
            .checked_add(name_len)
            .and_then(|offset| offset.checked_add(extra_len))
            .ok_or_else(|| invalid_office_archive("invalid local entry length"))?;
        let data_end = data_offset
            .checked_add(entry.compressed_size)
            .filter(|end| *end <= self.file_len)
            .ok_or_else(|| invalid_office_archive("truncated document content"))?;
        file.seek(SeekFrom::Start(data_offset))
            .map_err(BuildError::Io)?;
        let reader = file.take(data_end - data_offset);
        let mut reader: Box<dyn Read> = match entry.compression {
            0 => Box::new(reader),
            8 => Box::new(DeflateDecoder::new(reader)),
            _ => {
                return Err(invalid_office_archive(
                    "unsupported Office compression method",
                ))
            }
        };
        let mut bytes = Vec::with_capacity(request.max_bytes.min(READ_CHUNK_BYTES));
        let mut chunk = [0; READ_CHUNK_BYTES];
        let limit = request.max_bytes.saturating_add(1);
        while bytes.len() < limit {
            if request.interrupted() {
                return Err(BuildError::Cancelled);
            }
            let remaining = limit - bytes.len();
            let read = reader
                .read(&mut chunk[..remaining.min(READ_CHUNK_BYTES)])
                .map_err(BuildError::Io)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        let truncated =
            bytes.len() > request.max_bytes || entry.uncompressed_size > request.max_bytes as u64;
        bytes.truncate(request.max_bytes);
        Ok(Some(ExtractedOfficeText {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            truncated,
        }))
    }
}

fn preview_word_document(
    request: &PreviewRequest,
    package: &OfficePackage,
) -> Result<Preview, BuildError> {
    let document = package
        .text(request, "word/document.xml")?
        .ok_or_else(|| invalid_office_archive("missing Word document body"))?;
    let title = package
        .text(request, "docProps/core.xml")?
        .and_then(|core| xml_tag_text(&core.text, "dc:title"));
    let (lines, line_truncated) =
        text_preview_lines(&office_xml_text(&document.text), request.max_lines);
    let mut summary = Vec::new();
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        summary.push(format!("Title: {title}"));
    }
    summary.push("Text extracted locally; macros and embedded objects are not run.".to_string());
    Ok(Preview::Office {
        kind: "Word document",
        summary,
        lines,
        truncated: document.truncated || line_truncated,
    })
}

fn preview_excel_workbook(
    request: &PreviewRequest,
    package: &OfficePackage,
) -> Result<Preview, BuildError> {
    let workbook = package
        .text(request, "xl/workbook.xml")?
        .ok_or_else(|| invalid_office_archive("missing workbook metadata"))?;
    let sheets = workbook_sheet_names(&workbook.text);
    let sheet_path = first_worksheet_path(
        &workbook.text,
        package
            .text(request, "xl/_rels/workbook.xml.rels")?
            .as_ref()
            .map(|rels| rels.text.as_str()),
    );
    let shared_strings = package
        .text(request, "xl/sharedStrings.xml")?
        .map(|strings| shared_string_table(&strings.text))
        .unwrap_or_default();
    let mut summary = vec![format!(
        "{} sheet{}",
        sheets.len(),
        if sheets.len() == 1 { "" } else { "s" }
    )];
    if !sheets.is_empty() {
        summary.push(format!("Sheets: {}", sheets.join(", ")));
    }
    summary.push("Cell values extracted locally; formulas are not calculated.".to_string());
    let Some(sheet_path) = sheet_path else {
        return Ok(Preview::Office {
            kind: "Excel workbook",
            summary,
            lines: vec!["No readable worksheet was found.".to_string()],
            truncated: workbook.truncated,
        });
    };
    let worksheet = package
        .text(request, &sheet_path)?
        .ok_or_else(|| invalid_office_archive("workbook references a missing worksheet"))?;
    let (lines, line_truncated) =
        worksheet_preview_lines(&worksheet.text, &shared_strings, request.max_lines);
    Ok(Preview::Office {
        kind: "Excel workbook",
        summary,
        lines,
        truncated: workbook.truncated || worksheet.truncated || line_truncated,
    })
}

fn find_zip_end_record(bytes: &[u8]) -> Option<usize> {
    for index in (0..=bytes.len().saturating_sub(22)).rev() {
        if &bytes[index..index + 4] == b"PK\x05\x06" {
            let comment_len = le_u16(&bytes[index + 20..index + 22]) as usize;
            if index + 22 + comment_len == bytes.len() {
                return Some(index);
            }
        }
    }
    None
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("fixed ZIP field length"))
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("fixed ZIP field length"))
}

fn invalid_office_archive(message: &str) -> BuildError {
    BuildError::Io(io::Error::new(io::ErrorKind::InvalidData, message))
}

fn xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    let content_start = xml[start..].find('>')? + start + 1;
    let end = xml[content_start..].find(&format!("</{tag}>"))? + content_start;
    Some(office_xml_text(&xml[content_start..end]).trim().to_string())
}

fn xml_attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=");
    let start = tag.find(&marker)? + marker.len();
    let quote = tag.as_bytes().get(start).copied()?;
    if quote != b'\'' && quote != b'\"' {
        return None;
    }
    let end = tag[start + 1..].find(quote as char)? + start + 1;
    Some(decode_xml_entities(&tag[start + 1..end]))
}

fn office_xml_text(xml: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(relative_start) = xml[cursor..].find('<') {
        let start = cursor + relative_start;
        output.push_str(&decode_xml_entities(&xml[cursor..start]));
        let Some(relative_end) = xml[start..].find('>') else {
            break;
        };
        let tag = xml[start + 1..start + relative_end].trim();
        if tag.starts_with("w:tab") {
            output.push(' ');
        } else if tag.starts_with("w:br")
            || tag.starts_with("w:cr")
            || tag.starts_with("/w:p")
            || tag.starts_with("/w:tr")
        {
            output.push('\n');
        }
        cursor = start + relative_end + 1;
    }
    output.push_str(&decode_xml_entities(&xml[cursor..]));
    output
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn text_preview_lines(text: &str, max_lines: usize) -> (Vec<String>, bool) {
    let mut lines = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .map(|line| line.chars().take(MAX_RENDERED_LINE_CHARS).collect())
        .take(max_lines + 1)
        .collect::<Vec<String>>();
    let truncated = lines.len() > max_lines;
    lines.truncate(max_lines);
    (lines, truncated)
}

fn workbook_sheet_names(workbook: &str) -> Vec<String> {
    xml_opening_tags(workbook, "sheet")
        .into_iter()
        .filter_map(|tag| xml_attribute(tag, "name"))
        .take(20)
        .collect()
}

fn first_worksheet_path(workbook: &str, relationships: Option<&str>) -> Option<String> {
    let sheet = xml_opening_tags(workbook, "sheet").into_iter().next()?;
    let relationship_id = xml_attribute(sheet, "r:id")?;
    let relationships = relationships?;
    let target = xml_opening_tags(relationships, "Relationship")
        .into_iter()
        .find(|tag| xml_attribute(tag, "Id").as_deref() == Some(relationship_id.as_str()))
        .and_then(|tag| xml_attribute(tag, "Target"))?;
    let path = target.replace('\\', "/");
    if path.starts_with('/') || path.split('/').any(|part| part == "..") {
        return None;
    }
    Some(if path.starts_with("xl/") {
        path
    } else {
        format!("xl/{path}")
    })
}

fn xml_opening_tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let marker = format!("<{name} ");
    xml.match_indices(&marker)
        .filter_map(|(start, _)| {
            xml[start..]
                .find('>')
                .map(|end| &xml[start + 1..start + end])
        })
        .collect()
}

fn shared_string_table(xml: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = xml[cursor..].find("<si") {
        let start = cursor + relative_start;
        let Some(content_start) = xml[start..].find('>') else {
            break;
        };
        let content_start = start + content_start + 1;
        let Some(relative_end) = xml[content_start..].find("</si>") else {
            break;
        };
        let end = content_start + relative_end;
        values.push(office_xml_text(&xml[content_start..end]).trim().to_string());
        cursor = end + 5;
    }
    values
}

fn worksheet_preview_lines(
    xml: &str,
    shared_strings: &[String],
    max_lines: usize,
) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = xml[cursor..].find("<row") {
        let start = cursor + relative_start;
        let Some(open_end) = xml[start..].find('>') else {
            break;
        };
        let content_start = start + open_end + 1;
        let Some(relative_end) = xml[content_start..].find("</row>") else {
            break;
        };
        let end = content_start + relative_end;
        let row_tag = &xml[start + 1..start + open_end];
        let row_number =
            xml_attribute(row_tag, "r").unwrap_or_else(|| (lines.len() + 1).to_string());
        let values = worksheet_row_values(&xml[content_start..end], shared_strings);
        if !values.is_empty() {
            lines.push(format!("{row_number}: {}", values.join(" | ")));
        }
        if lines.len() > max_lines {
            break;
        }
        cursor = end + 6;
    }
    let truncated = lines.len() > max_lines;
    lines.truncate(max_lines);
    (lines, truncated)
}

fn worksheet_row_values(row: &str, shared_strings: &[String]) -> Vec<String> {
    let mut values = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = row[cursor..].find("<c ") {
        let start = cursor + relative_start;
        let Some(open_end) = row[start..].find('>') else {
            break;
        };
        let content_start = start + open_end + 1;
        let Some(relative_end) = row[content_start..].find("</c>") else {
            break;
        };
        let end = content_start + relative_end;
        let cell_tag = &row[start + 1..start + open_end];
        let cell_type = xml_attribute(cell_tag, "t").unwrap_or_default();
        let value = xml_tag_text(&row[content_start..end], "v")
            .or_else(|| xml_tag_text(&row[content_start..end], "t"))
            .unwrap_or_default();
        let value = if cell_type == "s" {
            value
                .parse::<usize>()
                .ok()
                .and_then(|index| shared_strings.get(index))
                .cloned()
                .unwrap_or(value)
        } else {
            value
        };
        if !value.is_empty() {
            let reference = xml_attribute(cell_tag, "r").unwrap_or_default();
            values.push(format!(
                "{reference}={}",
                value.chars().take(60).collect::<String>()
            ));
        }
        if values.len() >= 8 {
            break;
        }
        cursor = end + 4;
    }
    values
}

enum BuildError {
    Cancelled,
    Io(io::Error),
}

fn read_bounded(request: &PreviewRequest) -> Result<Vec<u8>, BuildError> {
    let mut file = fs::File::open(&request.path).map_err(BuildError::Io)?;
    let target = request.max_bytes.saturating_add(1);
    let mut bytes = Vec::with_capacity(target.min(READ_CHUNK_BYTES));
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    while bytes.len() < target {
        if request.interrupted() {
            return Err(BuildError::Cancelled);
        }
        let remaining = target - bytes.len();
        let read = file
            .read(&mut chunk[..remaining.min(READ_CHUNK_BYTES)])
            .map_err(BuildError::Io)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return false;
    }
    bytes.iter().take(8_192).any(|byte| *byte == 0) || std::str::from_utf8(bytes).is_err()
}

fn hex_header(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(32)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn binary_kind(path: &Path, bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        "PNG image"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "JPEG image"
    } else if bytes.starts_with(b"GIF8") {
        "GIF image"
    } else if bytes.starts_with(b"%PDF") {
        "PDF document"
    } else if bytes.starts_with(b"PK\x03\x04") {
        "ZIP archive"
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        "Windows executable"
    } else {
        "binary file"
    }
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") && bytes.len() >= 24 {
        return Some((
            u32::from_be_bytes(bytes[16..20].try_into().ok()?),
            u32::from_be_bytes(bytes[20..24].try_into().ok()?),
        ));
    }
    if bytes.starts_with(b"GIF8") && bytes.len() >= 10 {
        return Some((
            u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32,
            u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32,
        ));
    }
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut offset = 2usize;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        if matches!(marker, 0xD8 | 0xD9) || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if offset + 2 > bytes.len() {
            break;
        }
        let length = u16::from_be_bytes(bytes[offset..offset + 2].try_into().ok()?) as usize;
        if length < 2 || offset + length > bytes.len() {
            break;
        }
        if matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        ) && length >= 7
        {
            let height = u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().ok()?) as u32;
            return Some((width, height));
        }
        offset += length;
    }
    None
}

fn structured_kind(path: &Path, text: &str) -> Option<&'static str> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" if serde_json::from_str::<serde_json::Value>(text).is_ok() => Some("JSON"),
        "toml" if toml::from_str::<toml::Value>(text).is_ok() => Some("TOML"),
        "yaml" | "yml" => Some("YAML"),
        "md" | "markdown" => Some("Markdown source"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    use flate2::{write::DeflateEncoder, Compression};

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "caret-preview-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn request(path: PathBuf) -> PreviewRequest {
        PreviewRequest::new(
            path,
            1024,
            20,
            Duration::from_secs(1),
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn write_deflated_zip(path: &Path, files: &[(&str, &str)]) {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for (name, contents) in files {
            let offset = bytes.len() as u32;
            let name = name.as_bytes();
            let contents = contents.as_bytes();
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(contents).unwrap();
            let compressed = encoder.finish().unwrap();
            bytes.extend_from_slice(b"PK\x03\x04");
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&8u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&compressed);

            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&8u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name);
        }
        let central_offset = bytes.len() as u32;
        bytes.extend_from_slice(&central);
        bytes.extend_from_slice(b"PK\x05\x06");
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(central.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn providers_distinguish_source_binary_and_directories() {
        let root = temp_dir("providers");
        let json = root.join("data.json");
        fs::write(&json, "{\"ok\":true}\n").unwrap();
        assert!(matches!(
            generate(&request(json)),
            Preview::Text {
                structured: Some("JSON"),
                language: Language::Json,
                ..
            }
        ));

        let binary = root.join("image.png");
        let mut png = b"\x89PNG\r\n\x1A\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        png.push(0);
        fs::write(&binary, png).unwrap();
        assert!(matches!(
            generate(&request(binary)),
            Preview::Binary {
                kind: "PNG image",
                dimensions: Some((640, 480)),
                ..
            }
        ));
        assert!(matches!(
            generate(&request(root.clone())),
            Preview::Directory { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_is_observed_before_io() {
        let root = temp_dir("cancel");
        let path = root.join("data.txt");
        fs::write(&path, "data").unwrap();
        let cancelled = Arc::new(AtomicBool::new(true));
        let request = PreviewRequest::new(path, 1024, 20, Duration::from_secs(1), cancelled);
        assert_eq!(generate(&request), Preview::Cancelled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expired_request_is_cancelled() {
        let root = temp_dir("deadline");
        let path = root.join("data.txt");
        fs::write(&path, "data").unwrap();
        let request = PreviewRequest::new(
            path,
            1024,
            20,
            Duration::ZERO,
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(generate(&request), Preview::Cancelled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn office_documents_preview_text_and_cells_without_running_them() {
        let root = temp_dir("office-preview");
        let docx = root.join("brief.docx");
        write_deflated_zip(
            &docx,
            &[
                ("[Content_Types].xml", "<Types/>"),
                ("docProps/core.xml", "<dc:title>Launch plan</dc:title>"),
                (
                    "word/document.xml",
                    "<w:document><w:body><w:p><w:r><w:t>Safe preview</w:t></w:r></w:p></w:body></w:document>",
                ),
            ],
        );
        assert!(matches!(
            generate(&request(docx.clone())),
            Preview::Office { kind: "Word document", lines, .. } if lines == ["Safe preview"]
        ));
        assert!(office_text_content(&docx)
            .unwrap()
            .is_some_and(|text| text.contains("Safe preview")));

        let xlsx = root.join("plan.xlsx");
        write_deflated_zip(
            &xlsx,
            &[
                ("[Content_Types].xml", "<Types/>"),
                (
                    "xl/workbook.xml",
                    "<workbook><sheets><sheet name=\"Plan\" r:id=\"rId1\"/></sheets></workbook>",
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
                ),
                ("xl/sharedStrings.xml", "<sst><si><t>Ship it</t></si></sst>"),
                (
                    "xl/worksheets/sheet1.xml",
                    "<worksheet><sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\"><v>42</v></c></row></sheetData></worksheet>",
                ),
            ],
        );
        assert!(matches!(
            generate(&request(xlsx.clone())),
            Preview::Office { kind: "Excel workbook", lines, .. } if lines == ["1: A1=Ship it | B1=42"]
        ));
        assert!(office_text_content(&xlsx)
            .unwrap()
            .is_some_and(|text| text.contains("A1=Ship it")));
        let _ = fs::remove_dir_all(root);
    }
}
