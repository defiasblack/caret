//! Small platform boundary for behavior that cannot safely be expressed as a
//! portable `std` operation.

use std::{
    env, io,
    path::{Path, PathBuf},
};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as implementation;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as implementation;

pub fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    implementation::replace_file(source, destination)
}

pub fn app_data_dir() -> PathBuf {
    if let Some(path) = env::var_os("CARET_DATA_DIR") {
        return PathBuf::from(path);
    }
    directories::ProjectDirs::from("com", "Caret", "Caret")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("caret-data"))
}

pub fn config_dir() -> PathBuf {
    if let Some(path) = env::var_os("CARET_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    directories::ProjectDirs::from("com", "Caret", "Caret")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("caret-config"))
}

pub fn shell_command() -> (String, Vec<String>, String) {
    implementation::shell_command()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn replacement_preserves_destination_when_source_is_missing() {
        let root = std::env::temp_dir().join(format!("caret-platform-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source = root.join("missing.tmp");
        let destination = root.join("important.txt");
        fs::write(&destination, b"original").unwrap();

        let _ = replace_file(&source, &destination).unwrap_err();
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replacement_swaps_a_complete_file_over_an_existing_destination() {
        let root =
            std::env::temp_dir().join(format!("caret-platform-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source = root.join("complete.tmp");
        let destination = root.join("important.txt");
        fs::write(&source, b"replacement").unwrap();
        fs::write(&destination, b"original").unwrap();

        replace_file(&source, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"replacement");
        assert!(!source.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn application_data_directory_is_stable_and_nonempty() {
        assert!(!app_data_dir().as_os_str().is_empty());
    }

    #[test]
    fn shell_detection_returns_a_launchable_program_description() {
        let (program, _arguments, name) = shell_command();
        assert!(!program.is_empty());
        assert!(!name.is_empty());
    }
}
