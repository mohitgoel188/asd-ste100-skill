# ste-lint (Rust port)

A port of `scripts/ste-lint.py`. It has the same rules, the same options and the same JSON output. `scripts/ste-lint.py` stays the reference implementation, and the skill runs the Python linter.

## Build

```bash
cargo build --release --manifest-path rust/Cargo.toml
rust/target/release/ste-lint --selftest
```

## Parity

`parity/parity.py` runs both linters on the same inputs and compares the parsed JSON and the exit code. The inputs are every text that the Python self-test lints or compares, plus any Markdown files you name:

```bash
python3 -I rust/parity/parity.py path/to/*.md
```

Run it after every change to either linter. A rule change goes into the Python linter first, then into the port.

## Lookaround

The `regex` crate has no lookahead or lookbehind. Three Python patterns use them:

| Python pattern | Port |
|---|---|
| passive-voice `(?!\s+(?:to\|for\|by)\s+\w+ing)` | `Guard::Passive` checks the text after the match |
| present-perfect `(?<!\bmay )` and five more | `Guard::PresentPerfect` checks the text before the match |
| HTML block tag `(?=[\s/>])[^>]*>` | rewritten as `(?:>\|[\s/][^>]*>)`, which matches the same text |

A match that a guard rejects restarts the search one code point later. Python's `finditer` does the same after a failed lookaround.
