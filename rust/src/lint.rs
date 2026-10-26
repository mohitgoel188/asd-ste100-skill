//! The line rules and the main lint pass. Port of `lint()` in `ste-lint.py`.
//!
//! The `regex` crate has no lookaround. Two Python rules use it, so `Guard`
//! re-checks those matches by hand. A match that the Python lookaround would
//! reject restarts the search one code point later, as Python's `finditer` does.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Regex, escape};

use crate::config::{Config, avoid_patterns, phrasal_pattern, word_list_pattern};
use crate::markdown::{
    CODE_FENCE, INLINE_CODE, Piece, blank_front_matter, dangling_conjunction_findings,
    long_sentence_findings, markdown_table_cells,
};
use crate::text::{ADVISORY, Finding, HARD, chars, lstrip, mask, next_char, split_words, splitlines, strip};

const IRREGULAR_PARTICIPLES: &str = "given|taken|made|done|found|seen|known|shown|written|built|sent|set|run|read|kept|held|left|put|cut|hit|let|shut|split|spread|begun|become|come|gone|got|gotten|lost|met|paid|said|sold|told|thought|brought|bought|caught|taught|won|worn|torn|born|drawn|grown|thrown|flown|driven|risen|chosen|broken|spoken|frozen|hidden|ridden|forgotten|fallen|eaten|beaten|understood|stood|struck|stuck|swung|hung|led|fed|bled|fled|sped|bound|wound|dug|spun|slid|bit|lit|quit";
const PASSIVE_PARTICIPLES: &str =
    "given|taken|made|done|found|seen|known|shown|written|built|sent|set|run|read|kept|held|left|put";

const SYNONYM_GROUPS: &[&[&str]] = &[
    &["check", "verify", "confirm", "validate"],
    &["delete", "remove", "erase"],
    &["start", "launch", "begin", "initiate"],
    &["stop", "halt", "terminate"],
    &["show", "display"],
    &["use", "utilize", "employ"],
    &["fix", "repair", "correct"],
    &["send", "transmit"],
    &["get", "retrieve", "fetch", "obtain"],
    &["change", "modify", "alter"],
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Guard {
    None,
    Passive,
    PresentPerfect,
}

#[derive(Clone)]
pub struct Rule {
    pub id: &'static str,
    level: &'static str,
    pattern: Regex,
    message: String,
    guard: Guard,
}

fn rule(id: &'static str, level: &'static str, pattern: &str, message: &str, guard: Guard) -> Rule {
    Rule { id, level, pattern: Regex::new(pattern).unwrap(), message: message.into(), guard }
}

static BASE_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule("semicolon", HARD, ";",
             "STE bans the semicolon (Rule 8.1). Split into separate sentences.", Guard::None),
        rule("phrasal-verb", HARD,
             r"(?i)\b(spin(?:ning|s)? up|spun up|reach(?:ing|es|ed)? out|div(?:e|es|ing|ed) into|dove into|kick(?:ing|s|ed)? off|circl(?:e|es|ing|ed) back|touch(?:ing|es|ed)? base|figur(?:e|es|ed|ing) out|f(?:ind|inds|inding|ound) out|look(?:s|ed|ing)? into|c(?:ome|omes|oming|ame) up with|end(?:s|ed|ing)? up|point(?:s|ed|ing)? out)\b",
             "Soft phrasal verb. Use the single plain verb (start, contact, read, begin).", Guard::None),
        rule("marketing-adjective", HARD,
             r"(?i)\b(seamless(?:ly)?|robust(?:ly)?|cutting-edge|effortless(?:ly)?|blazing[- ]fast|world-class|state-of-the-art|game-chang(?:ing|er))\b",
             "Marketing adjective. Delete, or replace with the measurement that earns the claim.", Guard::None),
        rule("nominalization", HARD,
             r"(?i)\b(perform|performs|performed|conduct|conducts|conducted|carry out|carries out|carried out)\s+(?:a|an|the)\s+\w+(?:tion|sion|ment|ance|ence|ysis)\b",
             "Action frozen into a noun. Use the verb (analyze, not perform an analysis of).", Guard::None),
        rule("passive-voice", ADVISORY,
             &format!(r"(?i)\b(is|are|was|were|been|being)\s+(\w+ed|{PASSIVE_PARTICIPLES})\b"),
             "Possible passive voice. Name the actor and use an active verb, unless the actor is unknown or irrelevant.", Guard::Passive),
        rule("present-perfect", ADVISORY,
             &format!(r"(?i)\b(has|have|had)\s+(?:been\s+)?(?:\w+(?:ed|en)|{IRREGULAR_PARTICIPLES})\b"),
             "Compound tense. Use simple past/present unless current relevance is the point (then keep and flag).", Guard::PresentPerfect),
    ]
});

