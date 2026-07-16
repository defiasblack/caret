//! Small fuzzy matcher for the file picker.  Case-insensitive subsequence
//! matching with bonuses for word-boundary and consecutive hits, so
//! "apprs" finds "src/app.rs" without external dependencies.

/// Scores `candidate` against `query`.  Returns the score and the char
/// positions that matched, or None when `query` is not a subsequence.
/// Higher scores are better.
pub fn match_score(candidate: &str, query: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }

    let candidate_chars: Vec<char> = candidate.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    if query_chars.len() > candidate_chars.len() {
        return None;
    }

    let mut score = 0i32;
    let mut positions = Vec::with_capacity(query_chars.len());
    let mut candidate_index = 0usize;
    let mut previous_match: Option<usize> = None;

    for query_char in &query_chars {
        let mut found = None;
        while candidate_index < candidate_chars.len() {
            if chars_equal_fold(candidate_chars[candidate_index], *query_char) {
                found = Some(candidate_index);
                break;
            }
            candidate_index += 1;
        }
        let index = found?;

        score += 4;
        if is_word_boundary(&candidate_chars, index) {
            score += 8;
        }
        if candidate_chars[index].is_uppercase() && *query_char == candidate_chars[index] {
            score += 2;
        }
        match previous_match {
            Some(previous) if index == previous + 1 => score += 10,
            Some(previous) => score -= ((index - previous - 1).min(8)) as i32,
            None => score -= (index.min(12) / 3) as i32,
        }

        positions.push(index);
        previous_match = Some(index);
        candidate_index = index + 1;
    }

    // Prefer shorter candidates when hits are otherwise equal.
    score -= (candidate_chars.len() / 8) as i32;
    Some((score, positions))
}

fn chars_equal_fold(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

fn is_word_boundary(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = chars[index - 1];
    matches!(previous, '/' | '\\' | '_' | '-' | '.' | ' ')
        || (previous.is_lowercase() && chars[index].is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_case_insensitive_subsequences() {
        assert!(match_score("src/app.rs", "apprs").is_some());
        assert!(match_score("src/app.rs", "APP").is_some());
        assert!(match_score("src/app.rs", "xyz").is_none());
        assert!(match_score("short", "longer-than-candidate").is_none());
    }

    #[test]
    fn word_boundary_matches_beat_scattered_matches() {
        let (boundary, _) = match_score("src/main.rs", "main").unwrap();
        let (scattered, _) = match_score("submarine.rs", "main").unwrap();
        assert!(boundary > scattered);
    }

    #[test]
    fn consecutive_matches_beat_gapped_matches() {
        let (consecutive, _) = match_score("config.rs", "conf").unwrap();
        let (gapped, _) = match_score("c_o_n_f.rs", "conf").unwrap();
        assert!(consecutive > gapped);
    }

    #[test]
    fn positions_point_at_the_matched_characters() {
        let (_, positions) = match_score("src/app.rs", "app").unwrap();
        assert_eq!(positions, vec![4, 5, 6]);
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(match_score("anything", ""), Some((0, Vec::new())));
    }
}
