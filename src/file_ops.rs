#[cfg(unix)]
use std::ffi::CString;
use std::{
    fs,
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub fn create_file(path: &Path) -> io::Result<()> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.sync_all()
}

pub fn create_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

pub fn rename_without_replace(source: &Path, destination: &Path) -> io::Result<()> {
    rename_without_replace_platform(source, destination)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_without_replace_platform(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "source path contains a NUL byte"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "destination path contains a NUL byte",
        )
    })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE as libc::c_uint,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_without_replace_platform(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "source path contains a NUL byte"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            "destination path contains a NUL byte",
        )
    })?;
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_without_replace_platform(source: &Path, destination: &Path) -> io::Result<()> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    let source = wide(source.as_os_str());
    let destination = wide(destination.as_os_str());
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
)))]
fn rename_without_replace_platform(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            format!("destination already exists: {}", destination.display()),
        ));
    }
    fs::rename(source, destination)
}

pub fn copy_file_without_replace(source: &Path, destination: &Path) -> io::Result<u64> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = (|| {
        let copied = io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        if let Ok(metadata) = input.metadata() {
            fs::set_permissions(destination, metadata.permissions())?;
            output.sync_all()?;
        }
        Ok(copied)
    })();
    if result.is_err() {
        drop(output);
        let _ = fs::remove_file(destination);
    }
    result
}

pub fn delete_permanently(path: &Path) -> io::Result<()> {
    remove_path(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    Ask,
    Overwrite,
    Skip,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Move,
    Duplicate,
    Trash,
    Delete,
}

#[derive(Debug, Clone)]
pub struct OperationRequest {
    pub id: u64,
    pub kind: OperationKind,
    pub sources: Vec<PathBuf>,
    pub destination: Option<PathBuf>,
    pub conflict: ConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationFailure {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationSummary {
    pub id: u64,
    pub kind: OperationKind,
    pub completed: usize,
    pub skipped: usize,
    pub cancelled: bool,
    pub failures: Vec<OperationFailure>,
    /// Successful source-to-destination changes used to synchronize editor state.
    pub path_changes: Vec<(PathBuf, PathBuf)>,
}

impl OperationSummary {
    fn new(request: &OperationRequest) -> Self {
        Self {
            id: request.id,
            kind: request.kind,
            completed: 0,
            skipped: 0,
            cancelled: false,
            failures: Vec::new(),
            path_changes: Vec::new(),
        }
    }
}

/// Runs a filesystem operation outside the UI thread.
///
/// The callback is invoked after each source finishes. Cancellation is checked
/// before each source and while recursively copying directories. Completed and
/// failed items remain distinguishable when a multi-item operation is partial.
pub fn execute(
    request: &OperationRequest,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &Path),
) -> OperationSummary {
    let mut summary = OperationSummary::new(request);
    let total = request.sources.len();

    for (index, source) in request.sources.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }

        let result = match request.kind {
            OperationKind::Copy | OperationKind::Move => {
                let Some(directory) = request.destination.as_deref() else {
                    summary.failures.push(OperationFailure {
                        path: source.clone(),
                        message: "operation has no destination".to_string(),
                    });
                    progress(index + 1, total, source);
                    continue;
                };
                transfer_one(source, directory, request.kind, request.conflict, cancelled)
            }
            OperationKind::Duplicate => duplicate_one(
                source,
                request.destination.as_deref(),
                request.conflict,
                cancelled,
            ),
            OperationKind::Trash => move_to_trash(source).map(|()| None),
            OperationKind::Delete => remove_path(source).map(|()| None),
        };

        match result {
            Ok(Some(destination)) => {
                summary.completed += 1;
                if request.kind == OperationKind::Move {
                    summary.path_changes.push((source.clone(), destination));
                }
            }
            Ok(None) => summary.completed += 1,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => summary.skipped += 1,
            Err(error) if error.kind() == ErrorKind::Interrupted => {
                summary.cancelled = true;
                break;
            }
            Err(error) => summary.failures.push(OperationFailure {
                path: source.clone(),
                message: error.to_string(),
            }),
        }
        progress(index + 1, total, source);
    }

    summary
}

