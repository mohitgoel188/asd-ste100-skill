"""Check that a rewrite kept the facts and the confidence of its original.

The linter checks the form of one text. It cannot see what a rewrite lost.
This module compares two texts and reports what the original has and the
rewrite does not:

- literals: code spans, fenced code lines, URLs, ticket keys, identifiers
  (dotted, slashed, or snake_case names), and numbers
- negations: not, no, never, and the others in NEGATIONS, counted
- modals and hedges: may, must, likely, and the others in MODALS, counted
  word by word, so "should" changed to "must" shows as a lost "should"

It reports losses only. A rewrite may add a word such as "not" to split one
sentence into two, so additions are left to the reader.
"""

import re
from collections import Counter

CODE_FENCE = re.compile(r"^\s*(```|~~~)")
CODE_SPAN = re.compile(r"`([^`\n]+)`")
URL = re.compile(r"https?://[^\s<>()`\"']+")
URL_TRAILING = ".,:;!?"
TICKET = re.compile(r"\b[A-Z][A-Z0-9]+-[0-9]+\b")
# A name that joins word characters with ".", "/" or "_": apps/common, v6.3,
# get_list_display. Single letters joined by dots ("e.g") are abbreviations.
IDENTIFIER = re.compile(r"\b\w+(?:[./]\w+)+\b|\b\w*_\w+\b")
NUMBER = re.compile(r"\b[0-9]+(?:[.,:][0-9]+)*%?")
NEGATIONS = {
    "not",
    "no",
    "never",
    "none",
    "nothing",
    "nobody",
    "without",
    "neither",
    "nor",
}
MODALS = {
    "may",
    "might",
    "could",
    "should",
    "must",
    "would",
    "can",
    "will",
    "shall",
    "likely",
    "unlikely",
    "probably",
    "possibly",
    "perhaps",
    "usually",
    "sometimes",
    "often",
    "rarely",
}
WORD = re.compile(r"[A-Za-z]+(?:['’][A-Za-z]+)?")
# "can't" counts as one "can" and one negation, "won't" as one "will".
CONTRACTION_BASE = {"can't": "can", "won't": "will", "shan't": "shall"}


def _mask(text, pattern):
    return pattern.sub(lambda m: " " * len(m.group(0)), text)


def _split_fences(text):
    """Return (prose_lines, fence_lines) as lists of (lineno, line)."""
    prose, fenced, in_fence = [], [], False
    for lineno, line in enumerate(text.splitlines(), 1):
        if CODE_FENCE.match(line):
            in_fence = not in_fence
            prose.append((lineno, ""))
            continue
        (fenced if in_fence else prose).append((lineno, line))
    return prose, fenced


def _clean_url(url):
    return url.rstrip(URL_TRAILING)


def _is_abbreviation(token):
    return all(len(part) == 1 for part in re.split(r"[./]", token))


def _literals(prose):
    """Yield (kind, literal, lineno, col) for each literal in prose lines."""
    for lineno, line in prose:
        for m in CODE_SPAN.finditer(line):
            yield "code span", m.group(1).strip(), lineno, m.start() + 1
        line = _mask(line, CODE_SPAN)
        for m in URL.finditer(line):
            yield "URL", _clean_url(m.group(0)), lineno, m.start() + 1
        line = _mask(line, URL)
        for m in TICKET.finditer(line):
            yield "ticket key", m.group(0), lineno, m.start() + 1
        for m in IDENTIFIER.finditer(line):
            if not _is_abbreviation(m.group(0)):
                yield "identifier", m.group(0), lineno, m.start() + 1
        for m in NUMBER.finditer(_mask(line, IDENTIFIER)):
            yield "number", m.group(0), lineno, m.start() + 1


def _token_set(text):
    """Every ticket key, identifier and number in text, code spans included."""
    tokens = set()
    for pattern in (TICKET, IDENTIFIER, NUMBER):
        tokens.update(m.group(0) for m in pattern.finditer(text))
    tokens.update(_clean_url(m.group(0)) for m in URL.finditer(text))
    return tokens


def _word_counts(prose):
    """Count negations and modals, with the first position of each word."""
    negations, modals, first = Counter(), Counter(), {}
    for lineno, line in prose:
        for m in WORD.finditer(_mask(line, CODE_SPAN)):
            word = m.group(0).lower().replace("’", "'")
            base = word
            if word == "cannot":
                word = "can't"
            if word.endswith("n't"):
                negations["not"] += 1
                first.setdefault("not", (lineno, m.start() + 1))
                base = CONTRACTION_BASE.get(word, word[:-3])
            if base in NEGATIONS:
                negations[base] += 1
                first.setdefault(base, (lineno, m.start() + 1))
            if base in MODALS:
                modals[base] += 1
                first.setdefault(base, (lineno, m.start() + 1))
    return negations, modals, first


