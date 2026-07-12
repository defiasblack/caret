use std::path::Path;

use crossterm::style::Color;

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Json,
    Toml,
    Markdown,
    Python,
    Shell,
    Plain,
}

impl Language {
    pub fn from_path(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::Plain;
        };

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        match extension.as_str() {
            "rs" => Self::Rust,
            "json" | "jsonc" => Self::Json,
            "toml" => Self::Toml,
            "md" | "markdown" => Self::Markdown,
            "py" | "pyw" => Self::Python,
            "sh" | "bash" | "zsh" | "fish" => Self::Shell,
            _ => Self::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Python => "Python",
            Self::Shell => "Shell",
            Self::Plain => "Plain Text",
        }
    }

    pub fn comment_delimiters(self) -> Option<(&'static str, Option<&'static str>)> {
        match self {
            Self::Rust | Self::Json => Some(("//", None)),
            Self::Toml | Self::Python | Self::Shell => Some(("#", None)),
            Self::Markdown => Some(("<!--", Some("-->"))),
            Self::Plain => None,
        }
    }
}

pub fn highlight_line(line: &str, language: Language, theme: &Theme) -> Vec<Color> {
    let chars: Vec<char> = line.chars().collect();
    let mut colors = vec![theme.foreground; chars.len()];

    if chars.is_empty() {
        return colors;
    }

    if language == Language::Markdown {
        highlight_markdown(&chars, &mut colors, theme);
        return colors;
    }

    if language == Language::Plain {
        return colors;
    }

    let comment_marker = match language {
        Language::Rust | Language::Json => Some("//"),
        Language::Toml | Language::Python | Language::Shell => Some("#"),
        _ => None,
    };

    let mut index = 0;

    while index < chars.len() {
        if let Some(marker) = comment_marker {
            if marker == "//"
                && index + 1 < chars.len()
                && chars[index] == '/'
                && chars[index + 1] == '/'
            {
                for color in &mut colors[index..] {
                    *color = theme.comment;
                }
                break;
            }

            if marker == "#" && chars[index] == '#' {
                for color in &mut colors[index..] {
                    *color = theme.comment;
                }
                break;
            }
        }

        if chars[index] == '"' || chars[index] == '\'' {
            let quote = chars[index];
            let start = index;
            index += 1;
            let mut escaped = false;

            while index < chars.len() {
                let current = chars[index];

                if escaped {
                    escaped = false;
                } else if current == '\\' {
                    escaped = true;
                } else if current == quote {
                    index += 1;
                    break;
                }

                index += 1;
            }

            for color in &mut colors[start..index] {
                *color = theme.string;
            }
            continue;
        }

        if chars[index].is_ascii_digit() {
            let start = index;
            index += 1;

            while index < chars.len()
                && (chars[index].is_ascii_hexdigit()
                    || matches!(chars[index], '.' | '_' | 'x' | 'X' | 'o' | 'O' | 'b' | 'B'))
            {
                index += 1;
            }

            for color in &mut colors[start..index] {
                *color = theme.number;
            }
            continue;
        }

        if is_identifier_start(chars[index]) {
            let start = index;
            index += 1;

            while index < chars.len() && is_identifier_continue(chars[index]) {
                index += 1;
            }

            let token: String = chars[start..index].iter().collect();

            if is_keyword(language, &token) {
                for color in &mut colors[start..index] {
                    *color = theme.keyword;
                }
            } else if is_type_name(language, &token) {
                for color in &mut colors[start..index] {
                    *color = theme.type_name;
                }
            }

            continue;
        }

        if matches!(
            chars[index],
            '{' | '}' | '[' | ']' | '(' | ')' | ':' | ';' | ',' | '.' | '=' | '+' | '-' | '*'
                | '/' | '%' | '&' | '|' | '!' | '<' | '>' | '?'
        ) {
            colors[index] = theme.punctuation;
        }

        index += 1;
    }

    colors
}

fn highlight_markdown(chars: &[char], colors: &mut [Color], theme: &Theme) {
    let first_non_space = chars.iter().position(|character| !character.is_whitespace());

    if let Some(position) = first_non_space {
        if chars[position] == '#' {
            for color in &mut colors[position..] {
                *color = theme.heading;
            }
            return;
        }
    }

    let mut index = 0;
    let mut in_code = false;

    while index < chars.len() {
        if chars[index] == '`' {
            in_code = !in_code;
            colors[index] = theme.string;
            index += 1;
            continue;
        }

        if in_code {
            colors[index] = theme.string;
            index += 1;
            continue;
        }

        if chars[index] == '[' {
            let start = index;
            while index < chars.len() && chars[index] != ']' {
                index += 1;
            }
            if index < chars.len() {
                index += 1;
            }
            for color in &mut colors[start..index] {
                *color = theme.heading;
            }
            continue;
        }

        if chars[index] == '*' || chars[index] == '_' {
            colors[index] = theme.keyword;
        }

        if chars[index] == '>' && first_non_space == Some(index) {
            for color in &mut colors[index..] {
                *color = theme.comment;
            }
            return;
        }

        index += 1;
    }
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn is_keyword(language: Language, token: &str) -> bool {
    match language {
        Language::Rust => matches!(
            token,
            "as"
                | "async"
                | "await"
                | "break"
                | "const"
                | "continue"
                | "crate"
                | "dyn"
                | "else"
                | "enum"
                | "extern"
                | "false"
                | "fn"
                | "for"
                | "if"
                | "impl"
                | "in"
                | "let"
                | "loop"
                | "match"
                | "mod"
                | "move"
                | "mut"
                | "pub"
                | "ref"
                | "return"
                | "self"
                | "Self"
                | "static"
                | "struct"
                | "super"
                | "trait"
                | "true"
                | "type"
                | "unsafe"
                | "use"
                | "where"
                | "while"
                | "yield"
        ),
        Language::Json => matches!(token, "true" | "false" | "null"),
        Language::Toml => matches!(token, "true" | "false"),
        Language::Python => matches!(
            token,
            "and"
                | "as"
                | "assert"
                | "async"
                | "await"
                | "break"
                | "class"
                | "continue"
                | "def"
                | "del"
                | "elif"
                | "else"
                | "except"
                | "False"
                | "finally"
                | "for"
                | "from"
                | "global"
                | "if"
                | "import"
                | "in"
                | "is"
                | "lambda"
                | "None"
                | "nonlocal"
                | "not"
                | "or"
                | "pass"
                | "raise"
                | "return"
                | "True"
                | "try"
                | "while"
                | "with"
                | "yield"
        ),
        Language::Shell => matches!(
            token,
            "case"
                | "do"
                | "done"
                | "elif"
                | "else"
                | "esac"
                | "fi"
                | "for"
                | "function"
                | "if"
                | "in"
                | "select"
                | "then"
                | "time"
                | "until"
                | "while"
        ),
        _ => false,
    }
}

fn is_type_name(language: Language, token: &str) -> bool {
    match language {
        Language::Rust => matches!(
            token,
            "bool"
                | "char"
                | "f32"
                | "f64"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "str"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "String"
                | "Vec"
                | "Option"
                | "Result"
                | "Box"
        ),
        Language::Python => false,
        _ => false,
    }
}
