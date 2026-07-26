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

pub fn rename_safely(source: &Path, destination: &Path) -> io::Result<()> {
    if source == destination {
        return Ok(());
    }
    let same_entry = destination.exists()
        && source
            .canonicalize()
            .ok()
            .zip(destination.canonicalize().ok())
            .is_some_and(|(source, destination)| source == destination);
    if !same_entry {
        return rename_without_replace(source, destination);
    }

    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let temporary = unique_temporary_path(parent);
    rename_without_replace(source, &temporary)?;
    if let Err(error) = rename_without_replace(&temporary, destination) {
        let rollback = rename_without_replace(&temporary, source);
        return Err(io::Error::new(
            error.kind(),
            if let Err(rollback) = rollback {
                format!(
                    "case-only rename failed and rollback also failed; item remains at {}: {error}; rollback: {rollback}",
                    temporary.display()
                )
            } else {
                format!("case-only rename failed and was rolled back: {error}")
            },
        ));
    }
    Ok(())
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
    BulkRename,
    Trash,
    Delete,
    Undo,
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
    pub undo: Option<UndoRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoRecord {
    actions: Vec<UndoAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UndoAction {
    RemoveCreated {
        path: PathBuf,
        fingerprint: PathFingerprint,
    },
    MoveBack {
        from: PathBuf,
        to: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PathFingerprint {
    first: u64,
    second: u64,
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
            undo: None,
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
            OperationKind::BulkRename => Err(io::Error::new(
                ErrorKind::InvalidInput,
                "bulk rename plans must be executed with execute_bulk_rename",
            )),
            OperationKind::Trash => move_to_trash(source).map(|()| None),
            OperationKind::Delete => remove_path(source).map(|()| None),
            OperationKind::Undo => Err(io::Error::new(
                ErrorKind::InvalidInput,
                "undo records must be executed with execute_undo",
            )),
        };

        match result {
            Ok(Some(destination)) => {
                summary.completed += 1;
                match request.kind {
                    OperationKind::Move => {
                        summary
                            .path_changes
                            .push((source.clone(), destination.clone()));
                        if request.conflict != ConflictPolicy::Overwrite {
                            summary
                                .undo
                                .get_or_insert_with(|| UndoRecord {
                                    actions: Vec::new(),
                                })
                                .actions
                                .push(UndoAction::MoveBack {
                                    from: destination,
                                    to: source.clone(),
                                });
                        }
                    }
                    OperationKind::Copy | OperationKind::Duplicate
                        if request.conflict != ConflictPolicy::Overwrite =>
                    {
                        if let Ok(fingerprint) = fingerprint_path(&destination) {
                            summary
                                .undo
                                .get_or_insert_with(|| UndoRecord {
                                    actions: Vec::new(),
                                })
                                .actions
                                .push(UndoAction::RemoveCreated {
                                    path: destination,
                                    fingerprint,
                                });
                        }
                    }
                    OperationKind::Copy
                    | OperationKind::Duplicate
                    | OperationKind::BulkRename
                    | OperationKind::Trash
                    | OperationKind::Delete
                    | OperationKind::Undo => {}
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
                message: operation_error_message(error),
            }),
        }
        progress(index + 1, total, source);
    }

    summary
}

fn operation_error_message(error: io::Error) -> String {
    match error.kind() {
        ErrorKind::PermissionDenied => {
            format!("permission denied or filesystem item is locked/read-only: {error}")
        }
        ErrorKind::NotFound => format!("filesystem item became unavailable: {error}"),
        ErrorKind::StorageFull => {
            format!("copy or operation stopped because storage is full: {error}")
        }
        _ => error.to_string(),
    }
}

pub fn execute_undo(
    id: u64,
    record: &UndoRecord,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &Path),
) -> OperationSummary {
    let mut summary = OperationSummary {
        id,
        kind: OperationKind::Undo,
        completed: 0,
        skipped: 0,
        cancelled: false,
        failures: Vec::new(),
        path_changes: Vec::new(),
        undo: None,
    };
    let total = record.actions.len();
    let mut retry = vec![false; total];
    for (index, (original_index, action)) in record.actions.iter().enumerate().rev().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            summary.cancelled = true;
            retry[..=original_index].fill(true);
            break;
        }
        let (path, result) = match action {
            UndoAction::RemoveCreated { path, fingerprint } => {
                let result = fingerprint_path(path).and_then(|current| {
                    if current != *fingerprint {
                        return Err(io::Error::other(
                            "created item changed after the operation; refusing to remove it",
                        ));
                    }
                    remove_path(path)
                });
                (path.as_path(), result)
            }
            UndoAction::MoveBack { from, to } => {
                let result = move_back_without_replace(from, to).map(|()| {
                    summary.path_changes.push((from.clone(), to.clone()));
                });
                (from.as_path(), result)
            }
        };
        match result {
            Ok(()) => summary.completed += 1,
            Err(error) => {
                summary.failures.push(OperationFailure {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                });
                retry[original_index] = true;
            }
        }
        progress(index + 1, total, path);
    }
    if retry.iter().any(|retry| *retry) {
        summary.undo = Some(UndoRecord {
            actions: record
                .actions
                .iter()
                .zip(retry)
                .filter(|(_, retry)| *retry)
                .map(|(action, _)| action.clone())
                .collect(),
        });
    }
    summary
}