fn transfer_one(
    source: &Path,
    destination_directory: &Path,
    kind: OperationKind,
    conflict: ConflictPolicy,
    cancelled: &AtomicBool,
) -> io::Result<Option<PathBuf>> {
    let name = source.file_name().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} has no file name", source.display()),
        )
    })?;
    let requested_destination = destination_directory.join(name);
    if absolute_clean(source) == absolute_clean(&requested_destination) {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "source and destination are the same path",
        ));
    }
    let overwrite = conflict == ConflictPolicy::Overwrite && requested_destination.exists();
    let destination = resolve_conflict(&requested_destination, conflict)?;
    let Some(destination) = destination else {
        return Err(io::Error::new(ErrorKind::AlreadyExists, "skipped conflict"));
    };
    reject_descendant_transfer(source, &destination)?;

    if kind == OperationKind::Move {
        if overwrite {
            copy_path_safely(source, &destination, true, cancelled)?;
            remove_path(source)?;
            return Ok(Some(destination));
        }
        match fs::rename(source, &destination) {
            Ok(()) => return Ok(Some(destination)),
            Err(error) if is_cross_device(&error) => {}
            Err(error) => return Err(error),
        }
    }

    copy_path_safely(source, &destination, overwrite, cancelled)?;
    if kind == OperationKind::Move {
        if let Err(error) = remove_path(source) {
            let _ = remove_path(&destination);
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "copied to {}, but could not remove source: {error}",
                    destination.display()
                ),
            ));
        }
    }
    Ok(Some(destination))
}

fn duplicate_one(
    source: &Path,
    destination: Option<&Path>,
    conflict: ConflictPolicy,
    cancelled: &AtomicBool,
) -> io::Result<Option<PathBuf>> {
    let candidate = destination.map(Path::to_path_buf).unwrap_or_else(|| {
        let parent = source.parent().unwrap_or_else(|| Path::new("."));
        let stem = source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("copy");
        let extension = source.extension().and_then(|value| value.to_str());
        let name = extension.map_or_else(
            || format!("{stem} copy"),
            |extension| format!("{stem} copy.{extension}"),
        );
        parent.join(name)
    });
    let destination = resolve_conflict(&candidate, conflict)?;
    let Some(destination) = destination else {
        return Err(io::Error::new(ErrorKind::AlreadyExists, "skipped conflict"));
    };
    reject_descendant_transfer(source, &destination)?;
    let overwrite = conflict == ConflictPolicy::Overwrite && destination.exists();
    copy_path_safely(source, &destination, overwrite, cancelled)?;
    Ok(Some(destination))
}

fn resolve_conflict(path: &Path, policy: ConflictPolicy) -> io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(Some(path.to_path_buf()));
    }

    match policy {
        ConflictPolicy::Ask => Err(io::Error::new(
            ErrorKind::AlreadyExists,
            format!("{} already exists", path.display()),
        )),
        ConflictPolicy::Skip => Ok(None),
        ConflictPolicy::Overwrite => Ok(Some(path.to_path_buf())),
        ConflictPolicy::Rename => {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("copy");
            let extension = path.extension().and_then(|value| value.to_str());
            for number in 2..100_000 {
                let name = extension.map_or_else(
                    || format!("{stem} ({number})"),
                    |extension| format!("{stem} ({number}).{extension}"),
                );
                let candidate = parent.join(name);
                if !candidate.exists() {
                    return Ok(Some(candidate));
                }
            }
            Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "could not find an unused conflict name",
            ))
        }
    }
}

