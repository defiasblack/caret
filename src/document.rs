//! Durable document I/O.  Keeping this separate from editing makes the
//! destructive boundary small, testable, and usable by recovery tooling.
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
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

/// What happens to the last line's ending when a document is saved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinalNewline {
    /// Keep whatever the buffer already has (default).
    #[default]
    Preserve,
    /// Append a final newline when the buffer does not end with one.
    Always,
    /// Remove the final newline when the buffer ends with one.
    Strip,
}

impl FinalNewline {
    pub fn name(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Always => "always",
            Self::Strip => "strip",
        }
    }
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
        // The platform boundary handles replacement semantics. A failure
        // deliberately leaves the original intact.
        crate::platform::replace_file(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn recovery_dir() -> PathBuf {
    crate::platform::app_data_dir().join("recovery")
}

/// Return a process-local content fingerprint used alongside timestamps and
/// lengths. Network filesystems can delay or coarsen timestamp updates, so
/// metadata alone is not sufficient for conflict detection.
pub fn fingerprint(path: &Path) -> io::Result<u64> {
    let bytes = fs::read(path)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
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
    fn read_preserves_lf_and_missing_final_newline() {
        let path = temp("lf-format");
        fs::write(&path, b"one\ntwo").unwrap();
        let (text, format) = read_text(&path).unwrap();
        assert_eq!(text, "one\ntwo");
        assert_eq!(format.line_ending, LineEnding::Lf);
        assert!(!format.final_newline);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unsupported_utf8_is_rejected_with_a_useful_error() {
        let path = temp("encoding");
        fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
        let error = read_text(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("unsupported encoding"));
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

    #[test]
    fn write_failure_leaves_the_original_file_untouched() {
        let root =
            std::env::temp_dir().join(format!("caret-document-failure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let parent_file = root.join("not-a-directory");
        fs::write(&parent_file, b"parent").unwrap();
        let destination = parent_file.join("important.txt");
        let error = atomic_write(&destination, b"replacement").unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::NotADirectory | io::ErrorKind::AlreadyExists
        ));
        assert_eq!(fs::read(&parent_file).unwrap(), b"parent");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fingerprint_changes_when_same_length_content_changes() {
        let path = temp("fingerprint");
        fs::write(&path, b"one").unwrap();
        let first = fingerprint(&path).unwrap();
        fs::write(&path, b"two").unwrap();
        assert_ne!(first, fingerprint(&path).unwrap());
        let _ = fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_unix_file_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let path = temp("permissions");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o640);
        let _ = fs::remove_file(path);
    }

    #[cfg(windows)]
    #[allow(clippy::permissions_set_readonly_false)]
    #[test]
    fn atomic_write_reports_read_only_destinations() {
        let path = temp("read-only");
        fs::write(&path, b"old").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        let result = atomic_write(&path, b"new");
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
        let _ = fs::remove_file(path);
    }
}