/// Python's `(?!\s+(?:to|for|by)\s+\w+ing)` after a passive match.
static PASSIVE_EXCEPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s+(?:to|for|by)\s+\w+ing").unwrap());
/// Python's six `(?<!\bmay )` lookbehinds before a present-perfect match.
static PERFECT_LOOKBEHIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:may|might|could|should|would|must) $").unwrap());
static MODAL_PERFECT_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:may|might|could|should|would|must|can|will|shall)(?:\s+not|n['’]t)?\s+$").unwrap()
});
static STANDALONE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ {0,3}(?:#|<|(?:[-*_=] *){3,}$)").unwrap());
static BLOCKQUOTE_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ {0,3}(?:> ?)+").unwrap());
static GITHUB_ALERT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\[!(?:note|tip|important|warning|caution)\]$").unwrap());
/// Python's `(?=[\s/>])[^>]*>` written without lookahead: same matches.
static HTML_BLOCK_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)<(/?)(address|article|aside|blockquote|body|caption|center|dd|details|",
        r"dialog|dir|div|dl|dt|fieldset|figcaption|figure|footer|form|h[1-6]|",
        r"header|hgroup|html|iframe|legend|li|main|menu|nav|noframes|ol|",
        r"optgroup|option|p|pre|script|search|section|style|summary|table|",
        r"tbody|td|textarea|tfoot|th|thead|title|tr|ul)(?:>|[\s/][^>]*>)"
    ))
    .unwrap()
});
static ANY_LIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|[0-9]+[.)])(?:\s|$)").unwrap());

type SynonymGroup = Vec<(String, Regex)>;

fn word_re(base: &str) -> Regex {
    Regex::new(&format!(r"(?i)\b{}(?:s|es|ed|d|ing)?\b", escape(base))).unwrap()
}

static BASE_SYNONYMS: LazyLock<Vec<SynonymGroup>> = LazyLock::new(|| {
    SYNONYM_GROUPS
        .iter()
        .map(|group| group.iter().map(|b| (b.to_string(), word_re(b))).collect())
        .collect()
});

fn config_rules(config: &Config) -> Vec<Rule> {
    let mut rules = Vec::new();
    if let Some(pattern) = phrasal_pattern(&config.phrasal_verbs) {
        rules.push(Rule { id: "phrasal-verb", level: HARD, pattern,
                          message: "Project phrasal verb. Use the single plain verb.".into(),
                          guard: Guard::None });
    }
    if let Some(pattern) = word_list_pattern(&config.marketing_adjectives) {
        rules.push(Rule { id: "marketing-adjective", level: HARD, pattern,
                          message: "Project marketing adjective. Delete, or replace with a measurement.".into(),
                          guard: Guard::None });
    }
    for (term, replacement, pattern) in avoid_patterns(&config.avoid_terms) {
        let message = if replacement.is_empty() {
            format!("Project vocabulary: do not use '{term}'.")
        } else {
            format!("Project vocabulary: use '{replacement}', not '{term}'.")
        };
        rules.push(Rule { id: "avoid-term", level: HARD, pattern, message, guard: Guard::None });
    }
    rules
}

/// Byte start and text of each match the Python rule would report.
fn rule_matches<'a>(rule: &Rule, line: &'a str) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= line.len() {
        let Some(m) = rule.pattern.find_at(line, pos) else { break };
        let before = &line[..m.start()];
        let lookaround_rejects = match rule.guard {
            Guard::None => false,
            Guard::Passive => PASSIVE_EXCEPTION.is_match(&line[m.end()..]),
            Guard::PresentPerfect => PERFECT_LOOKBEHIND.is_match(before),
        };
        if lookaround_rejects {
            pos = next_char(line, m.start());
            continue;
        }
        // no rule pattern can match the empty string, so the search always advances
        pos = m.end();
        if rule.guard == Guard::PresentPerfect && MODAL_PERFECT_PREFIX.is_match(before) {
            continue;
        }
        out.push((m.start(), m.as_str()));
    }
    out
}

/// State that one pass over the lines carries forward.
struct Pass<'c> {
    filename: &'c str,
    findings: Vec<Finding>,
    block: Vec<Piece>,
    words_total: usize,
    seen_synonyms: HashMap<(usize, usize), (usize, usize, String)>,
}

impl Pass<'_> {
    fn flush(&mut self) {
        let found = long_sentence_findings(&self.block, self.filename);
        self.findings.extend(found);
        self.block.clear();
    }
}

/// Track open HTML block containers on a line that starts with a block tag.
fn track_html(raw_line: &str, containers: &mut Vec<String>) {
    let starts_with_tag = HTML_BLOCK_TAG.find(lstrip(raw_line)).is_some_and(|m| m.start() == 0);
    if !starts_with_tag {
        return;
    }
    for tag in HTML_BLOCK_TAG.captures_iter(&mask(raw_line, &INLINE_CODE)) {
        let name = tag[2].to_lowercase();
        if !tag[1].is_empty() {
            if containers.contains(&name) {
                while let Some(open) = containers.pop() {
                    if open == name {
                        break;
                    }
                }
            }
        } else if !tag[0].ends_with("/>") {
            containers.push(name);
        }
    }
}

