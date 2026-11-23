//! Plain-text Find & Replace (case-sensitive and case-insensitive).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchRange {
    pub start: usize,
    pub end: usize,
}

/// Find the next occurrence of `needle` in `haystack` starting at `from_byte`.
/// Wraps around from the start if not found after `from_byte`.
pub fn find_next(
    haystack: &str,
    needle: &str,
    from_byte: usize,
    case_sensitive: bool,
) -> Option<MatchRange> {
    if needle.is_empty() || haystack.is_empty() {
        return None;
    }
    let from = clamp_boundary(haystack, from_byte);
    if case_sensitive {
        if let Some(rel) = haystack[from..].find(needle) {
            let s = from + rel;
            return Some(MatchRange {
                start: s,
                end: s + needle.len(),
            });
        }
        if let Some(s) = haystack[..from.min(haystack.len())].find(needle) {
            return Some(MatchRange {
                start: s,
                end: s + needle.len(),
            });
        }
        return None;
    }

    let matches = find_all(haystack, needle, false);
    if matches.is_empty() {
        return None;
    }
    for m in &matches {
        if m.start >= from {
            return Some(*m);
        }
    }
    Some(matches[0])
}

/// Find the previous occurrence of `needle` strictly before `before_byte`.
/// Wraps around to the last match if none exists before `before_byte`.
pub fn find_prev(
    haystack: &str,
    needle: &str,
    before_byte: usize,
    case_sensitive: bool,
) -> Option<MatchRange> {
    if needle.is_empty() || haystack.is_empty() {
        return None;
    }
    let matches = find_all(haystack, needle, case_sensitive);
    if matches.is_empty() {
        return None;
    }
    for m in matches.iter().rev() {
        if m.start < before_byte {
            return Some(*m);
        }
    }
    matches.last().copied()
}

/// Find all non-overlapping matches of `needle` in `haystack`.
pub fn find_all(haystack: &str, needle: &str, case_sensitive: bool) -> Vec<MatchRange> {
    let mut out = Vec::new();
    if needle.is_empty() || haystack.is_empty() {
        return out;
    }
    if case_sensitive {
        let mut offset = 0;
        while offset <= haystack.len() {
            if let Some(rel) = haystack[offset..].find(needle) {
                let s = offset + rel;
                let e = s + needle.len();
                out.push(MatchRange { start: s, end: e });
                offset = e;
            } else {
                break;
            }
        }
        return out;
    }

    // Fast ASCII path when needle is pure ASCII (covers >99% of searches without allocations)
    if needle.is_ascii() {
        let hb = haystack.as_bytes();
        let nb = needle.as_bytes();
        let nlen = nb.len();
        if hb.len() < nlen {
            return out;
        }
        let mut i = 0;
        while i + nlen <= hb.len() {
            if hb[i..i + nlen].eq_ignore_ascii_case(nb)
                && haystack.is_char_boundary(i)
                && haystack.is_char_boundary(i + nlen)
            {
                out.push(MatchRange {
                    start: i,
                    end: i + nlen,
                });
                i += nlen;
            } else {
                i += 1;
            }
        }
        return out;
    }

    // Full Unicode case-folded fallback preserving original byte indices
    let needle_chars: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    let hay_chars: Vec<(usize, char)> = haystack
        .char_indices()
        .flat_map(|(idx, ch)| ch.to_lowercase().map(move |lc| (idx, lc)))
        .collect();
    if needle_chars.is_empty() || hay_chars.len() < needle_chars.len() {
        return out;
    }
    let mut i = 0;
    while i + needle_chars.len() <= hay_chars.len() {
        let mut matched = true;
        for j in 0..needle_chars.len() {
            if hay_chars[i + j].1 != needle_chars[j] {
                matched = false;
                break;
            }
        }
        if matched {
            let start = hay_chars[i].0;
            let last_orig_start = hay_chars[i + needle_chars.len() - 1].0;
            let end = haystack[last_orig_start..]
                .chars()
                .next()
                .map(|c| last_orig_start + c.len_utf8())
                .unwrap_or(haystack.len());
            out.push(MatchRange { start, end });
            i += needle_chars.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Replace all occurrences of `needle` with `replacement`, returning `(new_text, count)`.
pub fn replace_all(
    haystack: &str,
    needle: &str,
    replacement: &str,
    case_sensitive: bool,
) -> (String, usize) {
    let matches = find_all(haystack, needle, case_sensitive);
    if matches.is_empty() {
        return (haystack.to_string(), 0);
    }
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for m in &matches {
        out.push_str(&haystack[last..m.start]);
        out.push_str(replacement);
        last = m.end;
    }
    out.push_str(&haystack[last..]);
    (out, matches.len())
}

fn clamp_boundary(s: &str, mut idx: usize) -> usize {
    if idx > s.len() {
        idx = s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_and_replace_case_insensitive() {
        let text = "Kadex is fast. KADEX is lean.";
        let m1 = find_next(text, "kadex", 0, false).unwrap();
        assert_eq!((m1.start, m1.end), (0, 5));
        let m2 = find_next(text, "kadex", m1.end, false).unwrap();
        assert_eq!((m2.start, m2.end), (15, 20));
        let m3 = find_next(text, "kadex", m2.end, false).unwrap();
        assert_eq!((m3.start, m3.end), (0, 5)); // wrap

        let (replaced, count) = replace_all(text, "kadex", "Note", false);
        assert_eq!(count, 2);
        assert_eq!(replaced, "Note is fast. Note is lean.");
    }
}
