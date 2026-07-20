use std::{env, fs, io, path::Path};

pub fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)?;
    // Sync the directory entry after the atomic rename so a power loss cannot
    // lose the replacement even though the file contents were synchronized.
    if let Some(parent) = destination.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub fn shell_command() -> (String, Vec<String>, String) {
    let program = env::var("CARET_SHELL")
        .or_else(|_| env::var("SHELL"))
        .unwrap_or_else(|_| "/bin/sh".to_string());
    let name = Path::new(&program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("shell")
        .to_string();
    (program, Vec::new(), name)
}
