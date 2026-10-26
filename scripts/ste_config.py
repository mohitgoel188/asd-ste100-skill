"""Project vocabulary for ste-lint.py.

ASD's dictionary cannot ship with this skill, so the lexical rules stay
advisory. A project can still lock its own vocabulary down: a JSON file
names extra phrasal verbs, marketing adjectives, synonym groups, and terms
to avoid. Those rules are then checkable for that project.

File shape (every key is optional):

    {
      "phrasal_verbs": ["set up", "look into"],
      "marketing_adjectives": ["best-in-class"],
      "synonym_groups": [["centre", "hub", "warehouse"]],
      "avoid_terms": {"utilise": "use", "DC": "centre"}
    }

ste-lint.py reads the file named by --config. Without --config it looks for
.ste-lint.json in the current directory and each parent directory.
"""

import json
import os
import re

CONFIG_NAME = ".ste-lint.json"
KNOWN_KEYS = {"phrasal_verbs", "marketing_adjectives", "synonym_groups", "avoid_terms"}
EMPTY_CONFIG = {
    "phrasal_verbs": (),
    "marketing_adjectives": (),
    "synonym_groups": (),
    "avoid_terms": {},
}
# Inflections for the verb part of a phrasal verb: "look into" also matches
# "looks into", "looked into", "looking into". A doubled final consonant
# covers "setting up" and "stopped by".
VERB_SUFFIX = r"(?:s|es|ed|d|ing|{last}(?:ed|ing))?"


class ConfigError(ValueError):
    """The config file exists but cannot be used."""


def find_config(start):
    """Return the nearest .ste-lint.json at or above start, or None."""
    directory = os.path.abspath(start)
    while True:
        candidate = os.path.join(directory, CONFIG_NAME)
        if os.path.isfile(candidate):
            return candidate
        parent = os.path.dirname(directory)
        if parent == directory:
            return None
        directory = parent


def _string_list(data, key):
    value = data.get(key, [])
    if not isinstance(value, list) or not all(
        isinstance(v, str) and v.strip() for v in value
    ):
        raise ConfigError(f"'{key}' must be a list of non-empty strings")
    return tuple(v.strip() for v in value)


def parse_config(data, source="<config>"):
    """Validate a decoded JSON object and return a normalized config dict."""
    if not isinstance(data, dict):
        raise ConfigError(f"{source}: top level must be a JSON object")
    unknown = set(data) - KNOWN_KEYS
    if unknown:
        raise ConfigError(
            f"{source}: unknown key(s) {', '.join(sorted(unknown))}. "
            f"Known keys: {', '.join(sorted(KNOWN_KEYS))}"
        )
    try:
        groups = data.get("synonym_groups", [])
        if not isinstance(groups, list):
            raise ConfigError("'synonym_groups' must be a list of lists")
        synonym_groups = tuple(
            _string_list({"group": group}, "group") for group in groups
        )
        if any(len(group) < 2 for group in synonym_groups):
            raise ConfigError("each synonym group needs at least two terms")
        avoid = data.get("avoid_terms", {})
        if not isinstance(avoid, dict) or not all(
            isinstance(k, str) and k.strip() and isinstance(v, str)
            for k, v in avoid.items()
        ):
            raise ConfigError("'avoid_terms' must map each term to its replacement")
        return {
            "phrasal_verbs": _string_list(data, "phrasal_verbs"),
            "marketing_adjectives": _string_list(data, "marketing_adjectives"),
            "synonym_groups": synonym_groups,
            "avoid_terms": {k.strip(): v.strip() for k, v in avoid.items()},
        }
    except ConfigError as error:
        raise ConfigError(f"{source}: {error}") from None


def load_config(path):
    """Read and validate a config file. Raise ConfigError on any problem."""
    try:
        with open(path, encoding="utf-8") as fh:
            data = json.load(fh)
    except OSError as error:
        raise ConfigError(f"cannot read {path}: {error.strerror}") from None
    except json.JSONDecodeError as error:
        raise ConfigError(
            f"{path}: invalid JSON ({error.msg}, line {error.lineno})"
        ) from None
    return parse_config(data, source=path)


def _phrase_pattern(phrase):
    """Match a multi-word phrase across any whitespace, case-insensitively."""
    words = [re.escape(word) for word in phrase.split()]
    return r"\s+".join(words)


def phrasal_pattern(phrases):
    """Return one regex for configured phrasal verbs, or None."""
    if not phrases:
        return None
    alternatives = []
    for phrase in phrases:
        verb, *rest = phrase.split()
        tail = (r"\s+" + _phrase_pattern(" ".join(rest))) if rest else ""
        suffix = VERB_SUFFIX.format(last=re.escape(verb[-1]))
        alternatives.append(re.escape(verb) + suffix + tail)
    return re.compile(r"\b(?:" + "|".join(alternatives) + r")\b", re.IGNORECASE)


def word_list_pattern(words):
    """Return one regex that matches any of the given words or phrases, or None."""
    if not words:
        return None
    return re.compile(
        r"\b(?:" + "|".join(_phrase_pattern(w) for w in words) + r")\b", re.IGNORECASE
    )


def avoid_patterns(avoid_terms):
    """Return (term, replacement, regex) for each term to avoid.

    A term written with a capital letter (an acronym such as "DC") matches
    case-sensitively, so the ordinary word "dc" in a path stays clean.
    """
    patterns = []
    for term, replacement in avoid_terms.items():
        flags = 0 if any(c.isupper() for c in term) else re.IGNORECASE
        patterns.append(
            (
                term,
                replacement,
                re.compile(r"\b" + _phrase_pattern(term) + r"\b", flags),
            )
        )
    return patterns


def selftest():
    config = parse_config(
        {
            "phrasal_verbs": ["look into", "set up"],
            "marketing_adjectives": ["best-in-class"],
            "synonym_groups": [["centre", "hub"]],
            "avoid_terms": {"DC": "centre", "utilise": "use"},
        }
    )
    pattern = phrasal_pattern(config["phrasal_verbs"])
    assert pattern.search("We looked  into the log.")
    assert pattern.search("Setting up the job")
    assert not pattern.search("The setup is done.")
    assert word_list_pattern(config["marketing_adjectives"]).search(
        "A Best-in-class tool"
    )
    avoid = {term: regex for term, _, regex in avoid_patterns(config["avoid_terms"])}
    assert avoid["DC"].search("the DC stock")
    assert not avoid["DC"].search("the dc stock")
    assert avoid["utilise"].search("Utilise it")
    for bad in (
        [],
        {"typo": []},
        {"phrasal_verbs": "set up"},
        {"synonym_groups": [["only"]]},
        {"avoid_terms": ["x"]},
        {"phrasal_verbs": [""]},
    ):
        try:
            parse_config(bad)
        except ConfigError:
            continue
        raise AssertionError(bad)
    assert find_config(os.path.sep) in (None, os.path.join(os.path.sep, CONFIG_NAME))