pub fn execute_bulk_rename(
    id: u64,
    pairs: &[(PathBuf, PathBuf)],
    cancelled: &AtomicBool,
    mut progress: impl FnMut(usize, usize, &Path),
) -> OperationSummary {
    let mut summary = OperationSummary {
        id,
        kind: OperationKind::BulkRename,
        completed: 0,
        skipped: 0,
        cancelled: false,
        failures: Vec::new(),
        path_changes: Vec::new(),
        undo: None,
    };
    if cancelled.load(Ordering::Relaxed) {
        summary.cancelled = true;
        return summary;
    }
    let pairs = pairs
        .iter()
        .filter(|(source, destination)| source != destination)
        .cloned()
        .collect::<Vec<_>>();
    if let Err(error) = validate_bulk_rename(&pairs) {
        summary.failures.push(OperationFailure {
            path: error.0,
            message: error.1,
        });
        return summary;
    }

    let mut staged = Vec::with_capacity(pairs.len());
    for (source, destination) in &pairs {
        let parent = source.parent().unwrap_or_else(|| Path::new("."));
        let temporary = unique_temporary_path(parent);
        if let Err(error) = rename_without_replace(source, &temporary) {
            let rollback = rollback_staged_renames(&staged);
            summary.failures.push(OperationFailure {
                path: source.clone(),
                message: match rollback {
                    Ok(()) => format!("could not stage rename: {error}; staging was rolled back"),
                    Err(rollback) => format!(
                        "could not stage rename: {error}; rollback was incomplete: {rollback}"
                    ),
                },
            });
            return summary;
        }
        staged.push((source.clone(), temporary, destination.clone()));
    }

    for index in 0..staged.len() {
        let (source, temporary, destination) = &staged[index];
        if let Err(error) = rename_without_replace(temporary, destination) {
            let rollback = rollback_bulk_rename(&staged, index);
            summary.failures.push(OperationFailure {
                path: source.clone(),
                message: match rollback {
                    Ok(()) => {
                        format!("could not install bulk rename: {error}; changes were rolled back")
                    }
                    Err(rollback) => format!(
                        "could not install bulk rename: {error}; rollback was incomplete: {rollback}"
                    ),
                },
            });
            return summary;
        }
        progress(index + 1, staged.len(), source);
    }

    summary.completed = staged.len();
    summary.path_changes = staged
        .iter()
        .map(|(source, _, destination)| (source.clone(), destination.clone()))
        .collect();
    summary.undo = (!staged.is_empty()).then(|| UndoRecord {
        actions: staged
            .into_iter()
            .map(|(source, _, destination)| UndoAction::MoveBack {
                from: destination,
                to: source,
            })
            .collect(),
    });
    summary
}

