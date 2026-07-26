//! Safety boundary for the legacy synchronous project-tree operations.
//!
//! Milestone 0.7 will move these operations to a background service with
//! progress, cancellation, trash, and conflict handling. Until then, these
//! primitives enforce the 0.6 invariant that an ordinary operation never
//! replaces an existing destination implicitly.

#[cfg(unix)]
use std::ffi::CString;
use std::{
    fs,
    io::{self, Write},
    path::Path,
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

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "source path contains a NUL byte",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
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

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "source path contains a NUL byte",
        )
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
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
            io::ErrorKind::AlreadyExists,
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
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("caret-file-ops-{name}-{}", std::process::id()))
    }

    #[test]
    fn create_file_never_truncates_an_existing_path() {
        let root = temp_root("create");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("important.txt");
        fs::write(&path, "important").unwrap();

        assert_eq!(
            create_file(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "important");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_never_truncates_an_existing_destination() {
        let root = temp_root("copy");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.txt");
        let destination = root.join("destination.txt");
        fs::write(&source, "source").unwrap();
        fs::write(&destination, "important").unwrap();

        assert_eq!(
            copy_file_without_replace(&source, &destination)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&destination).unwrap(), "important");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rename_never_replaces_an_existing_destination() {
        let root = temp_root("rename");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.txt");
        let destination = root.join("destination.txt");
        fs::write(&source, "source").unwrap();
        fs::write(&destination, "important").unwrap();

        assert_eq!(
            rename_without_replace(&source, &destination)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&source).unwrap(), "source");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "important");
        let _ = fs::remove_dir_all(root);
    }
}
