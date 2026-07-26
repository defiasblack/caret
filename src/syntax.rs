use std::path::Path;

use crossterm::style::Color;
use tree_sitter::{
    InputEdit, Language as TreeLanguage, Node, Parser, Point, Query, QueryCursor,
    StreamingIterator, Tree,
};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Go,
    CSharp,
    Yaml,
    Json,
    Toml,
    Markdown,
    Python,
    JavaScript,
    TypeScript,
    Shell,
    Plain,
}

struct LanguageDefinition {
    language: Language,
    extensions: &'static [&'static str],
    grammar: fn() -> TreeLanguage,
    highlights: &'static str,
    indents: &'static str,
}

const BRACE_INDENTS: &str = r#"["{" "[" "("] @indent"#;
const COLLECTION_INDENTS: &str = r#"["{" "["] @indent"#;
const COLON_INDENTS: &str = r#"[":" "{" "[" "("] @indent"#;
const YAML_INDENTS: &str = r#"[":" "-" "{" "["] @indent"#;

const LANGUAGE_DEFINITIONS: &[LanguageDefinition] = &[
    LanguageDefinition {
        language: Language::Rust,
        extensions: &["rs"],
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        highlights: tree_sitter_rust::HIGHLIGHTS_QUERY,
        indents: BRACE_INDENTS,
    },
    LanguageDefinition {
        language: Language::Go,
        extensions: &["go"],
        grammar: || tree_sitter_go::LANGUAGE.into(),
        highlights: tree_sitter_go::HIGHLIGHTS_QUERY,
        indents: BRACE_INDENTS,
    },
    LanguageDefinition {
        language: Language::CSharp,
        extensions: &["cs"],
        grammar: || tree_sitter_c_sharp::LANGUAGE.into(),
        highlights: tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
        indents: BRACE_INDENTS,
    },
    LanguageDefinition {
        language: Language::Yaml,
        extensions: &["yaml", "yml"],
        grammar: || tree_sitter_yaml::LANGUAGE.into(),
        highlights: tree_sitter_yaml::HIGHLIGHTS_QUERY,
        indents: YAML_INDENTS,
    },
    LanguageDefinition {
        language: Language::Json,
        extensions: &["json", "jsonc"],
        grammar: || tree_sitter_json::LANGUAGE.into(),
        highlights: tree_sitter_json::HIGHLIGHTS_QUERY,
        indents: COLLECTION_INDENTS,
    },
    LanguageDefinition {
        language: Language::Toml,
        extensions: &["toml"],
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights: tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        indents: COLLECTION_INDENTS,
    },
    LanguageDefinition {
        language: Language::Python,
        extensions: &["py", "pyw"],
        grammar: || tree_sitter_python::LANGUAGE.into(),
        highlights: tree_sitter_python::HIGHLIGHTS_QUERY,
        indents: COLON_INDENTS,
    },
    LanguageDefinition {
        language: Language::JavaScript,
        extensions: &["js", "jsx", "mjs", "cjs"],
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        highlights: tree_sitter_javascript::HIGHLIGHT_QUERY,
        indents: BRACE_INDENTS,
    },
    LanguageDefinition {
        language: Language::TypeScript,
        extensions: &["ts", "tsx", "mts", "cts"],
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlights: tree_sitter_typescript::HIGHLIGHTS_QUERY,
        indents: BRACE_INDENTS,
    },
    LanguageDefinition {
        language: Language::Shell,
        extensions: &["sh", "bash", "zsh", "fish"],
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        highlights: tree_sitter_bash::HIGHLIGHT_QUERY,
        indents: BRACE_INDENTS,
    },
];

fn language_definition(language: Language) -> Option<&'static LanguageDefinition> {
    LANGUAGE_DEFINITIONS
        .iter()
        .find(|definition| definition.language == language)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: &'static str,
    pub start_line: usize,
    pub end_line: usize,
    pub depth: usize,
}

pub struct SyntaxDocument {
    language: Language,
    parser: Parser,
    tree: Tree,
    highlight_query: Option<Query>,
    indent_query: Option<Query>,
    source: String,
}