fn validate_bulk_rename(pairs: &[(PathBuf, PathBuf)]) -> Result<(), (PathBuf, String)> {
    let sources = pairs
        .iter()
        .map(|(source, _)| collision_key(source))
        .collect::<std::collections::HashSet<_>>();
    let mut destinations = std::collections::HashSet::new();
    for (source, destination) in pairs {
        if !source.exists() {
            return Err((
                source.clone(),
                "bulk rename source no longer exists".to_string(),
            ));
        }
        if source.parent() != destination.parent() || destination.file_name().is_none() {
            return Err((
                source.clone(),
                "bulk rename destinations must stay in the source directory".to_string(),
            ));
        }
        let destination_key = collision_key(destination);
        if !destinations.insert(destination_key.clone()) {
            return Err((
                destination.clone(),
                "bulk rename produced duplicate destination names".to_string(),
            ));
        }
        if sources.contains(&destination_key) && collision_key(source) != destination_key {
            return Err((
                destination.clone(),
                "bulk rename cannot swap or cycle existing names".to_string(),
            ));
        }
        let same_entry = destination.exists()
            && source
                .canonicalize()
                .ok()
                .zip(destination.canonicalize().ok())
                .is_some_and(|(source, destination)| source == destination);
        if destination.exists() && !same_entry {
            return Err((
                destination.clone(),
                "bulk rename destination already exists".to_string(),
            ));
        }
    }
    Ok(())
}

fn collision_key(path: &Path) -> String {
    let value = absolute_clean(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

fn rollback_staged_renames(staged: &[(PathBuf, PathBuf, PathBuf)]) -> Result<(), String> {
    let mut failures = Vec::new();
    for (source, temporary, _) in staged.iter().rev() {
        if let Err(error) = rename_without_replace(temporary, source) {
            failures.push(format!(
                "{} -> {}: {error}",
                temporary.display(),
                source.display()
            ));
        }
    }
    rollback_result(failures)
}

fn rollback_bulk_rename(
    staged: &[(PathBuf, PathBuf, PathBuf)],
    installed: usize,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for (source, _, destination) in staged[..installed].iter().rev() {
        if let Err(error) = rename_without_replace(destination, source) {
            failures.push(format!(
                "{} -> {}: {error}",
                destination.display(),
                source.display()
            ));
        }
    }
    for (source, temporary, _) in staged[installed..].iter().rev() {
        if let Err(error) = rename_without_replace(temporary, source) {
            failures.push(format!(
                "{} -> {}: {error}",
                temporary.display(),
                source.display()
            ));
        }
    }
    rollback_result(failures)
}

fn rollback_result(failures: Vec<String>) -> Result<(), String> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
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
        if let Err(error) = copy_path(source, destination, cancelled) {
            return Err(cleanup_failed_copy(destination, error));
        }
        return Ok(());
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let temporary = unique_temporary_path(parent);
    if let Err(error) = copy_path(source, &temporary, cancelled) {
        return Err(cleanup_failed_copy(&temporary, error));
    }
    install_completed_replacement(&temporary, destination)
}

fn cleanup_failed_copy(partial: &Path, error: io::Error) -> io::Error {
    let cleanup = if partial.exists() {
        remove_path(partial)
    } else {
        Ok(())
    };
    io::Error::new(
        error.kind(),
        if let Err(cleanup) = cleanup {
            format!(
                "copy failed: {error}; partial output remains at {} because cleanup failed: {cleanup}",
                partial.display()
            )
        } else {
            format!("copy failed and partial output was removed: {error}")
        },
    )
}

fn install_completed_replacement(temporary: &Path, destination: &Path) -> io::Result<()> {
    let temporary_metadata = fs::symlink_metadata(temporary)?;
    let destination_metadata = fs::symlink_metadata(destination)?;
    if temporary_metadata.is_file()
        && destination_metadata.is_file()
        && !temporary_metadata.file_type().is_symlink()
        && !destination_metadata.file_type().is_symlink()
    {
        return crate::platform::replace_file(temporary, destination).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "completed replacement remains at {}, and the original at {} was preserved: {error}",
                    temporary.display(),
                    destination.display()
                ),
            )
        });
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let backup = unique_temporary_path(parent);
    rename_without_replace(destination, &backup).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not preserve the existing destination before replacement; completed copy remains at {}: {error}",
                temporary.display()
            ),
        )
    })?;
    if let Err(error) = rename_without_replace(temporary, destination) {
        let rollback = rename_without_replace(&backup, destination);
        return Err(io::Error::new(
            error.kind(),
            match rollback {
                Ok(()) => format!(
                    "could not install replacement; the original destination was restored and the completed copy remains at {}: {error}",
                    temporary.display()
                ),
                Err(rollback) => format!(
                    "could not install replacement and could not restore the original; original remains at {}, completed copy remains at {}: {error}; rollback: {rollback}",
                    backup.display(),
                    temporary.display()
                ),
            },
        ));
    }
    if let Err(error) = remove_path(&backup) {
        return Err(io::Error::new(
            error.kind(),
            format!(
                "replacement was installed, but the preserved original could not be removed from {}: {error}",
                backup.display()
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

fn move_back_without_replace(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "original path is occupied; refusing to replace {}",
                destination.display()
            ),
        ));
    }
    match rename_without_replace(source, destination) {
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

fn fingerprint_path(path: &Path) -> io::Result<PathFingerprint> {
    let mut state = PathFingerprint {
        first: 0xcbf29ce484222325,
        second: 0x9e3779b97f4a7c15,
    };
    fingerprint_into(path, &mut state)?;
    Ok(state)
}

fn fingerprint_into(path: &Path, state: &mut PathFingerprint) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    update_fingerprint(state, &[u8::from(metadata.permissions().readonly())]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        update_fingerprint(state, &metadata.permissions().mode().to_le_bytes());
    }
    if metadata.file_type().is_symlink() {
        update_fingerprint(state, b"L");
        update_fingerprint_os(state, fs::read_link(path)?.as_os_str());
        return Ok(());
    }
    if metadata.is_dir() {
        update_fingerprint(state, b"D");
        let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            update_fingerprint_os(state, &entry.file_name());
            fingerprint_into(&entry.path(), state)?;
        }
        return Ok(());
    }

    update_fingerprint(state, b"F");
    update_fingerprint(state, &metadata.len().to_le_bytes());
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            break;
        }
        update_fingerprint(state, &buffer[..read]);
    }
    Ok(())
}

