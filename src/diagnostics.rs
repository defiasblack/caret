use std::{
    fs,
    io::{self, IsTerminal},
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
        "{}",
        format_record(timestamp, level, message)
    )
}

fn format_record(timestamp: u64, level: &str, message: &str) -> String {
    serde_json::json!({
        "timestamp": timestamp,
        "level": level,
        "message": message,
    })
    .to_string()
}

pub fn report(version: &str) -> String {
    let terminal = std::env::var("TERM").unwrap_or_else(|_| "not set".to_string());
    let shell = std::env::var("SHELL")
        .or_else(|_| std::env::var("COMSPEC"))
        .unwrap_or_else(|_| "not detected".to_string());
    let color = std::env::var("COLORTERM").unwrap_or_else(|_| "not reported".to_string());
    let desktop_clipboard = if arboard::Clipboard::new().is_ok() {
        "available"
    } else {
        "unavailable"
    };
    let ansi = terminal != "dumb" && terminal != "not set";
    let color_mode = if std::env::var_os("NO_COLOR").is_some() {
        "disabled by NO_COLOR"
    } else if color.to_ascii_lowercase().contains("truecolor")
        || color.to_ascii_lowercase().contains("24bit")
    {
        "truecolor reported"
    } else if terminal.contains("256color") {
        "256-color reported"
    } else {
        "not reported"
    };
    let ssh = std::env::var_os("SSH_CONNECTION").is_some()
        || std::env::var_os("SSH_TTY").is_some()
        || std::env::var_os("SSH_CLIENT").is_some();
    let tmux = std::env::var_os("TMUX").is_some();
    let osc52 = ssh || desktop_clipboard == "unavailable";
    let (settings, config_error) = crate::config::load();
    let configuration = config_error.map_or_else(
        || {
            format!(
                "valid · theme={} · keymap={} · startup={}",
                settings.theme.name(),
                settings.keymap.name(),
                settings.startup.name()
            )
        },
        |error| format!("invalid · {error}"),
    );
    format!(
        "Caret diagnostic report\nversion: {version}\nos: {} {}\nterminal: {terminal}\nterminal color: {color}\nterminal capabilities: stdin_tty={} stdout_tty={} ansi={ansi} color={color_mode} ssh={ssh} tmux={tmux}\nshell: {shell}\nconfig: {} ({configuration})\nrecovery: {}\nlog: {}\nlsp stderr: structured records in log\nfilesystem background: explorer failures are recorded in log\nwatcher: not enabled in 0.6\npreview service: not enabled in 0.6\nclipboard: desktop={desktop_clipboard} osc52_fallback={osc52} internal=available",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
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
        assert!(report.contains("terminal capabilities:"));
        assert!(report.contains("filesystem background:"));
        assert!(report.contains("watcher:"));
        assert!(report.contains("preview service:"));
        assert!(report.contains("clipboard: desktop="));
        assert!(report.contains("configuration") || report.contains("config:"));
    }

    #[test]
    fn log_records_are_valid_structured_json() {
        let value: serde_json::Value =
            serde_json::from_str(&format_record(123, "lsp", "server failed")).unwrap();
        assert_eq!(value["timestamp"], 123);
        assert_eq!(value["level"], "lsp");
        assert_eq!(value["message"], "server failed");
    }
}