fn scan_segment(pass: &mut Pass, rules: &[&Rule], groups: &[SynonymGroup],
                lineno: usize, line: &str, source_column: usize) {
    pass.words_total += split_words(line).count();
    for rule in rules {
        for (start, matched) in rule_matches(rule, line) {
            pass.findings.push(Finding {
                file: pass.filename.to_string(),
                line: lineno,
                col: source_column + chars(&line[..start]) + 1,
                rule: rule.id.into(),
                level: rule.level.into(),
                matched: matched.into(),
                message: rule.message.clone(),
            });
        }
    }
    for (gi, group) in groups.iter().enumerate() {
        for (bi, (_, re)) in group.iter().enumerate() {
            if pass.seen_synonyms.contains_key(&(gi, bi)) {
                continue;
            }
            if let Some(m) = re.find(line) {
                let col = source_column + chars(&line[..m.start()]) + 1;
                pass.seen_synonyms.insert((gi, bi), (lineno, col, m.as_str().to_string()));
            }
        }
    }
}

fn synonym_findings(pass: &mut Pass, groups: &[SynonymGroup]) {
    for (gi, group) in groups.iter().enumerate() {
        let mut present: Vec<((usize, usize, String), &str)> = group
            .iter()
            .enumerate()
            .filter_map(|(bi, (base, _))| {
                pass.seen_synonyms.get(&(gi, bi)).map(|seen| (seen.clone(), base.as_str()))
            })
            .collect();
        if present.len() < 2 {
            continue;
        }
        present.sort();
        let first_base = present[0].1;
        for ((lineno, col, matched), base) in &present[1..] {
            pass.findings.push(Finding {
                file: pass.filename.to_string(),
                line: *lineno,
                col: *col,
                rule: "synonym-rotation".into(),
                level: HARD.into(),
                matched: matched.clone(),
                message: format!(
                    "'{base}' and '{first_base}' name the same action. Pick one and use it every time."
                ),
            });
        }
    }
}

pub fn lint(text: &str, filename: &str, config: &Config) -> (Vec<Finding>, usize) {
    let extra = config_rules(config);
    let rules: Vec<&Rule> = BASE_RULES.iter().chain(extra.iter()).collect();
    let mut groups: Vec<SynonymGroup> = BASE_SYNONYMS.clone();
    groups.extend(config.synonym_groups.iter().map(|group| {
        group.iter().map(|b| (b.clone(), word_re(b))).collect::<SynonymGroup>()
    }));
    let text = blank_front_matter(text);
    let lines = splitlines(&text);
    let table_cells = markdown_table_cells(&lines);
    let mut pass = Pass { filename, findings: Vec::new(), block: Vec::new(),
                          words_total: 0, seen_synonyms: HashMap::new() };
    let mut html_containers: Vec<String> = Vec::new();
    let (mut in_fence, mut in_quote, mut after_boundary) = (false, false, true);

    for (index, raw_line) in lines.iter().copied().enumerate() {
        let lineno = index + 1;
        if CODE_FENCE.is_match(strip(raw_line)) {
            pass.flush();
            in_fence = !in_fence;
            after_boundary = true;
            continue;
        }
        if in_fence {
            continue;
        }
        track_html(raw_line, &mut html_containers);
        let table = table_cells.get(&index);
        let quote = BLOCKQUOTE_PREFIX.find(raw_line);
        let content = quote.map_or(raw_line, |q| &raw_line[q.end()..]);
        let alert = html_containers.is_empty()
            && raw_line.starts_with('>')
            && after_boundary
            && quote.is_some_and(|q| q.as_str().matches('>').count() == 1)
            && GITHUB_ALERT.is_match(strip(content));
        let standalone = STANDALONE_LINE.is_match(content) || alert;
        if table.is_some() || strip(content).is_empty() || standalone
            || ANY_LIST_ITEM.is_match(content) || quote.is_some() != in_quote
        {
            pass.flush();
        }
        in_quote = quote.is_some();
        after_boundary = strip(raw_line).is_empty();
        let whole_line = [(raw_line.to_string(), 0)];
        let segments = table.map_or(&whole_line[..], |cells| &cells[..]);
        for (segment, source_column) in segments {
            let line = mask(segment, &INLINE_CODE);
            scan_segment(&mut pass, &rules, &groups, lineno, &line, *source_column);
            // the blockquote prefix and the list marker are markup, not words
            let mut skip = match quote {
                Some(q) if table.is_none() => q.end(),
                _ => 0,
            };
            if table.is_none()
                && let Some(marker) = ANY_LIST_ITEM.find(&segment[skip..])
            {
                skip += marker.end();
            }
            if !strip(&line[skip..]).is_empty() {
                let column = source_column + chars(&segment[..skip]);
                pass.block.push((lineno, column, line[skip..].to_string()));
            }
            if table.is_some() {
                pass.flush();
            }
        }
        if standalone {
            pass.flush();
        }
    }
    pass.flush();
    synonym_findings(&mut pass, &groups);
    pass.findings.extend(dangling_conjunction_findings(&text, filename));
    pass.findings.sort_by_key(|f| (f.line, f.col));
    (pass.findings, pass.words_total)
}