impl SyntaxDocument {
    pub fn new(path: Option<&Path>, source: &str) -> Option<Self> {
        let language = Language::from_path(path);
        let definition = language_definition(language)?;
        let tree_language = (definition.grammar)();
        let mut parser = Parser::new();
        parser.set_language(&tree_language).ok()?;
        let tree = parser.parse(source, None)?;
        let highlight_query = Query::new(&tree_language, definition.highlights).ok();
        let indent_query = Query::new(&tree_language, definition.indents).ok();
        Some(Self {
            language,
            parser,
            tree,
            highlight_query,
            indent_query,
            source: source.to_string(),
        })
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn apply_edit(&mut self, at: usize, removed_chars: usize, inserted: &str) {
        let start_byte = char_index_to_byte(&self.source, at);
        let old_end_byte = char_index_to_byte(&self.source, at.saturating_add(removed_chars));
        let start_position = point_at_byte(&self.source, start_byte);
        let old_end_position = point_at_byte(&self.source, old_end_byte);
        let new_end_position = point_after_text(start_position, inserted);
        let new_end_byte = start_byte + inserted.len();
        self.tree.edit(&InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        });
        self.source
            .replace_range(start_byte..old_end_byte, inserted);
        if let Some(tree) = self.parser.parse(&self.source, Some(&self.tree)) {
            self.tree = tree;
        }
    }

    pub fn highlight_line(&self, line_index: usize, line: &str, theme: &Theme) -> Vec<Color> {
        let mut colors = highlight_line_base(line, self.language, theme);
        let line_start = line_start_byte(&self.source, line_index).unwrap_or(self.source.len());
        if let Some(query) = &self.highlight_query {
            apply_query_highlights(
                query,
                self.tree.root_node(),
                &self.source,
                line_start,
                line,
                theme,
                &mut colors,
            );
        } else {
            apply_document_node_highlights(
                self.tree.root_node(),
                line_index,
                line_start,
                line,
                theme,
                &mut colors,
            );
        }
        colors
    }

    pub fn fold_ranges(&self) -> Vec<(usize, usize)> {
        let root = self.tree.root_node();
        let mut ranges = Vec::new();
        collect_fold_ranges(root, root.id(), &mut ranges);
        ranges.sort_unstable();
        ranges.dedup();
        ranges
    }

    pub fn symbols(&self) -> Vec<Symbol> {
        let mut output = Vec::new();
        collect_symbols(self.tree.root_node(), &self.source, 0, &mut output);
        output
    }

    pub fn matching_bracket(&self, cursor_char: usize) -> Option<usize> {
        let characters = self.source.chars().collect::<Vec<_>>();
        let mut index = cursor_char.min(characters.len());
        if index >= characters.len() || !is_bracket(characters[index]) {
            if index == 0 || !is_bracket(characters[index - 1]) {
                return None;
            }
            index -= 1;
        }
        if !self.is_code_char(index) {
            return None;
        }

        let (opening, closing, forward) = bracket_pair(characters[index])?;
        let mut depth = 0usize;
        if forward {
            for (offset, character) in characters[index..].iter().enumerate() {
                let target = index + offset;
                if !self.is_code_char(target) {
                    continue;
                }
                if *character == opening {
                    depth += 1;
                } else if *character == closing {
                    depth -= 1;
                    if depth == 0 {
                        return Some(target);
                    }
                }
            }
        } else {
            for target in (0..=index).rev() {
                if !self.is_code_char(target) {
                    continue;
                }
                if characters[target] == closing {
                    depth += 1;
                } else if characters[target] == opening {
                    depth -= 1;
                    if depth == 0 {
                        return Some(target);
                    }
                }
            }
        }
        None
    }

