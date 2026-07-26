use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::syntax::Language;

const MAX_DIRECTORY_ENTRIES: usize = 50_000;
const READ_CHUNK_BYTES: usize = 16 * 1024;
const MAX_RENDERED_LINE_CHARS: usize = 240;

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
}
