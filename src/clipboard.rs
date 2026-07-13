//! Clipboard transport with a desktop-first and terminal-safe fallback.
use std::io::{self, IsTerminal, Write};

const OSC52_MAX_BYTES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMethod {
    System,
    TerminalOsc52,
}

pub fn copy(text: &str) -> io::Result<CopyMethod> {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        if clipboard.set_text(text).is_ok() {
            return Ok(CopyMethod::System);
        }
    }

    if !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no system clipboard or interactive terminal clipboard",
        ));
    }
    if text.len() > OSC52_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "selection is too large for terminal clipboard transfer",
        ));
    }

    let mut output = io::stdout().lock();
    output.write_all(osc52_sequence(text).as_bytes())?;
    output.flush()?;
    Ok(CopyMethod::TerminalOsc52)
}

fn osc52_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0b11) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0b1111) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0b11_1111) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_uses_a_base64_encoded_clipboard_payload() {
        assert_eq!(osc52_sequence("Hi!"), "\x1b]52;c;SGkh\x07");
        assert_eq!(osc52_sequence("a"), "\x1b]52;c;YQ==\x07");
    }
}
