use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

const SCROLLBACK_LINES: usize = 5_000;
const INITIAL_ROWS: u16 = 24;
const INITIAL_COLUMNS: u16 = 80;

pub struct TerminalPane {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    input: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    parser: vt100::Parser,
    exited: bool,
    pub cwd: PathBuf,
    pub shell_name: String,
}

impl TerminalPane {
    pub fn start(cwd: &Path) -> io::Result<Self> {
        let (program, args, shell_name) = shell_command();
        let pair = native_pty_system()
            .openpty(pty_size(INITIAL_ROWS, INITIAL_COLUMNS))
            .map_err(io_other)?;
        let mut command = CommandBuilder::new(&program);
        command.args(args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command).map_err(io_other)?;
        let reader = pair.master.try_clone_reader().map_err(io_other)?;
        let input = pair.master.take_writer().map_err(io_other)?;
        drop(pair.slave);

        let (sender, output) = mpsc::channel();
        spawn_reader(reader, sender);
        let mut terminal = Self {
            master: pair.master,
            child,
            input,
            output,
            parser: vt100::Parser::new(INITIAL_ROWS, INITIAL_COLUMNS, SCROLLBACK_LINES),
            exited: false,
            cwd: cwd.to_path_buf(),
            shell_name,
        };
        // Windows shells query the terminal cursor before accepting input.
        // Complete that short handshake so immediate keystrokes cannot arrive first.
        for _ in 0..400 {
            if terminal.poll() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(terminal)
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(bytes) = self.output.try_recv() {
            let cursor_position_requested = bytes.windows(4).any(|window| window == b"\x1b[6n");
            self.parser.process(&bytes);
            if cursor_position_requested {
                let (row, column) = self.parser.screen().cursor_position();
                let response = format!("\x1b[{};{}R", row + 1, column + 1);
                if let Err(error) = self
                    .input
                    .write_all(response.as_bytes())
                    .and_then(|()| self.input.flush())
                {
                    self.parser
                        .process(format!("\r\n[PTY response failed: {error}]\r\n").as_bytes());
                }
            }
            changed = true;
        }
        if !self.exited && self.child.try_wait().ok().flatten().is_some() {
            self.exited = true;
            self.parser
                .process(b"\r\n[shell exited -- use :terminal to restart]\r\n");
            changed = true;
        }
        changed
    }

    pub fn resize(&mut self, rows: usize, columns: usize) {
        let rows = rows.clamp(1, u16::MAX as usize) as u16;
        let columns = columns.clamp(1, u16::MAX as usize) as u16;
        if self.parser.screen().size() == (rows, columns) {
            return;
        }
        let _ = self.master.resize(pty_size(rows, columns));
        self.parser.screen_mut().set_size(rows, columns);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        let bytes = key_bytes(key);
        if !bytes.is_empty() {
            self.input.write_all(&bytes)?;
            self.input.flush()?;
            self.parser.screen_mut().set_scrollback(0);
        }
        Ok(())
    }

    pub fn visible_lines(&self, rows: usize) -> Vec<String> {
        let (_, columns) = self.parser.screen().size();
        self.parser.screen().rows(0, columns).take(rows).collect()
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        let (row, column) = self.parser.screen().cursor_position();
        (row as usize, column as usize)
    }

    pub fn is_exited(&self) -> bool {
        self.exited
    }

    pub fn scroll_up(&mut self, rows: usize) {
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_add(rows));
    }

    pub fn scroll_down(&mut self, rows: usize) {
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_sub(rows));
    }
}

impl Drop for TerminalPane {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn spawn_reader(mut reader: Box<dyn Read + Send>, sender: mpsc::Sender<Vec<u8>>) {
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) if sender.send(buffer[..count].to_vec()).is_err() => break,
                Ok(_) => {}
                Err(error) => {
                    let _ = sender.send(format!("\r\n[PTY read failed: {error}]\r\n").into_bytes());
                    break;
                }
            }
        }
    });
}

fn key_bytes(key: KeyEvent) -> Vec<u8> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(character) = key.code {
            let lower = character.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return vec![(lower as u8) - b'a' + 1];
            }
            return match character {
                '@' | ' ' => vec![0],
                '[' => vec![27],
                '\\' => vec![28],
                ']' => vec![29],
                '^' => vec![30],
                '_' => vec![31],
                _ => Vec::new(),
            };
        }
    }

    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
    match key.code {
        KeyCode::Char(character) => {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::F(number @ 1..=4) => {
            bytes.extend_from_slice(format!("\x1bO{}", (b'P' + number - 1) as char).as_bytes())
        }
        KeyCode::F(number @ 5..=12) => {
            const CODES: [&str; 8] = ["15", "17", "18", "19", "20", "21", "23", "24"];
            bytes.extend_from_slice(format!("\x1b[{}~", CODES[(number - 5) as usize]).as_bytes());
        }
        _ => {}
    }
    bytes
}

fn pty_size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn io_other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(windows)]
fn shell_command() -> (String, Vec<String>, String) {
    crate::platform::shell_command()
}

#[cfg(not(windows))]
fn shell_command() -> (String, Vec<String>, String) {
    crate::platform::shell_command()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn maps_terminal_control_and_navigation_keys() {
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            vec![3]
        );
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            b"\x1b[A"
        );
        assert_eq!(
            key_bytes(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            b"\x1bx"
        );
    }

    #[test]
    fn pty_shell_executes_and_streams_output() {
        let cwd = std::env::current_dir().expect("current directory");
        let mut terminal = TerminalPane::start(&cwd).expect("start shell");
        for character in "echo CARET_TERMINAL_TEST".chars() {
            terminal
                .handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
                .expect("type command");
        }
        terminal
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("submit command");

        let mut found = false;
        for _ in 0..200 {
            terminal.poll();
            if terminal
                .parser
                .screen()
                .contents()
                .lines()
                .any(|line| line.trim() == "CARET_TERMINAL_TEST")
            {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            found,
            "PTY output was not parsed (exited={}): {:?}",
            terminal.exited,
            terminal.visible_lines(100)
        );
    }
}
