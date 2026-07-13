//! Durable document I/O.  Keeping this separate from editing makes the
//! destructive boundary small, testable, and usable by recovery tooling.
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFormat {
    pub utf8_bom: bool,
    pub line_ending: LineEnding,
    pub final_newline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

pub fn read_text(path: &Path) -> io::Result<(String, FileFormat)> {
    let bytes = fs::read(path)?;
    if bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "binary file; Caret only opens text files",
        ));
    }
    let utf8_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let text = std::str::from_utf8(if utf8_bom { &bytes[3..] } else { &bytes })
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported encoding; expected UTF-8",
            )
        })?
        .to_owned();
    let line_ending = if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    let final_newline = text.ends_with('\n');
    Ok((
        text,
        FileFormat {
            utf8_bom,
            line_ending,
            final_newline,
        },
    ))
}

/// Writes a fully synchronized replacement beside `path` and only then swaps it
/// into place. The original is never opened for truncation.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_impl(path, bytes, |_| Ok(()))
}

fn atomic_write_impl<F>(path: &Path, bytes: &[u8], before_replace: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("untitled");
    let temp = parent.join(format!(
        ".{name}.caret-{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        use std::io::Write;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temp, metadata.permissions())?;
        }
        drop(file);
        before_replace(&temp)?;
        // std::fs::rename maps to an atomic replace on supported Windows and
        // Unix filesystems.  A failure deliberately leaves the original intact.
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn recovery_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "Caret", "Caret")
        .map(|dirs| dirs.data_local_dir().join("recovery"))
        .unwrap_or_else(|| PathBuf::from("caret-recovery"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("caret-document-{name}-{}", std::process::id()))
    }
    #[test]
    fn atomic_write_replaces_without_leaving_a_temp_file() {
        let path = temp("atomic");
        fs::write(&path, "old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn interrupted_save_leaves_the_original_file_untouched() {
        let path = temp("interrupted");
        fs::write(&path, "important original").unwrap();
        let error = atomic_write_impl(&path, b"replacement", |_| {
            Err(io::Error::other("simulated interruption"))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read_to_string(&path).unwrap(), "important original");
        let _ = fs::remove_file(path);
    }
    #[test]
    fn read_preserves_bom_and_crlf() {
        let path = temp("format");
        fs::write(&path, b"\xEF\xBB\xBFone\r\ntwo\r\n").unwrap();
        let (text, format) = read_text(&path).unwrap();
        assert_eq!(text, "one\r\ntwo\r\n");
        assert!(format.utf8_bom);
        assert_eq!(format.line_ending, LineEnding::Crlf);
        assert!(format.final_newline);
        let _ = fs::remove_file(path);
    }
    #[test]
    fn binary_input_is_rejected() {
        let path = temp("binary");
        fs::write(&path, b"a\0b").unwrap();
        assert_eq!(
            read_text(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let _ = fs::remove_file(path);
    }
}