fn copy_path_safely(
    source: &Path,
    destination: &Path,
    overwrite: bool,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    if !overwrite {
        return copy_path(source, destination, cancelled);
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let temporary = unique_temporary_path(parent);
    if let Err(error) = copy_path(source, &temporary, cancelled) {
        let _ = remove_path(&temporary);
        return Err(error);
    }
    if let Err(error) = remove_path(destination) {
        let _ = remove_path(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        // The completed replacement remains recoverable beside the intended
        // destination if the final rename fails.
        return Err(io::Error::new(
            error.kind(),
            format!(
                "replacement is complete at {}, but could not be installed: {error}",
                temporary.display()
            ),
        ));
    }
    Ok(())
}

fn unique_temporary_path(parent: &Path) -> PathBuf {
    for number in 0..100_000u32 {
        let candidate = parent.join(format!(".caret-copy-{}-{number}.tmp", std::process::id()));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!(".caret-copy-{}.tmp", std::process::id()))
}

fn copy_path(source: &Path, destination: &Path, cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            ErrorKind::Interrupted,
            "operation cancelled",
        ));
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return copy_symlink(source, destination);
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        fs::set_permissions(destination, metadata.permissions())?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_path(
                &entry.path(),
                &destination.join(entry.file_name()),
                cancelled,
            )?;
        }
        return Ok(());
    }
    fs::copy(source, destination)?;
    fs::set_permissions(destination, metadata.permissions())?;
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, destination)
}

#[cfg(windows)]
fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    let target = fs::read_link(source)?;
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}

fn reject_descendant_transfer(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Ok(());
    }
    let source = absolute_clean(source);
    let destination = absolute_clean(destination);
    if destination.starts_with(&source) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "cannot copy or move a directory into itself",
        ));
    }
    Ok(())
}

fn absolute_clean(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    if let Ok(canonical) = fs::canonicalize(&absolute) {
        return canonical;
    }
    if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
        if let Ok(parent) = fs::canonicalize(parent) {
            return parent.join(name);
        }
    }
    absolute
}

fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(windows)]
fn move_to_trash(path: &Path) -> io::Result<()> {
    use std::process::Command;

    // The path is a separate argument read through `$args`, so a file name can
    // never be interpreted as PowerShell source.
    let script = if path.is_dir() {
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory(\
         $args[0], 'OnlyErrorDialogs', 'SendToRecycleBin')"
    } else {
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile(\
         $args[0], 'OnlyErrorDialogs', 'SendToRecycleBin')"
    };
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .arg(path)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(io::Error::other(if message.is_empty() {
            "Windows recycle-bin operation failed".to_string()
        } else {
            message
        }))
    }
}

