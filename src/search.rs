//! Shared text-search engine.  One compiled query drives in-file find,
//! match highlighting, replace, and project-wide search, so every feature
//! agrees on what matches.

use regex::RegexBuilder;

/// User-visible search options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
}

/// A compiled search query.  Literal queries are escaped and share the regex
/// path so case-insensitive matching never changes match offsets.
#[derive(Debug, Clone)]
pub struct CompiledSearch {
    regex: regex::Regex,
    pub options: SearchOptions,
    pub pattern: String,
}

impl CompiledSearch {
    pub fn compile(pattern: &str, options: SearchOptions) -> Result<Self, String> {
        let source = if options.use_regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        let regex = RegexBuilder::new(&source)
            .case_insensitive(!options.case_sensitive)
            .build()
            .map_err(|error| match error {
                regex::Error::Syntax(message) => {
                    let reason = message
                        .lines()
                        .rev()
                        .find(|line| !line.trim().is_empty() && !line.contains('^'))
                        .unwrap_or("invalid pattern");
                    format!("Regex error: {}", reason.trim())
                }
                other => format!("Regex error: {other}"),
            })?;
        Ok(Self {
            regex,
            options,
            pattern: pattern.to_string(),
        })
    }

    /// Byte ranges of every match in `text`, honoring the whole-word option.
    /// Zero-width matches are skipped so `a*` never highlights nothing.
    pub fn find_byte_ranges(&self, text: &str) -> Vec<(usize, usize)> {
        if self.pattern.is_empty() {
            return Vec::new();
        }
        let mut ranges = Vec::new();
        let mut from = 0usize;
        while from <= text.len() {
            let Some(found) = self.regex.find_at(text, from) else {
                break;
            };
            let (start, end) = (found.start(), found.end());
            from = if end > start {
                end
            } else {
                match text[start..].chars().next() {
                    Some(character) => start + character.len_utf8(),
                    None => break,
                }
            };
            if end == start {
                continue;
            }
            if self.options.whole_word && !is_whole_word(text, start, end) {
                continue;
            }
            ranges.push((start, end));
        }
        ranges
    }

    /// The replacement text for one match, expanding `$1`-style capture
    /// references in regex mode.
    pub fn expand_replacement(
        &self,
        text: &str,
        start: usize,
        end: usize,
        replacement: &str,
    ) -> String {
        if !self.options.use_regex {
            return replacement.to_string();
        }
        if let Some(captures) = self.regex.captures_at(text, start) {
            if captures
                .get(0)
                .is_some_and(|whole| whole.start() == start && whole.end() == end)
            {
                let mut destination = String::new();
                captures.expand(replacement, &mut destination);
                return destination;
            }
        }
        replacement.to_string()
    }
}

/// Converts byte ranges into char ranges with a single pass over `text`.
pub fn byte_to_char_ranges(text: &str, ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut boundaries: Vec<usize> = ranges
        .iter()
        .flat_map(|(start, end)| [*start, *end])
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut byte_to_char = std::collections::HashMap::new();
    let mut next = boundaries.iter().copied().peekable();
    for (char_index, (byte_index, _)) in text.char_indices().enumerate() {
        while next.peek().is_some_and(|boundary| *boundary <= byte_index) {
            let boundary = next.next().unwrap();
            byte_to_char.insert(
                boundary,
                if boundary == byte_index {
                    char_index
                } else {
                    // Boundary inside a char cannot happen for match offsets;
                    // fall back to the containing character.
                    char_index
                },
            );
        }
    }
    let total_chars = text.chars().count();
    for boundary in next {
        byte_to_char.insert(boundary, total_chars);
    }

    ranges
        .iter()
        .map(|(start, end)| (byte_to_char[start], byte_to_char[end]))
        .collect()
}

/// Byte offset of a char index with a single pass over `text`.
pub fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte_index, _)| byte_index)
}

fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn is_word_char(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(pattern: &str, options: SearchOptions) -> CompiledSearch {
        CompiledSearch::compile(pattern, options).expect("compile search")
    }

    #[test]
    fn literal_search_is_case_insensitive_by_default() {
        let search = compile("hello", SearchOptions::default());
        assert_eq!(
            search.find_byte_ranges("Hello hello HELLO"),
            vec![(0, 5), (6, 11), (12, 17)]
        );
    }

    #[test]
    fn case_sensitive_search_narrows_matches() {
        let search = compile(
            "hello",
            SearchOptions {
                case_sensitive: true,
                ..Default::default()
            },
        );
        assert_eq!(search.find_byte_ranges("Hello hello HELLO"), vec![(6, 11)]);
    }

    #[test]
    fn whole_word_filters_substrings() {
        let search = compile(
            "cat",
            SearchOptions {
                whole_word: true,
                ..Default::default()
            },
        );
        assert_eq!(
            search.find_byte_ranges("cat catalog wildcat cat"),
            vec![(0, 3), (20, 23)]
        );
    }

    #[test]
    fn regex_mode_supports_captures_in_replacements() {
        let search = compile(
            r"(\w+)@(\w+)",
            SearchOptions {
                use_regex: true,
                ..Default::default()
            },
        );
        let text = "user@host";
        let ranges = search.find_byte_ranges(text);
        assert_eq!(ranges, vec![(0, 9)]);
        assert_eq!(search.expand_replacement(text, 0, 9, "$2:$1"), "host:user");
    }

    #[test]
    fn invalid_regex_reports_a_readable_error() {
        let error = CompiledSearch::compile(
            "(unclosed",
            SearchOptions {
                use_regex: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.starts_with("Regex error:"), "{error}");
    }

    #[test]
    fn literal_mode_escapes_regex_metacharacters() {
        let search = compile("a.b(", SearchOptions::default());
        assert_eq!(search.find_byte_ranges("a.b( axb("), vec![(0, 4)]);
    }

    #[test]
    fn zero_width_regex_matches_are_skipped() {
        let search = compile(
            "x*",
            SearchOptions {
                use_regex: true,
                ..Default::default()
            },
        );
        assert_eq!(search.find_byte_ranges("axxa"), vec![(1, 3)]);
    }

    #[test]
    fn byte_ranges_convert_to_char_ranges_with_unicode() {
        let text = "héllo héllo";
        let search = compile("héllo", SearchOptions::default());
        let bytes = search.find_byte_ranges(text);
        let chars = byte_to_char_ranges(text, &bytes);
        assert_eq!(chars, vec![(0, 5), (6, 11)]);
    }
}
