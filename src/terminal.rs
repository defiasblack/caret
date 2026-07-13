use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const MAX_OUTPUT_LINES: usize = 5_000;

pub struct TerminalPane {
    child: Child,
    input_stream: ChildStdin,
    output: Receiver<Vec<u8>>,
    lines: VecDeque<String>,
    partial_line: String,
    input: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    scroll: usize,
    exited: bool,
    pub cwd: PathBuf,
    pub shell_name: String,
}

impl TerminalPane {
    pub fn start(cwd: &Path) -> io::Result<Self> {
        let (program, args, shell_name) = shell_command();
        let mut child = Command::new(&program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let input_stream = child.stdin.take().ok_or_else(|| io::Error::other("shell stdin unavailable"))?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("shell stdout unavailable"))?;
        let stderr = child.stderr.take().ok_or_else(|| io::Error::other("shell stderr unavailable"))?;
        let (sender, output) = mpsc::channel();
        spawn_reader(stdout, sender.clone());
        spawn_reader(stderr, sender);
        let mut lines = VecDeque::new();
        lines.push_back(format!("Caret Terminal · {shell_name} · {}", cwd.display()));
        lines.push_back("Ctrl-` returns to editor · Ctrl-L clears · Up/Down history".to_string());
        Ok(Self {
            child,
            input_stream,
            output,
            lines,
            partial_line: String::new(),
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            scroll: 0,
            exited: false,
            cwd: cwd.to_path_buf(),
            shell_name,
        })
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output.try_recv() {
            self.push_output(&String::from_utf8_lossy(&bytes));
            changed = true;
        }
        if !self.exited && self.child.try_wait().ok().flatten().is_some() {
            self.exited = true;
            self.push_line("[shell exited — use :terminal to restart]".to_string());
            changed = true;
        }
        changed
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('l' | 'L') => {
                    self.lines.clear();
                    self.partial_line.clear();
                    self.scroll = 0;
                }
                KeyCode::Char('c' | 'C') => {
                    self.input.clear();
                    self.cursor = 0;
                    self.history_index = None;
                    self.push_line("^C".to_string());
                }
                _ => {}
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Enter => self.submit()?,
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let previous = previous_boundary(&self.input, self.cursor);
                    self.input.replace_range(previous..self.cursor, "");
                    self.cursor = previous;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    let next = next_boundary(&self.input, self.cursor);
                    self.input.replace_range(self.cursor..next, "");
                }
            }
            KeyCode::Left => self.cursor = previous_boundary(&self.input, self.cursor),
            KeyCode::Right => self.cursor = next_boundary(&self.input, self.cursor),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.len(),
            KeyCode::Up => self.history_move(false),
            KeyCode::Down => self.history_move(true),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(5),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(5),
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::ALT) => {
                self.input.insert(self.cursor, character);
                self.cursor += character.len_utf8();
            }
            _ => {}
        }
        Ok(())
    }

    fn submit(&mut self) -> io::Result<()> {
        let command = self.input.trim().to_string();
        self.push_line(format!("> {command}"));
        if !command.is_empty() {
            if self.history.last() != Some(&command) { self.history.push(command.clone()); }
            self.history.truncate(500);
            self.input_stream.write_all(command.as_bytes())?;
            self.input_stream.write_all(b"\n")?;
            self.input_stream.flush()?;
        }
        self.input.clear();
        self.cursor = 0;
        self.history_index = None;
        self.scroll = 0;
        Ok(())
    }

    fn history_move(&mut self, forward: bool) {
        if self.history.is_empty() { return; }
        let index = match (self.history_index, forward) {
            (None, false) => self.history.len() - 1,
            (Some(index), false) => index.saturating_sub(1),
            (Some(index), true) if index + 1 < self.history.len() => index + 1,
            (_, true) => {
                self.history_index = None;
                self.input.clear();
                self.cursor = 0;
                return;
            }
        };
        self.history_index = Some(index);
        self.input = self.history[index].clone();
        self.cursor = self.input.len();
    }

    fn push_output(&mut self, text: &str) {
        let clean = strip_ansi(text).replace("\r\n", "\n").replace('\r', "\n");
        for segment in clean.split_inclusive('\n') {
            if let Some(line) = segment.strip_suffix('\n') {
                self.partial_line.push_str(line);
                let complete = std::mem::take(&mut self.partial_line);
                self.push_line(complete);
            } else {
                self.partial_line.push_str(segment);
            }
        }
        self.scroll = 0;
    }

    fn push_line(&mut self, line: String) {
        self.lines.push_back(line);
        while self.lines.len() > MAX_OUTPUT_LINES { self.lines.pop_front(); }
    }

    pub fn visible_lines(&self, rows: usize) -> Vec<String> {
        let mut all = self.lines.iter().cloned().collect::<Vec<_>>();
        if !self.partial_line.is_empty() { all.push(self.partial_line.clone()); }
        let end = all.len().saturating_sub(self.scroll.min(all.len()));
        let start = end.saturating_sub(rows);
        all[start..end].to_vec()
    }

    pub fn input(&self) -> &str { &self.input }
    pub fn cursor_column(&self) -> usize { self.input[..self.cursor.min(self.input.len())].chars().count() + 2 }
    pub fn is_exited(&self) -> bool { self.exited }
    pub fn scroll_up(&mut self, rows: usize) { self.scroll = self.scroll.saturating_add(rows).min(self.lines.len()); }
    pub fn scroll_down(&mut self, rows: usize) { self.scroll = self.scroll.saturating_sub(rows); }
}

impl Drop for TerminalPane {
    fn drop(&mut self) { let _ = self.child.kill(); }
}

fn spawn_reader(mut reader: impl Read + Send + 'static, sender: mpsc::Sender<Vec<u8>>) {
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 { break; }
            if sender.send(buffer[..count].to_vec()).is_err() { break; }
        }
    });
}

#[cfg(windows)]
fn shell_command() -> (String, Vec<String>, String) {
    let program = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    (program, vec!["/Q".to_string()], "Command Prompt".to_string())
}

#[cfg(not(windows))]
fn shell_command() -> (String, Vec<String>, String) {
    let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let name = Path::new(&program).file_name().and_then(|name| name.to_str()).unwrap_or("shell").to_string();
    (program, Vec::new(), name)
}

fn strip_ansi(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) { break; }
                }
            }
            continue;
        }
        if character != '\u{8}' { output.push(character); }
    }
    output
}

fn previous_boundary(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].char_indices().last().map_or(0, |(offset, _)| offset)
}

fn next_boundary(text: &str, index: usize) -> usize {
    text[index.min(text.len())..].chars().next().map_or(text.len(), |character| index + character.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn removes_common_ansi_control_sequences() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn persistent_shell_executes_and_streams_output() {
        let cwd = std::env::current_dir().expect("current directory");
        let mut terminal = TerminalPane::start(&cwd).expect("start shell");
        for character in "echo CARET_TERMINAL_TEST".chars() {
            terminal.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)).expect("type command");
        }
        terminal.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).expect("submit command");

        let mut found = false;
        for _ in 0..40 {
            terminal.poll();
            if terminal.visible_lines(100).iter().any(|line| line.trim().ends_with("CARET_TERMINAL_TEST") && !line.contains("echo ")) {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(found, "shell output was not streamed back to the pane: {:?}", terminal.visible_lines(100));
    }
}