    pub fn indent_after(&self, cursor_char: usize) -> bool {
        let Some(query) = &self.indent_query else {
            return false;
        };
        let cursor_byte = char_index_to_byte(&self.source, cursor_char);
        let line_start = self.source[..cursor_byte]
            .rfind('\n')
            .map_or(0, |newline| newline + 1);
        let mut query_cursor = QueryCursor::new();
        query_cursor.set_byte_range(line_start..cursor_byte);
        let mut captures =
            query_cursor.captures(query, self.tree.root_node(), self.source.as_bytes());
        while let Some((query_match, capture_index)) = captures.next() {
            let node = query_match.captures[*capture_index].node;
            if node.end_byte() <= cursor_byte
                && self.source[node.end_byte()..cursor_byte]
                    .chars()
                    .all(char::is_whitespace)
                && self.is_code_char(self.source[..node.start_byte()].chars().count())
            {
                return true;
            }
        }
        false
    }

    pub fn node_range_at(
        &self,
        cursor_char: usize,
        current_range: Option<(usize, usize)>,
    ) -> Option<(usize, usize)> {
        let byte = char_index_to_byte(&self.source, cursor_char);
        let mut node = self
            .tree
            .root_node()
            .descendant_for_byte_range(byte, byte.saturating_add(1).min(self.source.len()))?;
        loop {
            if node.is_named() && node.start_byte() < node.end_byte() {
                let range = (
                    self.source[..node.start_byte()].chars().count(),
                    self.source[..node.end_byte()].chars().count(),
                );
                if current_range != Some(range) {
                    return Some(range);
                }
            }
            node = node.parent()?;
        }
    }

    fn is_code_char(&self, char_index: usize) -> bool {
        let byte = char_index_to_byte(&self.source, char_index);
        let Some(mut node) = self
            .tree
            .root_node()
            .descendant_for_byte_range(byte, byte.saturating_add(1))
        else {
            return true;
        };
        loop {
            let kind = node.kind();
            if kind.contains("comment")
                || kind.contains("string")
                || kind.contains("character")
                || kind.contains("quoted")
            {
                return false;
            }
            let Some(parent) = node.parent() else {
                return true;
            };
            node = parent;
        }
    }

    #[cfg(test)]
    pub fn source_matches(&self, source: &str) -> bool {
        self.source == source
    }
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

        if matches!(extension.as_str(), "md" | "markdown") {
            return Self::Markdown;
        }
        LANGUAGE_DEFINITIONS
            .iter()
            .find(|definition| definition.extensions.contains(&extension.as_str()))
            .map_or(Self::Plain, |definition| definition.language)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::CSharp => "C#",
            Self::Yaml => "YAML",
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Shell => "Shell",
            Self::Plain => "Plain Text",
        }
    }

    pub fn comment_delimiters(self) -> Option<(&'static str, Option<&'static str>)> {
        match self {
            Self::Rust
            | Self::Go
            | Self::CSharp
            | Self::Json
            | Self::JavaScript
            | Self::TypeScript => Some(("//", None)),
            Self::Yaml | Self::Toml | Self::Python | Self::Shell => Some(("#", None)),
            Self::Markdown => Some(("<!--", Some("-->"))),
            Self::Plain => None,
        }
    }
}

pub fn highlight_line(line: &str, language: Language, theme: &Theme) -> Vec<Color> {
    let mut colors = highlight_line_base(line, language, theme);
    apply_tree_sitter_highlights(line, language, theme, &mut colors);
    colors
}

fn highlight_line_base(line: &str, language: Language, theme: &Theme) -> Vec<Color> {
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
        Language::Rust
        | Language::Go
        | Language::CSharp
        | Language::Json
        | Language::JavaScript
        | Language::TypeScript => Some("//"),
        Language::Yaml | Language::Toml | Language::Python | Language::Shell => Some("#"),
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
            '{' | '}'
                | '['
                | ']'
                | '('
                | ')'
                | ':'
                | ';'
                | ','
                | '.'
                | '='
                | '+'
                | '-'
                | '*'
                | '/'
                | '%'
                | '&'
                | '|'
                | '!'
                | '<'
                | '>'
                | '?'
        ) {
            colors[index] = theme.punctuation;
        }

        index += 1;
    }

    colors
}

fn apply_tree_sitter_highlights(
    line: &str,
    language: Language,
    theme: &Theme,
    colors: &mut [Color],
) {
    let Some(tree_language) = tree_sitter_language(language) else {
        return;
    };
    let mut parser = Parser::new();
    if parser.set_language(&tree_language).is_err() {
        return;
    }
    let Some(tree) = parser.parse(line, None) else {
        return;
    };
    apply_node_highlights(tree.root_node(), line, theme, colors);
}

