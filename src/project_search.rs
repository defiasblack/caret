//! Project-wide search and replace.  Walks the project with gitignore and
//! hidden-file rules, matches lines with the shared search engine, and
//! rewrites files atomically so replacements stay predictable.

use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::search::CompiledSearch;
#[cfg(test)]
use std::{collections::HashSet, fs};

/// Files larger than this are skipped; they are almost never sources.
const MAX_FILE_BYTES: u64 = 2_000_000;

/// One match inside one file.
#[derive(Debug, Clone)]
pub struct ProjectMatch {
    pub path: PathBuf,
    pub fingerprint: u64,
    pub document_start: usize,
    pub document_end: usize,
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
#[cfg(test)]
pub fn search(root: &Path, query: &CompiledSearch, max_results: usize) -> ProjectSearchResults {
    search_cancelled(root, query, max_results, &AtomicBool::new(false))
}
pub fn search_cancelled(
    root: &Path,
    query: &CompiledSearch,
    max_results: usize,
    cancelled: &AtomicBool,
) -> ProjectSearchResults {
    let mut results = ProjectSearchResults::default();
    let walker = ignore::WalkBuilder::new(root)
        .sort_by_file_path(std::cmp::Ord::cmp)
        // Respect .gitignore files even when the folder is not a git repo.
        .require_git(false)
        .build();
    for entry in walker.flatten() {
        if results.truncated || cancelled.load(Ordering::Relaxed) {
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
        let Ok(snapshot) = crate::document::read_snapshot(entry.path()) else {
            continue;
        };
        let text = snapshot.text;
        let mut matched_file = false;
        let mut line_start = 0;
        for (line_index, raw_line) in text.split_inclusive('\n').enumerate() {
            if cancelled.load(Ordering::Relaxed) {
                return results;
            }
            let line = raw_line
                .strip_suffix('\n')
                .unwrap_or(raw_line)
                .strip_suffix('\r')
                .unwrap_or(raw_line.strip_suffix('\n').unwrap_or(raw_line));
            for (start, end) in query.find_byte_ranges(line) {
                if results.matches.len() >= max_results {
                    results.truncated = true;
                    break;
                }
                matched_file = true;
                results.matches.push(ProjectMatch {
                    path: entry.path().to_path_buf(),
                    fingerprint: snapshot.fingerprint,
                    document_start: line_start + start,
                    document_end: line_start + end,
                    line: line_index,
                    byte_start: start,
                    byte_end: end,
                    line_text: line.to_string(),
                });
            }
            line_start += raw_line.len();
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
#[cfg(test)]
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
#[cfg(test)]
pub fn replace_in_file(
    path: &Path,
    query: &CompiledSearch,
    replacement: &str,
    excluded: &HashSet<(usize, usize)>,
) -> io::Result<usize> {
    let snapshot = crate::document::read_snapshot(path)?;
    let expected_fingerprint = snapshot.fingerprint;
    let (text, format) = (snapshot.text, snapshot.format);
    let (new_text, replaced) = replace_in_text(&text, query, replacement, excluded);
    if replaced > 0 && new_text != text {
        // read_text strips a BOM; put it back so the file round-trips.
        let mut bytes = Vec::with_capacity(new_text.len() + 3);
        if format.utf8_bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(new_text.as_bytes());
        crate::document::atomic_write_if_unchanged(path, Some(expected_fingerprint), &bytes)?;
    }
    Ok(replaced)
}

/// Apply only reviewed ranges against exactly the bytes displayed by search.
pub fn replace_previewed(
    path: &Path,
    query: &CompiledSearch,
    replacement: &str,
    matches: &[ProjectMatch],
) -> io::Result<usize> {
    let snapshot = crate::document::read_snapshot(path)?;
    if matches
        .iter()
        .any(|found| found.fingerprint != snapshot.fingerprint)
    {
        return Err(io::Error::other(
            "file changed since replacement preview; search again",
        ));
    }
    let mut sorted: Vec<_> = matches.iter().collect();
    sorted.sort_by_key(|found| found.document_start);
    if sorted
        .windows(2)
        .any(|pair| pair[0].document_end > pair[1].document_start)
    {
        return Err(io::Error::other(
            "replacement preview contains overlapping ranges",
        ));
    }
    let mut text = snapshot.text;
    for found in sorted.into_iter().rev() {
        if text.get(found.document_start..found.document_end)
            != found.line_text.get(found.byte_start..found.byte_end)
        {
            return Err(io::Error::other("invalid replacement preview"));
        }
        let replacement = query.expand_replacement(
            &found.line_text,
            found.byte_start,
            found.byte_end,
            replacement,
        );
        text.replace_range(found.document_start..found.document_end, &replacement);
    }
    let mut bytes = Vec::new();
    if snapshot.format.utf8_bom {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(text.as_bytes());
    crate::document::atomic_write_resolved(
        path,
        &snapshot.resolved_path,
        Some(Some(snapshot.fingerprint)),
        &bytes,
    )?;
    Ok(matches.len())
}

pub struct SearchRequest {
    pub root: PathBuf,
    pub query: CompiledSearch,
    pub limit: usize,
}
pub type SearchWorker = crate::worker::LatestWorker<SearchRequest, ProjectSearchResults>;
pub fn search_worker() -> SearchWorker {
    SearchWorker::new("caret-project-search", |request, cancelled| {
        search_cancelled(&request.root, &request.query, request.limit, cancelled)
    })
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

#[cfg(test)]
mod replacement_regressions {
    use super::*;
    fn fixture(name: &str, bytes: &[u8]) -> (PathBuf, PathBuf, CompiledSearch) {
        let root =
            std::env::temp_dir().join(format!("caret-preview-{name}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("test.txt");
        fs::write(&path, bytes).unwrap();
        (
            root,
            path,
            CompiledSearch::compile("needle", Default::default()).unwrap(),
        )
    }
    #[test]
    fn preserves_bom_mixed_endings_and_unselected_matches() {
        let (root, path, query) = fixture("endings", b"\xef\xbb\xbfneedle\r\nneedle\nneedle\r\n");
        let results = search(&root, &query, 50);
        replace_previewed(&path, &query, "changed", &results.matches[1..2]).unwrap();
        assert_eq!(
            fs::read(path).unwrap(),
            b"\xef\xbb\xbfneedle\r\nchanged\nneedle\r\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_new_matches_or_rewritten_files_after_preview() {
        let (root, path, query) = fixture("stale", b"needle\n");
        let results = search(&root, &query, 50);
        fs::write(&path, "needle\nneedle\n").unwrap();
        assert!(replace_previewed(&path, &query, "changed", &results.matches).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "needle\nneedle\n");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelled_search_does_not_walk_the_project() {
        let (root, _path, query) = fixture("cancelled", b"needle\n");
        assert_eq!(search(&root, &query, 50).matches.len(), 1);
        let results = search_cancelled(&root, &query, 50, &AtomicBool::new(true));
        assert!(results.matches.is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
