#!/usr/bin/env python3
"""Share of the passages whose tokens change under `lexical.identifiers = "v1"`.

`crible rebuild-lexical` publishes no count, and comparing two token streams over
9 600 105 passages would mean re-analysing the whole corpus twice. This harness
counts something equivalent and cheaper: a passage's token stream changes under
`"v1"` exactly when its text holds at least one identifier of the forms of
ADR-0148, decision 3, because the canonical token is *added* and never put in
place of a 1.0 token (decision 1).

The predicate is a faithful transcription of `canonical_tokens`, `article_at`,
`number_at`, `starts_a_word` and `ends_a_word` of
`crates/crible-store/src/lexical/analyzer.rs`, including the boundary rule (a
character that is not alphanumeric) and the « already one whole token » case of
form A (`has_separator` false, e.g. `C100123`). It is checked against the strings
the unit tests of that file pin, before any number is reported.

Usage:

    python3 tools/bench/t14/retokenized.py --data data/t14-mes11/data \
        --out data/t14-mes11/raw/retokenized.json

One index per call, so no call runs long: `--index` may be repeated.
"""

import argparse
import json
import os
import sqlite3
import sys

INDEXES = ["dole", "sarde", "titres", "jorf", "articles", "decisions"]

# Strings whose canonical token the unit tests of the analyzer pin, with the
# expected verdict. `True` means: the chain adds a canonical token.
CASES = {
    "L. 1132-1": True,
    "L1132-1": True,
    "R. 233-45": True,
    "R233-45": True,
    "10-13.625": True,
    "81-70.489": True,
    "92-12345": True,
    "15-02-2011": True,
    "1240": False,
    "SANP0422530D": False,
    "ECLI:FR:CCASS:2011:C100123": False,
    "1.5": False,
    "1 240": False,
    "N°1234": False,
    "qu'il": False,
    "dommages-intérêts": False,
}


def digit(character):
    return "0" <= character <= "9"


def starts_a_word(text, offset):
    return offset == 0 or not text[offset - 1].isalnum()


def ends_a_word(text, offset):
    return offset == len(text) or not text[offset].isalnum()


def article_at(text, start):
    """Form A of ADR-0148: letter, optional `.`, optional space, digits, groups."""
    if not starts_a_word(text, start):
        return None
    cursor = start + 1
    size = len(text)
    if cursor < size and text[cursor] == ".":
        cursor += 1
    if cursor < size and text[cursor].isspace():
        cursor += 1
    digits_start = cursor
    while cursor < size and digit(text[cursor]):
        cursor += 1
    if cursor == digits_start:
        return None
    while cursor < size and text[cursor] == "-":
        group_end = cursor + 1
        while group_end < size and digit(text[group_end]):
            group_end += 1
        if group_end == cursor + 1:
            break
        cursor = group_end
    if not ends_a_word(text, cursor):
        return None
    if not any(ch in ".-" or ch.isspace() for ch in text[start:cursor]):
        return None
    return cursor


def number_at(text, start):
    """Form B of ADR-0148: two digits or more, then at least one `.`/`-` group."""
    if not starts_a_word(text, start):
        return None
    size = len(text)
    cursor = start
    while cursor < size and digit(text[cursor]):
        cursor += 1
    if cursor - start < 2:
        return None
    groups = 0
    while cursor < size and text[cursor] in ".-":
        group_end = cursor + 1
        while group_end < size and digit(text[group_end]):
            group_end += 1
        if group_end == cursor + 1:
            break
        cursor = group_end
        groups += 1
    if groups == 0 or not ends_a_word(text, cursor):
        return None
    return cursor


def adds_a_canonical(text):
    """Whether the chain of `"v1"` appends at least one canonical token."""
    cursor = 0
    size = len(text)
    while cursor < size:
        character = text[cursor]
        matched = None
        if character.isascii() and character.isalpha():
            matched = article_at(text, cursor)
        elif character.isascii() and digit(character):
            matched = number_at(text, cursor)
        if matched is not None:
            return True
        cursor += 1
    return False


def check_predicate():
    for text, expected in CASES.items():
        found = adds_a_canonical(text)
        if found != expected:
            raise SystemExit(
                f"the predicate disagrees with the unit tests on {text!r}: "
                f"{found} instead of {expected}"
            )


def count(index, data):
    path = os.path.join(data, "indexes", index, "epochs", "1", "records.sqlite")
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    connection.text_factory = bytes
    total = 0
    changed = 0
    cursor = connection.execute("SELECT text FROM passages WHERE dead_since_gen IS NULL")
    while True:
        rows = cursor.fetchmany(20000)
        if not rows:
            break
        for (blob,) in rows:
            total += 1
            text = blob.decode("utf-8", "replace") if isinstance(blob, bytes) else blob
            if adds_a_canonical(text):
                changed += 1
    connection.close()
    return {"passages": total, "changed": changed, "share": changed / total if total else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--index", action="append", dest="indexes")
    args = parser.parse_args()

    check_predicate()
    indexes = args.indexes or INDEXES
    report = {}
    if os.path.exists(args.out):
        report = json.load(open(args.out, encoding="utf-8"))
    for index in indexes:
        report[index] = count(index, args.data)
        print(index, json.dumps(report[index]))
        with open(args.out, "w", encoding="utf-8") as handle:
            json.dump(report, handle, ensure_ascii=False, indent=1)
            handle.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