def _finding(filename, lineno, col, rule, match, message):
    return {
        "file": filename,
        "line": lineno,
        "col": col,
        "rule": rule,
        "level": "advisory-free",
        "match": match,
        "message": message,
    }


def compare(original, rewrite, filename="<original>"):
    """Return findings for what original has and rewrite lost."""
    findings = []
    prose, fenced = _split_fences(original)
    _, rewrite_fenced = _split_fences(rewrite)
    rewrite_tokens = _token_set(rewrite)
    reported = set()
    for kind, literal, lineno, col in _literals(prose):
        if not literal or literal in reported:
            continue
        kept = (
            literal in rewrite
            if kind in ("code span", "URL")
            else literal in rewrite_tokens
        )
        if not kept:
            reported.add(literal)
            findings.append(
                _finding(
                    filename,
                    lineno,
                    col,
                    "dropped-literal",
                    literal,
                    f"The rewrite lost this {kind}. Keep it unless it was wrong.",
                )
            )
    rewrite_code = {line.strip() for _, line in rewrite_fenced}
    for lineno, line in fenced:
        if line.strip() and line.strip() not in rewrite_code:
            findings.append(
                _finding(
                    filename,
                    lineno,
                    1,
                    "dropped-literal",
                    line.strip(),
                    "The rewrite lost or changed this line of fenced code.",
                )
            )

    rewrite_prose, _ = _split_fences(rewrite)
    neg_a, mod_a, first = _word_counts(prose)
    neg_b, mod_b, _ = _word_counts(rewrite_prose)
    if sum(neg_a.values()) > sum(neg_b.values()):
        lost = sum(neg_a.values()) - sum(neg_b.values())
        word = next(iter(neg_a))
        findings.append(
            _finding(
                filename,
                *first[word],
                "dropped-negation",
                word,
                f"The original has {sum(neg_a.values())} negations and the rewrite "
                f"has {sum(neg_b.values())}. Check that {lost} negation(s) did not "
                "turn into the opposite claim.",
            )
        )
    for word, count in sorted(mod_a.items()):
        if mod_b[word] < count:
            findings.append(
                _finding(
                    filename,
                    *first[word],
                    "modal-changed",
                    word,
                    f"'{word}' appears {count} time(s) in the original and "
                    f"{mod_b[word]} in the rewrite. A changed modal changes the claim.",
                )
            )
    findings.sort(key=lambda f: (f["line"], f["col"]))
    return findings


def selftest():
    original = (
        "The job may have failed on 3 of 12 nodes. Do not rerun it.\n"
        "See DEL-8524 and `apps/common/tests` at https://example.com/x.\n"
        "Set get_list_display, e.g. in v6.3.\n"
        "```\nrun --all\n```\n"
    )
    assert compare(original, original) == []
    rules = {
        (f["rule"], f["match"])
        for f in compare(
            "The job may fail. Do not rerun it.", "The job fails. Rerun it."
        )
    }
    assert ("modal-changed", "may") in rules, rules
    assert ("dropped-negation", "not") in rules, rules
    findings = compare(original, "The job failed on nodes. Rerun it. See the docs.")
    dropped = {f["match"] for f in findings if f["rule"] == "dropped-literal"}
    for literal in (
        "3",
        "12",
        "DEL-8524",
        "apps/common/tests",
        "https://example.com/x",
        "get_list_display",
        "v6.3",
        "run --all",
    ):
        assert literal in dropped, (literal, dropped)
    assert "e.g" not in dropped, dropped
    # a literal moved out of backticks, or into them, still counts as kept
    assert compare("Edit `config.py` now.", "Edit config.py now.") == []
    assert compare("Edit config.py now.", "Edit `config.py` now.") == []
    # contractions count as negations and as their modal base
    assert compare("It can't run.", "It cannot run.") == []
    rules = {f["rule"] for f in compare("It won't run.", "It runs.")}
    assert rules == {"dropped-negation", "modal-changed"}, rules
    # "should" to "must" loses a "should"
    findings = compare("You should restart.", "You must restart.")
    assert [(f["rule"], f["match"]) for f in findings] == [("modal-changed", "should")]
    # added words are not reported
    assert compare("Restart the job.", "Do not wait. Restart the job.") == []
