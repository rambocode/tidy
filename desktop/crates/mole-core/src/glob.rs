// Bash `[[ string == pattern ]]` fnmatch semantics, which the shell layer uses
// for every bundle/path pattern: case-sensitive by default, and `*` crosses `/`
// (patterns are applied to whole paths as well as bundle ids). globset/glob
// special-case the path separator, so the matcher is implemented directly.

/// Match `text` against a bash-style glob (`*`, `?`, `[...]`, `\` escape).
pub fn fnmatch(text: &str, pattern: &str) -> bool {
    match_bytes(text.as_bytes(), pattern.as_bytes(), false)
}

/// Case-insensitive variant, mirroring shell code paths that enable `nocasematch`.
pub fn fnmatch_nocase(text: &str, pattern: &str) -> bool {
    match_bytes(text.as_bytes(), pattern.as_bytes(), true)
}

/// Core matcher: iterative with single-star backtracking (classic fnmatch algorithm).
fn match_bytes(text: &[u8], pat: &[u8], nocase: bool) -> bool {
    let (mut t, mut p) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (usize::MAX, 0usize);

    while t < text.len() {
        if p < pat.len() {
            match pat[p] {
                b'*' => {
                    // Record the star so a later mismatch can re-expand it by one byte.
                    star_p = p;
                    star_t = t;
                    p += 1;
                    continue;
                }
                b'?' => {
                    t += 1;
                    p += 1;
                    continue;
                }
                b'[' => {
                    if let Some((matched, next_p)) = match_bracket(text[t], pat, p, nocase) {
                        if matched {
                            t += 1;
                            p = next_p;
                            continue;
                        }
                    }
                }
                b'\\' if p + 1 < pat.len() => {
                    if eq_byte(text[t], pat[p + 1], nocase) {
                        t += 1;
                        p += 2;
                        continue;
                    }
                }
                c => {
                    if eq_byte(text[t], c, nocase) {
                        t += 1;
                        p += 1;
                        continue;
                    }
                }
            }
        }
        // Mismatch: backtrack to the last `*` and let it swallow one more byte.
        if star_p != usize::MAX {
            star_t += 1;
            t = star_t;
            p = star_p + 1;
            continue;
        }
        return false;
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

/// Compare two bytes, optionally ASCII-case-insensitively (matches bash nocasematch).
fn eq_byte(a: u8, b: u8, nocase: bool) -> bool {
    if nocase {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Match one byte against a `[...]` class starting at `pat[open]`; returns
/// (matched, index-after-class) or None when the bracket never closes (bash
/// then treats `[` as a literal, which the caller's fallthrough covers).
fn match_bracket(ch: u8, pat: &[u8], open: usize, nocase: bool) -> Option<(bool, usize)> {
    let mut i = open + 1;
    let negate = matches!(pat.get(i), Some(b'!') | Some(b'^'));
    if negate {
        i += 1;
    }
    let mut matched = false;
    let mut first = true;
    while i < pat.len() {
        if pat[i] == b']' && !first {
            return Some((matched != negate, i + 1));
        }
        first = false;
        // POSIX class like [:alpha:] inside the bracket.
        if pat[i] == b'[' && pat.get(i + 1) == Some(&b':') {
            if let Some(end) = find_class_end(pat, i + 2) {
                let name = &pat[i + 2..end];
                if posix_class_matches(ch, name) {
                    matched = true;
                }
                i = end + 2;
                continue;
            }
        }
        let lo = if pat[i] == b'\\' && i + 1 < pat.len() {
            i += 1;
            pat[i]
        } else {
            pat[i]
        };
        if pat.get(i + 1) == Some(&b'-') && pat.get(i + 2).is_some_and(|c| *c != b']') {
            let mut hi = pat[i + 2];
            let mut next = i + 3;
            if hi == b'\\' && i + 3 < pat.len() {
                hi = pat[i + 3];
                next = i + 4;
            }
            if in_range(ch, lo, hi, nocase) {
                matched = true;
            }
            i = next;
        } else {
            if eq_byte(ch, lo, nocase) {
                matched = true;
            }
            i += 1;
        }
    }
    None
}

/// Locate the `:]` closing a POSIX class opened at `start`.
fn find_class_end(pat: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < pat.len() {
        if pat[i] == b':' && pat[i + 1] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Range membership with the optional ASCII case fold bash applies under nocasematch.
fn in_range(ch: u8, lo: u8, hi: u8, nocase: bool) -> bool {
    if lo <= ch && ch <= hi {
        return true;
    }
    if nocase {
        let f = ch.to_ascii_lowercase();
        let g = ch.to_ascii_uppercase();
        (lo <= f && f <= hi) || (lo <= g && g <= hi)
    } else {
        false
    }
}

/// Evaluate the POSIX character classes the shell patterns actually use.
fn posix_class_matches(ch: u8, name: &[u8]) -> bool {
    match name {
        b"alpha" => ch.is_ascii_alphabetic(),
        b"digit" => ch.is_ascii_digit(),
        b"alnum" => ch.is_ascii_alphanumeric(),
        b"upper" => ch.is_ascii_uppercase(),
        b"lower" => ch.is_ascii_lowercase(),
        b"space" => ch.is_ascii_whitespace(),
        b"cntrl" => ch.is_ascii_control(),
        b"punct" => ch.is_ascii_punctuation(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_crosses_slash() {
        assert!(fnmatch("/Users/x/Library/Caches/Claude", "*Claude*"));
        assert!(fnmatch(
            "com.apple.Settings.extension",
            "com.apple.Settings*"
        ));
        assert!(!fnmatch("com.apple.settings", "com.apple.Settings*"));
    }

    #[test]
    fn brackets_and_ranges() {
        assert!(fnmatch("SystemSettings", "[Ss]ystem[Ss]ettings*"));
        assert!(fnmatch(
            "com.fabfilter.q.3.plist",
            "com.fabfilter.*.[0-9].plist"
        ));
        assert!(!fnmatch(
            "com.fabfilter.q.x.plist",
            "com.fabfilter.*.[0-9].plist"
        ));
        assert!(fnmatch("a", "[!b]"));
    }

    #[test]
    fn question_and_literal() {
        assert!(fnmatch("abc", "a?c"));
        assert!(!fnmatch("abc", "a?d"));
        assert!(fnmatch("a*c", "a\\*c"));
        assert!(!fnmatch("abc", "a\\*c"));
    }

    #[test]
    fn nocase_variant() {
        assert!(fnmatch_nocase("/PRIVATE/VAR", "/private/var"));
        assert!(!fnmatch("/PRIVATE/VAR", "/private/var"));
    }
}
