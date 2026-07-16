//! Project-wide search and replace.  Walks the project with gitignore and
//! hidden-file rules, matches lines with the shared search engine, and
//! rewrites files atomically so replacements stay predictable.

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use crate::search::CompiledSearch;

/// Files larger than this are skipped; they are almost never sources.
const MAX_FILE_BYTES: u64 = 2_000_000;

/// One match inside one file.
#[derive(Debug, Clone)]
pub struct ProjectMatch {
    pub path: PathBuf,
    /// 0-based line number.
    pub line: usize,
    /// Byte range of the match within `line_text`.
    pub byte_start: usize,
    pub byte_end: usize,
    /// The matched line, for display in the results list.
    pub line_text: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectSearchResults {
    pub matches: Vec<ProjectMatch>,
    pub files_with_matches: usize,
    pub truncated: bool,
}

/// Searches every text file under `root`, honoring .gitignore, skipping
/// hidden and binary files, and stopping after `max_results` matches.
pub fn search(root: &Path, query: &CompiledSearch, max_results: usize) -> ProjectSearchResults {
    let mut results = ProjectSearchResults::default();
    let walker = ignore::WalkBuilder::new(root)
        .sort_by_file_path(std::cmp::Ord::cmp)
        // Respect .gitignore files even when the folder is not a git repo.
        .require_git(false)
        .build();
    for entry in walker.flatten() {
        if results.truncated {
            break;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if entry
            .metadata()
            .map(|metadata| metadata.len() > MAX_FILE_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };

        let mut matched_file = false;
        for (line_index, line) in text.lines().enumerate() {
            for (start, end) in query.find_byte_ranges(line) {
                if results.matches.len() >= max_results {
                    results.truncated = true;
                    break;
                }
                matched_file = true;
                results.matches.push(ProjectMatch {
                    path: entry.path().to_path_buf(),
                    line: line_index,
                    byte_start: start,
                    byte_end: end,
                    line_text: line.to_string(),
                });
            }
            if results.truncated {
                break;
            }
        }
        if matched_file {
            results.files_with_matches += 1;
        }
    }
    results
}

/// Applies `replacement` to every match in `text`, skipping matches whose
/// (line, byte_start) is in `excluded`.  Returns the new text and the number
/// of replacements.  Matching is line-based, mirroring `search`.
pub fn replace_in_text(
    text: &str,
    query: &CompiledSearch,
    replacement: &str,
    excluded: &HashSet<(usize, usize)>,
) -> (String, usize) {
    let mut replaced = 0usize;
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let had_final_newline = text.ends_with('\n');

    let mut lines = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let ranges = query.find_byte_ranges(line);
        if ranges.is_empty() {
            lines.push(line.to_string());
            continue;
        }
        let mut new_line = String::with_capacity(line.len());
        let mut copied = 0usize;
        for (start, end) in ranges {
            if excluded.contains(&(line_index, start)) {
                continue;
            }
            new_line.push_str(&line[copied..start]);
            new_line.push_str(&query.expand_replacement(line, start, end, replacement));
            copied = end;
            replaced += 1;
        }
        new_line.push_str(&line[copied..]);
        lines.push(new_line);
    }

    let mut new_text = lines.join(ending);
    if had_final_newline {
        new_text.push_str(ending);
    }
    (new_text, replaced)
}

/// Rewrites one file on disk atomically.  Returns the number of replacements.
pub fn replace_in_file(
    path: &Path,
    query: &CompiledSearch,
    replacement: &str,
    excluded: &HashSet<(usize, usize)>,
) -> io::Result<usize> {
    let (text, format) = crate::document::read_text(path)?;
    let (new_text, replaced) = replace_in_text(&text, query, replacement, excluded);
    if replaced > 0 && new_text != text {
        // read_text strips a BOM; put it back so the file round-trips.
        let mut bytes = Vec::with_capacity(new_text.len() + 3);
        if format.utf8_bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(new_text.as_bytes());
        crate::document::atomic_write(path, &bytes)?;
    }
    Ok(replaced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchOptions;

    fn temp_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "caret-project-search-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        root
    }

    fn compiled(pattern: &str) -> CompiledSearch {
        CompiledSearch::compile(pattern, SearchOptions::default()).expect("compile")
    }

    #[test]
    fn search_finds_matches_and_respects_gitignore() {
        let root = temp_project("gitignore");
        fs::write(root.join("src/main.rs"), "fn alpha() {}\nalpha();\n").unwrap();
        fs::write(root.join("notes.txt"), "alpha note\n").unwrap();
        fs::write(root.join("ignored.log"), "alpha ignored\n").unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();

        let results = search(&root, &compiled("alpha"), 100);
        assert_eq!(results.matches.len(), 3);
        assert_eq!(results.files_with_matches, 2);
        assert!(!results.truncated);
        assert!(results
            .matches
            .iter()
            .all(|found| !found.path.ends_with("ignored.log")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_skips_binary_files_and_caps_results() {
        let root = temp_project("binary");
        fs::write(root.join("data.bin"), b"alpha\0alpha").unwrap();
        fs::write(root.join("many.txt"), "alpha\n".repeat(10)).unwrap();

        let results = search(&root, &compiled("alpha"), 4);
        assert_eq!(results.matches.len(), 4);
        assert!(results.truncated);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replace_in_text_honors_exclusions_and_line_endings() {
        let query = compiled("old");
        let text = "old one\r\nkeep old\r\n";
        let mut excluded = HashSet::new();
        excluded.insert((1usize, 5usize));

        let (new_text, replaced) = replace_in_text(text, &query, "new", &excluded);
        assert_eq!(replaced, 1);
        assert_eq!(new_text, "new one\r\nkeep old\r\n");
    }

    #[test]
    fn replace_in_file_rewrites_matches_atomically() {
        let root = temp_project("rewrite");
        let file = root.join("src/lib.rs");
        fs::write(&file, "count(); count();\n").unwrap();

        let replaced =
            replace_in_file(&file, &compiled("count"), "total", &HashSet::new()).unwrap();
        assert_eq!(replaced, 2);
        assert_eq!(fs::read_to_string(&file).unwrap(), "total(); total();\n");
        let _ = fs::remove_dir_all(root);
    }
}
