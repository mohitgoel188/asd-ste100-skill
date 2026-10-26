//! `ste-lint` — Rust port of `scripts/ste-lint.py`. Same rules, same options,
//! same JSON. `rust/parity/parity.py` checks that both report the same findings.

mod compare;
mod config;
mod lint;
mod markdown;
mod text;

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use config::Config;
use text::{Finding, HARD, universal_newlines};

const PROG: &str = "ste-lint";
const RULE_IDS: &[&str] = &[
    "avoid-term", "dangling-conjunction", "dropped-literal", "dropped-negation",
    "long-sentence", "marketing-adjective", "modal-changed", "nominalization",
    "passive-voice", "phrasal-verb", "present-perfect", "semicolon", "synonym-rotation",
];
const USAGE: &str = "usage: ste-lint [-h] [--json] [--baseline N] [--disable RULES] \
[--config PATH | --no-config] [--compare ORIGINAL] [--selftest] [FILE ...]";

#[derive(Default)]
struct Args {
    paths: Vec<String>,
    json: bool,
    baseline: usize,
    disable: Vec<String>,
    config: Option<String>,
    no_config: bool,
    compare: Option<String>,
    selftest: bool,
}

/// A usage error: print it the way argparse does and exit 2.
fn usage_error(message: &str) -> ! {
    eprintln!("{USAGE}\n{PROG}: error: {message}");
    std::process::exit(2);
}

fn parse_rules(value: &str) -> Vec<String> {
    let rules: Vec<String> = value.split(',').filter(|r| !r.is_empty()).map(String::from).collect();
    let mut unknown: Vec<&str> =
        rules.iter().map(String::as_str).filter(|r| !RULE_IDS.contains(r)).collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        usage_error(&format!(
            "argument --disable: unknown rule(s): {}. Known rules: {}",
            unknown.join(", "),
            RULE_IDS.join(", ")
        ));
    }
    rules
}

fn parse_args(argv: &[String]) -> Args {
    let mut args = Args::default();
    let mut iter = argv.iter();
    let mut positional_only = false;
    while let Some(token) = iter.next() {
        if positional_only || !token.starts_with("--") {
            if !positional_only && token.starts_with('-') && token != "-" {
                if token == "-h" {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                usage_error(&format!("unrecognized arguments: {token}"));
            }
            args.paths.push(token.clone());
            continue;
        }
        if token == "--" {
            positional_only = true;
            continue;
        }
        let (name, inline) = match token.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (token.as_str(), None),
        };
        let mut value = || {
            inline.clone().or_else(|| iter.next().cloned()).unwrap_or_else(|| {
                usage_error(&format!("argument {name}: expected one argument"))
            })
        };
        match name {
            "--json" => args.json = true,
            "--no-config" => args.no_config = true,
            "--selftest" => args.selftest = true,
            "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--baseline" => {
                let v = value();
                args.baseline = v.parse().unwrap_or_else(|_| {
                    usage_error(&format!(
                        "argument --baseline: expected a non-negative integer, got '{v}'"
                    ))
                });
            }
            "--disable" => args.disable = parse_rules(&value()),
            "--config" => args.config = Some(value()),
            "--compare" => args.compare = Some(value()),
            _ => usage_error(&format!("unrecognized arguments: {token}")),
        }
    }
    if args.config.is_some() && args.no_config {
        usage_error("argument --no-config: not allowed with argument --config");
    }
    args
}

fn read(path: &str) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => universal_newlines(text),
        Err(e) => usage_error(&format!("cannot read {path}: {e}")),
    }
}

fn load_config(args: &Args) -> Config {
    if args.no_config {
        return Config::default();
    }
    let path = match &args.config {
        Some(p) => Some(Path::new(p).to_path_buf()),
        None => std::env::current_dir().ok().and_then(|cwd| config::find_config(&cwd)),
    };
    match path {
        None => Config::default(),
        Some(path) => config::load_config(&path).unwrap_or_else(|e| usage_error(&e)),
    }
}

/// Python's `round(x, 1)` printed with `repr`: one decimal, always.
fn rate(findings: usize, words: usize) -> String {
    if words == 0 {
        return "0.0".into();
    }
    format!("{:.1}", findings as f64 * 100.0 / words as f64)
}

fn report(findings: &[Finding], words: usize, json: bool, hard: usize, baseline: usize) {
    let per_100 = rate(findings.len(), words);
    if json {
        let value = serde_json::json!({
            "violations": findings,
            "count": findings.len(),
            "hard_count": hard,
            "baseline": baseline,
            "words": words,
            "per_100_words": per_100.parse::<f64>().unwrap(),
        });
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        return;
    }
    for f in findings {
        println!("{}:{}:{} {}: {} [{}]", f.file, f.line, f.col, f.rule, f.message, f.matched);
    }
    println!(
        "\n{} violations ({hard} hard, baseline {baseline}), {words} words, {per_100} per 100 words",
        findings.len()
    );
    println!("Hedges/modality (may, might, could) are never flagged: confidence is content.");
}

fn selftest() {
    let none = Config::default();
    let rules = |text: &str| -> Vec<String> {
        lint::lint(text, "<stdin>", &none).0.into_iter().map(|f| f.rule).collect()
    };
    let found = rules("The panel is removed; spin up the job. Perform an analysis of the \
                       seamless log. We have received the report.");
    for expected in ["semicolon", "phrasal-verb", "nominalization", "marketing-adjective",
                     "passive-voice", "present-perfect"] {
        assert!(found.iter().any(|r| r == expected), "{expected}");
    }
    assert!(rules("The request may have failed. It could be a timeout. The disk might have filled.")
        .is_empty());
    assert!(rules("The task couldn't have run. The task MAY NOT HAVE RUN.").is_empty());
    assert!(rules("```\nx = a; y = b\n```").is_empty());
    assert_eq!(rules("---\ntitle: a; b\n---\nUse a; b."), vec!["semicolon"]);
    let lost: Vec<String> = compare::compare("The job may fail on 3 nodes.", "The job fails.", "o")
        .into_iter()
        .map(|f| f.rule)
        .collect();
    assert_eq!(lost, vec!["modal-changed", "dropped-literal"]); // "may" is col 9, "3" col 20
    println!("selftest OK");
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);
    if args.selftest {
        selftest();
        return ExitCode::SUCCESS;
    }
    if args.compare.is_some() && args.paths.len() > 1 {
        usage_error("--compare takes one rewrite: give one FILE, or use stdin");
    }
    let config = load_config(&args);
    let texts: Vec<(String, String)> = if args.paths.is_empty() {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut text) {
            usage_error(&format!("cannot read stdin: {e}"));
        }
        vec![("<stdin>".into(), universal_newlines(text))]
    } else {
        args.paths.iter().map(|p| (p.clone(), read(p))).collect()
    };
    let (mut findings, mut words) = (Vec::new(), 0);
    for (name, text) in &texts {
        let (found, count) = lint::lint(text, name, &config);
        findings.extend(found);
        words += count;
    }
    if let Some(original) = &args.compare {
        findings.extend(compare::compare(&read(original), &texts[0].1, original));
    }
    findings.retain(|f| !args.disable.contains(&f.rule));
    let hard = findings.iter().filter(|f| f.level == HARD).count();
    report(&findings, words, args.json, hard, args.baseline);
    if hard > args.baseline { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