fn update_fingerprint(state: &mut PathFingerprint, bytes: &[u8]) {
    for byte in bytes {
        state.first ^= u64::from(*byte);
        state.first = state.first.wrapping_mul(0x100000001b3);
        state.second ^= u64::from(*byte).wrapping_add(0x9e3779b9);
        state.second = state.second.rotate_left(7).wrapping_mul(0x9ddfea08eb382d69);
    }
}

#[cfg(unix)]
fn update_fingerprint_os(state: &mut PathFingerprint, value: &std::ffi::OsStr) {
    use std::os::unix::ffi::OsStrExt;
    update_fingerprint(state, value.as_bytes());
}

#[cfg(windows)]
fn update_fingerprint_os(state: &mut PathFingerprint, value: &std::ffi::OsStr) {
    use std::os::windows::ffi::OsStrExt;
    for unit in value.encode_wide() {
        update_fingerprint(state, &unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_fingerprint_os(state: &mut PathFingerprint, value: &std::ffi::OsStr) {
    update_fingerprint(state, value.to_string_lossy().as_bytes());
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
    fn directory_overwrite_preserves_then_replaces_the_complete_tree() {
        let root = temp_dir("overwrite-directory");
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        let source = source_dir.join("folder");
        let destination = destination_dir.join("folder");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("new.txt"), "new").unwrap();
        fs::write(destination.join("old.txt"), "old").unwrap();

        let result = execute(
            &OperationRequest {
                id: 51,
                kind: OperationKind::Copy,
                sources: vec![source],
                destination: Some(destination_dir.clone()),
                conflict: ConflictPolicy::Overwrite,
            },
            &AtomicBool::new(false),
            |_, _, _| {},
        );

        assert_eq!(result.completed, 1, "{:?}", result.failures);
        assert_eq!(
            fs::read_to_string(destination.join("new.txt")).unwrap(),
            "new"
        );
        assert!(!destination.join("old.txt").exists());
        assert!(fs::read_dir(&destination_dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".caret-copy")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn permission_and_unavailable_failures_are_actionable() {
        let locked = operation_error_message(io::Error::new(
            ErrorKind::PermissionDenied,
            "sharing violation",
        ));
        assert!(locked.contains("locked/read-only"));
        let missing =
            operation_error_message(io::Error::new(ErrorKind::NotFound, "path disappeared"));
        assert!(missing.contains("became unavailable"));
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

    #[test]
    fn case_only_rename_preserves_contents_without_replacement() {
        let root = temp_dir("case-rename");
        let source = root.join("name.txt");
        let destination = root.join("NAME.txt");
        fs::write(&source, "content").unwrap();

        rename_safely(&source, &destination).unwrap();

        assert_eq!(fs::read_to_string(&destination).unwrap(), "content");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert!(fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".caret-copy")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_undo_removes_only_an_unchanged_created_item() {
        let root = temp_dir("copy-undo");
        let source = root.join("source.txt");
        let destination = root.join("destination");
        fs::write(&source, "source").unwrap();
        fs::create_dir_all(&destination).unwrap();
        let request = OperationRequest {
            id: 7,
            kind: OperationKind::Copy,
            sources: vec![source],
            destination: Some(destination.clone()),
            conflict: ConflictPolicy::Ask,
        };
        let result = execute(&request, &AtomicBool::new(false), |_, _, _| {});
        let undo = result.undo.expect("copy should be undoable");
        let copied = destination.join("source.txt");
        assert!(copied.exists());

        let undone = execute_undo(8, &undo, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(undone.completed, 1);
        assert!(!copied.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_undo_refuses_to_remove_a_changed_item() {
        let root = temp_dir("changed-copy-undo");
        let source = root.join("source.txt");
        let destination = root.join("destination");
        fs::write(&source, "source").unwrap();
        fs::create_dir_all(&destination).unwrap();
        let result = execute(
            &OperationRequest {
                id: 9,
                kind: OperationKind::Copy,
                sources: vec![source],
                destination: Some(destination.clone()),
                conflict: ConflictPolicy::Ask,
            },
            &AtomicBool::new(false),
            |_, _, _| {},
        );
        let undo = result.undo.unwrap();
        let copied = destination.join("source.txt");
        fs::write(&copied, "changed after copy").unwrap();

        let undone = execute_undo(10, &undo, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(undone.failures.len(), 1);
        assert_eq!(fs::read_to_string(copied).unwrap(), "changed after copy");
        assert!(undone.undo.is_some(), "a refused undo remains retryable");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn move_undo_returns_the_item_without_replacing_anything() {
        let root = temp_dir("move-undo");
        let source_dir = root.join("source");
        let destination_dir = root.join("destination");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        let source = source_dir.join("note.txt");
        fs::write(&source, "content").unwrap();
        let result = execute(
            &OperationRequest {
                id: 11,
                kind: OperationKind::Move,
                sources: vec![source.clone()],
                destination: Some(destination_dir.clone()),
                conflict: ConflictPolicy::Ask,
            },
            &AtomicBool::new(false),
            |_, _, _| {},
        );
        let undo = result.undo.unwrap();
        let moved = destination_dir.join("note.txt");
        assert!(moved.exists());

        let undone = execute_undo(12, &undo, &AtomicBool::new(false), |_, _, _| {});
        assert_eq!(undone.completed, 1);
        assert!(source.exists());
        assert!(!moved.exists());
        assert_eq!(undone.path_changes, vec![(moved, source)]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bulk_rename_is_atomic_and_undoable() {
        let root = temp_dir("bulk-rename");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        let first_next = root.join("item-1.txt");
        let second_next = root.join("item-2.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        let result = execute_bulk_rename(
            13,
            &[
                (first.clone(), first_next.clone()),
                (second.clone(), second_next.clone()),
            ],
            &AtomicBool::new(false),
            |_, _, _| {},
        );

        assert_eq!(result.completed, 2);
        assert_eq!(fs::read_to_string(&first_next).unwrap(), "first");
        assert_eq!(fs::read_to_string(&second_next).unwrap(), "second");
        let undone = execute_undo(
            14,
            &result.undo.unwrap(),
            &AtomicBool::new(false),
            |_, _, _| {},
        );
        assert_eq!(undone.completed, 2);
        assert_eq!(fs::read_to_string(first).unwrap(), "first");
        assert_eq!(fs::read_to_string(second).unwrap(), "second");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bulk_rename_rejects_collisions_before_changing_sources() {
        let root = temp_dir("bulk-collision");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        let occupied = root.join("occupied.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        fs::write(&occupied, "important").unwrap();
        let result = execute_bulk_rename(
            15,
            &[
                (first.clone(), occupied.clone()),
                (second.clone(), root.join("other.txt")),
            ],
            &AtomicBool::new(false),
            |_, _, _| {},
        );

        assert_eq!(result.failures.len(), 1);
        assert_eq!(fs::read_to_string(first).unwrap(), "first");
        assert_eq!(fs::read_to_string(second).unwrap(), "second");
        assert_eq!(fs::read_to_string(occupied).unwrap(), "important");
        let _ = fs::remove_dir_all(root);
    }
}
