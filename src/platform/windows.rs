use std::{env, ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path};

use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source.as_os_str());
    let destination = wide(destination.as_os_str());
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

pub fn shell_command() -> (String, Vec<String>, String) {
    if let Ok(program) = env::var("CARET_SHELL") {
        let name = Path::new(&program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shell")
            .to_string();
        return (program, Vec::new(), name);
    }
    (
        "powershell.exe".to_string(),
        vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
        "PowerShell".to_string(),
    )
}