fn tree_sitter_language(language: Language) -> Option<TreeLanguage> {
    language_definition(language).map(|definition| (definition.grammar)())
}

pub fn fold_ranges(source: &str, language: Language) -> Vec<(usize, usize)> {
    let Some(tree_language) = tree_sitter_language(language) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&tree_language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut ranges = Vec::new();
    collect_fold_ranges(root, root.id(), &mut ranges);
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

pub fn symbols(source: &str, language: Language) -> Vec<Symbol> {
    let Some(tree_language) = tree_sitter_language(language) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&tree_language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    collect_symbols(tree.root_node(), source, 0, &mut output);
    output
}

#[cfg(test)]
pub fn breadcrumbs(source: &str, language: Language, line: usize) -> Vec<Symbol> {
    symbols(source, language)
        .into_iter()
        .filter(|symbol| symbol.start_line <= line && line <= symbol.end_line)
        .collect()
}

fn collect_symbols(node: Node<'_>, source: &str, depth: usize, output: &mut Vec<Symbol>) {
    if let Some(kind) = symbol_kind(node.kind()) {
        let name = node
            .child_by_field_name("name")
            .or_else(|| first_identifier(node))
            .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            .unwrap_or(node.kind())
            .to_string();
        output.push(Symbol {
            name,
            kind,
            start_line: node.start_position().row,
            end_line: node.end_position().row,
            depth,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(
            child,
            source,
            depth + usize::from(symbol_kind(node.kind()).is_some()),
            output,
        );
    }
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let identifier = node
        .children(&mut cursor)
        .find(|child| child.kind().contains("identifier"));
    identifier
}

fn symbol_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "namespace_declaration" | "file_scoped_namespace_declaration" | "module" => Some("module"),
        "class_declaration"
        | "struct_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration" => Some("type"),
        "function_declaration"
        | "function_definition"
        | "method_declaration"
        | "constructor_declaration" => Some("function"),
        _ => None,
    }
}

fn collect_fold_ranges(node: Node<'_>, root_id: usize, ranges: &mut Vec<(usize, usize)>) {
    let start = node.start_position().row;
    let end = node.end_position().row;
    if node.id() != root_id && node.is_named() && start < end {
        ranges.push((start, end));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_fold_ranges(child, root_id, ranges);
    }
}

fn apply_node_highlights(node: Node<'_>, line: &str, theme: &Theme, colors: &mut [Color]) {
    let color = match node.kind() {
        kind if kind.contains("comment") => Some(theme.comment),
        kind if kind.contains("string") || kind.contains("quoted") => Some(theme.string),
        kind if kind.contains("integer") || kind.contains("float") || kind.contains("number") => {
            Some(theme.number)
        }
        kind if kind.contains("type") => Some(theme.type_name),
        _ => None,
    };
    if let Some(color) = color {
        let start = line[..node.start_byte().min(line.len())].chars().count();
        let end = line[..node.end_byte().min(line.len())].chars().count();
        let color_count = colors.len();
        let start = start.min(color_count);
        let end = end.min(color_count);
        for slot in &mut colors[start..end] {
            *slot = color;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        apply_node_highlights(child, line, theme, colors);
    }
}

fn char_index_to_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

fn point_at_byte(text: &str, byte: usize) -> Point {
    let byte = byte.min(text.len());
    let prefix = &text[..byte];
    let row = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let column = prefix
        .rfind('\n')
        .map_or(prefix.len(), |newline| prefix.len() - newline - 1);
    Point::new(row, column)
}

fn point_after_text(start: Point, text: &str) -> Point {
    let newline_count = text.bytes().filter(|byte| *byte == b'\n').count();
    if newline_count == 0 {
        Point::new(start.row, start.column + text.len())
    } else {
        Point::new(
            start.row + newline_count,
            text.rsplit_once('\n').map_or(0, |(_, tail)| tail.len()),
        )
    }
}

fn line_start_byte(source: &str, line_index: usize) -> Option<usize> {
    if line_index == 0 {
        return Some(0);
    }
    let mut remaining = line_index;
    for (byte, character) in source.char_indices() {
        if character == '\n' {
            remaining -= 1;
            if remaining == 0 {
                return Some(byte + 1);
            }
        }
    }
    None
}

fn apply_query_highlights(
    query: &Query,
    root: Node<'_>,
    source: &str,
    line_start_byte: usize,
    line: &str,
    theme: &Theme,
    colors: &mut [Color],
) {
    let line_end_byte = line_start_byte.saturating_add(line.len()).min(source.len());
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(line_start_byte..line_end_byte.saturating_add(1).min(source.len()));
    let mut captures = cursor.captures(query, root, source.as_bytes());
    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        let name = query.capture_names()[capture.index as usize];
        let Some(color) = capture_color(name, theme) else {
            continue;
        };
        let start_byte = capture
            .node
            .start_byte()
            .max(line_start_byte)
            .min(line_end_byte);
        let end_byte = capture
            .node
            .end_byte()
            .max(line_start_byte)
            .min(line_end_byte);
        let local_start = start_byte.saturating_sub(line_start_byte);
        let local_end = end_byte.saturating_sub(line_start_byte);
        let start = line[..local_start.min(line.len())].chars().count();
        let end = line[..local_end.min(line.len())].chars().count();
        let start = start.min(colors.len());
        let end = end.min(colors.len());
        for slot in &mut colors[start..end] {
            *slot = color;
        }
    }
}

fn capture_color(name: &str, theme: &Theme) -> Option<Color> {
    if name.contains("comment") {
        Some(theme.comment)
    } else if name.contains("string") || name.contains("character") {
        Some(theme.string)
    } else if name.contains("number") || name.contains("float") {
        Some(theme.number)
    } else if name.contains("type")
        || name.contains("constructor")
        || name.contains("namespace")
        || name.contains("module")
    {
        Some(theme.type_name)
    } else if name.contains("keyword") || name.contains("boolean") || name.contains("constant") {
        Some(theme.keyword)
    } else if name.contains("function") || name.contains("method") || name.contains("property") {
        Some(theme.heading)
    } else if name.contains("operator") || name.contains("punctuation") {
        Some(theme.punctuation)
    } else {
        None
    }
}

fn is_bracket(character: char) -> bool {
    matches!(character, '(' | ')' | '[' | ']' | '{' | '}')
}

fn bracket_pair(character: char) -> Option<(char, char, bool)> {
    match character {
        '(' => Some(('(', ')', true)),
        '[' => Some(('[', ']', true)),
        '{' => Some(('{', '}', true)),
        ')' => Some(('(', ')', false)),
        ']' => Some(('[', ']', false)),
        '}' => Some(('{', '}', false)),
        _ => None,
    }
}

fn apply_document_node_highlights(
    node: Node<'_>,
    line_index: usize,
    line_start_byte: usize,
    line: &str,
    theme: &Theme,
    colors: &mut [Color],
) {
    if node.start_position().row > line_index || node.end_position().row < line_index {
        return;
    }

    let color = match node.kind() {
        kind if kind.contains("comment") => Some(theme.comment),
        kind if kind.contains("string") || kind.contains("quoted") => Some(theme.string),
        kind if kind.contains("integer") || kind.contains("float") || kind.contains("number") => {
            Some(theme.number)
        }
        kind if kind.contains("type") => Some(theme.type_name),
        _ => None,
    };
    if let Some(color) = color {
        let line_end_byte = line_start_byte.saturating_add(line.len());
        let start_byte = node.start_byte().max(line_start_byte).min(line_end_byte);
        let end_byte = node.end_byte().max(line_start_byte).min(line_end_byte);
        let local_start = start_byte.saturating_sub(line_start_byte);
        let local_end = end_byte.saturating_sub(line_start_byte);
        let start = line[..local_start.min(line.len())].chars().count();
        let end = line[..local_end.min(line.len())].chars().count();
        let start = start.min(colors.len());
        let end = end.min(colors.len());
        for slot in &mut colors[start..end] {
            *slot = color;
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        apply_document_node_highlights(child, line_index, line_start_byte, line, theme, colors);
    }
}

fn highlight_markdown(chars: &[char], colors: &mut [Color], theme: &Theme) {
    let first_non_space = chars
        .iter()
        .position(|character| !character.is_whitespace());

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
            "as" | "async"
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
        Language::CSharp => matches!(
            token,
            "abstract"
                | "as"
                | "async"
                | "await"
                | "base"
                | "break"
                | "case"
                | "catch"
                | "checked"
                | "class"
                | "const"
                | "continue"
                | "default"
                | "delegate"
                | "do"
                | "else"
                | "enum"
                | "event"
                | "explicit"
                | "extern"
                | "finally"
                | "fixed"
                | "for"
                | "foreach"
                | "goto"
                | "if"
                | "implicit"
                | "in"
                | "interface"
                | "internal"
                | "is"
                | "lock"
                | "namespace"
                | "new"
                | "operator"
                | "out"
                | "override"
                | "params"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "ref"
                | "return"
                | "sealed"
                | "sizeof"
                | "stackalloc"
                | "static"
                | "struct"
                | "switch"
                | "this"
                | "throw"
                | "try"
                | "typeof"
                | "unchecked"
                | "unsafe"
                | "using"
                | "virtual"
                | "void"
                | "volatile"
                | "while"
                | "yield"
                | "true"
                | "false"
                | "null"
        ),
        Language::Go => matches!(
            token,
            "break"
                | "case"
                | "chan"
                | "const"
                | "continue"
                | "default"
                | "defer"
                | "else"
                | "fallthrough"
                | "for"
                | "func"
                | "go"
                | "goto"
                | "if"
                | "import"
                | "interface"
                | "map"
                | "package"
                | "range"
                | "return"
                | "select"
                | "struct"
                | "switch"
                | "type"
                | "var"
        ),
        Language::JavaScript | Language::TypeScript => matches!(
            token,
            "as" | "async"
                | "await"
                | "break"
                | "case"
                | "catch"
                | "class"
                | "const"
                | "continue"
                | "debugger"
                | "default"
                | "delete"
                | "do"
                | "else"
                | "enum"
                | "export"
                | "extends"
                | "false"
                | "finally"
                | "for"
                | "from"
                | "function"
                | "get"
                | "if"
                | "implements"
                | "import"
                | "in"
                | "instanceof"
                | "interface"
                | "let"
                | "new"
                | "null"
                | "of"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "readonly"
                | "return"
                | "set"
                | "static"
                | "super"
                | "switch"
                | "this"
                | "throw"
                | "true"
                | "try"
                | "type"
                | "typeof"
                | "undefined"
                | "var"
                | "void"
                | "while"
                | "with"
                | "yield"
        ),
        Language::Yaml => matches!(token, "true" | "false" | "null" | "yes" | "no"),
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
        Language::CSharp => matches!(
            token,
            "bool"
                | "byte"
                | "char"
                | "decimal"
                | "double"
                | "dynamic"
                | "float"
                | "int"
                | "long"
                | "object"
                | "sbyte"
                | "short"
                | "string"
                | "uint"
                | "ulong"
                | "ushort"
                | "var"
                | "DateTime"
                | "Guid"
                | "Task"
        ),
        Language::Go => matches!(
            token,
            "bool"
                | "byte"
                | "complex64"
                | "complex128"
                | "error"
                | "float32"
                | "float64"
                | "int"
                | "int8"
                | "int16"
                | "int32"
                | "int64"
                | "rune"
                | "string"
                | "uint"
                | "uint8"
                | "uint16"
                | "uint32"
                | "uint64"
                | "uintptr"
        ),
        Language::Python => false,
        Language::JavaScript | Language::TypeScript => matches!(
            token,
            "any"
                | "bigint"
                | "boolean"
                | "never"
                | "number"
                | "object"
                | "string"
                | "symbol"
                | "unknown"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_tree_sitter_languages_parse_valid_source() {
        for (language, source) in [
            (Language::Rust, "fn main() { let value: u32 = 42; }"),
            (
                Language::Go,
                "package main\nfunc main() { value := 42; _ = value }",
            ),
            (
                Language::CSharp,
                "class Program { static void Main() { var value = 42; } }",
            ),
            (Language::Yaml, "value: 42\nitems:\n  - one"),
            (Language::Json, r#"{"value": 42}"#),
            (Language::Toml, "value = 42"),
            (Language::Python, "def main():\n    return 42"),
            (
                Language::JavaScript,
                "function main() { const value = 42; return value; }",
            ),
            (
                Language::TypeScript,
                "function main(value: number): number { return value; }",
            ),
            (Language::Shell, "value=42\necho $value"),
        ] {
            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_language(language).expect("configured grammar"))
                .expect("load grammar");
            let tree = parser.parse(source, None).expect("parse source");
            assert!(!tree.root_node().has_error(), "{language:?}");
            let definition = language_definition(language).expect("language definition");
            let grammar = tree_sitter_language(language).unwrap();
            assert!(
                Query::new(&grammar, definition.highlights).is_ok(),
                "highlight query for {language:?}"
            );
            assert!(
                Query::new(&grammar, definition.indents).is_ok(),
                "indent query for {language:?}"
            );
        }
    }

    #[test]
    fn csharp_keywords_are_colored_as_keywords() {
        let theme = Theme::for_kind(crate::theme::ThemeKind::Oxide);
        let line = "public class CustomsValueService { private double value; }";
        let colors = highlight_line(line, Language::CSharp, &theme);

        for keyword in ["public", "class", "private"] {
            let index = line.find(keyword).expect("keyword index");
            assert_eq!(colors[index], theme.keyword, "{keyword}");
        }
        let type_index = line.find("double").expect("type index");
        assert_eq!(colors[type_index], theme.type_name);
    }

    #[test]
    fn finds_multiline_syntax_folds() {
        let ranges = fold_ranges(
            "fn main() {\n    if true {\n        work();\n    }\n}\n",
            Language::Rust,
        );
        assert!(ranges.contains(&(0, 4)));
        assert!(ranges.contains(&(1, 3)));
    }

    #[test]
    fn syntax_document_incrementally_updates_multiline_source() {
        let mut document =
            SyntaxDocument::new(Some(Path::new("main.rs")), "fn main() {\n    work();\n}\n")
                .expect("Rust syntax document");

        document.apply_edit(16, 7, "if ready {\n        work();\n    }");

        let expected = "fn main() {\n    if ready {\n        work();\n    }\n}\n";
        assert!(document.source_matches(expected));
        assert!(document.fold_ranges().contains(&(0, 4)));
    }

    #[test]
    fn syntax_aware_bracket_matching_ignores_strings_and_comments() {
        let source = "fn main() { let text = \"}\"; /* } */ work(); }";
        let document =
            SyntaxDocument::new(Some(Path::new("main.rs")), source).expect("Rust syntax document");
        let opening = source.find('{').unwrap();
        let closing = source.rfind('}').unwrap();

        assert_eq!(document.matching_bracket(opening), Some(closing));
        assert_eq!(
            document.matching_bracket(source.find("\"}\"").unwrap() + 1),
            None
        );
        assert!(document.indent_after(opening + 1));

        let cursor = source.find("work").unwrap();
        let inner = document.node_range_at(cursor, None).unwrap();
        let outer = document.node_range_at(cursor, Some(inner)).unwrap();
        assert!(outer.0 <= inner.0 && outer.1 >= inner.1);
        assert_ne!(outer, inner);
    }

    #[test]
    fn extracts_nested_csharp_symbols_for_outline_and_breadcrumbs() {
        let source = "namespace Demo { class Program { void Run() { } } }";
        let symbols = symbols(source, Language::CSharp);
        assert!(symbols.iter().any(|symbol| symbol.name == "Demo"));
        assert!(symbols.iter().any(|symbol| symbol.name == "Program"));
        assert!(symbols.iter().any(|symbol| symbol.name == "Run"));
        assert_eq!(
            breadcrumbs(source, Language::CSharp, 0)
                .last()
                .map(|symbol| symbol.name.as_str()),
            Some("Run")
        );
    }
}