#[cfg(target_os = "macos")]
fn move_to_trash(path: &Path) -> io::Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "HOME is not set"))?;
    let trash = home.join(".Trash");
    fs::create_dir_all(&trash)?;
    let destination = unique_trash_path(&trash, path)?;
    move_or_copy_to_trash(path, &destination)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn move_to_trash(path: &Path) -> io::Result<()> {
    use std::io::Write;

    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "HOME and XDG_DATA_HOME are not set"))?;
    let trash = data_home.join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    fs::create_dir_all(&files)?;
    fs::create_dir_all(&info)?;
    let destination = unique_trash_path(&files, path)?;
    let name = destination
        .file_name()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "path has no file name"))?;
    let info_path = info.join(format!("{}.trashinfo", name.to_string_lossy()));
    let encoded = percent_encode_path(&absolute_clean(path));
    let mut info_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&info_path)?;
    writeln!(
        info_file,
        "[Trash Info]\nPath={encoded}\nDeletionDate={}",
        trash_deletion_date()
    )?;
    if let Err(error) = move_or_copy_to_trash(path, &destination) {
        let _ = fs::remove_file(info_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn move_or_copy_to_trash(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if is_cross_device(&error) => {
            let cancelled = AtomicBool::new(false);
            copy_path(source, destination, &cancelled)?;
            if let Err(error) = remove_path(source) {
                let _ = remove_path(destination);
                return Err(error);
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn unique_trash_path(directory: &Path, source: &Path) -> io::Result<PathBuf> {
    let name = source.file_name().ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} has no file name", source.display()),
        )
    })?;
    let initial = directory.join(name);
    if !initial.exists() {
        return Ok(initial);
    }
    let name = name.to_string_lossy();
    for number in 2..100_000 {
        let candidate = directory.join(format!("{name}.{number}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        "could not find an unused trash name",
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn percent_encode_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str()
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'-' | b'_' | b'.' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn trash_deletion_date() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format_utc_timestamp(seconds)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn format_utc_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    // Howard Hinnant's civil-from-days algorithm; valid across the complete
    // non-negative Unix timestamp range without a date/time dependency.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

fn is_cross_device(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(18)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "caret-file-ops-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn recursive_copy_preserves_contents_and_rejects_descendants() {
        let root = temp_dir("copy");
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("nested/file.txt"), "value").unwrap();

        let request = OperationRequest {
            id: 1,
            kind: OperationKind::Copy,
            sources: vec![source.clone()],
            destination: Some(destination),
            conflict: ConflictPolicy::Ask,
        };
        let result = execute(&request, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(result.completed, 1);
        assert_eq!(
            fs::read_to_string(root.join("destination/source/nested/file.txt")).unwrap(),
            "value"
        );

        let request = OperationRequest {
            id: 2,
            kind: OperationKind::Copy,
            sources: vec![source.clone()],
            destination: Some(source.join("nested")),
            conflict: ConflictPolicy::Rename,
        };
        let result = execute(&request, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(result.failures.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rename_policy_finds_a_non_conflicting_name() {
        let root = temp_dir("rename");
        let source = root.join("note.txt");
        fs::write(&source, "one").unwrap();
        fs::write(root.join("note copy.txt"), "existing").unwrap();

        let request = OperationRequest {
            id: 3,
            kind: OperationKind::Duplicate,
            sources: vec![source],
            destination: None,
            conflict: ConflictPolicy::Rename,
        };
        let result = execute(&request, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(result.completed, 1);
        assert_eq!(
            fs::read_to_string(root.join("note copy (2).txt")).unwrap(),
            "one"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_keeps_remaining_items_untouched() {
        let root = temp_dir("cancel");
        let source = root.join("one.txt");
        fs::write(&source, "one").unwrap();
        let cancelled = AtomicBool::new(true);
        let request = OperationRequest {
            id: 4,
            kind: OperationKind::Delete,
            sources: vec![source.clone()],
            destination: None,
            conflict: ConflictPolicy::Ask,
        };
        let result = execute(&request, &cancelled, |_, _, _| {});
        assert!(result.cancelled);
        assert!(source.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn overwrite_stages_a_complete_replacement() {
        let root = temp_dir("overwrite");
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        let source = source_dir.join("note.txt");
        fs::write(&source, "new value").unwrap();
        fs::write(destination_dir.join("note.txt"), "old value").unwrap();
        let request = OperationRequest {
            id: 5,
            kind: OperationKind::Copy,
            sources: vec![source],
            destination: Some(destination_dir.clone()),
            conflict: ConflictPolicy::Overwrite,
        };
        let result = execute(&request, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(result.completed, 1);
        assert_eq!(
            fs::read_to_string(destination_dir.join("note.txt")).unwrap(),
            "new value"
        );
        assert!(fs::read_dir(&destination_dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".caret-copy")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn moving_onto_the_same_path_never_deletes_the_source() {
        let root = temp_dir("same-path");
        let source = root.join("note.txt");
        fs::write(&source, "keep me").unwrap();
        let request = OperationRequest {
            id: 6,
            kind: OperationKind::Move,
            sources: vec![source.clone()],
            destination: Some(root.clone()),
            conflict: ConflictPolicy::Overwrite,
        };
        let result = execute(&request, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(result.skipped, 1);
        assert_eq!(fs::read_to_string(source).unwrap(), "keep me");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn synchronous_create_and_copy_never_replace_existing_files() {
        let root = temp_dir("legacy-no-replace");
        let source = root.join("source.txt");
        let destination = root.join("destination.txt");
        fs::write(&source, "source").unwrap();
        fs::write(&destination, "important").unwrap();

        assert_eq!(
            create_file(&destination).unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(
            copy_file_without_replace(&source, &destination)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(destination).unwrap(), "important");
        let _ = fs::remove_dir_all(root);
    }
}
