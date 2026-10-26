//! Report what a rewrite lost from its original. Port of `scripts/ste_compare.py`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::text::{Finding, HARD, chars, mask, splitlines, strip};

type Lines = Vec<(usize, String)>;

static CODE_FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(```|~~~)").unwrap());
static CODE_SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`\n]+)`").unwrap());
static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>()`"']+"#).unwrap());
static TICKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z][A-Z0-9]+-[0-9]+\b").unwrap());
static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\w+(?:[./]\w+)+\b|\b\w*_\w+\b").unwrap());
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[0-9]+(?:[.,:][0-9]+)*%?").unwrap());
static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z]+(?:['’][A-Za-z]+)?").unwrap());
const URL_TRAILING: &[char] = &['.', ',', ':', ';', '!', '?'];
const NEGATIONS: &[&str] =
    &["not", "no", "never", "none", "nothing", "nobody", "without", "neither", "nor"];
const MODALS: &[&str] = &[
    "may", "might", "could", "should", "must", "would", "can", "will", "shall", "likely",
    "unlikely", "probably", "possibly", "perhaps", "usually", "sometimes", "often", "rarely",
];

/// A counter that remembers the order in which keys first appeared.
#[derive(Default)]
struct Counter {
    order: Vec<String>,
    counts: HashMap<String, usize>,
}

impl Counter {
    fn add(&mut self, key: &str) {
        let count = self.counts.entry(key.to_string()).or_insert_with(|| {
            self.order.push(key.to_string());
            0
        });
        *count += 1;
    }
    fn get(&self, key: &str) -> usize {
        self.counts.get(key).copied().unwrap_or(0)
    }
    fn total(&self) -> usize {
        self.counts.values().sum()
    }
}

fn split_fences(text: &str) -> (Lines, Lines) {
    let (mut prose, mut fenced, mut in_fence) = (Vec::new(), Vec::new(), false);
    for (index, line) in splitlines(text).into_iter().enumerate() {
        if CODE_FENCE.is_match(line) {
            in_fence = !in_fence;
            prose.push((index + 1, String::new()));
            continue;
        }
        let target = if in_fence { &mut fenced } else { &mut prose };
        target.push((index + 1, line.to_string()));
    }
    (prose, fenced)
}

fn clean_url(url: &str) -> &str {
    url.trim_end_matches(URL_TRAILING)
}

fn is_abbreviation(token: &str) -> bool {
    token.split(['.', '/']).all(|part| chars(part) == 1)
}

/// (kind, literal, line, column) for each literal in the prose lines.
fn literals(prose: &Lines) -> Vec<(&'static str, String, usize, usize)> {
    let mut out = Vec::new();
    for (lineno, line) in prose {
        let col = |text: &str, start: usize| chars(&text[..start]) + 1;
        for caps in CODE_SPAN.captures_iter(line) {
            let start = caps.get(0).unwrap().start();
            out.push(("code span", strip(&caps[1]).to_string(), *lineno, col(line, start)));
        }
        let line = mask(line, &CODE_SPAN);
        for m in URL.find_iter(&line) {
            out.push(("URL", clean_url(m.as_str()).to_string(), *lineno, col(&line, m.start())));
        }
        let line = mask(&line, &URL);
        for m in TICKET.find_iter(&line) {
            out.push(("ticket key", m.as_str().to_string(), *lineno, col(&line, m.start())));
        }
        for m in IDENTIFIER.find_iter(&line) {
            if !is_abbreviation(m.as_str()) {
                out.push(("identifier", m.as_str().to_string(), *lineno, col(&line, m.start())));
            }
        }
        let numbers = mask(&line, &IDENTIFIER);
        for m in NUMBER.find_iter(&numbers) {
            out.push(("number", m.as_str().to_string(), *lineno, col(&numbers, m.start())));
        }
    }
    out
}

fn token_set(text: &str) -> HashSet<String> {
    let mut tokens: HashSet<String> = [&*TICKET, &*IDENTIFIER, &*NUMBER]
        .iter()
        .flat_map(|re| re.find_iter(text).map(|m| m.as_str().to_string()))
        .collect();
    tokens.extend(URL.find_iter(text).map(|m| clean_url(m.as_str()).to_string()));
    tokens
}

/// Negation and modal counts, and the first position of each counted word.
fn word_counts(prose: &Lines) -> (Counter, Counter, HashMap<String, (usize, usize)>) {
    let (mut negations, mut modals, mut first) = (Counter::default(), Counter::default(), HashMap::new());
    for (lineno, line) in prose {
        let masked = mask(line, &CODE_SPAN);
        for m in WORD.find_iter(&masked) {
            let position = (*lineno, chars(&masked[..m.start()]) + 1);
            let mut word = m.as_str().to_lowercase().replace('’', "'");
            let mut base = word.clone();
            if word == "cannot" {
                word = "can't".into();
            }
            if let Some(stem) = word.strip_suffix("n't") {
                negations.add("not");
                first.entry("not".to_string()).or_insert(position);
                base = match word.as_str() {
                    "can't" => "can".into(),
                    "won't" => "will".into(),
                    "shan't" => "shall".into(),
                    _ => stem.to_string(),
                };
            }
            if NEGATIONS.contains(&base.as_str()) {
                negations.add(&base);
                first.entry(base.clone()).or_insert(position);
            }
            if MODALS.contains(&base.as_str()) {
                modals.add(&base);
                first.entry(base.clone()).or_insert(position);
            }
        }
    }
    (negations, modals, first)
}

fn finding(file: &str, (line, col): (usize, usize), rule: &str, matched: &str, message: String) -> Finding {
    Finding { file: file.into(), line, col, rule: rule.into(), level: HARD.into(),
              matched: matched.into(), message }
}

pub fn compare(original: &str, rewrite: &str, filename: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let (prose, fenced) = split_fences(original);
    let (rewrite_prose, rewrite_fenced) = split_fences(rewrite);
    let rewrite_tokens = token_set(rewrite);
    let mut reported = HashSet::new();
    for (kind, literal, lineno, col) in literals(&prose) {
        if literal.is_empty() || reported.contains(&literal) {
            continue;
        }
        let kept = if matches!(kind, "code span" | "URL") {
            rewrite.contains(literal.as_str())
        } else {
            rewrite_tokens.contains(&literal)
        };
        if !kept {
            let message = format!("The rewrite lost this {kind}. Keep it unless it was wrong.");
            findings.push(finding(filename, (lineno, col), "dropped-literal", &literal, message));
            reported.insert(literal);
        }
    }
    let rewrite_code: HashSet<&str> = rewrite_fenced.iter().map(|(_, l)| strip(l)).collect();
    for (lineno, line) in &fenced {
        let code = strip(line);
        if !code.is_empty() && !rewrite_code.contains(code) {
            let message = "The rewrite lost or changed this line of fenced code.".into();
            findings.push(finding(filename, (*lineno, 1), "dropped-literal", code, message));
        }
    }

    let (neg_a, mod_a, first) = word_counts(&prose);
    let (neg_b, mod_b, _) = word_counts(&rewrite_prose);
    if neg_a.total() > neg_b.total() {
        let word = &neg_a.order[0];
        let message = format!(
            "The original has {} negations and the rewrite has {}. Check that {} negation(s) \
             did not turn into the opposite claim.",
            neg_a.total(), neg_b.total(), neg_a.total() - neg_b.total()
        );
        findings.push(finding(filename, first[word], "dropped-negation", word, message));
    }
    let mut modal_words: Vec<&String> = mod_a.order.iter().collect();
    modal_words.sort();
    for word in modal_words {
        let (count, kept) = (mod_a.get(word), mod_b.get(word));
        if kept < count {
            let message = format!(
                "'{word}' appears {count} time(s) in the original and {kept} in the rewrite. \
                 A changed modal changes the claim."
            );
            findings.push(finding(filename, first[word], "modal-changed", word, message));
        }
    }
    findings.sort_by_key(|f| (f.line, f.col));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(original: &str, rewrite: &str) -> Vec<(String, String)> {
        compare(original, rewrite, "o").into_iter().map(|f| (f.rule, f.matched)).collect()
    }

    #[test]
    fn identical_text_is_clean() {
        let text = "The job may have failed on 3 of 12 nodes. Do not rerun it.\n\
                    See DEL-8524 and `apps/common/tests` at https://example.com/x.\n";
        assert!(compare(text, text, "o").is_empty());
    }

    #[test]
    fn reports_lost_modal_and_negation() {
        let found = rules("The job may fail. Do not rerun it.", "The job fails. Rerun it.");
        assert!(found.contains(&("modal-changed".into(), "may".into())));
        assert!(found.contains(&("dropped-negation".into(), "not".into())));
    }

    #[test]
    fn contractions_and_backticks() {
        assert!(rules("It can't run.", "It cannot run.").is_empty());
        assert!(rules("Edit `config.py` now.", "Edit config.py now.").is_empty());
        assert_eq!(rules("You should restart.", "You must restart."),
                   vec![("modal-changed".into(), "should".into())]);
    }
}
