//! Project vocabulary (`.ste-lint.json`). Port of `scripts/ste_config.py`.

use std::path::{Path, PathBuf};

use regex::{Regex, escape};
use serde_json::Value;

pub const CONFIG_NAME: &str = ".ste-lint.json";
const KNOWN_KEYS: [&str; 4] =
    ["avoid_terms", "marketing_adjectives", "phrasal_verbs", "synonym_groups"];

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub phrasal_verbs: Vec<String>,
    pub marketing_adjectives: Vec<String>,
    pub synonym_groups: Vec<Vec<String>>,
    /// (term, replacement) in file order.
    pub avoid_terms: Vec<(String, String)>,
}

/// Nearest `.ste-lint.json` at or above `start`.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(CONFIG_NAME))
        .find(|candidate| candidate.is_file())
}

fn string_list(value: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let error = || format!("'{key}' must be a list of non-empty strings");
    let items = value.as_array().ok_or_else(error)?;
    items
        .iter()
        .map(|item| match item.as_str().map(str::trim) {
            Some(s) if !s.is_empty() => Ok(s.to_string()),
            _ => Err(error()),
        })
        .collect()
}

fn parse_fields(data: &serde_json::Map<String, Value>) -> Result<Config, String> {
    let mut synonym_groups = Vec::new();
    if let Some(groups) = data.get("synonym_groups") {
        let groups = groups.as_array().ok_or("'synonym_groups' must be a list of lists")?;
        for group in groups {
            let group = string_list(Some(group), "group")?;
            if group.len() < 2 {
                return Err("each synonym group needs at least two terms".into());
            }
            synonym_groups.push(group);
        }
    }
    let mut avoid_terms = Vec::new();
    if let Some(avoid) = data.get("avoid_terms") {
        let error = "'avoid_terms' must map each term to its replacement";
        for (term, replacement) in avoid.as_object().ok_or(error)? {
            let replacement = replacement.as_str().ok_or(error)?;
            if term.trim().is_empty() {
                return Err(error.into());
            }
            avoid_terms.push((term.trim().to_string(), replacement.trim().to_string()));
        }
    }
    Ok(Config {
        phrasal_verbs: string_list(data.get("phrasal_verbs"), "phrasal_verbs")?,
        marketing_adjectives: string_list(data.get("marketing_adjectives"), "marketing_adjectives")?,
        synonym_groups,
        avoid_terms,
    })
}

/// Validate a decoded JSON value and return the config.
pub fn parse_config(data: &Value, source: &str) -> Result<Config, String> {
    let data = data
        .as_object()
        .ok_or_else(|| format!("{source}: top level must be a JSON object"))?;
    let mut unknown: Vec<&str> =
        data.keys().map(String::as_str).filter(|k| !KNOWN_KEYS.contains(k)).collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(format!(
            "{source}: unknown key(s) {}. Known keys: {}",
            unknown.join(", "),
            KNOWN_KEYS.join(", ")
        ));
    }
    parse_fields(data).map_err(|e| format!("{source}: {e}"))
}

pub fn load_config(path: &Path) -> Result<Config, String> {
    let shown = path.display();
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    let data: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{shown}: invalid JSON ({e})"))?;
    parse_config(&data, &shown.to_string())
}

fn phrase_pattern(phrase: &str) -> String {
    phrase.split_whitespace().map(escape).collect::<Vec<_>>().join(r"\s+")
}

/// One regex for the configured phrasal verbs, inflections included.
pub fn phrasal_pattern(phrases: &[String]) -> Option<Regex> {
    if phrases.is_empty() {
        return None;
    }
    let alternatives: Vec<String> = phrases
        .iter()
        .map(|phrase| {
            let mut words = phrase.split_whitespace();
            let verb = words.next().unwrap_or_default();
            let rest: Vec<&str> = words.collect();
            let last = escape(&verb.chars().last().map(String::from).unwrap_or_default());
            let tail = if rest.is_empty() {
                String::new()
            } else {
                format!(r"\s+{}", phrase_pattern(&rest.join(" ")))
            };
            format!(r"{}(?:s|es|ed|d|ing|{last}(?:ed|ing))?{tail}", escape(verb))
        })
        .collect();
    Some(Regex::new(&format!(r"(?i)\b(?:{})\b", alternatives.join("|"))).unwrap())
}

pub fn word_list_pattern(words: &[String]) -> Option<Regex> {
    if words.is_empty() {
        return None;
    }
    let body: Vec<String> = words.iter().map(|w| phrase_pattern(w)).collect();
    Some(Regex::new(&format!(r"(?i)\b(?:{})\b", body.join("|"))).unwrap())
}

/// (term, replacement, regex) per term. A capitalised term matches case-sensitively.
pub fn avoid_patterns(avoid_terms: &[(String, String)]) -> Vec<(String, String, Regex)> {
    avoid_terms
        .iter()
        .map(|(term, replacement)| {
            let flags = if term.chars().any(char::is_uppercase) { "" } else { "(?i)" };
            let re = Regex::new(&format!(r"{flags}\b{}\b", phrase_pattern(term))).unwrap();
            (term.clone(), replacement.clone(), re)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patterns_match_inflections_and_case_rules() {
        let config = parse_config(
            &json!({"phrasal_verbs": ["look into", "set up"],
                    "avoid_terms": {"DC": "centre", "utilise": "use"}}),
            "t",
        )
        .unwrap();
        let phrasal = phrasal_pattern(&config.phrasal_verbs).unwrap();
        assert!(phrasal.is_match("We looked  into it."));
        assert!(phrasal.is_match("Setting up the job"));
        assert!(!phrasal.is_match("The setup is done."));
        let avoid = avoid_patterns(&config.avoid_terms);
        assert!(avoid[0].2.is_match("the DC stock") && !avoid[0].2.is_match("the dc stock"));
        assert!(avoid[1].2.is_match("Utilise it"));
    }

    #[test]
    fn rejects_bad_config() {
        for bad in [
            json!([]),
            json!({"typo": []}),
            json!({"phrasal_verbs": "set up"}),
            json!({"synonym_groups": [["only"]]}),
            json!({"avoid_terms": ["x"]}),
            json!({"phrasal_verbs": [""]}),
        ] {
            assert!(parse_config(&bad, "t").is_err(), "{bad}");
        }
    }
}
