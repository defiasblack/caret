use std::{
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn log_path() -> PathBuf {
    crate::document::recovery_dir()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("caret.log")
}

pub fn append(level: &str, message: &str) -> io::Result<()> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?,
        "{{\"timestamp\":{timestamp},\"level\":{level:?},\"message\":{message:?}}}"
    )
}

pub fn report(version: &str) -> String {
    let terminal = std::env::var("TERM").unwrap_or_else(|_| "not set".to_string());
    let shell = std::env::var("SHELL")
        .or_else(|_| std::env::var("COMSPEC"))
        .unwrap_or_else(|_| "not detected".to_string());
    let color = std::env::var("COLORTERM").unwrap_or_else(|_| "not reported".to_string());
    let clipboard = if arboard::Clipboard::new().is_ok() {
        "available"
    } else {
        "unavailable (internal clipboard remains available)"
    };
    format!(
        "Caret diagnostic report\nversion: {version}\nos: {} {}\nterminal: {terminal}\nterminal color: {color}\nshell: {shell}\nconfig: {}\nrecovery: {}\nlog: {}\nclipboard: {clipboard}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::config::config_path().display(),
        crate::document::recovery_dir().display(),
        log_path().display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_includes_core_support_context() {
        let report = report("test");
        assert!(report.contains("version: test"));
        assert!(report.contains("os:"));
        assert!(report.contains("clipboard:"));
    }
}
