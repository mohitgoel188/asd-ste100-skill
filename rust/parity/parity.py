"""Check that the Rust ste-lint reports exactly what scripts/ste-lint.py reports.

Corpus:
  1. every text the Python self-test passes to lint() or compare(), recorded by
     wrapping both functions while the self-test runs (the edge-case suite)
  2. every Markdown file given on the command line (real documents)

Both CLIs run on each input through stdin. The harness compares the parsed
JSON (findings, counts, words) and the exit code.

Usage:
    python3 -I rust/parity/parity.py [--binary PATH] [MARKDOWN_FILE ...]
"""

import argparse
import contextlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(ROOT, "scripts", "ste-lint.py")
DEFAULT_BINARY = os.path.join(ROOT, "rust", "target", "release", "ste-lint")


def _load_linter():
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    spec = importlib.util.spec_from_file_location("ste_lint", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def record_selftest_inputs():
    """Run the Python self-test and capture each lint() and compare() input."""
    linter = _load_linter()
    lint_cases, compare_cases = [], []
    real_lint, real_compare = linter.lint, linter.ste_compare.compare

    def lint(text, filename="<stdin>", config=None):
        lint_cases.append((text, config))
        return real_lint(text, filename, config)

    def compare(original, rewrite, filename="<original>"):
        compare_cases.append((original, rewrite))
        return real_compare(original, rewrite, filename)

    linter.lint, linter.ste_compare.compare = lint, compare
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        linter.selftest()
    return lint_cases, compare_cases


def _config_json(config):
    if config is None:
        return None
    return {
        "phrasal_verbs": list(config["phrasal_verbs"]),
        "marketing_adjectives": list(config["marketing_adjectives"]),
        "synonym_groups": [list(g) for g in config["synonym_groups"]],
        "avoid_terms": dict(config["avoid_terms"]),
    }


def _run(command, stdin_text):
    result = subprocess.run(
        command, input=stdin_text.encode("utf-8"), capture_output=True, check=False
    )
    out = result.stdout.decode("utf-8")
    try:
        return result.returncode, json.loads(out)
    except json.JSONDecodeError:
        return result.returncode, {"raw": out, "stderr": result.stderr.decode()}


def _normalized(report):
    report = dict(report)
    report.pop("per_100_words", None)  # derived from count and words
    return report


def check(case, binary, tmp):
    """Return None when both CLIs agree, else a description of the mismatch."""
    kind, index, text, extra = case
    flags = ["--json"]
    if kind == "compare":
        original = os.path.join(tmp, f"original-{index}.md")
        with open(original, "w", encoding="utf-8", newline="") as fh:
            fh.write(extra)
        flags += ["--no-config", "--compare", original]
    elif extra is not None:
        config = os.path.join(tmp, f"config-{index}.json")
        with open(config, "w", encoding="utf-8") as fh:
            json.dump(extra, fh)
        flags += ["--config", config]
    else:
        flags.append("--no-config")
    py = _run([sys.executable, "-I", SCRIPT, *flags], text)
    rs = _run([binary, *flags], text)
    if py[0] == rs[0] and _normalized(py[1]) == _normalized(rs[1]):
        return None
    return {"case": f"{kind}#{index}", "input": text[:300], "python": py, "rust": rs}


def build_corpus(paths):
    lint_cases, compare_cases = record_selftest_inputs()
    corpus = [("lint", i, text, _config_json(config)) for i, (text, config) in enumerate(lint_cases)]
    corpus += [("compare", i, rewrite, original)
               for i, (original, rewrite) in enumerate(compare_cases)]
    for i, path in enumerate(paths):
        with open(path, encoding="utf-8", errors="replace", newline="") as fh:
            corpus.append(("file", i, fh.read(), None))
    return corpus


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("files", nargs="*")
    parser.add_argument("--binary", default=DEFAULT_BINARY)
    args = parser.parse_args()
    corpus = build_corpus(args.files)
    with tempfile.TemporaryDirectory() as tmp, ThreadPoolExecutor(max_workers=8) as pool:
        mismatches = [m for m in pool.map(lambda c: check(c, args.binary, tmp), corpus) if m]
    kinds = {k: sum(1 for c in corpus if c[0] == k) for k in ("lint", "compare", "file")}
    print(f"{len(corpus)} cases ({kinds['lint']} self-test lint, {kinds['compare']} compare, "
          f"{kinds['file']} files): {len(mismatches)} mismatches")
    for mismatch in mismatches[:10]:
        print(json.dumps(mismatch, indent=1, ensure_ascii=False)[:3000])
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
