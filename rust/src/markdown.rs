//! Markdown structure: front matter, tables, sentence splits, long sentences
//! and dangling list conjunctions. Ports of the matching `ste-lint.py` helpers.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::text::{
    Finding, HARD, chars, lstrip, mask, rstrip, split_words, splitlines, strip,
};

/// (line number, source column, text) of one line of a prose block.
pub type Piece = (usize, usize, String);
/// Trimmed cell text and its zero-based source column.
pub type Cell = (String, usize);

pub const MAX_WORDS: usize = 25;
const OPENERS: &[char] = &['"', '\'', '“', '‘', '(', '[', '*', '_'];
const ABBREVIATIONS: &[&str] = &["vs.", "cf.", "approx.", "mr.", "mrs.", "ms.", "dr.", "fig.", "no."];

pub static CODE_FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(```|~~~)").unwrap());
pub static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`]*`").unwrap());
static LIST_ITEM_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<indent> {0,3})(?P<marker>[-*+]|[0-9]+[.)])(?P<gap> +)(?P<body>.*)$").unwrap()
});
static CONJUNCTION_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:and|or)\s*$").unwrap());
static TABLE_SEPARATOR_CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^:?-{3,}:?$").unwrap());
static WORD_CHAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\W_]").unwrap());
static SENTENCE_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[.!?]["'”’)\]*_]*$"#).unwrap());
static DOTTED_ABBREVIATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:[a-z]\.){2,}$").unwrap());
static NON_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\S+").unwrap());

/// Blank a leading YAML front-matter block, keeping its line count.
pub fn blank_front_matter(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if rstrip(lines[0]) != "---" {
        return text.to_string();
    }
    for index in 1..lines.len() {
        let line = rstrip(lines[index]);
        if line == "---" || line == "..." {
            return "\n".repeat(index + 1) + &lines[index + 1..].join("\n");
        }
    }
    text.to_string()
}

fn count_words(text: &str) -> usize {
    split_words(text).filter(|t| WORD_CHAR.is_match(t)).count()
}

/// Byte offset of each sentence start in `text`.
fn sentence_starts(text: &str) -> Vec<usize> {
    let words: Vec<regex::Match> = NON_SPACE.find_iter(text).collect();
    if words.is_empty() {
        return vec![0];
    }
    let mut starts = vec![words[0].start()];
    for pair in words.windows(2) {
        let (word, next_word) = (pair[0].as_str(), pair[1]);
        let Some(end) = SENTENCE_END.find(word) else { continue };
        let core = word[..end.start() + 1].trim_start_matches(OPENERS);
        if ABBREVIATIONS.contains(&core.to_lowercase().as_str()) {
            continue;
        }
        let next_upper = next_word
            .as_str()
            .trim_start_matches(OPENERS)
            .chars()
            .next()
            .is_some_and(char::is_uppercase);
        if DOTTED_ABBREVIATION.is_match(core) && !next_upper {
            continue;
        }
        starts.push(next_word.start());
    }
    starts
}

/// Flag long sentences in one prose block, at the line and column they start.
pub fn long_sentence_findings(pieces: &[Piece], filename: &str) -> Vec<Finding> {
    let mut offsets = Vec::with_capacity(pieces.len());
    let mut position = 0;
    for (_, _, text) in pieces {
        offsets.push(position);
        position += text.len() + 1;
    }
    let joined = pieces.iter().map(|p| p.2.as_str()).collect::<Vec<_>>().join(" ");
    let starts = sentence_starts(&joined);
    let mut findings = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(joined.len());
        let n = count_words(&joined[start..end]);
        if n <= MAX_WORDS {
            continue;
        }
        let index = offsets.partition_point(|&o| o <= start) - 1;
        let (lineno, source_column, _) = &pieces[index];
        findings.push(Finding {
            file: filename.to_string(),
            line: *lineno,
            col: source_column + chars(&joined[offsets[index]..start]) + 1,
            rule: "long-sentence".into(),
            level: HARD.into(),
            matched: format!("{n} words"),
            message: format!("Sentence has {n} words (cap {MAX_WORDS}). Split it."),
        });
    }
    findings
}

/// Trimmed cells and their source columns, or None when the line is not a row.
fn split_table_row(line: &str) -> Option<Vec<Cell>> {
    let left_b = line.len() - lstrip(line).len();
    let right_b = rstrip(line).len();
    if left_b >= right_b {
        return None;
    }
    let mut content = &line[left_b..right_b];
    let mut left = left_b;
    if !content.contains('|') {
        return None;
    }
    if let Some(rest) = content.strip_prefix('|') {
        content = rest;
        left += 1;
    }
    if let Some(rest) = content.strip_suffix('|') {
        content = rest;
    }
    // split on pipes that no backslash escapes
    let mut raw_cells = Vec::new();
    let (mut cell_start, mut prev) = (0, '\0');
    for (i, c) in content.char_indices() {
        if c == '|' && prev != '\\' {
            raw_cells.push((cell_start, &content[cell_start..i]));
            cell_start = i + 1;
        }
        prev = c;
    }
    raw_cells.push((cell_start, &content[cell_start..]));
    if raw_cells.len() < 2 {
        return None;
    }
    let cells = raw_cells
        .into_iter()
        .map(|(offset, raw)| {
            let leading = raw.len() - lstrip(raw).len();
            (strip(raw).to_string(), chars(&line[..left + offset + leading]))
        })
        .collect();
    Some(cells)
}

/// Map ordinary Markdown table rows (anchored by a separator row) to cells.
pub fn markdown_table_cells(lines: &[&str]) -> HashMap<usize, Vec<Cell>> {
    let mut table_cells = HashMap::new();
    let mut index = 1;
    while index < lines.len() {
        let separator = split_table_row(lines[index]);
        let header = split_table_row(lines[index - 1]);
        let (Some(separator), Some(header)) = (separator, header) else {
            index += 1;
            continue;
        };
        if separator.len() != header.len()
            || !separator.iter().all(|(cell, _)| TABLE_SEPARATOR_CELL.is_match(cell))
        {
            index += 1;
            continue;
        }
        table_cells.insert(index - 1, header);
        table_cells.insert(index, Vec::new());
        index += 1;
        while index < lines.len() {
            match split_table_row(lines[index]) {
                Some(row) if row.len() == separator.len() => {
                    table_cells.insert(index, row);
                    index += 1;
                }
                _ => break,
            }
        }
    }
    table_cells
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_list_continuation(line: &str, content_indent: usize) -> bool {
    if strip(line).is_empty() {
        return true;
    }
    if LIST_ITEM_START.is_match(line) {
        return false;
    }
    leading_spaces(line) >= content_indent
}

/// Flag list items whose last meaningful line ends with "and" or "or".
pub fn dangling_conjunction_findings(text: &str, filename: &str) -> Vec<Finding> {
    let lines = splitlines(text);
    let mut findings = Vec::new();
    let mut in_fence = false;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if CODE_FENCE.is_match(strip(line)) {
            in_fence = !in_fence;
            index += 1;
            continue;
        }
        let start = if in_fence { None } else { LIST_ITEM_START.captures(line) };
        let Some(start) = start else {
            index += 1;
            continue;
        };
        let content_indent = start["indent"].len() + start["marker"].len() + start["gap"].len();
        let mut item_lines: Vec<(usize, &str)> =
            vec![(index, start.name("body").unwrap().as_str())];
        let mut next_index = index + 1;
        let mut item_fence = false;
        while next_index < lines.len() {
            let candidate = lines[next_index];
            if CODE_FENCE.is_match(strip(candidate)) {
                item_fence = !item_fence;
                next_index += 1;
                continue;
            }
            if item_fence {
                next_index += 1;
                continue;
            }
            if !is_list_continuation(candidate, content_indent) {
                break;
            }
            item_lines.push((next_index, candidate));
            next_index += 1;
        }
        let last_meaningful = item_lines.iter().rev().find_map(|&(line_index, item_line)| {
            let cleaned = INLINE_CODE.replace_all(item_line, " CODE ");
            let cleaned = strip(&cleaned).to_string();
            (!cleaned.is_empty()).then_some((line_index, cleaned))
        });
        if let Some((end_index, end_line)) = last_meaningful
            && CONJUNCTION_END.is_match(&end_line)
        {
            let (finding_line, finding_col) = if end_index == index {
                (index + 1, start.name("marker").unwrap().start() + 1)
            } else {
                let masked = mask(lines[end_index], &INLINE_CODE);
                let col = CONJUNCTION_END
                    .find(&masked)
                    .map_or(1, |m| chars(&masked[..m.start()]) + 1);
                (end_index + 1, col)
            };
            findings.push(Finding {
                file: filename.to_string(),
                line: finding_line,
                col: finding_col,
                rule: "dangling-conjunction".into(),
                level: HARD.into(),
                matched: end_line,
                message: "List item ends with a coordinating conjunction. \
                          Complete the item or join it with the next item."
                    .into(),
            });
        }
        index = next_index;
    }
    findings
}
