//! String helpers that copy Python's semantics, so columns and line splits
//! match `ste-lint.py` exactly. Columns are counted in code points, as Python
//! counts them; Rust slices by byte, so every column goes through `chars`.

use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub rule: String,
    pub level: String,
    #[serde(rename = "match")]
    pub matched: String,
    pub message: String,
}

pub const HARD: &str = "advisory-free";
pub const ADVISORY: &str = "advisory";

/// Python's `str.isspace()`: Unicode White_Space plus the separators 0x1C-0x1F.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_space)
}

/// Python's `str.split()` with no argument.
pub fn split_words(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_space).filter(|w| !w.is_empty())
}

/// Number of code points in `s`: a Python string length.
pub fn chars(s: &str) -> usize {
    s.chars().count()
}

fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Python's `str.splitlines()`: every line boundary Python knows, "\r\n" as one.
pub fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if !is_line_break(c) {
            continue;
        }
        lines.push(&text[start..i]);
        let mut end = i + c.len_utf8();
        if c == '\r' && matches!(iter.peek(), Some(&(_, '\n'))) {
            iter.next();
            end += 1;
        }
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Python's universal-newline read: "\r\n" and "\r" become "\n".
pub fn universal_newlines(text: String) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}

/// Replace each match with spaces, one per code point, so columns stay true.
pub fn mask(text: &str, pattern: &Regex) -> String {
    pattern
        .replace_all(text, |caps: &regex::Captures| " ".repeat(chars(&caps[0])))
        .into_owned()
}

/// Byte index of the code point after the one at `index`.
pub fn next_char(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map_or(text.len() + 1, |c| index + c.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("\n"), vec![""]);
        assert_eq!(splitlines("a\r\nb\rc\u{2028}d\n"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn mask_keeps_code_point_columns() {
        let re = Regex::new("`[^`]*`").unwrap();
        assert_eq!(mask("x `é` y", &re), "x     y");
    }
}
